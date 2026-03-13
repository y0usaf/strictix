use std::borrow::Cow;

use crate::{
    LintMap,
    fix::{FixResult, Fixed},
    utils,
};
use lib::Report;
use rnix::{Root, parser::ParseError as RnixParseErr};

fn collect_fixes(source: &str, lints: &LintMap) -> Result<Vec<Report>, RnixParseErr> {
    let parsed = Root::parse(source);
    let _ = parsed.clone().ok()?;

    Ok(utils::collect_filtered_reports(&parsed, lints, |report| {
        report.total_suggestion_range().is_some()
    }))
}

fn reorder(mut reports: Vec<Report>) -> Vec<Report> {
    use std::collections::VecDeque;

    reports.sort_by(|a, b| {
        let a_end = a.range().map(rnix::TextRange::end);
        let b_end = b.range().map(rnix::TextRange::end);
        a_end.cmp(&b_end)
    });

    reports
        .into_iter()
        .fold(VecDeque::new(), |mut deque: VecDeque<Report>, new_elem| {
            let front = deque.front();
            let new_range = new_elem.range();
            if let Some(Some(front_range)) = front.map(lib::Report::range) {
                if let Some(new_range) = new_range {
                    // TextRange::end() is exclusive, so start >= end means
                    // ranges are non-overlapping (abutting ranges are safe to apply together)
                    if new_range.start() >= front_range.end() {
                        deque.push_front(new_elem);
                    }
                } else {
                    deque.push_front(new_elem);
                }
            } else {
                deque.push_front(new_elem);
            }
            deque
        })
        .into()
}

impl<'a> Iterator for FixResult<'a> {
    type Item = FixResult<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        let all_reports = collect_fixes(&self.src, self.lints).ok()?;
        if all_reports.is_empty() {
            return None;
        }

        let reordered = reorder(all_reports);
        let fixed = reordered
            .iter()
            .filter_map(|r| {
                Some(Fixed {
                    at: r.range()?,
                    code: r.code,
                })
            })
            .collect::<Vec<_>>();
        for report in reordered {
            report.apply(self.src.to_mut());
        }

        Some(FixResult {
            src: self.src.clone(),
            fixed,
            lints: self.lints,
        })
    }
}

const MAX_FIX_PASSES: usize = 10;

pub fn all_with<'a>(src: &'a str, lints: &'a LintMap) -> Option<FixResult<'a>> {
    let src = Cow::from(src);
    let _ = Root::parse(&src).ok().ok()?;
    let initial = FixResult::empty(src, lints);
    let mut last = None;
    let mut passes = 0usize;
    for result in initial.into_iter().take(MAX_FIX_PASSES) {
        passes += 1;
        last = Some(result);
    }
    if passes == MAX_FIX_PASSES {
        eprintln!(
            "warning: fix did not converge after {MAX_FIX_PASSES} passes; some fixes may be incomplete"
        );
    }
    last
}
