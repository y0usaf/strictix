//! strictix command-line interface.
//!
//! The CLI is the imperative shell around the functional core: it owns
//! argument parsing, file discovery, config loading, worker-thread
//! orchestration, and rendering. Linting itself happens entirely inside
//! the core + lints crates, which never touch host state.
//!
//! Orchestration: parse args, load config, collect files, then process
//! every file on its own worker thread (std::thread::scope - one
//! thread per file is fine at linter scale). The rule registry is built
//! exactly once and shared across workers; rules are Sync and the
//! schema rule caches its options.json parse in a process-wide
//! OnceLock, so sharing is both safe and correct.

mod args;
mod config;
mod format;
mod import;
mod merge;
mod merged;
mod render;
mod split;
mod walk;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{Args, Command, Format};
use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::rules::Rule;

/// Print a usage error to stderr and exit 1.
fn usage_error(message: &str) -> ExitCode {
    eprintln!("error: {message}\n\n{}", args::usage());
    ExitCode::FAILURE
}

/// The lowercase severity word shared by list/explain rendering.
fn severity_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match args::parse(&argv) {
        Ok(parsed) => parsed,
        Err(message) => return usage_error(&message),
    };
    if parsed.help {
        print!("{}", args::usage());
        return ExitCode::SUCCESS;
    }
    if parsed.version {
        println!("strictix {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let Some(command) = parsed.command else {
        return usage_error(
            "no command given (expected check, fix, explain, list, merge, split, or import)",
        );
    };
    match command {
        Command::Check => run_check(&parsed, false),
        Command::Fix => run_check(&parsed, true),
        Command::Explain => run_explain(&parsed),
        Command::List => run_list(),
        Command::Merge => run_merge(&parsed),
        Command::Split => run_split(&parsed),
        Command::Import => run_import(&parsed),
    }
}

fn run_import(args: &Args) -> ExitCode {
    let options = import::Options {
        root: command_root(args),
        paths: args.paths.clone(),
        generators: args.generators.clone(),
        dry_run: args.dry_run,
        split: args.split,
    };
    let report = match import::run(&options) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (language, _) in &report.inlined {
        *counts.entry(language.as_str()).or_default() += 1;
    }
    let languages: Vec<String> = counts
        .iter()
        .map(|(language, count)| format!("{language} {count}"))
        .collect();
    println!(
        "inlined {} files into {} modules ({}); deleted {} files",
        report.inlined.len(),
        report.modules.len(),
        if languages.is_empty() {
            "nothing to convert".to_owned()
        } else {
            languages.join(", ")
        },
        report.deleted.len()
    );
    for (file, reason) in &report.kept {
        println!("kept {}: {reason}", file.display());
    }
    ExitCode::SUCCESS
}

fn command_root(args: &Args) -> PathBuf {
    args.paths
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run_merge(args: &Args) -> ExitCode {
    let (Some(entry), Some(out)) = (&args.entry, &args.out) else {
        return usage_error("merge requires --entry and --out");
    };
    let options = merge::MergeOptions {
        root: command_root(args),
        entry: entry.clone(),
        out: out.clone(),
        expand: args.expand.clone(),
        dry_run: args.dry_run,
    };
    let report = match merge::merge(&options) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "inlined {} files into {out} ({} lines)",
        report.inlined.len(),
        report.out_lines
    );
    for (call, count) in &report.expansions {
        println!("expanded `{call}` into {count} modules");
    }
    for (file, reason) in &report.kept {
        println!("kept {file} as a file: {reason}");
    }
    for name in &report.dropped_bindings {
        println!("removed let binding {name}: every use was expanded");
    }
    for file in &report.keyed_modules {
        println!("keyed module {file}: listed as a module more than once");
    }
    for path in &report.partial_paths {
        println!("sub-path reference {path}: directory not kept whole");
    }
    for file in &report.rewritten {
        println!("rewrote {file}: entry reference now points at {out}");
    }
    for file in &report.verbatim {
        println!("recorded {file} verbatim: the flake formatter would change it");
    }
    for file in &report.unreached {
        println!("left {file} as a file: not reached from {entry}");
    }
    if !args.dry_run && !report.formatted {
        println!("{out} not formatted: the root has no flake formatter for this system");
    }
    let verb = if args.dry_run {
        "would delete"
    } else {
        "deleted"
    };
    let deleted = report.inlined.iter().filter(|file| *file != out).count();
    println!(
        "{verb} {deleted} inlined files; {} other .nix files untouched",
        report.untouched
    );
    ExitCode::SUCCESS
}

fn run_split(args: &Args) -> ExitCode {
    let Some(merged) = &args.merged else {
        return usage_error("split requires --merged");
    };
    let options = split::SplitOptions {
        root: command_root(args),
        merged: merged.clone(),
        dry_run: args.dry_run,
    };
    let report = match split::split(&options) {
        Ok(report) => report,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let verb = if args.dry_run {
        "would restore"
    } else {
        "restored"
    };
    println!("{verb} {} files from {merged}", report.restored.len());
    println!("restored {} expanded calls", report.expansions);
    for file in &report.redirected {
        println!("rewrote {file}: entry reference points back at the entry");
    }
    for note in &report.notes {
        println!("note: {note}");
    }
    ExitCode::SUCCESS
}

/// The list command: one line per rule in the registry.
fn run_list() -> ExitCode {
    let rules = strictix_lints::all_rules();
    for rule in &rules {
        let kind = if rule.node_kind().is_some() {
            "node"
        } else {
            "file"
        };
        println!(
            "{:<24} {:<28} {:<8} {}{}",
            rule.code(),
            rule.name(),
            severity_str(rule.severity()),
            kind,
            if rule.default_enabled() {
                ""
            } else {
                " (opt-in)"
            }
        );
    }
    ExitCode::SUCCESS
}

/// The explain command: a rule's name, code, severity, kind, and
/// description. Unknown code is an error.
fn run_explain(args: &Args) -> ExitCode {
    let Some(code_arg) = args.paths.first() else {
        return usage_error("explain requires a rule code");
    };
    let code = code_arg.to_string_lossy();
    let rules = strictix_lints::all_rules();
    let Some(rule) = rules.iter().find(|r| r.code() == code) else {
        eprintln!("error: unknown rule '{code}'");
        return ExitCode::FAILURE;
    };
    let kind = if rule.node_kind().is_some() {
        "node"
    } else {
        "file"
    };
    println!("code: {}", rule.code());
    println!("name: {}", rule.name());
    println!("severity: {}", severity_str(rule.severity()));
    println!("kind: {kind}");
    println!("default-enabled: {}", rule.default_enabled());
    println!("description: {}", rule.description());
    ExitCode::SUCCESS
}

/// The check and fix commands (one code path; fix_mode toggles writing
/// files back). Returns the process exit code: 1 when any diagnostic
/// was found (check only) or any hard error occurred.
fn run_check(args: &Args, fix_mode: bool) -> ExitCode {
    let config = match resolve_config(args) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let ignore_file = resolve_ignore_file(args);
    let paths: Vec<PathBuf> = if args.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        args.paths.clone()
    };
    let need_scattered = config.is_enabled("scattered-documentation")
        && config
            .enabled
            .iter()
            .any(|c| c == "scattered-documentation");
    let need_prose =
        config.is_enabled("wrapped-prose") && config.enabled.iter().any(|c| c == "wrapped-prose");
    let need_others = need_scattered || need_prose;

    let (files, others, filter) =
        match walk::collect_files_filtered(&paths, ignore_file.as_deref(), need_others) {
            Ok(found) => found,
            Err(message) => {
                eprintln!("error: {message}");
                return ExitCode::FAILURE;
            }
        };

    let color = render::color_enabled();
    let rules = strictix_lints::all_rules();
    let workers = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1)
        .min(8);

    let (mut inputs, warnings): (Vec<Input>, Vec<Option<String>>) =
        if files.len() >= 16 && workers > 1 {
            let next = std::sync::atomic::AtomicUsize::new(0);
            let slots: Vec<std::sync::Mutex<Option<(Input, Option<String>)>>> =
                (0..files.len()).map(|_| std::sync::Mutex::new(None)).collect();
            std::thread::scope(|scope| {
                for _ in 0..workers.min(files.len()) {
                    scope.spawn(|| loop {
                        let idx = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if idx >= files.len() {
                            break;
                        }
                        let path = &files[idx];
                        let source = std::fs::read_to_string(path)
                            .map_err(|err| format!("{}: {err}", path.display()));
                        let mut warn = None;
                        let virtuals = match source
                            .as_deref()
                            .map(|s| merged::expand(path, s, &filter))
                        {
                            Ok(Ok(virtuals)) => virtuals,
                            Ok(Err(message)) => {
                                warn = Some(format!(
                                    "warning: {}: linted as one file, not per merged binding: {message}",
                                    path.display()
                                ));
                                None
                            }
                            Err(_) => None,
                        };
                        let source = match source {
                            Ok(s) => InputSource::Owned(s),
                            Err(e) => InputSource::Err(e),
                        };
                        *slots[idx].lock().unwrap() = Some((
                            Input {
                                path: path.clone(),
                                source,
                                virtuals,
                            },
                            warn,
                        ));
                    });
                }
            });
            slots
                .into_iter()
                .map(|s| s.into_inner().unwrap().unwrap())
                .unzip()
        } else {
            files
                .iter()
                .map(|path| {
                    let source = std::fs::read_to_string(path)
                        .map_err(|err| format!("{}: {err}", path.display()));
                    let mut warn = None;
                    let virtuals = match source.as_deref().map(|s| merged::expand(path, s, &filter)) {
                        Ok(Ok(virtuals)) => virtuals,
                        Ok(Err(message)) => {
                            warn = Some(format!(
                                "warning: {}: linted as one file, not per merged binding: {message}",
                                path.display()
                            ));
                            None
                        }
                        Err(_) => None,
                    };
                    let source = match source {
                        Ok(s) => InputSource::Owned(s),
                        Err(e) => InputSource::Err(e),
                    };
                    (
                        Input {
                            path: path.clone(),
                            source,
                            virtuals,
                        },
                        warn,
                    )
                })
                .unzip()
        };
    for warn in warnings.into_iter().flatten() {
        eprintln!("{warn}");
    }

    let mut project_files: Vec<strictix_core::project::ProjectFile> =
        Vec::with_capacity(inputs.len());
    let mut skipped: Vec<PathBuf> = Vec::new();
    for input in &mut inputs {
        if let Some(virtuals) = &input.virtuals {
            if matches!(input.source, InputSource::Owned(_)) {
                for file in virtuals {
                    if file.skipped {
                        skipped.push(file.path.clone());
                    } else {
                        project_files.push(strictix_core::project::ProjectFile {
                            path: file.path.clone(),
                            source: file.mapped.text.clone(),
                        });
                    }
                }
            }
        } else {
            for file in input.virtuals.iter().flatten() {
                if file.skipped {
                    skipped.push(file.path.clone());
                }
            }
            if matches!(input.source, InputSource::Owned(_)) {
                let taken = std::mem::replace(&mut input.source, InputSource::Err(String::new()));
                if let InputSource::Owned(source) = taken {
                    let idx = project_files.len();
                    project_files.push(strictix_core::project::ProjectFile {
                        path: input.path.clone(),
                        source,
                    });
                    input.source = InputSource::Project(idx);
                }
            }
        }
    }
    let project = strictix_core::project::ProjectContext::new(project_files).with_present(skipped);
    let formatter = format::command(&config);

    let run_one = |input: &Input| {
        let mut result = process_file(
            input,
            &rules,
            &config,
            fix_mode,
            !args.dry_run,
            &project,
            &filter,
        );
        if let Some(command) = formatter {
            format::apply(
                command,
                &input.path,
                input_source(input, &project).ok(),
                &mut result,
            );
        }
        result
    };

    let results: Vec<FileResult> = if inputs.len() > 1 && workers > 1 {
        const LARGE_BYTES: usize = 512 * 1024;
        let mut large = Vec::new();
        let mut small = Vec::new();
        for (idx, input) in inputs.iter().enumerate() {
            let len = input_source(input, &project).map_or(0, str::len);
            if len >= LARGE_BYTES {
                large.push((idx, len));
            } else {
                small.push(idx);
            }
        }
        large.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let next = std::sync::atomic::AtomicUsize::new(0);
        let slots: Vec<std::sync::Mutex<Option<FileResult>>> =
            (0..inputs.len()).map(|_| std::sync::Mutex::new(None)).collect();
        let run_small = || loop {
            let pos = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if pos >= small.len() {
                break;
            }
            let idx = small[pos];
            let result = run_one(&inputs[idx]);
            *slots[idx].lock().unwrap() = Some(result);
        };
        std::thread::scope(|scope| {
            for _ in 0..(workers - 1).min(small.len()) {
                scope.spawn(&run_small);
            }
            for &(idx, _) in &large {
                let result = run_one(&inputs[idx]);
                *slots[idx].lock().unwrap() = Some(result);
            }
            run_small();
        });
        slots
            .into_iter()
            .map(|s| s.into_inner().unwrap().unwrap())
            .collect()
    } else {
        inputs.iter().map(run_one).collect()
    };

    let path_results: Vec<(FileResult, String)> = if need_others {
        others
            .iter()
            .filter_map(|path| {
                let source = if need_prose
                    && path.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                    }) {
                    std::fs::read_to_string(path).unwrap_or_default()
                } else {
                    String::new()
                };
                let diagnostics =
                    strictix_core::rules::run_path_rules(&rules, path, &source, &config);
                (!diagnostics.is_empty()).then(|| {
                    (
                        FileResult {
                            path: path.clone(),
                            diagnostics,
                            read_error: None,
                            format_error: None,
                            fixes_applied: 0,
                            fixed: None,
                            write_error: None,
                            apply_error: None,
                        },
                        source,
                    )
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    let mut any_error = false;
    let mut linted = 0usize;
    let mut total_diags = 0usize;
    let mut linted_results: Vec<(&FileResult, &str)> = Vec::with_capacity(results.len());
    for (input, result) in inputs.iter().zip(&results) {
        if let Some(err) = &result.read_error {
            eprintln!("error: {err}");
            any_error = true;
        } else {
            if let Some(err) = &result.format_error {
                eprintln!("error: {err}");
                any_error = true;
            }
            linted += 1;
            total_diags += result.diagnostics.len();
            let src = if args.format == Format::Human {
                input_source(input, &project).unwrap_or("")
            } else {
                ""
            };
            linted_results.push((result, src));
        }
    }
    for (result, src) in &path_results {
        total_diags += result.diagnostics.len();
        linted_results.push((result, src.as_str()));
    }

    match args.format {
        Format::Human => {
            for &(result, src) in &linted_results {
                let path = result.path.display().to_string();
                for (i, diag) in result.diagnostics.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    let diag_src = if diag.code == format::CODE {
                        result.fixed.as_deref().unwrap_or(src)
                    } else {
                        src
                    };
                    println!("{}", render::human(diag, &path, diag_src, color));
                }
            }
        }
        Format::Json => {
            let entries: Vec<(&Path, &[Diagnostic])> = linted_results
                .iter()
                .map(|&(r, _)| (r.path.as_path(), r.diagnostics.as_slice()))
                .collect();
            let stdout = std::io::stdout();
            let mut out = std::io::BufWriter::with_capacity(64 * 1024, stdout.lock());
            if let Err(err) = render::write_json_doc(&entries, total_diags, &mut out) {
                eprintln!("error: {err}");
                return ExitCode::FAILURE;
            }
            use std::io::Write as _;
            if let Err(err) = out.flush() {
                eprintln!("error: {err}");
                return ExitCode::FAILURE;
            }
        }
    }

    if fix_mode {
        for (input, result) in inputs.iter().zip(&results) {
            if args.format == Format::Human && result.fixes_applied > 0 {
                let verb = if args.dry_run {
                    "would apply"
                } else {
                    "applied"
                };
                println!(
                    "{}: {} {} fix(es)",
                    result.path.display(),
                    verb,
                    result.fixes_applied
                );
                if let Some(fixed) = &result.fixed {
                    let src = input_source(input, &project).unwrap_or("");
                    print!("{}", render::diff(src, fixed, color));
                }
            }
            if let Some(err) = &result.write_error {
                eprintln!("error: {}: {err}", result.path.display());
                any_error = true;
            }
            if let Some(err) = &result.apply_error {
                eprintln!("warning: {}: {err}", result.path.display());
            }
        }
    }

    if args.format == Format::Human {
        println!("{}", render::summary_line(linted, total_diags, color));
    }

    if any_error {
        return ExitCode::FAILURE;
    }
    if !fix_mode && total_diags > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Resolve the config file: --config if given (must exist), else the
/// default ./strictix.toml when present, else no config.
fn resolve_config(args: &Args) -> Result<LintConfig, String> {
    let config_path = match &args.config {
        Some(path) => Some(path.as_path()),
        None => {
            let default = Path::new("strictix.toml");
            if default.exists() {
                Some(default)
            } else {
                None
            }
        }
    };
    let mut config = config::load_config(config_path, &args.disabled, args.schema.as_deref())?;
    config.enabled.extend(args.enabled.iter().cloned());
    Ok(config)
}

/// Resolve the ignore file: --ignore-file if given, else the default
/// ./.strictixignore when present, else none.
fn resolve_ignore_file(args: &Args) -> Option<PathBuf> {
    match &args.ignore_file {
        Some(path) => Some(path.clone()),
        None => {
            let default = Path::new(".strictixignore");
            if default.exists() {
                Some(default.to_path_buf())
            } else {
                None
            }
        }
    }
}

struct FileResult {
    path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    read_error: Option<String>,
    format_error: Option<String>,
    fixes_applied: usize,
    fixed: Option<String>,
    write_error: Option<String>,
    apply_error: Option<String>,
}

enum InputSource {
    Owned(String),
    Project(usize),
    Err(String),
}

fn input_source<'a>(
    input: &'a Input,
    project: &'a strictix_core::project::ProjectContext,
) -> Result<&'a str, &'a str> {
    match &input.source {
        InputSource::Owned(source) => Ok(source.as_str()),
        InputSource::Project(idx) => Ok(project.sources()[*idx].source.as_str()),
        InputSource::Err(err) => Err(err.as_str()),
    }
}

struct Input {
    path: PathBuf,
    source: InputSource,
    virtuals: Option<Vec<merged::Virtual>>,
}

/// Lint one file: parse, build the (lazy) semantic model, run every
/// enabled rule, sort findings by range start, and, in fix mode,
/// splice the fixes, write the file back when it changed, and count
/// what was applied. A merged file is linted binding by binding as the
/// files its manifest names, with positions and fixes mapped back into
/// it. A read failure is reported and the file is skipped; a
/// fix-application failure leaves the file untouched and is reported
/// as a warning.
fn process_file(
    input: &Input,
    rules: &[Box<dyn Rule>],
    config: &LintConfig,
    fix_mode: bool,
    write: bool,
    project: &strictix_core::project::ProjectContext,
    filter: &walk::Filter,
) -> FileResult {
    let path = &input.path;
    let source = match input_source(input, project) {
        Ok(source) => source,
        Err(err) => {
            return FileResult {
                path: path.clone(),
                diagnostics: Vec::new(),
                read_error: Some(err.to_owned()),
                format_error: None,
                fixes_applied: 0,
                fixed: None,
                write_error: None,
                apply_error: None,
            };
        }
    };

    let mut fixes_applied = 0usize;
    let mut write_error = None;
    let mut apply_error = None;

    let (diagnostics, fixed, error) = match &input.virtuals {
        Some(virtuals) => {
            let run = merged::lint(
                path,
                source,
                virtuals,
                rules,
                config,
                fix_mode,
                Some(project),
                filter,
            );
            (run.diagnostics, run.fixed, run.error)
        }
        None => {
            let run = strictix_core::rules::lint_project(
                rules,
                source,
                Some(path),
                config,
                fix_mode,
                Some(project),
            );
            (
                run.diagnostics,
                run.fixed,
                run.error.map(|err| format!("{err:?}")),
            )
        }
    };
    let fixed = if fix_mode { fixed } else { None };
    if fix_mode {
        let fix_count = diagnostics.iter().filter(|d| d.fix.is_some()).count();
        if let Some(result) = &fixed {
            if write {
                match std::fs::write(path, result) {
                    Ok(()) => fixes_applied = fix_count,
                    Err(err) => write_error = Some(err.to_string()),
                }
            } else {
                fixes_applied = fix_count;
            }
        }
        apply_error = error;
    }

    FileResult {
        path: path.clone(),
        diagnostics,
        read_error: None,
        format_error: None,
        fixes_applied,
        fixed,
        write_error,
        apply_error,
    }
}
