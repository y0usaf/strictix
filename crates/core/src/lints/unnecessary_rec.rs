use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode,
    ast::{Attr, AttrSet, Entry, HasEntry, Ident},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `rec { }` attribute sets where no binding references
/// any other binding in the same set.
///
/// ## Why is this bad?
/// `rec` is unnecessary and misleading if none of the bindings
/// actually reference sibling bindings.
///
/// ## Example
///
/// ```nix
/// rec {
///   a = 1;
///   b = 2;
/// }
/// ```
///
/// Remove `rec`:
///
/// ```nix
/// {
///   a = 1;
///   b = 2;
/// }
/// ```
#[lint(
    name = "unnecessary_rec",
    note = "This `rec` attrset has no self-referential bindings",
    code = 29,
    match_with = SyntaxKind::NODE_ATTR_SET
)]
struct UnnecessaryRec;

impl Rule for UnnecessaryRec {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let attrset = AttrSet::cast(node.clone())?;

        // Only applies to rec attrsets
        let rec_token = attrset.rec_token()?;

        // Collect all simple (ident) top-level binding names
        let binding_names: Vec<String> = attrset
            .entries()
            .filter_map(|entry| {
                let Entry::AttrpathValue(kv) = entry else {
                    return None;
                };
                let mut attrs = kv.attrpath()?.attrs();
                let first = attrs.next()?;
                // Only single-component ident keys (not `a.b` or `${...}`)
                if attrs.next().is_some() {
                    return None;
                }
                let Attr::Ident(ident) = first else {
                    return None;
                };
                Some(ident.to_string())
            })
            .collect();

        if binding_names.is_empty() {
            return None;
        }

        // Check if any binding value references any sibling binding name
        let any_self_ref = attrset.entries().any(|entry| {
            let Entry::AttrpathValue(kv) = entry else {
                return false;
            };
            let Some(value) = kv.value() else {
                return false;
            };
            mentions_any_name(value.syntax(), &binding_names)
        });

        if any_self_ref {
            return None;
        }

        let at = node.text_range();
        // Remove `rec` and any whitespace immediately following it
        let fix_end = rec_token
            .next_sibling_or_token()
            .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
            .map_or(rec_token.text_range().end(), |t| t.text_range().end());
        let fix_range = rnix::TextRange::new(rec_token.text_range().start(), fix_end);

        Some(self.report().suggest(
            at,
            "Remove `rec`: no binding references a sibling binding",
            Suggestion::with_empty(fix_range),
        ))
    }
}

fn mentions_any_name(node: &SyntaxNode, names: &[String]) -> bool {
    if let Some(ident) = Ident::cast(node.clone()) {
        return names.contains(&ident.to_string());
    }
    node.children()
        .any(|child| mentions_any_name(&child, names))
}
