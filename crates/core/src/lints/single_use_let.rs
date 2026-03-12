use crate::{Metadata, Report, Rule, Suggestion, make};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, TextRange,
    ast::{Attr, Entry, HasEntry as _, Ident, Inherit, LetIn},
};
use rowan::ast::AstNode as _;
use std::collections::HashMap;

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
    #[allow(clippy::too_many_lines)]
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let let_in_expr = LetIn::cast(node.clone())?;
        let body = let_in_expr.body()?;
        let entries: Vec<_> = let_in_expr.entries().collect();
        let body_refs = ident_ref_counts(body.syntax());
        let entry_refs: Vec<_> = entries.iter().map(entry_ref_counts).collect();
        let total_entry_refs = merge_ref_counts(&entry_refs);

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

            let refs = ref_counts(&name, &body_refs, &total_entry_refs, &entry_refs[i]);

            if refs.total > 1 {
                continue;
            }

            let binding_at = kv.syntax().text_range();

            let external_refs = refs.in_siblings + body_refs.get(&name).copied().unwrap_or(0);

            if external_refs == 0 {
                let message = format!("`{name}` is never used");
                if fix_allocated {
                    report = report.diagnostic(binding_at, message);
                } else {
                    let removal = binding_removal_range(kv.syntax());
                    report = report.suggest(binding_at, message, Suggestion::with_empty(removal));
                    fix_allocated = true;
                }
                found = true;
            } else if !fix_allocated && external_refs == 1 && refs.in_own_value == 0 {
                // Single use outside the binding itself: inline with two
                // ordered suggestions. The higher offset replacement is
                // applied before the binding removal.
                let message = format!("`{name}` is only used once; consider inlining");
                let value_node = kv.value()?.syntax().clone();
                let removal = Suggestion::with_empty(binding_removal_range(kv.syntax()));

                match find_external_ident_ref(&entries, i, body.syntax(), &name) {
                    Some(InlineTarget::Direct {
                        range: ref_range,
                        needs_parens,
                    }) => {
                        let replacement = if needs_parens {
                            make::parenthesize(&value_node).syntax().clone()
                        } else {
                            value_node.clone()
                        };
                        report = report
                            .suggest(
                                ref_range,
                                &message,
                                Suggestion::with_replacement(ref_range, replacement),
                            )
                            .suggest(binding_at, message, removal);
                        fix_allocated = true;
                    }
                    Some(InlineTarget::BareInherit { inherit_node }) => {
                        let replacement =
                            bare_inherit_replacement(&inherit_node, &name, &value_node)?;
                        let inherit_range = inherit_node.text_range();
                        report = report
                            .suggest(
                                inherit_range,
                                &message,
                                Suggestion::with_text(inherit_range, replacement),
                            )
                            .suggest(binding_at, message, removal);
                        fix_allocated = true;
                    }
                    Some(InlineTarget::StringInterpol { interpol_range }) => {
                        report = report
                            .suggest(
                                interpol_range,
                                &message,
                                Suggestion::with_text(
                                    interpol_range,
                                    interpol_replacement(&value_node),
                                ),
                            )
                            .suggest(binding_at, message, removal);
                        fix_allocated = true;
                    }
                    _ => {
                        report = report.diagnostic(binding_at, message);
                    }
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
    name: &str,
    body_refs: &HashMap<String, usize>,
    total_entry_refs: &HashMap<String, usize>,
    own_entry_refs: &HashMap<String, usize>,
) -> RefCounts {
    let in_own_value = own_entry_refs.get(name).copied().unwrap_or(0);

    let in_siblings = total_entry_refs
        .get(name)
        .copied()
        .unwrap_or(0)
        .saturating_sub(in_own_value);

    let in_body = body_refs.get(name).copied().unwrap_or(0);

    RefCounts {
        total: in_own_value + in_siblings + in_body,
        in_siblings,
        in_own_value,
    }
}

fn entry_ref_counts(entry: &Entry) -> HashMap<String, usize> {
    match entry {
        Entry::AttrpathValue(kv) => kv
            .value()
            .map_or_else(HashMap::new, |value| ident_ref_counts(value.syntax())),
        Entry::Inherit(inherit) => inherit_ref_counts(inherit),
    }
}

fn merge_ref_counts(ref_maps: &[HashMap<String, usize>]) -> HashMap<String, usize> {
    let mut merged = HashMap::new();

    for ref_map in ref_maps {
        for (name, count) in ref_map {
            *merged.entry(name.clone()).or_insert(0) += count;
        }
    }

    merged
}

fn inherit_ref_counts(inherit: &Inherit) -> HashMap<String, usize> {
    let mut counts = inherit
        .from()
        .map_or_else(HashMap::new, |from| ident_ref_counts(from.syntax()));

    for attr in inherit.attrs() {
        *counts.entry(attr.to_string()).or_insert(0) += 1;
    }

    counts
}

fn ident_ref_counts(node: &SyntaxNode) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    collect_ident_refs(node, &mut counts);
    counts
}

fn collect_ident_refs(node: &SyntaxNode, counts: &mut HashMap<String, usize>) {
    if let Some(ident) = Ident::cast(node.clone()) {
        let parent_is_attrpath = node
            .parent()
            .is_some_and(|p| p.kind() == SyntaxKind::NODE_ATTRPATH);
        if !parent_is_attrpath {
            *counts.entry(ident.to_string()).or_insert(0) += 1;
        }
        return;
    }

    for child in node.children() {
        collect_ident_refs(&child, counts);
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

enum InlineTarget {
    /// Replace the ident directly; `needs_parens` controls whether the inlined
    /// value should be wrapped in `(…)`.
    Direct {
        range: TextRange,
        needs_parens: bool,
    },
    /// The ident appears as the sole attribute of a bare `inherit foo;`.
    /// Replace the entire inherit statement with `foo = value;`.
    BareInherit { inherit_node: SyntaxNode },
    /// The ident appears inside a `${foo}` string interpolation.
    /// Replace the entire `${…}` with the literal string content.
    StringInterpol { interpol_range: TextRange },
}

fn find_external_ident_ref(
    entries: &[Entry],
    current_index: usize,
    body: &SyntaxNode,
    name: &str,
) -> Option<InlineTarget> {
    for (index, entry) in entries.iter().enumerate() {
        if index == current_index {
            continue;
        }

        let target = match entry {
            Entry::AttrpathValue(kv) => kv
                .value()
                .and_then(|value| find_ident_ref(value.syntax(), name)),
            Entry::Inherit(inherit) => find_ident_ref(inherit.syntax(), name),
        };

        if target.is_some() {
            return target;
        }
    }

    find_ident_ref(body, name)
}

/// Walk `node` looking for the first reference to `name` and return how it
/// should be inlined.
fn find_ident_ref(node: &SyntaxNode, name: &str) -> Option<InlineTarget> {
    if let Some(ident) = Ident::cast(node.clone()) {
        let parent = node.parent();
        let parent_kind = parent.as_ref().map(SyntaxNode::kind);

        if parent_kind == Some(SyntaxKind::NODE_INHERIT) {
            let inherit_node = parent?;
            let inherit = Inherit::cast(inherit_node.clone())?;
            if inherit.from().is_none() && ident.to_string() == name {
                return Some(InlineTarget::BareInherit { inherit_node });
            }
            return None;
        }

        if parent_kind == Some(SyntaxKind::NODE_INTERPOL) && ident.to_string() == name {
            let interpol_range = parent?.text_range();
            return Some(InlineTarget::StringInterpol { interpol_range });
        }

        let parent_is_attrpath = parent_kind == Some(SyntaxKind::NODE_ATTRPATH);
        if !parent_is_attrpath && ident.to_string() == name {
            // In `inherit (from) attrs`, parens are already provided by the
            // inherit syntax so no extra wrapping is needed.
            let needs_parens = parent_kind != Some(SyntaxKind::NODE_INHERIT_FROM);
            return Some(InlineTarget::Direct {
                range: node.text_range(),
                needs_parens,
            });
        }
        return None;
    }
    node.children()
        .find_map(|child| find_ident_ref(&child, name))
}

fn bare_inherit_replacement(
    inherit_node: &SyntaxNode,
    name: &str,
    value_node: &SyntaxNode,
) -> Option<String> {
    let inherit = Inherit::cast(inherit_node.clone())?;
    let inherit_text = inherit_node.to_string();
    let prefix = inherit_text
        .find("inherit")
        .map_or("", |index| &inherit_text[..index]);
    let value_text = value_node.to_string();
    let value_text = value_text.trim_start();

    let remaining = inherit
        .attrs()
        .map(|attr| attr.to_string())
        .filter(|attr| attr != name)
        .collect::<Vec<_>>();

    let mut replacement = format!("{prefix}{name} = {value_text};");
    if !remaining.is_empty() {
        replacement.push(' ');
        replacement.push_str("inherit ");
        replacement.push_str(&remaining.join(" "));
        replacement.push(';');
    }

    Some(replacement)
}

fn interpol_replacement(value_node: &SyntaxNode) -> String {
    simple_string_content(value_node).unwrap_or_else(|| format!("${{{value_node}}}"))
}

/// If `value_node` is a plain `"…"` string with no interpolations or escape
/// sequences, return its content (the text between the quotes).
fn simple_string_content(value_node: &SyntaxNode) -> Option<String> {
    if value_node.kind() != SyntaxKind::NODE_STRING {
        return None;
    }
    // Reject strings that contain interpolations
    if value_node
        .children()
        .any(|c| c.kind() == SyntaxKind::NODE_INTERPOL)
    {
        return None;
    }
    let text = value_node.to_string();
    let trimmed = text.trim();
    // Must be a simple double-quoted string (not a `''…''` indented string)
    if !trimmed.starts_with('"') || !trimmed.ends_with('"') || trimmed.len() < 2 {
        return None;
    }
    let content = &trimmed[1..trimmed.len() - 1];
    // Skip strings with backslash escapes to avoid mis-representing them
    if content.contains('\\') {
        return None;
    }
    Some(content.to_string())
}
