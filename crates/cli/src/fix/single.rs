use std::{borrow::Cow, convert::TryFrom};

use lib::Report;
use rnix::{Root, TextSize};

use crate::{LintMap, err::SingleFixErr, fix::Source, utils};

pub struct SingleFixResult<'a> {
    pub src: Source<'a>,
}

fn pos_to_byte(line: usize, col: usize, src: &str) -> Result<TextSize, SingleFixErr> {
    let mut byte: TextSize = TextSize::of("");
    for (l, _) in src
        .split_inclusive('\n')
        .zip(1..)
        .take_while(|(_, i)| *i < line)
    {
        byte += TextSize::of(l);
    }
    byte += TextSize::try_from(col).map_err(|_| SingleFixErr::Conversion(col))?;

    if usize::from(byte) >= src.len() {
        Err(SingleFixErr::OutOfBounds(line, col))
    } else {
        Ok(byte)
    }
}

fn find(offset: TextSize, src: &str, lints: &LintMap) -> Result<Report, SingleFixErr> {
    // we don't really need the source to form a completely parsed tree
    let parsed = Root::parse(src);

    utils::find_report(&parsed, lints, |report| {
        report.total_suggestion_range().is_some()
            && report
                .total_diagnostic_range()
                .is_some_and(|range| range.contains(offset))
    })
    .ok_or(SingleFixErr::NoOp)
}

pub fn single<'a>(
    line: usize,
    col: usize,
    src: &'a str,
    lints: &LintMap,
) -> Result<SingleFixResult<'a>, SingleFixErr> {
    let mut src = Cow::from(src);
    let offset = pos_to_byte(line, col, &src)?;
    let report = find(offset, &src, lints)?;

    report.apply(src.to_mut());

    Ok(SingleFixResult { src })
}
