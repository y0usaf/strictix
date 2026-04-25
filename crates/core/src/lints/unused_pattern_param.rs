use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{Metadata, Report, Rule, Suggestion, utils};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Lambda, PatEntry, Pattern},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for unused named parameters in variadic function patterns.
///
/// ## Why is this bad?
/// Top-level Nix files often use a variadic pattern as their import list, for
/// example `{ lib, pkgs, ... }:`. Keeping names that are never referenced makes
/// the file header noisier than necessary.
///
/// ## Safety
/// This lint only rewrites variadic patterns (`...`) so extra arguments remain
/// accepted by default. Set `[lints.unused_pattern_param].remove_ellipsis = true`
/// to also remove the ellipsis when the pattern is rewritten, closing the
/// function interface. Closed patterns are intentionally left alone because
/// removing a parameter there can make existing calls fail due to unexpected
/// extra arguments.
///
/// ## Example
///
/// ```nix
/// { config, lib, pkgs, ... }: config
/// ```
///
/// Remove the unused parameters while keeping `...`:
///
/// ```nix
/// { config, ... }: config
/// ```
#[lint(
    name = "unused_pattern_param",
    note = "Function pattern parameter is never used",
    code = 38,
    match_with = SyntaxKind::NODE_PATTERN,
    default_enabled = false
)]
struct UnusedPatternParam;

static REMOVE_ELLIPSIS: AtomicBool = AtomicBool::new(false);

pub fn set_remove_ellipsis(remove: bool) {
    REMOVE_ELLIPSIS.store(remove, Ordering::Relaxed);
}

fn remove_ellipsis() -> bool {
    REMOVE_ELLIPSIS.load(Ordering::Relaxed)
}

impl Rule for UnusedPatternParam {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let pattern = Pattern::cast(node.clone())?;
        pattern.ellipsis_token()?;

        let entries: Vec<_> = pattern.pat_entries().collect();
        if entries.is_empty() || pattern.syntax().to_string().contains('#') {
            return None;
        }

        let lambda = node.ancestors().find_map(Lambda::cast)?;
        let body = lambda.body()?;

        let body_refs = utils::ident_ref_counts(body.syntax());
        if pattern
            .pat_bind()
            .and_then(|bind| bind.ident())
            .is_some_and(|ident| body_refs.contains_key(&ident.to_string()))
        {
            return None;
        }

        let mut refs = body_refs;
        for entry in &entries {
            if let Some(default) = entry.default() {
                merge_counts(&mut refs, utils::ident_ref_counts(default.syntax()));
            }
        }

        let unused = unused_entries(&entries, &refs);
        if unused.is_empty() {
            return None;
        }
        let remove_ellipsis = remove_ellipsis();

        let unused_names: HashSet<_> = unused.iter().cloned().collect();
        let remaining = entries
            .iter()
            .filter(|entry| {
                entry
                    .ident()
                    .is_none_or(|ident| !unused_names.contains(&ident.to_string()))
            })
            .map(|entry| entry.syntax().to_string().trim().to_owned())
            .collect::<Vec<_>>();

        let at = pattern.syntax().text_range();
        let message = message(&unused, remove_ellipsis);
        let replacement = pattern_text(&pattern, &remaining, !remove_ellipsis);

        Some(
            self.report()
                .suggest(at, message, Suggestion::with_text(at, replacement)),
        )
    }
}

fn merge_counts(into: &mut HashMap<String, usize>, from: HashMap<String, usize>) {
    for (name, count) in from {
        *into.entry(name).or_insert(0) += count;
    }
}

fn unused_entries(entries: &[PatEntry], refs: &HashMap<String, usize>) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| entry.ident().map(|ident| ident.to_string()))
        .filter(|name| refs.get(name).copied().unwrap_or(0) == 0)
        .collect()
}

fn pattern_text(pattern: &Pattern, remaining: &[String], keep_ellipsis: bool) -> String {
    let mut inner_parts = remaining.to_vec();
    if keep_ellipsis {
        inner_parts.push("...".to_owned());
    }

    let pattern_only = if inner_parts.is_empty() {
        "{ }".to_owned()
    } else {
        format!("{{ {} }}", inner_parts.join(", "))
    };

    pattern
        .pat_bind()
        .and_then(|bind| bind.ident().map(|ident| ident.to_string()))
        .map_or(pattern_only.clone(), |bind_name| {
            let original = pattern.syntax().to_string();
            if original.trim_start().starts_with('{') {
                format!("{pattern_only} @ {bind_name}")
            } else {
                format!("{bind_name} @ {pattern_only}")
            }
        })
}

fn message(unused: &[String], remove_ellipsis: bool) -> String {
    if remove_ellipsis {
        format!(
            "{}; `...` is configured to be removed",
            unused_message(unused)
        )
    } else {
        unused_message(unused)
    }
}

fn unused_message(names: &[String]) -> String {
    let formatted = format_names(names);
    if names.len() == 1 {
        format!("{formatted} pattern parameter is never used")
    } else {
        format!("{formatted} pattern parameters are never used")
    }
}

fn format_names(names: &[String]) -> String {
    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
