//! File rules: semantic lints that run once per file against the lazy
//! [SemanticModel].
//!
//! Node rules in [super::node_rules] inspect single nodes; these rules
//! answer questions that need the whole file's meaning: is this let
//! binding ever used, does this lambda parameter shadow an outer one,
//! is this with scope doing any work. Every rule here is a unit struct
//! implementing [Rule] with only [Rule::check_file] overridden — the
//! registry ([super::all_rules]) declares them all the same way.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{AstNode, Binding, Formals, SyntaxKind, SyntaxNode, TextRange};

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
            if !binding.references.is_empty() {
                continue;
            }
            let name = binding.name.text(model.source());
            if name.starts_with('_') {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("parameter '{name}' is never used"),
                range,
            ));
        }
    }
}

/// Flags unused formal parameters of lambdas without an ellipsis.
///
/// `{ a, b }: b` — `a` is declared but never used. With an ellipsis
/// (`{ a, b, ... }: b`) the extra formals are an intentional interface,
/// so only `_`-prefixed names are exempt otherwise.
pub struct UnusedFormal;

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
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("parameter '{name}' is never used"),
                range,
            ));
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
            // it never hides anything, so it cannot shadow.
            if binding.kind == BindingKind::InheritName {
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
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "with-scope is never used",
                    site.scope_range,
                ));
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
