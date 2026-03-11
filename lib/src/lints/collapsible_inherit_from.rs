use crate::{Metadata, Report, Rule, Suggestion, make};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, TextRange,
    ast::{Attr, Inherit},
};
use rowan::{Direction, ast::AstNode as _};

/// ## What it does
/// Checks for consecutive `inherit (x)` statements from the same source
/// that can be merged into a single statement.
///
/// ## Why is this bad?
/// Redundant code; multiple inherits from the same set can be written as one.
///
/// ## Example
///
/// ```nix
/// {
///   inherit (spec) command;
///   inherit (spec) args;
/// }
/// ```
///
/// Merge into a single inherit statement:
///
/// ```nix
/// {
///   inherit (spec) command args;
/// }
/// ```
#[lint(
    name = "collapsible_inherit_from",
    note = "These inherit statements can be merged",
    code = 25,
    match_with = SyntaxKind::NODE_INHERIT
)]
struct CollapsibleInheritFrom;

impl Rule for CollapsibleInheritFrom {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let inherit = Inherit::cast(node.clone())?;
        let from = inherit.from()?;
        let namespace_text = from.expr()?.syntax().to_string();

        // Find the next sibling NODE_INHERIT
        let next_inherit = node
            .siblings_with_tokens(Direction::Next)
            .skip(1)
            .find(|s| !matches!(s.kind(), SyntaxKind::TOKEN_WHITESPACE))?;

        let NodeOrToken::Node(next_node) = next_inherit else {
            return None;
        };

        let next_inherit_stmt = Inherit::cast(next_node.clone())?;
        let next_from = next_inherit_stmt.from()?;

        if next_from.expr()?.syntax().to_string() != namespace_text {
            return None;
        }

        // Collect all ident attrs from both inherits
        let all_attrs: Vec<_> = inherit.attrs().chain(next_inherit_stmt.attrs()).collect();

        let idents: Vec<_> = all_attrs
            .iter()
            .filter_map(|a| {
                if let Attr::Ident(i) = a {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        if idents.len() != all_attrs.len() {
            // Non-ident attrs (dynamic ${...}) — skip
            return None;
        }

        let merged = make::inherit_from_stmt_text(&namespace_text, idents);

        // Replace the span from start of node1 to end of node2 with the merged form
        let at = TextRange::new(node.text_range().start(), next_node.text_range().end());

        Some(self.report().suggest(
            at,
            format!("Merge `inherit ({namespace_text}) ...;` statements into one"),
            Suggestion::with_replacement(at, merged.syntax().clone()),
        ))
    }
}
