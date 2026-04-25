use std::{borrow::Cow, path::Path};

use crate::{
    LintMap,
    config::{
        FixOut, Single as SingleConfig, {ConfFile, Fix as FixConfig},
    },
    err::{FixErr, StatixErr},
};

use rnix::TextRange;
use similar::TextDiff;

mod all;
use all::all_with;

mod single;
use single::single;

type Source<'a> = Cow<'a, str>;

pub struct FixResult<'a> {
    pub src: Source<'a>,
    pub fixed: Vec<Fixed>,
    pub lints: &'a LintMap,
}

#[derive(Debug, Clone)]
pub struct Fixed {
    pub at: TextRange,
    pub code: u32,
}

impl<'a> FixResult<'a> {
    fn empty(src: Source<'a>, lints: &'a LintMap) -> Self {
        Self {
            src,
            fixed: Vec::new(),
            lints,
        }
    }
}

pub fn run_all(fix_config: &FixConfig) -> Result<(), StatixErr> {
    let mut conf_file = ConfFile::discover_from_target_or_override(
        &fix_config.target,
        fix_config.conf_path.as_ref(),
    )?;
    conf_file.apply_lint_options();
    // Apply CLI overrides
    if fix_config.strict {
        conf_file.set_strict(true);
    }
    if !fix_config.enable.is_empty() {
        conf_file.enable_lints(&fix_config.enable);
    }
    let vfs = fix_config.vfs(conf_file.ignore.as_slice())?;

    let lints = conf_file.lints();

    for entry in vfs.iter() {
        match (fix_config.out(), all_with(entry.contents, &lints)) {
            (FixOut::Diff, fix_result) => {
                let src = fix_result
                    .map(|r| r.src)
                    .unwrap_or(Cow::Borrowed(entry.contents));
                let text_diff = TextDiff::from_lines(entry.contents, &src);
                let old_file = format!("{}", entry.file_path.display());
                let new_file = format!("{} [fixed]", entry.file_path.display());
                println!(
                    "{}",
                    text_diff
                        .unified_diff()
                        .context_radius(4)
                        .header(&old_file, &new_file)
                );
            }
            (FixOut::Stream, fix_result) => {
                let src = fix_result
                    .map(|r| r.src)
                    .unwrap_or(Cow::Borrowed(entry.contents));
                println!("{}", &src);
            }
            (FixOut::Write, Some(fix_result)) => {
                let path = entry.file_path;
                std::fs::write(path, &*fix_result.src).map_err(FixErr::InvalidPath)?;
            }
            _ => (),
        }
    }
    Ok(())
}

pub fn run_single(single_config: &SingleConfig) -> Result<(), StatixErr> {
    let conf_file = ConfFile::discover_from_target_or_override(
        single_config
            .target
            .as_deref()
            .unwrap_or_else(|| Path::new(".")),
        single_config.conf_path.as_ref(),
    )?;
    conf_file.apply_lint_options();
    let lints = conf_file.lints();
    let vfs = single_config.vfs()?;
    let entry = vfs
        .iter()
        .next()
        .ok_or(FixErr::InvalidPath(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "empty VFS",
        )))?;
    let path = entry.file_path.display().to_string();
    let original_src = entry.contents;
    let (line, col) = single_config.position;

    match (single_config.out(), single(line, col, original_src, &lints)) {
        (FixOut::Diff, single_result) => {
            let fixed_src = single_result
                .map(|r| r.src)
                .unwrap_or(Cow::Borrowed(original_src));
            let text_diff = TextDiff::from_lines(original_src, &fixed_src);
            let old_file = &path;
            let new_file = format!("{} [fixed]", &path);
            println!(
                "{}",
                text_diff
                    .unified_diff()
                    .context_radius(4)
                    .header(old_file, &new_file)
            );
        }
        (FixOut::Stream, single_result) => {
            let src = single_result
                .map(|r| r.src)
                .unwrap_or(Cow::Borrowed(original_src));
            println!("{}", &src);
        }
        (FixOut::Write, Ok(single_result)) => {
            let path = entry.file_path;
            std::fs::write(path, &*single_result.src).map_err(FixErr::InvalidPath)?;
        }
        (_, Err(e)) => return Err(e.into()),
    }
    Ok(())
}
