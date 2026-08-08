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
        let bindings = model.bindings();
        for (idx, binding) in bindings.iter().enumerate() {
            let name = binding.name.text(model.source());
            let offset = binding.name.range().start();
            let Some(other) = model.resolve_lexical(name, offset) else {
                continue;
            };
            // Identity by index: rec attrs see their own name at the
            // attr-name position (rec binds inside values, but the
            // model is position-based), which is not a shadow.
            let other_idx = bindings.iter().position(|b| std::ptr::eq(b, other));
            if other_idx == Some(idx) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("binding '{name}' shadows an outer binding"),
                binding.name.range(),
            ));
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
        "Flags a let binding whose value references its own name with nothing else bound to that name — `let x = x;` evaluates to an infinite recursion at runtime."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for binding in model.bindings() {
            if binding.kind != BindingKind::LetBinding {
                continue;
            }
            let Some(binding_node) = containing_binding(model.root(), binding.name.range()) else {
                continue;
            };
            let Some(value) = binding_node.value() else {
                continue;
            };
            let value_range = value.range();
            let name = binding.name.text(model.source());
            for reference in model.references() {
                let range = reference.name.range();
                if !(value_range.contains(range.start()) && value_range.end() >= range.end()) {
                    continue;
                }
                if reference.name.text(model.source()) != name {
                    continue;
                }
                if model.is_bound(name, range.start()) {
                    continue;
                }
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
