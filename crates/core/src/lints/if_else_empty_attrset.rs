use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Expr, HasEntry, IfElse},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `if cond then attrset else {}` patterns.
///
/// ## Why is this bad?
/// This is a verbose way to conditionally include attributes.
/// `lib.optionalAttrs` expresses the same intent more concisely.
///
/// ## Example
///
/// ```nix
/// base // (if config.foo.enable then { bar = 1; } else {})
/// ```
///
/// Use `lib.optionalAttrs` instead:
///
/// ```nix
/// base // lib.optionalAttrs config.foo.enable { bar = 1; }
/// ```
#[lint(
    name = "if_else_empty_attrset",
    note = "Prefer `lib.optionalAttrs` over `if cond then attrs else {}`",
    code = 28,
    match_with = SyntaxKind::NODE_IF_ELSE
)]
struct IfElseEmptyAttrset;

impl Rule for IfElseEmptyAttrset {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let if_else = IfElse::cast(node.clone())?;
        let cond = if_else.condition()?;
        let then_body = if_else.body()?;
        let else_body = if_else.else_body()?;

        // `else` must be an empty non-rec attrset
        let Expr::AttrSet(else_set) = else_body else {
            return None;
        };
        if else_set.rec_token().is_some() || else_set.entries().count() != 0 {
            return None;
        }

        // `then` must be a non-empty attrset
        let Expr::AttrSet(then_set) = &then_body else {
            return None;
        };
        if then_set.entries().count() == 0 {
            return None;
        }

        let at = node.text_range();
        Some(self.report().suggest(
            at,
            format!(
                "Replace `if ... then {{ ... }} else {{}}` with a `builtins.listToAttrs` expansion for `{}`",
                cond.syntax()
            ),
            Suggestion::with_text(at, optional_attrs_replacement(cond.syntax(), then_set.syntax())),
        ))
    }
}

fn optional_attrs_replacement(cond: &rnix::SyntaxNode, then_body: &rnix::SyntaxNode) -> String {
    format!(
        "builtins.listToAttrs (if {cond} then builtins.mapAttrsToList (name: value: {{ inherit name value; }}) {then_body} else [])"
    )
}
