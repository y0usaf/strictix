//! File rule: bare `import` used as a list element.
//!
//! Inside `[ ... ]`, whitespace-separated "simple" expressions are
//! separate elements; application binds only within a single expression.
//! So `[ import ./a.nix ]` is a TWO-element list: the bare `import`
//! function, then the path `./a.nix` — NOT an import of `./a.nix`. The
//! bare `import` floats into the value and, when forced, becomes the
//! evaluator error `expected a path but got a function`, or the renamed
//! import list is silently wrong. The user almost surely meant
//! `[ (import ./a.nix) ]`.
//!
//! Function-scope `import` is Nix's builtin; a user can shadow it with a
//! `let import = ...`, which is why this is a file rule: it consults the
//! [SemanticModel] to skip a shadowed (local) `import`.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AstNode, Expr, ListExpr, StringPart, SyntaxKind, TextRange};

/// The diagnostic message: a bare `import` in a list binds only the next
/// whitespace-separated element, so it must be wrapped in parens.
const MESSAGE: &str =
    "'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)";

/// Whether the expression immediately following a bare `import` is one we
/// can safely parenthesize as the imported argument.
///
/// A `./path` or a plain (non-interpolating) string is a self-contained
/// expression whose source text we can lift verbatim into
/// `(import …)`. Interpolated strings and every compound expression are
/// NOT safe to splice blindly, so those get no fix.
fn fixable_following(following: Expr<'_>) -> bool {
    match following {
        Expr::Path(_) => true,
        Expr::String(s) => !s
            .parts()
            .any(|part| matches!(part, StringPart::Interp(_))),
        // Compound nodes (lambdas, applies, the whole rest) are either
        // already a single expression we don't need to touch, or too
        // risky to rewrite by spanning text; give the user the plain
        // diagnostic with no auto-fix for them.
        _ => false,
    }
}

/// Flags a bare builtin `import` used as a list element.
///
/// This is a FILE rule because the shadow guard needs the semantic
/// model: `import` is only this rule's business when it resolves to
/// nothing (the global builtin), not when a local `let import = ...`
/// has shadowed it.
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
                // A `import` that resolves to a binding is a shadowed
                // local function the user defined, not Nix's builtin —
                // then this list is their business. (Same shadow-guard
                // shape as UnknownBuiltin's `builtins` check.)
                if model.resolve(name).is_some() {
                    continue;
                }
                // Nothing follows in the list → nothing to import; the
                // bare `import` is just a value, leave it alone.
                let Some(following) = items.get(idx + 1) else {
                    continue;
                };
                let following = *following;
                let mut diag = Diagnostic::new(
                    self.code(),
                    self.severity(),
                    MESSAGE,
                    name.range(),
                );
                if fixable_following(following) {
                    // Replace the span from the bare `import` token
                    // through the end of the following item, reusing the
                    // ORIGINAL following text inside a paren frame:
                    // `(import <following>)`.
                    let range =
                        TextRange::new(name.range().start(), following.range().end());
                    let replacement =
                        format!("(import {})", following.text(source));
                    diag = diag.with_fix(
                        Fix::new("parenthesize the import argument")
                            .edit(range, replacement),
                    );
                }
                diags.push(diag);
            }
        }
    }
}