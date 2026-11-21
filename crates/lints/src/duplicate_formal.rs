use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::SyntaxKind as K;
use strictix_syntax::{AstNode, Formals, SyntaxKind, SyntaxNode};

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

        fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(formals) = Formals::cast(node) else {
            return;
        };
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
