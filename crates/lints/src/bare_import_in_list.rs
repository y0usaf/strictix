use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AstNode, Expr, ListExpr, StringPart, SyntaxKind, TextRange};

const MESSAGE: &str =
    "'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)";

fn fixable_following(following: Expr<'_>) -> bool {
    match following {
        Expr::Path(_) => true,
        Expr::String(s) => !s.parts().any(|part| matches!(part, StringPart::Interp(_))),
        _ => false,
    }
}

pub struct BareImportInList;

impl Rule for BareImportInList {
    fn code(&self) -> &'static str {
        "bare-import-in-list"
    }

    fn name(&self) -> &'static str {
        "Bare import in list"
    }

    fn description(&self) -> &'static str {
        "Flags a bare `import` as a list element. In `[ ... ]` a whitespace-separated `import ./a.nix` is two elements (the import function, then the path), not an import — the author almost surely meant `[ (import ./a.nix) ]`."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            if node.kind() != SyntaxKind::ListExpr {
                continue;
            }
            let Some(list) = ListExpr::cast(node) else {
                continue;
            };
            let items: Vec<Expr<'_>> = list.items().collect();
            for (idx, item) in items.iter().enumerate() {
                let Expr::Ident(name) = *item else {
                    continue;
                };
                if name.text(source) != "import" {
                    continue;
                }
                if model.resolve(name).is_some() {
                    continue;
                }
                let Some(following) = items.get(idx + 1) else {
                    continue;
                };
                let following = *following;
                let mut diag = Diagnostic::new(self.code(), self.severity(), MESSAGE, name.range());
                if fixable_following(following) {
                    let range = TextRange::new(name.range().start(), following.range().end());
                    let replacement = format!("(import {})", following.text(source));
                    diag = diag.with_fix(
                        Fix::new("parenthesize the import argument").edit(range, replacement),
                    );
                }
                diags.push(diag);
            }
        }
    }
}
