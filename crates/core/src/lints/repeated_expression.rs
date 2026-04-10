use crate::{Metadata, Report, Rule, utils};

use macros::lint;
use rnix::{NodeOrToken, SyntaxElement, SyntaxKind, SyntaxNode, TextRange};
use rowan::ast::AstNode as _;
use std::collections::{HashMap, HashSet};

/// ## What it does
/// Checks for attribute-access expressions with a common prefix of at least
/// 3 components (e.g. `pkgs.hello.meta`) that appear more than once within
/// the same `let-in` expression, **excluding** occurrences inside string
/// interpolations.
///
/// Selects that only occur inside `${…}` are intentionally ignored: swapping
/// `${config.user.name}` for `${name}` saves almost nothing and the suggestion
/// would fire constantly in legitimate config code.
///
/// ## Why is this bad?
/// Repeating the same sub-expression adds noise and makes future changes
/// error-prone. Giving the expression a name via a `let` binding improves
/// readability and reduces the risk of the copies diverging. This lint is the
/// counterpart to `single_use_let`.
///
/// Shallow prefixes (2 components like `config.user`) are not flagged since
/// extracting them rarely improves readability when the suffixes diverge
/// (e.g. `.name` vs `.appearance.wallust`).
///
/// ## Example
///
/// ```nix
/// let
///   a = pkgs.hello.meta.description;
///   b = pkgs.hello.meta.license;
/// in
///   null
/// ```
///
/// Extract the repeated sub-expression:
///
/// ```nix
/// let
///   meta = pkgs.hello.meta;
///   a = meta.description;
///   b = meta.license;
/// in
///   null
/// ```
#[lint(
    name = "repeated_expression",
    note = "Expression repeated; consider extracting into a let binding",
    code = 34,
    match_with = SyntaxKind::NODE_LET_IN
)]
struct RepeatedExpression;

impl Rule for RepeatedExpression {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };
        // Confirm this is a let-in node.
        rnix::ast::LetIn::cast(node.clone())?;

        // Collect all attribute-access (select) expressions in the subtree,
        // excluding those inside string interpolations.
        // The boolean tracks whether the entry was promoted to its enclosing
        // application expression (true) or is a plain select (false).
        let mut selects: Vec<(String, TextRange, bool)> = Vec::new();
        collect_selects(node, &mut selects);

        if selects.is_empty() {
            return None;
        }

        // For each select node, generate all prefixes of length ≥ 2 (i.e.
        // containing at least one dot). Map each prefix to the ranges of the
        // select nodes that contain it.
        //
        // rnix represents `a.b.c.d` as a flat NODE_SELECT (set=`a`,
        // attrpath=`b.c.d`), so `a.b.c` does not appear as a sub-node.
        // We detect common prefixes by splitting the text on `.`.
        let mut prefix_ranges: HashMap<String, Vec<TextRange>> = HashMap::new();
        // Track keys that originate from application expressions so we can
        // relax the component-count filter for them.
        let mut app_keys: HashSet<String> = HashSet::new();

        for (raw_text, range, is_app) in &selects {
            let text = utils::normalize_select(raw_text);
            if *is_app {
                // Application expressions are registered as exact-match keys
                // only.  Splitting by `.` would produce meaningless
                // sub-prefixes because the text includes function arguments
                // (e.g. `lib.mkOption { type = ... }`).
                app_keys.insert(text.clone());
                prefix_ranges.entry(text).or_default().push(*range);
                continue;
            }
            let parts: Vec<&str> = text.split('.').collect();
            if parts.len() < 2 {
                continue;
            }
            let mut prefix = String::new();
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    prefix.push('.');
                }
                prefix.push_str(part);
                // Only record prefixes with at least two components.
                if i >= 1 {
                    prefix_ranges
                        .entry(prefix.clone())
                        .or_default()
                        .push(*range);
                }
            }
        }

        // Identify prefixes that appear in 2+ select nodes AND have at least 3
        // components. Requiring 3 components avoids false positives for shallow
        // prefixes like `config.user` where the suffixes diverge significantly
        // (e.g. `.name` vs `.appearance.wallust`).
        // Application-derived keys bypass the component-count requirement
        // because they are already full expressions, not dot-split prefixes.
        let repeated: HashSet<String> = prefix_ranges
            .iter()
            .filter(|(prefix, ranges)| {
                if ranges.len() < 2 {
                    return false;
                }
                if app_keys.contains(prefix.as_str()) {
                    return true;
                }
                prefix.split('.').count() >= 3
            })
            .map(|(p, _)| p.clone())
            .collect();

        if repeated.is_empty() {
            return None;
        }

        let mut report = self.report();

        // Only report the longest (most specific) repeated prefix to avoid
        // redundant diagnostics. E.g. if `pkgs.hello.meta` is repeated, skip
        // reporting `pkgs.hello` separately.
        let mut to_report: Vec<(&String, &Vec<TextRange>)> = Vec::new();
        for prefix in &repeated {
            let is_subsumed = repeated.iter().any(|other| {
                other != prefix
                    && other.starts_with(prefix.as_str())
                    && other.as_bytes().get(prefix.len()) == Some(&b'.')
            });
            if is_subsumed {
                continue;
            }
            to_report.push((prefix, &prefix_ranges[prefix]));
        }

        // Drop prefixes whose diagnostic ranges are ALL strictly contained
        // within another reported prefix's ranges.  This prevents overlapping
        // ariadne labels when, e.g., every occurrence of `lib.types.str` sits
        // inside a repeated `lib.mkOption { type = lib.types.str; … }` call.
        let to_report: Vec<_> = to_report
            .iter()
            .filter(|(_, ranges)| {
                !to_report.iter().any(|(_, other_ranges)| {
                    !std::ptr::eq(*ranges, *other_ranges)
                        && ranges.iter().all(|r| {
                            other_ranges
                                .iter()
                                .any(|or| *or != *r && or.contains_range(*r))
                        })
                })
            })
            .collect();

        for (prefix, ranges) in &to_report {
            let message = format!("`{prefix}` is repeated; consider extracting into a let binding");
            for &range in *ranges {
                report = report.diagnostic(range, &message);
            }
        }

        (!report.diagnostics.is_empty()).then_some(report)
    }
}

fn collect_selects(node: &SyntaxNode, result: &mut Vec<(String, TextRange, bool)>) {
    // Do not descend into strings. A select like `config.user.name` that only
    // appears inside `${config.user.name}` would require `${name}` after
    // extraction – almost no improvement.
    if node.kind() == SyntaxKind::NODE_STRING {
        return;
    }
    if node.kind() == SyntaxKind::NODE_SELECT {
        // When a select is the function of an application (e.g. `builtins.match
        // "pat" val`), comparing bare function references produces false positives:
        // `builtins.match "a" x` and `builtins.match "b" y` share the select
        // `builtins.match` but are completely different expressions.  Instead,
        // climb to the outermost application in the call chain and use the full
        // call expression as the key.  If the complete call appears verbatim more
        // than once (e.g. `pkgs.stdenv.mkDerivation { }` twice) it will still be
        // reported; if the calls differ only in their arguments they will not.
        let in_fn_position = node
            .parent()
            .filter(|p| p.kind() == SyntaxKind::NODE_APPLY)
            .and_then(|p| p.children().next())
            .is_some_and(|first| first == *node);
        if in_fn_position {
            // Walk up through consecutive function-position NODE_APPLYs.
            let Some(mut current) = node.parent() else {
                return;
            };
            while let Some(parent) = current.parent() {
                if parent.kind() == SyntaxKind::NODE_APPLY
                    && parent.children().next().is_some_and(|c| c == current)
                {
                    current = parent;
                } else {
                    break;
                }
            }
            result.push((current.to_string(), current.text_range(), true));
        } else {
            result.push((node.to_string(), node.text_range(), false));
        }
    }
    for child in node.children() {
        collect_selects(&child, result);
    }
}
