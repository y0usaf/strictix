//! File rules: semantic lints that run once per file against the lazy
//! [SemanticModel].
//!
//! Node rules in [super::node_rules] inspect single nodes; these rules
//! answer questions that need the whole file's meaning: is this let
//! binding ever used, does this lambda parameter shadow an outer one,
//! is this with scope doing any work. Every rule here is a unit struct
//! implementing [Rule] with only [Rule::check_file] overridden — the
//! registry ([super::all_rules]) declares them all the same way.

use std::collections::HashSet;
use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, ScopeId, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, BinExpr, Binding, Expr, Formals, LambdaExpr, LambdaParam, LetExpr,
    RecAttrsetExpr, SelectExpr, SyntaxKind, SyntaxNode, TextRange, WithExpr,
};
/// The innermost [Binding] node whose range contains `name_range`, if
/// any. Bindings nest when a value contains a `let` or attrset, so a
/// range scan must pick the smallest containing node — the direct
/// parent of the name token — not the first one found.
fn containing_binding<'a>(root: &'a SyntaxNode, name_range: TextRange) -> Option<Binding<'a>> {
    let start = name_range.start();
    root.descendants()
        .filter(|n| n.kind() == SyntaxKind::Binding && n.range().contains(start))
        .min_by_key(|n| n.range().end() - n.range().start())
        .and_then(Binding::cast)
}

/// Whether `name_range` sits inside a Formals node. A lambda's bare
/// ident parameter is not inside one; its formal parameters are.
fn inside_formals(root: &SyntaxNode, name_range: TextRange) -> bool {
    let start = name_range.start();
    root.descendants()
        .any(|n| n.kind() == SyntaxKind::Formals && n.range().contains(start))
}

/// The innermost [Formals] node whose range contains `name_range`.
/// Formals can nest when a formal's default is itself a lambda, so the
/// smallest containing node is the one that actually owns the
/// parameter.
fn containing_formals<'a>(root: &'a SyntaxNode, name_range: TextRange) -> Option<Formals<'a>> {
    let start = name_range.start();
    root.descendants()
        .filter(|n| n.kind() == SyntaxKind::Formals && n.range().contains(start))
        .min_by_key(|n| n.range().end() - n.range().start())
        .and_then(Formals::cast)
}

/// Maximum allowed cyclomatic complexity for one lambda.
///
/// Complexity starts at one and increases for each independent decision
/// path. Nested lambdas are measured separately, so an inner function does
/// not inflate the score of its enclosing function.
pub const MAX_CYCLOMATIC_COMPLEXITY: usize = 5;

/// Counts decision points in one lambda, excluding nested lambdas.
fn lambda_complexity(lambda: LambdaExpr<'_>) -> usize {
    fn walk(node: &SyntaxNode, root_start: u32, score: &mut usize) {
        if node.kind() == SyntaxKind::LambdaExpr && node.range().start() != root_start {
            return;
        }
        match node.kind() {
            SyntaxKind::IfExpr | SyntaxKind::AssertExpr | SyntaxKind::HasAttrExpr => *score += 1,
            SyntaxKind::BinExpr => {
                if BinExpr::cast(node)
                    .and_then(|expr| expr.op())
                    .is_some_and(|op| matches!(op, SyntaxKind::AndAnd | SyntaxKind::OrOr))
                {
                    *score += 1;
                }
            }
            SyntaxKind::SelectExpr => {
                if SelectExpr::cast(node).is_some_and(|expr| expr.default().is_some()) {
                    *score += 1;
                }
            }
            _ => {}
        }
        for child in node.child_nodes() {
            walk(child, root_start, score);
        }
    }

    let mut decisions = 0;
    let root = lambda.syntax();
    for child in root.child_nodes() {
        walk(child, root.range().start(), &mut decisions);
    }
    decisions + 1
}

/// Flags lambdas whose independent decision paths exceed the maintainability threshold.
pub struct CyclomaticComplexity;

impl Rule for CyclomaticComplexity {
    fn code(&self) -> &'static str {
        "cyclomatic-complexity"
    }

    fn name(&self) -> &'static str {
        "Cyclomatic complexity"
    }

    fn description(&self) -> &'static str {
        "Flags lambdas with more than 5 independent decision paths. Each if, assert, boolean short-circuit, attribute test, and `or` fallback adds one path. Nested lambdas are measured separately."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let Some(lambda) = LambdaExpr::cast(node) else {
                continue;
            };
            let complexity = lambda_complexity(lambda);
            if complexity > MAX_CYCLOMATIC_COMPLEXITY {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "lambda has cyclomatic complexity {complexity}; maximum is {MAX_CYCLOMATIC_COMPLEXITY}"
                    ),
                    lambda.syntax().content_range(),
                ));
            }
        }
    }
}

/// Flags `let` bindings that are never referenced.
///
/// `let x = 1; in 2` — `x` is dead weight. The suggested fix deletes
/// the whole binding statement.
pub struct UnusedLetBinding;

impl Rule for UnusedLetBinding {
    fn code(&self) -> &'static str {
        "unused-let-binding"
    }

    fn name(&self) -> &'static str {
        "Unused let binding"
    }

    fn description(&self) -> &'static str {
        "Flags let bindings that are never referenced. Dead bindings are misleading and can be removed."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for binding in model.bindings() {
            if binding.kind != BindingKind::LetBinding || !binding.references.is_empty() {
                continue;
            }
            let name = binding.name.text(model.source());
            let range = binding.name.range();
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!("binding '{name}' is never used"),
                range,
            );
            if let Some(binding_node) = containing_binding(model.root(), range) {
                diag =
                    diag.with_fix(Fix::new("remove unused binding").edit(binding_node.range(), ""));
            }
            diags.push(diag);
        }
    }
}

/// Byte ranges of the two bare params in an overlay lambda
/// (`a: b: { ... }`). The Nixpkgs overlay convention is a bare-param
/// lambda whose body is another bare-param lambda whose body is an
/// attrset; the params are a fixed positional interface, not
/// accidentally-unused names, so UnusedLambdaParam must skip them.
fn overlay_param_ranges(root: &SyntaxNode) -> HashSet<u32> {
    let mut skip = HashSet::new();
    for node in root.descendants() {
        if node.kind() != SyntaxKind::LambdaExpr {
            continue;
        }
        let Some(outer) = LambdaExpr::cast(node) else {
            continue;
        };
        let LambdaParam::Ident(outer_param) = outer.param() else {
            continue;
        };
        let Some(Expr::Lambda(inner)) = outer.body() else {
            continue;
        };
        let LambdaParam::Ident(inner_param) = inner.param() else {
            continue;
        };
        if !matches!(
            inner.body(),
            Some(Expr::Attrset(_)) | Some(Expr::RecAttrset(_))
        ) {
            continue;
        }
        skip.insert(outer_param.range().start());
        skip.insert(inner_param.range().start());
    }
    skip
}

/// Flags bare lambda parameters that are never used.
///
/// `x: 1` — the parameter `x` is never referenced in the body. Formal
/// parameters (`{ a, b }: ...`) are handled by [UnusedFormal] because
/// they follow a different rule (the ellipsis changes what counts).
pub struct UnusedLambdaParam;

impl Rule for UnusedLambdaParam {
    fn code(&self) -> &'static str {
        "unused-lambda-param"
    }

    fn name(&self) -> &'static str {
        "Unused lambda parameter"
    }

    fn description(&self) -> &'static str {
        "Flags bare lambda parameters that are never used in the body. An unused parameter is usually a mistake; a leading underscore name opts out."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let overlay_params = overlay_param_ranges(model.root());
        for binding in model.bindings() {
            if binding.kind != BindingKind::LambdaParam {
                continue;
            }
            let range = binding.name.range();
            // Formal parameters belong to UnusedFormal; this rule only
            // covers the bare `x: body` form.
            if inside_formals(model.root(), range) {
                continue;
            }
            if overlay_params.contains(&range.start()) {
                continue;
            }
            if !binding.references.is_empty() {
                continue;
            }
            let name = binding.name.text(model.source());
            if name.starts_with('_') {
                continue;
            }
            // Fix: rename to the leading-underscore form. The parameter
            // has zero references, so renaming just its token cannot
            // break a use; the `_` prefix is this rule's own documented
            // opt-out, so the fix makes the intent explicit instead of
            // deleting an interface.
            let range = binding.name.range();
            let fix = Fix::new("rename to _-prefixed").edit(range, format!("_{name}"));
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("parameter '{name}' is never used"),
                    range,
                )
                .with_fix(fix),
            );
        }
    }
}

/// Flags unused formal parameters of lambdas without an ellipsis.
///
/// `{ a, b }: b` — `a` is declared but never used. With an ellipsis
/// (`{ a, b, ... }: b`) the extra formals are an intentional interface,
/// so only `_`-prefixed names are exempt otherwise.
pub struct UnusedFormal;

/// Flags inherited names that are never referenced by the scope that can
/// still see them. A plain attrset's inherit defines one of the attrset's
/// own output fields — the attribute definition is the use, and the field
/// is consumed outside this scope — so a dead inherit can only live in a
/// `rec` attrset or a let-binding block, where the name stays in scope.
pub struct UnusedInherit;

impl Rule for UnusedInherit {
    fn code(&self) -> &'static str {
        "unused-inherit"
    }
    fn name(&self) -> &'static str {
        "Unused inherit"
    }
    fn description(&self) -> &'static str {
        "Flags inherited names in rec attrsets and let bindings that are never used in that scope."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            match node.kind() {
                SyntaxKind::LetExpr => {
                    if let Some(let_expr) = LetExpr::cast(node) {
                        if let Some(bindings) = let_expr.bindings() {
                            check_inherit_scope(bindings.items(), model, diags);
                        }
                    }
                }
                SyntaxKind::RecAttrsetExpr => {
                    if let Some(attrset) = RecAttrsetExpr::cast(node).and_then(|r| r.attrset()) {
                        check_inherit_scope(attrset.items(), model, diags);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Checks the inherits of one scope: a let-binding block or a `rec`
/// attrset body. The inherited name is dead when nothing else in the scope
/// references it; the name token itself is not a reference.
fn check_inherit_scope<'a>(
    items: impl Iterator<Item = AttrItem<'a>>,
    model: &SemanticModel,
    diags: &mut Vec<Diagnostic>,
) {
    for item in items {
        let AttrItem::Inherit(inherit) = item else {
            continue;
        };
        for name_token in inherit.names() {
            let Some(binding) = model
                .bindings()
                .iter()
                .find(|binding| binding.name.range() == name_token.range())
            else {
                continue;
            };
            let used_elsewhere = binding
                .references
                .iter()
                .any(|range| *range != name_token.range());
            if used_elsewhere {
                continue;
            }
            let name = name_token.text(model.source());
            if name.starts_with('_') {
                continue;
            }
            diags.push(Diagnostic::new(
                "unused-inherit",
                Severity::Warning,
                format!("inherited name '{name}' is never used"),
                name_token.range(),
            ));
        }
    }
}

/// Flags a formal parameter that hides a visible outer binding.
pub struct ShadowedFormal;

impl Rule for ShadowedFormal {
    fn code(&self) -> &'static str {
        "shadowed-formal"
    }
    fn name(&self) -> &'static str {
        "Shadowed formal parameter"
    }
    fn description(&self) -> &'static str {
        "Flags a formal parameter whose name shadows an outer binding."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for binding in model.bindings() {
            if binding.kind != BindingKind::LambdaParam
                || !inside_formals(model.root(), binding.name.range())
            {
                continue;
            }
            let name = binding.name.text(model.source());
            if name.starts_with('_') || model.outer_shadow(binding).is_none() {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("formal parameter '{name}' shadows an outer binding"),
                binding.name.range(),
            ));
        }
    }
}

impl Rule for UnusedFormal {
    fn code(&self) -> &'static str {
        "unused-formal"
    }

    fn name(&self) -> &'static str {
        "Unused formal parameter"
    }

    fn description(&self) -> &'static str {
        "Flags formal parameters that are never used when the formals have no ellipsis. An ellipsis signals an open argument set, so unused names there are tolerated."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for binding in model.bindings() {
            if binding.kind != BindingKind::LambdaParam {
                continue;
            }
            let range = binding.name.range();
            let Some(formals) = containing_formals(model.root(), range) else {
                continue; // bare ident param — UnusedLambdaParam's job
            };
            if formals.has_ellipsis() {
                continue;
            }
            if !binding.references.is_empty() {
                continue;
            }
            let name = binding.name.text(model.source());
            if name.starts_with('_') {
                continue;
            }
            // Same rename-to-_-prefixed fix as UnusedLambdaParam: the
            // formal is never referenced, so renaming just its token is
            // safe and aligns the name with the rule's opt-out.
            let range = binding.name.range();
            let fix = Fix::new("rename to _-prefixed").edit(range, format!("_{name}"));
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("parameter '{name}' is never used"),
                    range,
                )
                .with_fix(fix),
            );
        }
    }
}

/// Flags bindings whose name re-binds one already visible.
///
/// `let a = 1; in let a = 2; in a` — the inner `a` hides the outer one,
/// which is almost always a mistake (the outer binding is still
/// reachable, just under a different shadow).
pub struct ShadowedBinding;

impl Rule for ShadowedBinding {
    fn code(&self) -> &'static str {
        "shadowed-binding"
    }

    fn name(&self) -> &'static str {
        "Shadowed binding"
    }

    fn description(&self) -> &'static str {
        "Flags a binding whose name is already bound and visible at its own position. Shadowing hides the outer binding from every use after this point, which is usually unintended."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for binding in model.bindings() {
            // InheritName binds only as a field of a fresh (non-rec) attrset;
            // it never hides anything, so it cannot shadow. Formal parameters
            // have their own diagnostic owner in ShadowedFormal.
            if binding.kind == BindingKind::InheritName
                || (binding.kind == BindingKind::LambdaParam
                    && inside_formals(model.root(), binding.name.range()))
            {
                continue;
            }
            let name = binding.name.text(model.source());
            // Names starting with `_` are idiomatically shadowed
            // (nested lambdas, ignored params) — skip.
            if name.starts_with('_') {
                continue;
            }
            // outer_shadow resolves from the enclosing scope outward, so a
            // recursive `let` binding does not count itself.
            if model.outer_shadow(binding).is_some() {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("binding '{name}' shadows an outer binding"),
                    binding.name.range(),
                ));
            }
        }
    }
}

/// Flags a default on an attribute selection whose literal attrset already
/// contains the selected key. The default branch cannot be reached.
pub struct UnnecessaryOr;

impl Rule for UnnecessaryOr {
    fn code(&self) -> &'static str {
        "unnecessary-or"
    }
    fn name(&self) -> &'static str {
        "Unnecessary attribute default"
    }
    fn description(&self) -> &'static str {
        "Flags an attribute default whose literal attrset base contains the selected key, so the default can never be used."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(select) = SelectExpr::cast(node) else {
                continue;
            };
            if select.default().is_none() {
                continue;
            }
            let Some(Expr::Attrset(set)) = select.base() else {
                continue;
            };
            let Some(path) = select.attrpath() else {
                continue;
            };
            let mut elements = path.elements();
            let Some(AttrName::Ident(key)) = elements.next() else {
                continue;
            };
            if elements.next().is_some() {
                continue;
            }
            let found = set.items().any(|item| match item {
                AttrItem::Binding(binding) => binding.attrpath().is_some_and(|attrpath| {
                    let mut elements = attrpath.elements();
                    matches!(elements.next(), Some(AttrName::Ident(name)) if name.text(source) == key.text(source))
                        && elements.next().is_none()
                }),
                AttrItem::Inherit(_) => false,
            });
            if found {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "attribute '{}' is present; the `or` default is unreachable",
                        key.text(source)
                    ),
                    node.content_range(),
                ));
            }
        }
    }
}

/// Flags `with` scopes whose body never needs them.
///
/// `let pkgs = ...; in with pkgs; pkgs.hello` — every name in the body
/// resolves lexically, so the `with` adds nothing.
pub struct RedundantWith;

impl Rule for RedundantWith {
    fn code(&self) -> &'static str {
        "redundant-with"
    }

    fn name(&self) -> &'static str {
        "Redundant with"
    }

    fn description(&self) -> &'static str {
        "Flags a with scope whose body references all resolve lexically — the with is never consulted and can be removed. Bodies with no references at all are left alone."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for site in model.with_sites() {
            let body_refs: Vec<_> = model
                .references()
                .iter()
                .filter(|r| {
                    let range = r.name.range();
                    site.body_range.contains(range.start()) && site.body_range.end() >= range.end()
                })
                .collect();
            if body_refs.is_empty() {
                continue;
            }
            let all_lexical = body_refs
                .iter()
                .all(|r| r.resolved.is_some() && r.via_with.is_none());
            if all_lexical {
                // Safe to remove: the rule only fires when every body
                // reference resolves lexically (no `via_with`), so the
                // `with` scope is never consulted and deleting it cannot
                // break a reference. The deletion spans from the scope
                // start through the semicolon that terminates it.
                //
                // The terminator is found as a direct child token of the
                // WithExpr node: nested semicolons (inside an attrset or
                // let on the right) live in child nodes, so `child_tokens`
                // yields exactly the scope's own `;`. If it cannot be
                // found, no fix is offered rather than risk a corrupting
                // edit.
                let with_expr = model.root().descendants().find_map(|node| {
                    WithExpr::cast(node).filter(|w| w.range() == site.scope_range)
                });
                let semi_end = with_expr.and_then(|w| {
                    w.syntax()
                        .child_tokens()
                        .find(|t| t.kind() == SyntaxKind::Semicolon)
                        .map(|t| t.range().end())
                });
                let mut diag = Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "with-scope is never used",
                    site.scope_range,
                );
                if let Some(semi_end) = semi_end {
                    diag = diag.with_fix(
                        Fix::new("remove with scope")
                            .edit(TextRange::new(site.scope_range.start(), semi_end), ""),
                    );
                }
                diags.push(diag);
            }
        }
    }
}

/// Flags `let x = x;` infinite recursion.
///
/// A let binding is not visible in its own value, so `let x = x;` has
/// `x` on the right resolve to nothing — the classic AI-slop
/// infinite-recursion bug. A with-fallback or an outer binding that
/// could supply the name is not this bug.
pub struct SelfReferentialLet;

impl Rule for SelfReferentialLet {
    fn code(&self) -> &'static str {
        "self-referential-let"
    }

    fn name(&self) -> &'static str {
        "Self-referential let binding"
    }

    fn description(&self) -> &'static str {
        "Flags a let binding whose value forces its own name — `let x = x;` or `let x = x + 1;`. Evaluation of the value immediately re-enters the binding, so forcing it never terminates. References behind lazy barriers (lambda bodies, list items, attrset values) are fine: `let f = n: f (n - 1);` recurses safely."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for (idx, binding) in model.bindings().iter().enumerate() {
            if binding.kind != BindingKind::LetBinding {
                continue;
            }
            // An inherit name has no value expression: skip it.
            let name_range = binding.name.range();
            let is_inherit = model.root().descendants().any(|n| {
                n.kind() == strictix_syntax::SyntaxKind::InheritStmt
                    && n.range().contains(name_range.start())
            });
            if is_inherit {
                continue;
            }
            let Some(binding_node) = containing_binding(model.root(), name_range) else {
                continue;
            };
            let Some(value) = binding_node.value() else {
                continue;
            };
            let value_range = value.range();
            let name = binding.name.text(source);
            // A self-reference is only a bug when the value's EAGER
            // evaluation forces it: direct, arithmetic, select base,
            // string interpolation, condition. Behind a lambda body, a
            // list item, or an attrset value it is guarded by laziness.
            let forced = model.references().iter().any(|r| {
                let range = r.name.range();
                r.name.text(source) == name
                    && r.resolved == Some(idx)
                    && value_range.contains(range.start())
                    && value_range.end() >= range.end()
                    && eager_contains(value, range)
            });
            if forced {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("binding '{name}' references itself: infinite recursion"),
                    binding.name.range(),
                ));
            }
        }
    }
}

/// Whether forcing `expr` would evaluate the ident at `range`.
///
/// Lambda bodies, list items, and attrset values are lazy: the ident is
/// only evaluated if the enclosing thunk is forced later, which is normal
/// recursion, not this bug. Everything else (operands, bases, conditions,
/// interpolations, application functions) is forced with the value.
fn eager_contains(expr: strictix_syntax::Expr<'_>, range: TextRange) -> bool {
    use strictix_syntax::{AttrItem, AttrName, Expr, StringPart};
    match expr {
        Expr::Ident(t) => t.range() == range,
        Expr::Let(e) => {
            let bindings = e
                .bindings()
                .map(|b| {
                    b.items().any(|item| match item {
                        AttrItem::Binding(binding) => {
                            binding.value().is_some_and(|v| eager_contains(v, range))
                        }
                        AttrItem::Inherit(inh) => {
                            inh.source().is_some_and(|src| eager_contains(src, range))
                        }
                    })
                })
                .unwrap_or(false);
            bindings || e.body().is_some_and(|b| eager_contains(b, range))
        }
        Expr::With(w) => {
            w.scope().is_some_and(|s| eager_contains(s, range))
                || w.body().is_some_and(|b| eager_contains(b, range))
        }
        Expr::Assert(a) => {
            a.cond().is_some_and(|c| eager_contains(c, range))
                || a.body().is_some_and(|b| eager_contains(b, range))
        }
        Expr::If(i) => {
            i.cond().is_some_and(|c| eager_contains(c, range))
                || i.then_branch().is_some_and(|b| eager_contains(b, range))
                || i.else_branch().is_some_and(|b| eager_contains(b, range))
        }
        Expr::Apply(a) => a.func().is_some_and(|f| eager_contains(f, range)),
        Expr::Unary(u) => u.operand().is_some_and(|o| eager_contains(o, range)),
        Expr::Bin(b) => {
            b.lhs().is_some_and(|l| eager_contains(l, range))
                || b.rhs().is_some_and(|r| eager_contains(r, range))
        }
        Expr::Select(s) => {
            let base = s.base().is_some_and(|b| eager_contains(b, range));
            let interp = s
                .attrpath()
                .map(|ap| {
                    ap.elements().any(|e| {
                        if let AttrName::Interp(i) = e {
                            i.expr().is_some_and(|x| eager_contains(x, range))
                        } else {
                            false
                        }
                    })
                })
                .unwrap_or(false);
            base || interp
        }
        Expr::HasAttr(h) => h.base().is_some_and(|b| eager_contains(b, range)),
        Expr::String(s) => s.parts().any(|part| match part {
            StringPart::Content(_) => false,
            StringPart::Interp(i) => i.expr().is_some_and(|x| eager_contains(x, range)),
        }),
        Expr::IndString(s) => s.parts().any(|part| match part {
            StringPart::Content(_) => false,
            StringPart::Interp(i) => i.expr().is_some_and(|x| eager_contains(x, range)),
        }),
        Expr::Paren(p) => p.expr().is_some_and(|i| eager_contains(i, range)),
        // Barriers: lambda body, list items, attrset/rec values.
        Expr::Lambda(_) | Expr::List(_) | Expr::Attrset(_) | Expr::RecAttrset(_) => false,
        Expr::Int(_) | Expr::Float(_) | Expr::Path(_) | Expr::SearchPath(_) | Expr::Uri(_) => false,
    }
}

/// Flags eager reference cycles among sibling let/rec bindings.
///
/// `let a = b; b = a; in a` — forcing either binding forces the other
/// before its own value exists, so evaluation never terminates. The
/// same holds for `rec { a = b; b = a; }`. A cycle is reported once,
/// on its first binding in source order. Length-1 cycles (self-edges)
/// are excluded: `let x = x;` is [SelfReferentialLet]'s finding. Note
/// that self-referential-let inspects only let bindings, so an eager
/// rec self-reference (`rec { a = a + 1; }`) is covered by NEITHER
/// rule — we still exclude self-edges here to keep the two rules'
/// ownership disjoint.
pub struct CircularLet;

impl Rule for CircularLet {
    fn code(&self) -> &'static str {
        "circular-let"
    }

    fn name(&self) -> &'static str {
        "Circular let bindings"
    }

    fn description(&self) -> &'static str {
        "Flags sibling bindings whose values reference each other eagerly (`let a = b; b = a;`): forcing any of them is guaranteed infinite recursion. Lazy positions (attrset values, list items, lambda bodies) are ordinary recursion and never flagged."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let bindings = model.bindings();
        // Sibling groups: every let and every rec attrset owns exactly
        // one ScopeId, so grouping by scope id recovers the sibling
        // sets. Declaration order is preserved, which makes "first in
        // source order" the smallest local index within a group.
        let mut groups: Vec<(ScopeId, Vec<usize>)> = Vec::new();
        for (idx, binding) in bindings.iter().enumerate() {
            if !matches!(binding.kind, BindingKind::LetBinding | BindingKind::RecAttr) {
                continue;
            }
            // Inherit names register as let bindings but have no value
            // expression of their own: skip them.
            let name_range = binding.name.range();
            let is_inherit = model.root().descendants().any(|n| {
                n.kind() == SyntaxKind::InheritStmt && n.range().contains(name_range.start())
            });
            if is_inherit {
                continue;
            }
            match groups.iter_mut().find(|(scope, _)| *scope == binding.scope) {
                Some((_, members)) => members.push(idx),
                None => groups.push((binding.scope, vec![idx])),
            }
        }
        for (_, members) in &groups {
            if members.len() < 2 {
                continue;
            }
            // Edge i -> j (local indices) when member i's value eagerly
            // forces a reference that model-resolves to member j.
            // References inside the value that resolve elsewhere (inner
            // shadows, outer bindings) never match a sibling index, so
            // nested scopes are handled by resolution itself.
            let mut adj: Vec<Vec<usize>> = vec![Vec::new(); members.len()];
            for (local, &idx) in members.iter().enumerate() {
                let Some(binding_node) =
                    containing_binding(model.root(), bindings[idx].name.range())
                else {
                    continue;
                };
                let Some(value) = binding_node.value() else {
                    continue;
                };
                let value_range = value.range();
                for r in model.references() {
                    let Some(target) = r.resolved else { continue };
                    let Some(to) = members.iter().position(|&m| m == target) else {
                        continue;
                    };
                    if to == local || adj[local].contains(&to) {
                        continue;
                    }
                    let range = r.name.range();
                    if value_range.contains(range.start())
                        && value_range.end() >= range.end()
                        && eager_contains(value, range)
                    {
                        adj[local].push(to);
                    }
                }
            }
            let mut color = vec![Color::White; members.len()];
            let mut stack = Vec::new();
            let mut cycles: Vec<Vec<usize>> = Vec::new();
            for start in 0..members.len() {
                if color[start] == Color::White {
                    cycle_dfs(start, &adj, &mut color, &mut stack, &mut cycles);
                }
            }
            // One report per distinct cycle: the same member set can be
            // reached through several back-edges, so dedupe on the
            // sorted member set, then rotate the recorded edge order so
            // the first-in-source member leads.
            let mut seen: HashSet<Vec<usize>> = HashSet::new();
            for cycle in cycles {
                let mut key = cycle.clone();
                key.sort_unstable();
                if !seen.insert(key) {
                    continue;
                }
                let lead = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|&(_, &local)| local)
                    .map(|(pos, _)| pos)
                    .unwrap_or(0);
                let names = cycle[lead..]
                    .iter()
                    .chain(cycle[..lead].iter())
                    .chain(std::iter::once(&cycle[lead]))
                    .map(|&local| format!("'{}'", bindings[members[local]].name.text(source)))
                    .collect::<Vec<_>>()
                    .join(" -> ");
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "bindings {names} form an eager reference cycle; forcing any of them is infinite recursion"
                    ),
                    bindings[members[cycle[lead]]].name.range(),
                ));
            }
        }
    }
}

/// DFS visit state for [CircularLet]'s cycle scan.
#[derive(Clone, Copy, PartialEq)]
enum Color {
    White,
    Gray,
    Black,
}

/// Depth-first cycle scan for [CircularLet]: a back-edge to a node
/// still on the path stack closes a cycle, recorded as the stack slice
/// from that node onward (edge order). Self-edges never enter the
/// adjacency lists, so every recorded cycle has length >= 2; the guard
/// here is belt and braces. Sibling groups are tiny, so recursion
/// depth is bounded by the group size.
fn cycle_dfs(
    node: usize,
    adj: &[Vec<usize>],
    color: &mut [Color],
    stack: &mut Vec<usize>,
    cycles: &mut Vec<Vec<usize>>,
) {
    color[node] = Color::Gray;
    stack.push(node);
    for &next in &adj[node] {
        match color[next] {
            Color::Gray => {
                let pos = stack
                    .iter()
                    .position(|&n| n == next)
                    .expect("gray node is on the path stack");
                if stack.len() - pos >= 2 {
                    cycles.push(stack[pos..].to_vec());
                }
            }
            Color::White => cycle_dfs(next, adj, color, stack, cycles),
            Color::Black => {}
        }
    }
    stack.pop();
    color[node] = Color::Black;
}

/// Flags bindings that rebind `true`, `false`, or `null`.
///
/// Nix has no boolean keywords: `true`, `false`, and `null` are
/// ordinary names bound in builtins, so `let true = false; in ...`
/// parses and evaluates — and makes every later use of the name a
/// lie. Every model binding counts (let, rec attr, lambda param,
/// @-name, inherit). Plain attrset keys are NOT model bindings — a
/// bare attrset binds nothing for resolution — so a legitimate data
/// key like `{ true = 1; }` stays silent automatically.
pub struct ReboundConstant;

impl Rule for ReboundConstant {
    fn code(&self) -> &'static str {
        "rebound-constant"
    }

    fn name(&self) -> &'static str {
        "Rebound constant"
    }

    fn description(&self) -> &'static str {
        "Flags a binding named true, false, or null. Nix accepts `let true = false;` — the names are ordinary globals — but rebinding them makes every later use a lie."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for binding in model.bindings() {
            let name = binding.name.text(source);
            if matches!(name, "true" | "false" | "null") {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("binding rebinds the global constant '{name}'"),
                    binding.name.range(),
                ));
            }
        }
    }
}

fn relative_import(model: &SemanticModel, range: TextRange) -> Option<std::path::PathBuf> {
    let expr = model
        .root()
        .descendants()
        .find(|n| n.range() == range)
        .and_then(Expr::cast)?;
    let source = model.source();
    match expr {
        Expr::Path(path) => {
            let text = path.text(source);
            (text.starts_with("./") || text.starts_with("../")).then(|| text.into())
        }
        Expr::String(string) => {
            let mut text = String::new();
            for part in string.parts() {
                match part {
                    strictix_syntax::StringPart::Content(token) => {
                        text.push_str(token.text(source))
                    }
                    strictix_syntax::StringPart::Interp(_) => return None,
                }
            }
            (text.starts_with("./") || text.starts_with("../")).then(|| text.into())
        }
        _ => None,
    }
}

/// Reports relative imports that are absent from the supplied project (or disk).
pub struct MissingImport;
impl Rule for MissingImport {
    fn code(&self) -> &'static str {
        "missing-import"
    }
    fn name(&self) -> &'static str {
        "Missing import"
    }
    fn description(&self) -> &'static str {
        "Flags relative imports whose target file does not exist."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file_project(
        &self,
        model: &SemanticModel,
        _config: &LintConfig,
        project: Option<&strictix_core::project::ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let Some(base) = model.path().and_then(|p| p.parent()) else {
            return;
        };
        for site in model.import_sites() {
            let Some(raw) = relative_import(model, site.path_range) else {
                continue;
            };
            let target = base.join(raw);
            let exists = project
                .map(|p| p.contains(&target))
                .unwrap_or_else(|| target.is_file());
            if !exists {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "relative import target does not exist",
                    site.path_range,
                ));
            }
        }
    }
}

/// Reports an import edge that reaches its source again in the project graph.
pub struct ImportCycle;
impl Rule for ImportCycle {
    fn code(&self) -> &'static str {
        "import-cycle"
    }
    fn name(&self) -> &'static str {
        "Import cycle"
    }
    fn description(&self) -> &'static str {
        "Flags relative imports that participate in a project import cycle."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file_project(
        &self,
        model: &SemanticModel,
        _config: &LintConfig,
        project: Option<&strictix_core::project::ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let (Some(project), Some(path)) = (project, model.path()) else {
            return;
        };
        for site in model.import_sites() {
            let Some(raw) = relative_import(model, site.path_range) else {
                continue;
            };
            let target = path.parent().unwrap_or(std::path::Path::new(".")).join(raw);
            if project.contains(&target) && project.has_cycle_from(path, &target) {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "relative import participates in an import cycle",
                    site.path_range,
                ));
            }
        }
    }
}
