use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{NodeOrToken, SyntaxElement, SyntaxKind, ast::Apply};
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

        Some(self.report().suggest(
            at,
            message,
            Suggestion::with_text(at, to_path_replacement(&apply)),
        ))
    }
}

fn to_path_replacement(apply: &Apply) -> String {
    let apply_text = apply.syntax().to_string();
    let binding = fresh_name(&apply_text, "__strictix_to_path_arg");
    let argument = apply
        .argument()
        .map(|argument| argument.syntax().to_string())
        .unwrap_or_default();

    format!(
        "let {binding} = builtins.toString ({argument}); in if builtins.substring 0 1 {binding} == \"/\" then /. + {binding} else ./. + \"/${{{binding}}}\""
    )
}

fn fresh_name(haystack: &str, base: &str) -> String {
    let mut candidate = base.to_string();
    while haystack.contains(&candidate) {
        candidate.push('_');
    }
    candidate
}
