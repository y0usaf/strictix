use std::collections::HashSet;

use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode,
    ast::{Attr, AttrSet, Entry, Expr, HasEntry as _, Ident, With},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `with` expressions.
///
/// ## Why is this bad?
/// `with` introduces implicit scope that makes code harder to reason about:
/// it breaks language server features (goto-definition, completions), can
/// cause subtle shadowing bugs, and makes refactoring fragile. Prefer
/// explicit attribute access or `inherit`.
///
/// ## Example
///
/// ```nix
/// with pkgs; [ git curl ]
/// ```
///
/// Use explicit attribute access instead:
///
/// ```nix
/// [ pkgs.git pkgs.curl ]
/// ```
#[lint(
    name = "with_expression",
    note = "Avoid `with` expressions",
    code = 24,
    match_with = SyntaxKind::NODE_WITH,
    default_enabled = false
)]
struct WithExpression;

impl Rule for WithExpression {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let with_expr = With::cast(node.clone())?;
        let namespace = with_expr.namespace()?;
        let body = with_expr.body()?;
        let at = node.text_range();
        let message = format!(
            "`with {};` introduces implicit scope; use explicit attribute access or `inherit` instead",
            namespace.syntax()
        );

        Some(match literal_with_replacement(&namespace, &body) {
            Some(replacement) => {
                self.report()
                    .suggest(at, message, Suggestion::with_text(at, replacement))
            }
            None => self.report().diagnostic(at, message),
        })
    }
}

fn literal_with_replacement(namespace: &Expr, body: &Expr) -> Option<String> {
    let Expr::AttrSet(attrset) = namespace else {
        return None;
    };

    let namespace_keys = literal_attrset_keys(attrset);
    let used = body_identifiers(body.syntax());

    if used.iter().any(|name| !namespace_keys.contains(name)) {
        return None;
    }

    if used.is_empty() {
        return Some(body.syntax().to_string());
    }

    Some(format!(
        "let inherit ({namespace}) {}; in {body}",
        used.join(" "),
    ))
}

fn literal_attrset_keys(attrset: &AttrSet) -> HashSet<String> {
    let mut keys = HashSet::new();

    for entry in attrset.entries() {
        match entry {
            Entry::AttrpathValue(kv) => {
                let Some(attrpath) = kv.attrpath() else {
                    continue;
                };
                let mut attrs = attrpath.attrs();
                let Some(Attr::Ident(ident)) = attrs.next() else {
                    continue;
                };
                if attrs.next().is_none() {
                    keys.insert(ident.to_string());
                }
            }
            Entry::Inherit(inherit) => {
                for attr in inherit.attrs() {
                    keys.insert(attr.to_string());
                }
            }
        }
    }

    keys
}

fn body_identifiers(node: &SyntaxNode) -> Vec<String> {
    let mut ordered = Vec::new();
    let mut seen = HashSet::new();
    collect_body_identifiers(node, &mut ordered, &mut seen);
    ordered
}

fn collect_body_identifiers(
    node: &SyntaxNode,
    ordered: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if let Some(ident) = Ident::cast(node.clone()) {
        let parent_kind = node.parent().map(|parent| parent.kind());
        let is_attrpath = parent_kind == Some(SyntaxKind::NODE_ATTRPATH);
        let is_binding = matches!(
            parent_kind,
            Some(
                SyntaxKind::NODE_IDENT_PARAM
                    | SyntaxKind::NODE_PAT_BIND
                    | SyntaxKind::NODE_PAT_ENTRY
            )
        );
        let text = ident.to_string();

        if !is_attrpath
            && !is_binding
            && !matches!(text.as_str(), "true" | "false" | "null")
            && seen.insert(text.clone())
        {
            ordered.push(text);
        }
        return;
    }

    for child in node.children() {
        collect_body_identifiers(&child, ordered, seen);
    }
}
