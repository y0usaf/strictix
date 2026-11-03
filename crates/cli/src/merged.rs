use std::path::{Path, PathBuf};

use strictix_core::config::LintConfig;
use strictix_core::context::Context;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::fix::{Fix, TextEdit};
use strictix_core::project::ProjectContext;
use strictix_core::rules::{lint_project, Rule, MAX_FIX_PASSES};

use crate::merge::parent;
use crate::split::{restore, Mapped};
use crate::walk::Filter;

pub struct Virtual {
    pub path: PathBuf,
    pub mapped: Mapped,
    pub skipped: bool,
}

pub fn expand(path: &Path, source: &str, filter: &Filter) -> Result<Option<Vec<Virtual>>, String> {
    let Some(restored) = restore(source, None)? else {
        return Ok(None);
    };
    let mut root = path.parent().map(Path::to_path_buf).unwrap_or_default();
    if let Some(out) = &restored.out {
        for _ in parent(out).split('/').filter(|s| !s.is_empty()) {
            root.pop();
        }
    }
    Ok(Some(
        restored
            .files
            .into_iter()
            .map(|(rel, mapped)| {
                let path = root.join(rel);
                Virtual {
                    skipped: filter.skips(&path),
                    path,
                    mapped,
                }
            })
            .collect(),
    ))
}

pub struct MergedRun {
    pub diagnostics: Vec<Diagnostic>,
    pub fixed: Option<String>,
    pub error: Option<String>,
}

fn map_fix(fix: Fix, mapped: &Mapped) -> Option<Fix> {
    let edits = fix
        .edits
        .into_iter()
        .map(|edit| {
            let rebased = edit.replacement.contains("./");
            (!rebased)
                .then(|| mapped.exact(edit.range))
                .flatten()
                .map(|range| TextEdit::new(range, edit.replacement))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Fix {
        label: fix.label,
        edits,
    })
}

fn lint_files(
    files: &[Virtual],
    rules: &[Box<dyn Rule>],
    config: &LintConfig,
    project: Option<&ProjectContext>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for file in files.iter().filter(|file| !file.skipped) {
        let run = lint_project(
            rules,
            &file.mapped.text,
            Some(&file.path),
            config,
            false,
            project,
        );
        for mut diag in run.diagnostics {
            diag.range = file.mapped.range(diag.range);
            if let Some(codec) = &mut diag.codec {
                codec.replace = file.mapped.range(codec.replace);
            }
            diag.fix = diag.fix.and_then(|fix| map_fix(fix, &file.mapped));
            out.push(diag);
        }
    }
    out.sort_by_key(|diag| diag.range.start());
    out
}

#[allow(clippy::too_many_arguments)]
pub fn lint(
    path: &Path,
    source: &str,
    files: &[Virtual],
    rules: &[Box<dyn Rule>],
    config: &LintConfig,
    fix: bool,
    project: Option<&ProjectContext>,
    filter: &Filter,
) -> MergedRun {
    let mut context = Context::new(source.to_string());
    let mut diagnostics = Vec::new();
    let mut error = None;
    let mut passes = 0usize;
    loop {
        let reexpanded;
        let current = if passes == 0 {
            files
        } else {
            match expand(path, context.source(), filter) {
                Ok(Some(files)) => {
                    reexpanded = files;
                    &reexpanded
                }
                Ok(None) => {
                    error = Some("the fixed text is no longer a merged file".to_string());
                    context.rollback_all();
                    break;
                }
                Err(message) => {
                    error = Some(message);
                    context.rollback_all();
                    break;
                }
            }
        };
        let diags = lint_files(current, rules, config, project);
        let edits: Vec<TextEdit> = if fix {
            diags
                .iter()
                .filter_map(|d| d.fix.as_ref())
                .flat_map(|f| f.edits.iter().cloned())
                .collect()
        } else {
            Vec::new()
        };
        if passes == 0 {
            diagnostics = diags;
        }
        if edits.is_empty() {
            break;
        }
        if let Err(err) = context.commit(&edits) {
            context.rollback_all();
            error = Some(format!("{err:?}"));
            break;
        }
        passes += 1;
        if passes >= MAX_FIX_PASSES {
            break;
        }
    }
    let fixed = (context.source() != source).then(|| context.source().to_string());
    MergedRun {
        diagnostics,
        fixed,
        error,
    }
}
