use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
        Check,
        Fix,
        Explain,
        List,
        Merge,
        Split,
        Import,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
        Human,
        Json,
}

impl Format {
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

pub struct Args {
    pub command: Option<Command>,
    pub paths: Vec<PathBuf>,
    pub config: Option<PathBuf>,
    pub format: Format,
    pub ignore_file: Option<PathBuf>,
    pub disabled: Vec<String>,
    pub enabled: Vec<String>,
    pub schema: Option<PathBuf>,
    pub entry: Option<String>,
    pub out: Option<String>,
    pub expand: Vec<String>,
    pub merged: Option<String>,
    pub generators: Option<String>,
    pub split: bool,
    pub dry_run: bool,
    pub help: bool,
    pub version: bool,
}

pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut command = None;
    let mut paths = Vec::new();
    let mut config = None;
    let mut format = Format::Human;
    let mut ignore_file = None;
    let mut disabled = Vec::new();
    let mut enabled = Vec::new();
    let mut schema = None;
    let mut entry = None;
    let mut out = None;
    let mut expand = Vec::new();
    let mut merged = None;
    let mut generators = None;
    let mut split = false;
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
                "--enable" => {
                    enabled.push(next_value(args, &mut i, "--enable")?);
                }
                "--schema" => {
                    let value = next_value(args, &mut i, "--schema")?;
                    schema = Some(PathBuf::from(value));
                }
                "--entry" => entry = Some(next_value(args, &mut i, "--entry")?),
                "--out" => out = Some(next_value(args, &mut i, "--out")?),
                "--expand" => expand.push(next_value(args, &mut i, "--expand")?),
                "--merged" => merged = Some(next_value(args, &mut i, "--merged")?),
                "--generators" => generators = Some(next_value(args, &mut i, "--generators")?),
                "--split" => split = true,
                "--dry-run" => dry_run = true,
                other => return Err(format!("unknown flag '{other}'")),
            }
            i += 1;
            continue;
        }
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
        enabled,
        schema,
        entry,
        out,
        expand,
        merged,
        generators,
        split,
        dry_run,
        help,
        version,
    })
}

fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    if *i + 1 >= args.len() {
        return Err(format!("flag '{flag}' requires a value"));
    }
    *i += 1;
    Ok(args[*i].clone())
}

fn parse_command(word: &str) -> Result<Command, String> {
    match word {
        "check" => Ok(Command::Check),
        "fix" => Ok(Command::Fix),
        "explain" => Ok(Command::Explain),
        "list" => Ok(Command::List),
        "merge" => Ok(Command::Merge),
        "split" => Ok(Command::Split),
        "import" => Ok(Command::Import),
        other => Err(format!("unknown command '{other}'")),
    }
}

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
    merge     Inline every .nix file reachable from --entry into --out
              (path: merge root, default .)
    split     Write the bindings of --merged back to their original files
              (path: merge root, default .)
    import    Inline every file a module reads as the generator data that
              prints it: the path at the option becomes the finix generator
              call and the source file is deleted. `--split` writes each tree
              to its own `<file>.nix` beside the source and imports it there
              instead, which also lets several modules share one file. Used
              by finix's `nix run .#lint` (css, glsl, javascript, lisp, lua,
              luau, markdown, patch, python, shell, ts, vbs)

OPTIONS:
    -h, --help            Print this help
    -V, --version         Print version
    --config FILE         TOML config file (default: ./strictix.toml if present)
    --format FORMAT       Output format: human or json (default: human)
    --ignore-file FILE    Ignore patterns (default: ./.strictixignore if present)
    --disable CODE        Skip a rule by code (repeatable)
    --enable CODE         Enable an opt-in rule (repeatable; --disable wins)
    --schema FILE         options.json path, enabling the unknown-option rule
    --dry-run             (fix, merge, split) show what would change without writing
    --entry FILE          (merge) root-relative entry file
    --out FILE            (merge) root-relative output file (may be the entry)
    --expand NAME         (merge) evaluate calls of NAME, bound in the top-level
                          let of a merged file, into explicit module lists
                          (repeatable)
    --merged FILE         (split) root-relative merged file
    --generators EXPR     (import) Nix expression for the finix generators
                          (default: the current flake's nix-generators lib)
    --split               (import) write each tree to `<file>.nix` beside the
                          source and import it, instead of inlining it
"
}
