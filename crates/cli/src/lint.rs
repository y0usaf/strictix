use std::io;

use crate::{
    LintMap,
    config::{Check as CheckConfig, ConfFile},
    err::StatixErr,
    traits::WriteDiagnostic,
    utils,
};

use ariadne::{Color, Fmt as _};
use lib::Report;
use rayon::prelude::*;
use rnix::Root;
use vfs::{FileId, VfsEntry};

#[derive(Debug)]
pub struct LintResult {
    pub file_id: FileId,
    pub reports: Vec<Report>,
}

#[must_use]
pub fn lint_with(vfs_entry: &VfsEntry, lints: &LintMap) -> LintResult {
    let file_id = vfs_entry.file_id;
    let source = vfs_entry.contents;
    let parsed = Root::parse(source);

    let error_reports = parsed
        .errors()
        .iter()
        .map(|err: &rnix::parser::ParseError| Report::from_parse_err(err));
    let reports = utils::collect_reports(&parsed, lints)
        .into_iter()
        .chain(error_reports)
        .collect();

    LintResult { file_id, reports }
}

pub fn run(check_config: &CheckConfig) -> Result<(), StatixErr> {
    let mut conf_file = ConfFile::discover_from_target_or_override(
        &check_config.target,
        check_config.conf_path.as_ref(),
    )?;
    conf_file.apply_lint_options();
    // Apply CLI overrides
    if check_config.strict {
        conf_file.set_strict(true);
    }
    if !check_config.enable.is_empty() {
        conf_file.enable_lints(&check_config.enable);
    }
    let lints = conf_file.lints();

    let vfs = check_config.vfs(conf_file.ignore.as_slice())?;
    let file_count = vfs.len();

    let mut stdout = io::stdout();
    let lint = |vfs_entry| lint_with(&vfs_entry, &lints);
    let mut results = vfs
        .par_iter()
        .map(lint)
        .filter(|lr| !lr.reports.is_empty())
        .collect::<Vec<_>>();
    results.sort_by_key(|result| result.file_id);

    if results.is_empty() {
        let files = format!(
            "{} {}",
            file_count,
            if file_count == 1 { "file" } else { "files" }
        );
        eprintln!(
            "{} No issues found across {}",
            "✓".fg(Color::Green),
            files.fg(Color::Fixed(8)),
        );
        return Ok(());
    }

    for r in &results {
        if stdout.write(r, &vfs, check_config.format).is_err() {
            break;
        }
    }

    let warning_count: usize = results.iter().map(|r| r.reports.len()).sum();
    let file_count = results.len();
    eprintln!(
        "\n{} {} {} across {} {}",
        "✗".fg(Color::Red),
        warning_count.to_string().fg(Color::Red),
        if warning_count == 1 {
            "warning"
        } else {
            "warnings"
        },
        file_count.to_string().fg(Color::Yellow),
        if file_count == 1 { "file" } else { "files" },
    );

    Err(StatixErr::LintsFailed)
}
