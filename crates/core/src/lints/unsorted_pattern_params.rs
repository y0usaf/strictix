use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{NodeOrToken, SyntaxElement, SyntaxKind, ast::Pattern};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for function pattern parameters that are not in canonical order.
///
/// ## Why is this bad?
/// Inconsistent parameter ordering across a codebase makes it harder to scan
/// function signatures at a glance. A canonical order reduces cognitive load
/// during review.
///
/// ## Canonical order
/// 1. `config`
/// 2. `lib`
/// 3. `pkgs`
/// 4. All remaining parameters in alphabetical order
///
/// The ellipsis (`...`) is always placed last.
///
/// ## Example
///
/// ```nix
/// { config, pkgs, lib, ... }:
/// ```
///
/// Reorder the parameters:
///
/// ```nix
/// { config, lib, pkgs, ... }:
/// ```
#[lint(
    name = "unsorted_pattern_params",
    note = "Function pattern parameters are not in canonical order",
    code = 35,
    match_with = SyntaxKind::NODE_PATTERN
)]
struct UnsortedPatternParams;

/// Priority tiers for well-known NixOS module parameters.
/// Lower index → appears earlier.
const PRIORITY: &[&str] = &["config", "lib", "pkgs"];

fn sort_key(name: &str) -> (usize, String) {
    if let Some(pos) = PRIORITY.iter().position(|&p| p == name) {
        (pos, String::new())
    } else {
        (PRIORITY.len(), name.to_owned())
    }
}

impl Rule for UnsortedPatternParams {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let pattern = Pattern::cast(node.clone())?;

        let entries: Vec<_> = pattern.pat_entries().collect();
        if entries.len() < 2 {
            return None;
        }

        // Collect (sort_key, full entry text including default)
        let mut named: Vec<((usize, String), String)> = Vec::new();
        for entry in &entries {
            let ident = entry.ident()?;
            let name = ident.to_string();
            let key = sort_key(&name);
            let text = entry.syntax().to_string().trim().to_owned();
            named.push((key, text));
        }

        // Check if already in canonical order
        let is_sorted = named.windows(2).all(|w| w[0].0 <= w[1].0);
        if is_sorted {
            return None;
        }

        // Sort
        let mut sorted = named.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));

        // Reconstruct the pattern text
        let has_ellipsis = pattern.ellipsis_token().is_some();
        let bind = pattern
            .pat_bind()
            .and_then(|b| b.ident().map(|i| i.to_string()));

        let mut inner_parts: Vec<&str> = sorted.iter().map(|(_, text)| text.as_str()).collect();
        if has_ellipsis {
            inner_parts.push("...");
        }
        let inner = inner_parts.join(", ");

        let replacement = match bind {
            Some(ref bind_name) => {
                // Detect whether the original had `name @ { }` or `{ } @ name`
                let orig = pattern.syntax().to_string();
                if orig.trim_start().starts_with('{') {
                    format!("{{ {inner} }} @ {bind_name}")
                } else {
                    format!("{bind_name} @ {{ {inner} }}")
                }
            }
            None => format!("{{ {inner} }}"),
        };

        let at = pattern.syntax().text_range();
        Some(self.report().suggest(
            at,
            "Reorder parameters to canonical order: config, lib, pkgs, then alphabetical",
            Suggestion::with_text(at, replacement),
        ))
    }
}
