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
mod render;
mod walk;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use args::{Args, Command, Format};
use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::{apply_fixes, TextEdit};
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::parse;

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
        return usage_error("no command given (expected check, fix, explain, or list)");
    };
    match command {
        Command::Check => run_check(&parsed, false),
        Command::Fix => run_check(&parsed, true),
        Command::Explain => run_explain(&parsed),
        Command::List => run_list(),
    }
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
            "{:<24} {:<28} {:<8} {}",
            rule.code(),
            rule.name(),
            severity_str(rule.severity()),
            kind
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
    let files = match walk::collect_files(&paths, ignore_file.as_deref()) {
        Ok(files) => files,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    if files.is_empty() {
        match args.format {
            Format::Human => println!("{}", render::summary_line(0, 0)),
            Format::Json => print_json(&[]),
        }
        return ExitCode::SUCCESS;
    }

    // Build the registry once and share it across worker threads.
    let rules = strictix_lints::all_rules();
    let results: Vec<FileResult> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for file in &files {
            handles
                .push(scope.spawn(|| process_file(file, &rules, &config, fix_mode, !args.dry_run)));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("worker thread panicked"))
            .collect()
    });

    // Classify: files that read cleanly are linted; read errors go to
    // stderr and force a failure exit.
    let mut any_error = false;
    let mut linted = 0usize;
    let mut total_diags = 0usize;
    let mut linted_results: Vec<&FileResult> = Vec::new();
    for result in &results {
        if let Some(err) = &result.read_error {
            eprintln!("error: {err}");
            any_error = true;
        } else {
            linted += 1;
            total_diags += result.diagnostics.len();
            linted_results.push(result);
        }
    }

    match args.format {
        Format::Human => {
            for result in &linted_results {
                let path = result.path.display().to_string();
                for (i, diag) in result.diagnostics.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    println!("{}", render::human(diag, &path, &result.source));
                }
            }
        }
        Format::Json => {
            let files: Vec<render::JsonFile> = linted_results
                .iter()
                .map(|result| render::JsonFile {
                    path: result.path.display().to_string(),
                    diagnostics: result.diagnostics.clone(),
                })
                .collect();
            print_json(&files);
        }
    }

    if fix_mode && args.format == Format::Human {
        for result in &results {
            if result.fixes_applied > 0 {
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
                    print!("{}", render::diff(&result.source, fixed));
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
        println!("{}", render::summary_line(linted, total_diags));
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
    config::load_config(config_path, &args.disabled, args.schema.as_deref())
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

/// Print the JSON document for a run.
fn print_json(files: &[render::JsonFile]) {
    let value = render::json(files);
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("JSON serialization cannot fail")
    );
}

/// One file's outcome, carried out of the worker thread.
struct FileResult {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
    read_error: Option<String>,
    fixes_applied: usize,
    /// The result of splicing all fixes, `Some` when any fix exists (the
    /// text the file would become). Written to disk unless dry-run.
    fixed: Option<String>,
    write_error: Option<String>,
    apply_error: Option<String>,
}

/// Lint one file: read, parse, build the (lazy) semantic model, run
/// every enabled rule, sort findings by range start, and, in fix mode,
/// splice the fixes, write the file back when it changed, and count
/// what was applied. A read failure is reported and the file is
/// skipped; a fix-application failure leaves the file untouched and is
/// reported as a warning.
fn process_file(
    path: &Path,
    rules: &[Box<dyn Rule>],
    config: &LintConfig,
    fix_mode: bool,
    write: bool,
) -> FileResult {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            return FileResult {
                path: path.to_path_buf(),
                source: String::new(),
                diagnostics: Vec::new(),
                read_error: Some(format!("{}: {err}", path.display())),
                fixes_applied: 0,
                fixed: None,
                write_error: None,
                apply_error: None,
            };
        }
    };

    let tree = parse(&source);
    let model = SemanticModel::new(&source, &tree);
    let mut diagnostics = Vec::new();
    strictix_core::rules::run_rules(rules, &tree, &model, config, &source, &mut diagnostics);
    diagnostics.sort_by_key(|diag| diag.range.start());

    let mut fixes_applied = 0usize;
    let mut fixed = None;
    let mut write_error = None;
    let mut apply_error = None;
    if fix_mode {
        let fix_count = diagnostics.iter().filter(|d| d.fix.is_some()).count();
        let edits: Vec<TextEdit> = diagnostics
            .iter()
            .filter_map(|d| d.fix.as_ref())
            .flat_map(|fix| fix.edits.iter().cloned())
            .collect();
        match apply_fixes(&source, &edits) {
            Ok(result) if result != source => {
                if write {
                    match std::fs::write(path, &result) {
                        Ok(()) => fixes_applied = fix_count,
                        Err(err) => write_error = Some(err.to_string()),
                    }
                } else {
                    fixes_applied = fix_count; // reported as would-apply
                }
                fixed = Some(result);
            }
            Ok(_) => {}
            Err(err) => apply_error = Some(format!("{err:?}")),
        }
    }

    FileResult {
        path: path.to_path_buf(),
        source,
        diagnostics,
        read_error: None,
        fixes_applied,
        fixed,
        write_error,
        apply_error,
    }
}
