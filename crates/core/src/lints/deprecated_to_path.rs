use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Apply, Expr},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for usage of the `toPath` function.
///
/// ## Why is this bad?
/// `toPath` is deprecated.
///
/// ## Example
///
/// ```nix
/// builtins.toPath "/path"
/// ```
///
/// Try these instead:
///
/// ```nix
/// # to convert the string to an absolute path:
/// /. + "/path"
/// # => /abc
///
/// # to convert the string to a path relative to the current directory:
/// ./. + "/bin"
/// # => /home/np/statix/bin
/// ```
#[lint(
    name = "deprecated_to_path",
    note = "Found usage of deprecated builtin toPath",
    code = 17,
    match_with = SyntaxKind::NODE_APPLY
)]
struct DeprecatedToPath;

static ALLOWED_PATHS: &[&str; 2] = &["builtins.toPath", "toPath"];

impl Rule for DeprecatedToPath {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let apply = Apply::cast(node.clone())?;
        let lambda_path = apply.lambda()?.to_string();
        if !ALLOWED_PATHS.contains(&lambda_path.as_str()) {
            return None;
        }

        let at = node.text_range();
        let message = format!(
            "`{lambda_path}` is deprecated, see `:doc builtins.toPath` within the REPL for more"
        );

        Some(match absolute_string_replacement(&apply) {
            Some(replacement) => {
                self.report()
                    .suggest(at, message, Suggestion::with_text(at, replacement))
            }
            None => self.report().diagnostic(at, message),
        })
    }
}

fn absolute_string_replacement(apply: &Apply) -> Option<String> {
    let argument = apply.argument()?;
    let Expr::Str(string) = argument else {
        return None;
    };

    if string
        .syntax()
        .children()
        .any(|child| child.kind() == SyntaxKind::NODE_INTERPOL)
    {
        return None;
    }

    let string_text = string.to_string();
    if !string_text.starts_with("\"/") {
        return None;
    }

    Some(format!("/. + {string_text}"))
}
