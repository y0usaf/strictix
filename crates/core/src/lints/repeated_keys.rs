use std::fmt::Write as _;

use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind, TextRange,
    ast::{Attr, AttrSet, AttrpathValue, Entry, HasEntry as _},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for keys in attribute sets with repetitive keys, and suggests using
/// an attribute set instead.
///
/// ## Why is this bad?
/// Avoiding repetetion helps improve readibility.
///
/// ## Example
/// ```nix
/// {
///   foo.a = 1;
///   foo.b = 2;
///   foo.c = 3;
/// }
/// ```
///
/// Don't repeat.
/// ```nix
/// {
///   foo = {
///     a = 1;
///     b = 2;
///     c = 3;
///   };
/// }
/// ```

#[lint(
    name = "repeated_keys",
    note = "Avoid repeated keys in attribute sets",
    code = 20,
    match_with = SyntaxKind::NODE_ATTRPATH_VALUE
)]
struct RepeatedKeys;

impl Rule for RepeatedKeys {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let attrpath_value = AttrpathValue::cast(node.clone())?;
        let attrpath = attrpath_value.attrpath()?;
        let mut components = attrpath.attrs();
        let first_component = components.next()?;

        let first_component_text = match &first_component {
            Attr::Ident(ident) => ident.to_string(),
            Attr::Str(s) => s.to_string(),
            _ => return None,
        };

        // ensure that there are >1 components
        components.next()?;

        let parent_node = node.parent()?;
        let parent_attr_set = AttrSet::cast(parent_node)?;

        if parent_attr_set.rec_token().is_some() {
            return None;
        }

        let occurrences = parent_attr_set
            .entries()
            .filter_map(|entry| repeated_key_occurrence(&entry, first_component_text.as_str()))
            .collect::<Vec<_>>();

        if occurrences.first()?.0 != attrpath.syntax().text_range() {
            return None;
        }

        if occurrences.len() < 3 {
            return None;
        }

        let mut iter = occurrences.iter();

        let (first_annotation, first_subkey, _) = iter.next().unwrap();
        let first_message = format!("The key `{first_component_text}` is first assigned here ...");

        let (second_annotation, second_subkey, _) = iter.next().unwrap();
        let second_message = "... repeated here ...";

        let (third_annotation, third_subkey, _) = iter.next().unwrap();
        let third_message = {
            let remaining_occurrences = iter.count();
            let mut message = match remaining_occurrences {
                0 => "... and here.".to_string(),
                1 => "... and here (`1` occurrence omitted).".to_string(),
                n => format!("... and here (`{n}` occurrences omitted)."),
            };
            write!(
                message,
                " Try `{first_component_text} = {{ {}=...; {}=...; {}=...; }}` instead.",
                first_subkey.join("."),
                second_subkey.join("."),
                third_subkey.join("."),
            )
            .unwrap();
            message
        };

        let mut report = self
            .report()
            .diagnostic(*first_annotation, first_message)
            .diagnostic(*second_annotation, second_message);

        if let Some(rewrite) =
            grouped_rewrite(&parent_attr_set, &first_component_text, &occurrences)
        {
            report = report.suggest(
                rewrite.first_range,
                third_message.clone(),
                Suggestion::with_text(rewrite.first_range, rewrite.first_replacement),
            );

            for removal in rewrite.removals {
                report = report.suggest(
                    removal,
                    third_message.clone(),
                    Suggestion::with_empty(removal),
                );
            }
        } else {
            report = report.diagnostic(*third_annotation, third_message);
        }

        Some(report)
    }
}

type Occurrence = (TextRange, Vec<String>, AttrpathValue);
struct GroupedRewrite {
    first_range: TextRange,
    first_replacement: String,
    removals: Vec<TextRange>,
}

fn repeated_key_occurrence(entry: &Entry, first_component: &str) -> Option<Occurrence> {
    let Entry::AttrpathValue(attrpath_value) = entry else {
        return None;
    };

    let attrpath = attrpath_value.attrpath()?;
    let mut components = attrpath.attrs();
    let first = components.next()?;
    let first_text = match first {
        Attr::Ident(ident) => ident.to_string(),
        Attr::Str(s) => s.to_string(),
        _ => return None,
    };

    if first_text != first_component {
        return None;
    }

    let suffix = components.map(|attr| attr.to_string()).collect::<Vec<_>>();
    if suffix.is_empty() {
        return None;
    }

    Some((
        attrpath.syntax().text_range(),
        suffix,
        attrpath_value.clone(),
    ))
}

fn grouped_rewrite(
    parent_attr_set: &AttrSet,
    first_component: &str,
    occurrences: &[Occurrence],
) -> Option<GroupedRewrite> {
    let indent = indentation_before(occurrences.first()?.2.syntax());
    let inner_indent = format!("{indent}  ");
    let mut common_prefix = longest_common_prefix(
        &occurrences
            .iter()
            .map(|(_, suffix, _)| suffix.clone())
            .collect::<Vec<_>>(),
    );

    while !common_prefix.is_empty()
        && (parent_attr_set
            .entries()
            .any(|entry| direct_assignment(&entry, first_component, &common_prefix))
            || occurrences
                .iter()
                .any(|(_, suffix, _)| suffix.len() == common_prefix.len())
            || has_prefix_conflicts(
                &occurrences
                    .iter()
                    .map(|(_, suffix, _)| suffix[common_prefix.len()..].to_vec())
                    .collect::<Vec<_>>(),
            ))
    {
        common_prefix.pop();
    }

    if parent_attr_set
        .entries()
        .any(|entry| direct_assignment(&entry, first_component, &common_prefix))
        || has_prefix_conflicts(
            &occurrences
                .iter()
                .map(|(_, suffix, _)| suffix[common_prefix.len()..].to_vec())
                .collect::<Vec<_>>(),
        )
    {
        return None;
    }

    let mut replacement = format!("{first_component} = {{\n");
    let mut nested_indent = inner_indent.clone();
    for component in &common_prefix {
        writeln!(replacement, "{nested_indent}{component} = {{").ok()?;
        nested_indent.push_str("  ");
    }

    for (_, suffix, occurrence) in occurrences {
        let rewritten = rewritten_entry_text(occurrence, suffix, common_prefix.len())?;
        replacement.push_str(&indent_block(&rewritten, &nested_indent));
        replacement.push('\n');
    }

    for _ in &common_prefix {
        nested_indent.truncate(nested_indent.len().saturating_sub(2));
        writeln!(replacement, "{nested_indent}}};").ok()?;
    }
    write!(replacement, "{indent}}};").ok()?;

    Some(GroupedRewrite {
        first_range: occurrences.first()?.2.syntax().text_range(),
        first_replacement: replacement,
        removals: occurrences
            .iter()
            .skip(1)
            .map(|(_, _, occurrence)| removal_range(occurrence.syntax()))
            .collect(),
    })
}

fn direct_assignment(entry: &Entry, first_component: &str, suffix: &[String]) -> bool {
    let Entry::AttrpathValue(attrpath_value) = entry else {
        return false;
    };

    let Some(attrpath) = attrpath_value.attrpath() else {
        return false;
    };
    let mut components = attrpath.attrs();
    let Some(first) = components.next() else {
        return false;
    };
    let first_text = match first {
        Attr::Ident(ident) => ident.to_string(),
        Attr::Str(s) => s.to_string(),
        _ => return false,
    };

    if first_text != first_component {
        return false;
    }

    let path = components.map(|attr| attr.to_string()).collect::<Vec<_>>();
    path == suffix
}

fn indentation_before(node: &rnix::SyntaxNode) -> String {
    node.prev_sibling_or_token()
        .filter(|token| token.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map_or_else(String::new, |token| {
            token
                .to_string()
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .to_string()
        })
}

fn removal_range(node: &rnix::SyntaxNode) -> TextRange {
    let start = node
        .prev_sibling_or_token()
        .filter(|token| token.kind() == SyntaxKind::TOKEN_WHITESPACE)
        .map_or(node.text_range().start(), |token| {
            token.text_range().start()
        });
    TextRange::new(start, node.text_range().end())
}

fn rewritten_entry_text(
    occurrence: &AttrpathValue,
    suffix: &[String],
    common_prefix_len: usize,
) -> Option<String> {
    let entry_text = occurrence.syntax().to_string();
    let attrpath_text = occurrence.attrpath()?.to_string();
    let remaining = suffix[common_prefix_len..].join(".");

    Some(entry_text.replacen(&attrpath_text, &remaining, 1))
}

fn indent_block(text: &str, indent: &str) -> String {
    text.lines()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn longest_common_prefix(paths: &[Vec<String>]) -> Vec<String> {
    let Some(first) = paths.first() else {
        return Vec::new();
    };

    let mut prefix = Vec::new();
    for (index, component) in first.iter().enumerate() {
        if paths.iter().all(|path| path.get(index) == Some(component)) {
            prefix.push(component.clone());
        } else {
            break;
        }
    }

    prefix
}

fn has_prefix_conflicts(paths: &[Vec<String>]) -> bool {
    paths.iter().enumerate().any(|(index, path)| {
        path.is_empty()
            || paths
                .iter()
                .enumerate()
                .any(|(other_index, other)| index != other_index && is_prefix(path, other))
    })
}

fn is_prefix(prefix: &[String], path: &[String]) -> bool {
    prefix.len() <= path.len() && path.starts_with(prefix)
}
