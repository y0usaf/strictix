use std::collections::{HashMap, HashSet};

use crate::{Metadata, Report, Rule, Suggestion, utils};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode,
    ast::{Attr, HasEntry as _, Inherit, LetIn},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for names imported by a `let`-level `inherit` statement that are
/// never referenced in the surrounding `let-in` expression.
///
/// ## Why is this bad?
/// Unused inherited names are dead bindings. They add noise to the file header
/// or local import list and make it harder to see which helpers are actually
/// used.
///
/// ## Example
///
/// ```nix
/// let
///   inherit (lib) mkIf mkOption types;
/// in
///   mkIf cond { }
/// ```
///
/// Remove the unused inherited names:
///
/// ```nix
/// let
///   inherit (lib) mkIf;
/// in
///   mkIf cond { }
/// ```
#[lint(
    name = "unused_inherit",
    note = "Inherited name is never used",
    code = 37,
    match_with = SyntaxKind::NODE_INHERIT
)]
struct UnusedInherit;

impl Rule for UnusedInherit {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let inherit = Inherit::cast(node.clone())?;
        let let_in = node.parent().and_then(LetIn::cast)?;
        let attrs: Vec<_> = inherit.attrs().collect();
        if attrs.is_empty() || inherit.syntax().to_string().contains('#') {
            return None;
        }

        let refs = refs_outside_current_inherit(&let_in, inherit.syntax());
        let unused = unused_ident_attrs(&attrs, &refs);
        if unused.is_empty() {
            return None;
        }

        let unused_names: HashSet<_> = unused.iter().cloned().collect();
        let remaining = attrs
            .iter()
            .filter(|attr| attr_name(attr).is_none_or(|name| !unused_names.contains(&name)))
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>();

        let at = inherit.syntax().text_range();
        let message = unused_message(&unused);

        let suggestion = if remaining.is_empty() {
            Suggestion::with_empty(utils::with_preceding_whitespace(inherit.syntax()))
        } else {
            Suggestion::with_text(at, inherit_text(&inherit, &remaining))
        };

        Some(self.report().suggest(at, message, suggestion))
    }
}

fn refs_outside_current_inherit(let_in: &LetIn, current: &SyntaxNode) -> HashMap<String, usize> {
    let mut refs = let_in
        .body()
        .map_or_else(HashMap::new, |body| utils::ident_ref_counts(body.syntax()));

    for entry in let_in.entries() {
        if entry.syntax().text_range() == current.text_range() {
            continue;
        }
        merge_counts(&mut refs, utils::ident_ref_counts(entry.syntax()));
    }

    refs
}

fn merge_counts(into: &mut HashMap<String, usize>, from: HashMap<String, usize>) {
    for (name, count) in from {
        *into.entry(name).or_insert(0) += count;
    }
}

fn unused_ident_attrs(attrs: &[Attr], refs: &HashMap<String, usize>) -> Vec<String> {
    attrs
        .iter()
        .filter_map(attr_name)
        .filter(|name| refs.get(name).copied().unwrap_or(0) == 0)
        .collect()
}

fn attr_name(attr: &Attr) -> Option<String> {
    let Attr::Ident(ident) = attr else {
        return None;
    };
    Some(ident.to_string())
}

fn inherit_text(inherit: &Inherit, attrs: &[String]) -> String {
    let attrs = attrs.join(" ");
    inherit.from().map_or_else(
        || format!("inherit {attrs};"),
        |from| format!("inherit {from} {attrs};"),
    )
}

fn unused_message(names: &[String]) -> String {
    let formatted = format_names(names);
    if names.len() == 1 {
        format!("{formatted} is inherited but never used")
    } else {
        format!("{formatted} are inherited but never used")
    }
}

fn format_names(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
