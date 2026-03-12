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

        let Attr::Ident(first_component_ident) = first_component else {
            return None;
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
            .filter_map(|entry| repeated_key_occurrence(&entry, first_component_ident.to_string().as_str()))
            .collect::<Vec<_>>();

        if occurrences.first()?.0 != attrpath.syntax().text_range() {
            return None;
        }

        if occurrences.len() < 3 {
            return None;
        }

        let mut iter = occurrences.iter();

        let (first_annotation, first_subkey, _) = iter.next().unwrap();
        let first_message = format!("The key `{first_component_ident}` is first assigned here ...");

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
                " Try `{first_component_ident} = {{ {first_subkey}=...; {second_subkey}=...; {third_subkey}=...; }}` instead."
            )
            .unwrap();
            message
        };

        let mut report = self
            .report()
            .diagnostic(*first_annotation, first_message)
            .diagnostic(*second_annotation, second_message);

        if let Some((fix_range, replacement)) =
            grouped_rewrite(&parent_attr_set, &first_component_ident.to_string(), &occurrences)
        {
            report = report.suggest(
                fix_range,
                third_message,
                Suggestion::with_text(fix_range, replacement),
            );
        } else {
            report = report.diagnostic(*third_annotation, third_message);
        }

        Some(report)
    }
}

type Occurrence = (TextRange, String, AttrpathValue);

fn repeated_key_occurrence(entry: &Entry, first_component: &str) -> Option<Occurrence> {
    let Entry::AttrpathValue(attrpath_value) = entry else {
        return None;
    };

    let attrpath = attrpath_value.attrpath()?;
    let mut components = attrpath.attrs();
    let first = components.next()?;
    let Attr::Ident(ident) = first else {
        return None;
    };

    if ident.to_string() != first_component {
        return None;
    }

    let suffix = components.map(|attr| attr.to_string()).collect::<Vec<_>>().join(".");
    if suffix.is_empty() {
        return None;
    }

    Some((attrpath.syntax().text_range(), suffix, attrpath_value.clone()))
}

fn grouped_rewrite(
    parent_attr_set: &AttrSet,
    first_component: &str,
    occurrences: &[Occurrence],
) -> Option<(TextRange, String)> {
    let entries = parent_attr_set.entries().collect::<Vec<_>>();
    let mut positions = Vec::with_capacity(occurrences.len());

    for (_, _, occurrence) in occurrences {
        let position = entries.iter().position(|entry| {
            let Entry::AttrpathValue(attrpath_value) = entry else {
                return false;
            };
            attrpath_value.syntax().text_range() == occurrence.syntax().text_range()
        })?;
        positions.push(position);
    }

    if !positions.windows(2).all(|window| window[1] == window[0] + 1) {
        return None;
    }

    let range = TextRange::new(
        occurrences.first()?.2.syntax().text_range().start(),
        occurrences.last()?.2.syntax().text_range().end(),
    );

    if has_comments_in_range(parent_attr_set, range) {
        return None;
    }

    if parent_attr_set.entries().any(|entry| direct_assignment(&entry, first_component)) {
        return None;
    }

    let indent = indentation_before(occurrences.first()?.2.syntax());
    let inner_indent = format!("{indent}  ");

    let mut replacement = format!("{first_component} = {{\n");
    for (_, suffix, occurrence) in occurrences {
        let value = occurrence.value()?.syntax().to_string();
        if value.contains('\n') {
            return None;
        }
        writeln!(replacement, "{inner_indent}{suffix} = {value};").ok()?;
    }
    write!(replacement, "{indent}}};").ok()?;

    Some((range, replacement))
}

fn direct_assignment(entry: &Entry, first_component: &str) -> bool {
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
    let Attr::Ident(ident) = first else {
        return false;
    };

    ident.to_string() == first_component && components.next().is_none()
}

fn has_comments_in_range(parent_attr_set: &AttrSet, range: TextRange) -> bool {
    parent_attr_set.syntax().children_with_tokens().any(|child| {
        child.kind() == SyntaxKind::TOKEN_COMMENT
            && child.text_range().start() >= range.start()
            && child.text_range().end() <= range.end()
    })
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
