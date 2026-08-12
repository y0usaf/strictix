//! Hand-written command-line argument parsing (no clap).
//!
//! strictix deliberately owns its argument parser: the surface is one
//! command plus a handful of flags, so a dependency would cost more to
//! understand than the parser itself. The grammar is deliberately
//! simple — the first non-flag argument is the command, `--flag value`
//! pairs carry options, `--disable` repeats, and `--` ends flag
//! parsing so a path that begins with `-` stays addressable.

use std::path::PathBuf;

/// The subcommand chosen on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Lint files or directories, printing diagnostics.
    Check,
    /// Lint and apply automatic fixes, writing changed files back.
    Fix,
    /// Print one rule's details by code.
    Explain,
    /// Print the whole rule registry as a table.
    List,
}

/// Output format for diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Human-readable lines with source excerpts.
    Human,
    /// A machine-readable JSON document.
    Json,
}

impl Format {
    /// Parse a `--format` value. Anything but `human`/`json` is an
    /// error; the message is a usage error, not a lint finding.
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "human" => Ok(Format::Human),
            "json" => Ok(Format::Json),
            other => Err(format!(
                "unknown format '{other}' (expected 'human' or 'json')"
            )),
        }
    }
}

/// Everything parsed from the command line.
///
/// `paths` holds check/fix path arguments; for `explain` it holds the
/// rule code as its single element (the struct shape is shared across
/// commands). `help`/`version` are latched flags: main handles them
/// before dispatching on `command`.
pub struct Args {
    pub command: Option<Command>,
    pub paths: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub format: Format,
    pub ignore_file: Option<PathBuf>,
    pub disabled: Vec<String>,
    pub schema: Option<PathBuf>,
    pub dry_run: bool,
    pub help: bool,
    pub version: bool,
}

/// Parse `args` (already without argv[0]) into an [Args].
///
/// # Errors
///
/// Returns a usage-error message on an unknown flag, a flag missing its
/// value, an unknown command, or a bad `--format` value.
pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut command = None;
    let mut paths = Vec::new();
    let mut config = None;
    let mut format = Format::Human;
    let mut ignore_file = None;
    let mut disabled = Vec::new();
    let mut schema = None;
    let mut dry_run = false;
    let mut help = false;
    let mut version = false;
    let mut end_of_flags = false;

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if !end_of_flags && arg == "--" {
            end_of_flags = true;
            i += 1;
            continue;
        }
        if !end_of_flags && arg.starts_with('-') && arg.len() > 1 {
            match arg.as_str() {
                "-h" | "--help" => help = true,
                "-V" | "--version" => version = true,
                "--config" => {
                    let value = next_value(args, &mut i, "--config")?;
                    config = Some(PathBuf::from(value));
                }
                "--format" => {
                    let value = next_value(args, &mut i, "--format")?;
                    format = Format::parse(&value)?;
                }
                "--ignore-file" => {
                    let value = next_value(args, &mut i, "--ignore-file")?;
                    ignore_file = Some(PathBuf::from(value));
                }
                "--disable" => {
                    let value = next_value(args, &mut i, "--disable")?;
                    disabled.push(value);
                }
                "--schema" => {
                    let value = next_value(args, &mut i, "--schema")?;
                    schema = Some(PathBuf::from(value));
                }
                "--dry-run" => dry_run = true,
                other => return Err(format!("unknown flag '{other}'")),
            }
            i += 1;
            continue;
        }
        // Positional: the first one is the command, the rest are paths
        // (or the explain code).
        if command.is_none() {
            command = Some(parse_command(arg)?);
        } else {
            paths.push(PathBuf::from(arg));
        }
        i += 1;
    }

    Ok(Args {
        command,
        paths,
        config,
        format,
        ignore_file,
        disabled,
        schema,
        dry_run,
        help,
        version,
    })
}

/// The value of a `--flag value` pair, consuming the next argument.
fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    if *i + 1 >= args.len() {
        return Err(format!("flag '{flag}' requires a value"));
    }
    *i += 1;
    Ok(args[*i].clone())
}

/// Map a command word to a [Command].
fn parse_command(word: &str) -> Result<Command, String> {
    match word {
        "check" => Ok(Command::Check),
        "fix" => Ok(Command::Fix),
        "explain" => Ok(Command::Explain),
        "list" => Ok(Command::List),
        other => Err(format!("unknown command '{other}'")),
    }
}

/// The full usage/help text, printed for `--help` and on usage errors.
#[must_use]
pub fn usage() -> &'static str {
    "strictix — strict lints and suggestions for the Nix language

USAGE:
    strictix <COMMAND> [PATH...] [OPTIONS]

COMMANDS:
    check     Lint files or directories (default path: .)
    fix       Lint and apply automatic fixes
    explain   Explain a lint rule by code
    list      List all lint rules

OPTIONS:
    -h, --help            Print this help
    -V, --version         Print version
    --config FILE         TOML config file (default: ./strictix.toml if present)
    --format FORMAT       Output format: human or json (default: human)
    --ignore-file FILE    Ignore patterns (default: ./.strictixignore if present)
    --disable CODE        Skip a rule by code (repeatable)
    --schema FILE         options.json path, enabling the unknown-option rule
    --dry-run             (fix) show what would change without writing
"
}
