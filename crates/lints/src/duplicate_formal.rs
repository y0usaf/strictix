//! Flags a lambda formal set that declares the same name twice.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::SyntaxKind as K;
use strictix_syntax::{AstNode, Formals, SyntaxKind, SyntaxNode};

/// Flags a formal parameter declared more than once in one lambda
/// pattern.
///
/// `{ a, a }: a` redefines `a`; Nix rejects this at parse time with
/// "duplicate formal function argument", so the second occurrence is a
/// hard error. Only the declared *names* are compared — a default value
/// like `{ a, b ? a }` is not a duplicate of `a`, because it binds `b`
/// and merely *refers to* `a`.
pub struct DuplicateFormal;

impl Rule for DuplicateFormal {
    fn code(&self) -> &'static str {
        "duplicate-formal"
    }

    fn name(&self) -> &'static str {
        "Duplicate formal argument"
    }

    fn description(&self) -> &'static str {
        "Flags a lambda pattern that declares the same formal name twice. Nix already errors on this, so it is a hard bug; which occurrence to delete is ambiguous, so there is no auto-fix."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(K::Formals)
    }

    /// Walk the declared names of one `{ ... }` formals node. Each time a
    /// name repeats an earlier one, drop a diagnostic on that second (or
    /// later) occurrence's name token. Defaults are never consulted — a
    /// formal's default expression is ordinary syntax and cannot collide
    /// with a declared name.
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(formals) = Formals::cast(node) else {
            return;
        };
        // The syntax library models each parameter as a name token plus
        // an optional default expr; only the name token participates in
        // the duplicate check. Names are Idents (the parser rejects
        // quoted-string formals), so equality is exact token-text
        // equality.
        let mut seen: Vec<&str> = Vec::new();
        for param in formals.params() {
            let name = param.name.text(source);
            if seen.contains(&name) {
                diags.push(Diagnostic::new(
                    "duplicate-formal",
                    Severity::Error,
                    format!("duplicate formal argument '{name}'"),
                    param.name.range(),
                ));
            } else {
                seen.push(name);
            }
        }
    }
}