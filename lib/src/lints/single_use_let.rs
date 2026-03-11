use crate::{Metadata, Report, Rule, Suggestion, make};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, TextRange,
    ast::{Attr, Entry, HasEntry as _, Ident, LetIn},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for `let-in` bindings that are used at most once — or not at all —
/// across the entire `let-in` expression.
///
/// ## Why is this bad?
/// A binding used only once adds indirection without improving clarity.
/// Inlining it produces more direct, readable code. An unused binding is
/// dead code.
///
/// ## Example
///
/// ```nix
/// let
///   x = pkgs.hello;
/// in
///   x.meta.description
/// ```
///
/// Inline the binding:
///
/// ```nix
/// (pkgs.hello).meta.description
/// ```
#[lint(
    name = "single_use_let",
    note = "Let binding used only once; consider inlining",
    code = 30,
    match_with = SyntaxKind::NODE_LET_IN
)]
struct SingleUseLet;

impl Rule for SingleUseLet {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let let_in_expr = LetIn::cast(node.clone())?;
        let body = let_in_expr.body()?;
        let entries: Vec<_> = let_in_expr.entries().collect();

        let mut report = self.report();
        let mut found = false;
        // Only one binding gets fix suggestions per pass to avoid offset
        // corruption when Report::apply applies multiple edits sequentially.
        let mut fix_allocated = false;

        for (i, entry) in entries.iter().enumerate() {
            let Entry::AttrpathValue(kv) = entry else {
                continue;
            };

            let Some(name) = simple_binding_name(kv) else {
                continue;
            };

            let refs = ref_counts(kv, i, &entries, body.syntax(), &name);

            if refs.total > 1 {
                continue;
            }

            let binding_at = kv.syntax().text_range();

            if refs.total == 0 {
                let message = format!("`{name}` is never used");
                if fix_allocated {
                    report = report.diagnostic(binding_at, message);
                } else {
                    let removal = binding_removal_range(kv.syntax());
                    report = report.suggest(binding_at, message, Suggestion::with_empty(removal));
                    fix_allocated = true;
                }
                found = true;
            } else if !fix_allocated && refs.in_siblings == 0 && refs.in_own_value == 0 {
                // Single use in body: inline with two ordered suggestions.
                // Reference replacement (higher offset) is applied first so
                // the binding removal (lower offset) stays valid.
                let message = format!("`{name}` is only used once; consider inlining");
                let value_node = kv.value()?.syntax().clone();
                if let Some(ref_range) = find_ident_ref_range(body.syntax(), &name) {
                    let replacement = make::parenthesize(&value_node);
                    report = report
                        .suggest(
                            ref_range,
                            &message,
                            Suggestion::with_replacement(ref_range, replacement.syntax().clone()),
                        )
                        .suggest(
                            binding_at,
                            message,
                            Suggestion::with_empty(binding_removal_range(kv.syntax())),
                        );
                    fix_allocated = true;
                } else {
                    report = report.diagnostic(binding_at, message);
                }
                found = true;
            } else {
                let message = format!("`{name}` is only used once; consider inlining");
                report = report.diagnostic(binding_at, message);
                found = true;
            }
        }

        found.then_some(report)
    }
}

struct RefCounts {
    total: usize,
    in_siblings: usize,
    in_own_value: usize,
}

fn ref_counts(
    kv: &rnix::ast::AttrpathValue,
    index: usize,
    entries: &[Entry],
    body: &SyntaxNode,
    name: &str,
) -> RefCounts {
    let in_own_value = kv.value().map_or(0, |v| count_ident_refs(v.syntax(), name));

    let in_siblings: usize = entries
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != index)
        .filter_map(|(_, e)| {
            if let Entry::AttrpathValue(other_kv) = e {
                other_kv.value().map(|v| count_ident_refs(v.syntax(), name))
            } else {
                None
            }
        })
        .sum();

    let in_body = count_ident_refs(body, name);

    RefCounts {
        total: in_own_value + in_siblings + in_body,
        in_siblings,
        in_own_value,
    }
}

/// Extract the name from a simple single-component ident binding.
/// Returns `None` for compound paths (`a.b = …`) and dynamic keys (`${…}`).
fn simple_binding_name(kv: &rnix::ast::AttrpathValue) -> Option<String> {
    let mut attrs = kv.attrpath()?.attrs();
    let first = attrs.next()?;
    if attrs.next().is_some() {
        return None;
    }
    let Attr::Ident(ident) = first else {
        return None;
    };
    Some(ident.to_string())
}

/// Returns the range covering the preceding whitespace (if any) plus the
/// binding node itself (which includes its semicolon in rnix's CST).
/// Removing this range cleanly deletes the binding "line".
fn binding_removal_range(binding: &SyntaxNode) -> TextRange {
    let end = binding.text_range().end();
    let start = match binding
        .prev_sibling_or_token()
        .filter(|t| t.kind() == SyntaxKind::TOKEN_WHITESPACE)
    {
        Some(ws) => ws.text_range().start(),
        None => binding.text_range().start(),
    };
    TextRange::new(start, end)
}

/// Count how many times `name` appears as a variable reference inside `node`,
/// excluding attrpath components (attribute keys in bindings/selections).
fn count_ident_refs(node: &SyntaxNode, name: &str) -> usize {
    if let Some(ident) = Ident::cast(node.clone()) {
        let parent_is_attrpath = node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NODE_ATTRPATH);
        if !parent_is_attrpath && ident.to_string() == name {
            return 1;
        }
        return 0;
    }
    node.children()
        .map(|child| count_ident_refs(&child, name))
        .sum()
}

/// Find the text range of the first variable reference to `name` in `node`.
fn find_ident_ref_range(node: &SyntaxNode, name: &str) -> Option<TextRange> {
    if let Some(ident) = Ident::cast(node.clone()) {
        let parent_is_attrpath = node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NODE_ATTRPATH);
        if !parent_is_attrpath && ident.to_string() == name {
            return Some(node.text_range());
        }
        return None;
    }
    node.children()
        .find_map(|child| find_ident_ref_range(&child, name))
}
