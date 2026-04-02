use crate::{Metadata, Report, Rule, Suggestion, make};

use macros::lint;
use rnix::{NodeOrToken, SyntaxElement, SyntaxKind, ast::Dynamic};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for antiquote/splice expressions that are not quoted.
///
/// ## Why is this bad?
/// An *anti*quoted expression should always occur within a *quoted*
/// expression.
///
/// ## Example
///
/// ```nix
/// let
///   pkgs = nixpkgs.legacyPackages.${system};
/// in
///   pkgs
/// ```
///
/// Quote the splice expression:
///
/// ```nix
/// let
///   pkgs = nixpkgs.legacyPackages."${system}";
/// in
///   pkgs
/// ```
#[lint(
    name = "unquoted_splice",
    note = "Found unquoted splice expression",
    code = 9,
    match_with = SyntaxKind::NODE_DYNAMIC
)]
struct UnquotedSplice;

impl Rule for UnquotedSplice {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };
        Dynamic::cast(node.clone())?;

        let at = node.text_range();
        let replacement = make::quote(node);
        let message = "Consider quoting this splice expression";
        Some(self.report().suggest(
            at,
            message,
            Suggestion::with_replacement(at, replacement?.syntax().clone()),
        ))
    }
}
