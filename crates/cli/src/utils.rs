use crate::LintMap;

use lib::{LINTS, Lint};
use rnix::{Parse, Root, SyntaxElement, WalkEvent};

pub fn lint_map_of(lints: &[&'static dyn Lint]) -> LintMap {
    let mut map = LintMap::new();
    for lint in lints {
        for &m in lint.match_kind() {
            map.entry(m)
                .and_modify(|v: &mut Vec<_>| v.push(*lint))
                .or_insert_with(|| vec![*lint]);
        }
    }
    map
}

#[must_use]
pub fn lint_map() -> LintMap {
    lint_map_of(&LINTS)
}

#[must_use]
pub fn collect_reports(root: &Parse<Root>, lints: &LintMap) -> Vec<lib::Report> {
    reports(root, lints, |_| true).collect()
}

pub fn collect_filtered_reports(
    root: &Parse<Root>,
    lints: &LintMap,
    predicate: impl Fn(&lib::Report) -> bool,
) -> Vec<lib::Report> {
    reports(root, lints, predicate).collect()
}

pub fn find_report(
    root: &Parse<Root>,
    lints: &LintMap,
    predicate: impl Fn(&lib::Report) -> bool,
) -> Option<lib::Report> {
    reports(root, lints, predicate).next()
}

fn reports<'a, P>(
    root: &'a Parse<Root>,
    lints: &'a LintMap,
    predicate: P,
) -> impl Iterator<Item = lib::Report> + 'a
where
    P: Fn(&lib::Report) -> bool + 'a,
{
    root.syntax()
        .preorder_with_tokens()
        .filter_map(move |event| match event {
            WalkEvent::Enter(child) => Some(child),
            WalkEvent::Leave(_) => None,
        })
        .flat_map(move |child| reports_for_element(child, lints))
        .filter(predicate)
}

fn reports_for_element(
    child: SyntaxElement,
    lints: &LintMap,
) -> impl Iterator<Item = lib::Report> + '_ {
    lints.get(&child.kind()).into_iter().flat_map(move |rules| {
        let child = child.clone();
        rules.iter().filter_map(move |rule| rule.validate(&child))
    })
}
