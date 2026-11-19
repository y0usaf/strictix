use std::collections::{HashMap, HashSet};
use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, ScopeId, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, BinExpr, Binding, Expr, Formals, InheritStmt, LambdaExpr,
    LambdaParam, LetExpr, RecAttrsetExpr, SelectExpr, SyntaxKind, SyntaxNode, TextRange, WithExpr,
};
fn containing_binding<'a>(root: &'a SyntaxNode, name_range: TextRange) -> Option<Binding<'a>> {
    root.find_innermost_at(SyntaxKind::Binding, name_range.start())
        .and_then(Binding::cast)
}

fn inside_formals(root: &SyntaxNode, name_range: TextRange) -> bool {
    root.find_innermost_at(SyntaxKind::Formals, name_range.start())
        .is_some()
}

fn containing_formals<'a>(root: &'a SyntaxNode, name_range: TextRange) -> Option<Formals<'a>> {
    root.find_innermost_at(SyntaxKind::Formals, name_range.start())
        .and_then(Formals::cast)
}

pub const MAX_CYCLOMATIC_COMPLEXITY: usize = 5;

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
            SyntaxKind::SelectExpr
                if SelectExpr::cast(node).is_some_and(|expr| expr.default().is_some()) =>
            {
                *score += 1;
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
        let source = model.source();
        let mut used_in_scope: HashSet<(ScopeId, &str)> = HashSet::new();
        for binding in model.bindings() {
            if binding.kind == BindingKind::LetBinding && !binding.references.is_empty() {
                used_in_scope.insert((binding.scope, binding.name.text(source)));
            }
        }

        for binding in model.bindings() {
            if binding.kind != BindingKind::LetBinding || !binding.references.is_empty() {
                continue;
            }
            let name = binding.name.text(source);
            if used_in_scope.contains(&(binding.scope, name)) {
                continue;
            }
            let range = binding.name.range();
            if model
                .root()
                .find_innermost_at(SyntaxKind::InheritStmt, range.start())
                .is_some()
            {
                continue;
            }
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!("binding '{name}' is never used"),
                range,
            );
            if let Some(binding_node) = containing_binding(model.root(), range) {
                if binding_node
                    .attrpath()
                    .is_some_and(|ap| ap.range().contains(range.start()))
                {
                    let delete_range = binding_node.range();
                    let replacement = crate::safe_delete_replacement(source, delete_range);
                    diag = diag.with_fix(
                        Fix::new("remove unused binding").edit(delete_range, replacement),
                    );
                }
            }
            diags.push(diag);
        }
    }
}

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

pub struct UnusedFormal;

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
                            check_inherit_scope(bindings.items(), model, true, diags);
                        }
                    }
                }
                SyntaxKind::RecAttrsetExpr => {
                    if let Some(attrset) = RecAttrsetExpr::cast(node).and_then(|r| r.attrset()) {
                        check_inherit_scope(attrset.items(), model, false, diags);
                    }
                }
                _ => {}
            }
        }
    }
}

fn check_inherit_scope<'a>(
    items: impl Iterator<Item = AttrItem<'a>>,
    model: &SemanticModel,
    is_let: bool,
    diags: &mut Vec<Diagnostic>,
) {
    let source = model.source();
    for item in items {
        let AttrItem::Inherit(inherit) = item else {
            continue;
        };
        let names: Vec<_> = inherit.names().collect();
        let mut unused_indices = Vec::new();
        for (idx, &name_token) in names.iter().enumerate() {
            let Some(binding) = model.binding_at(name_token.range()) else {
                continue;
            };
            let used_elsewhere = binding
                .references
                .iter()
                .any(|range| *range != name_token.range());
            if used_elsewhere {
                continue;
            }
            let name = name_token.text(source);
            if name.starts_with('_') {
                continue;
            }
            unused_indices.push(idx);
        }
        if unused_indices.is_empty() {
            continue;
        }
        let all_unused = unused_indices.len() == names.len();
        for (pos, &idx) in unused_indices.iter().enumerate() {
            let name_token = names[idx];
            let name = name_token.text(source);
            let mut diag = Diagnostic::new(
                "unused-inherit",
                Severity::Warning,
                format!("inherited name '{name}' is never used"),
                name_token.range(),
            );
            if is_let {
                if all_unused {
                    if pos == 0 {
                        let del_range = inherit.syntax().range();
                        let replacement = crate::safe_delete_replacement(source, del_range);
                        diag = diag.with_fix(
                            Fix::new("remove unused inherit").edit(del_range, replacement),
                        );
                    }
                } else {
                    let mut start = name_token.range().start() as usize;
                    let bytes = source.as_bytes();
                    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
                        start -= 1;
                    }
                    let del_range = TextRange::new(start as u32, name_token.range().end());
                    let replacement = crate::safe_delete_replacement(source, del_range);
                    diag = diag.with_fix(
                        Fix::new("remove unused inherited name").edit(del_range, replacement),
                    );
                }
            }
            diags.push(diag);
        }
    }
}

pub struct ShadowedFormal;

impl Rule for ShadowedFormal {
    fn code(&self) -> &'static str {
        "shadowed-formal"
    }
    fn name(&self) -> &'static str {
        "Shadowed formal parameter"
    }
    fn description(&self) -> &'static str {
        "Flags a formal parameter whose name shadows an outer binding. A generator syntax tree (the argument of a `generators.<language>.render` call) is exempt: its nested block lambdas bind the names each block of the generated code declares, so an inner block's local shadows an outer one exactly where the code's own scoping does."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let trees = crate::suggested_rules::tree_ranges(model);
        for binding in model.bindings() {
            if binding.kind != BindingKind::LambdaParam
                || !inside_formals(model.root(), binding.name.range())
                || crate::suggested_rules::in_ranges(&trees, binding.name.range())
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
                continue;
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
            if binding.kind == BindingKind::InheritName
                || (binding.kind == BindingKind::LambdaParam
                    && inside_formals(model.root(), binding.name.range()))
            {
                continue;
            }
            if model
                .root()
                .find_innermost_at(SyntaxKind::InheritStmt, binding.name.range().start())
                .and_then(InheritStmt::cast)
                .is_some_and(|inherit| inherit.source().is_none())
            {
                continue;
            }
            let name = binding.name.text(model.source());
            if name.starts_with('_') {
                continue;
            }
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
                let drop_range = TextRange::new(
                    path.syntax().content_range().end(),
                    node.content_range().end(),
                );
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!(
                            "attribute '{}' is present; the `or` default is unreachable",
                            key.text(source)
                        ),
                        node.content_range(),
                    )
                    .with_fix(Fix::new("remove unreachable default").edit(drop_range, "")),
                );
            }
        }
    }
}

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
            let mut has_any = false;
            let mut all_lexical = true;
            for r in model.references_in_range(site.body_range) {
                let range = r.name.range();
                if site.body_range.contains(range.start()) && site.body_range.end() >= range.end() {
                    has_any = true;
                    if r.resolved.is_none() || r.via_with.is_some() {
                        all_lexical = false;
                        break;
                    }
                }
            }
            if has_any && all_lexical {
                let mut with_expr = None;
                model
                    .root()
                    .walk_containing(site.scope_range.start(), |node| {
                        if let Some(w) =
                            WithExpr::cast(node).filter(|w| w.range() == site.scope_range)
                        {
                            with_expr = Some(w);
                            true
                        } else {
                            false
                        }
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
                    let del_range = TextRange::new(site.scope_range.start(), semi_end);
                    let replacement = crate::safe_delete_replacement(model.source(), del_range);
                    diag =
                        diag.with_fix(Fix::new("remove with scope").edit(del_range, replacement));
                }
                diags.push(diag);
            }
        }
    }
}

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
        for binding in model.bindings() {
            if binding.kind != BindingKind::LetBinding || binding.references.is_empty() {
                continue;
            }
            let name_range = binding.name.range();
            if model
                .root()
                .find_innermost_at(SyntaxKind::InheritStmt, name_range.start())
                .is_some()
            {
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
            let forced = binding.references.iter().any(|&range| {
                value_range.contains(range.start())
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
        Expr::Lambda(_) | Expr::List(_) | Expr::Attrset(_) | Expr::RecAttrset(_) => false,
        Expr::Int(_) | Expr::Float(_) | Expr::Path(_) | Expr::SearchPath(_) | Expr::Uri(_) => false,
    }
}

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
        let mut groups: Vec<(ScopeId, Vec<usize>)> = Vec::new();
        let mut group_idx: HashMap<ScopeId, usize> = HashMap::new();
        for (idx, binding) in bindings.iter().enumerate() {
            if !matches!(binding.kind, BindingKind::LetBinding | BindingKind::RecAttr) {
                continue;
            }
            let name_range = binding.name.range();
            if model
                .root()
                .find_innermost_at(SyntaxKind::InheritStmt, name_range.start())
                .is_some()
            {
                continue;
            }
            match group_idx.get(&binding.scope).copied() {
                Some(gi) => groups[gi].1.push(idx),
                None => {
                    group_idx.insert(binding.scope, groups.len());
                    groups.push((binding.scope, vec![idx]));
                }
            }
        }
        for (_, members) in &groups {
            if members.len() < 2 {
                continue;
            }
            let member_pos: HashMap<usize, usize> = members
                .iter()
                .enumerate()
                .map(|(local, &idx)| (idx, local))
                .collect();
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
                for r in model.references_in_range(value_range) {
                    let Some(target) = r.resolved else { continue };
                    let Some(&to) = member_pos.get(&target) else {
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

#[derive(Clone, Copy, PartialEq)]
enum Color {
    White,
    Gray,
    Black,
}

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
