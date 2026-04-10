use std::{
    default::Default,
    env, fmt, fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::{LintMap, dirs, err::ConfigErr, utils};

use clap::Parser;
use lib::LINTS;
use serde::{Deserialize, Serialize};
use vfs::ReadOnlyVfs;

#[derive(Parser, Debug)]
#[command(version, author, about)]
pub struct Opts {
    #[command(subcommand)]
    pub cmd: SubCommand,
}

#[derive(Parser, Debug)]
pub enum SubCommand {
    /// Lints and suggestions for the nix programming language
    Check(Check),
    /// Find and fix issues raised by strictix-check
    Fix(Fix),
    /// Fix exactly one issue at provided position
    Single(Single),
    /// Print detailed explanation for a lint warning
    Explain(Explain),
    /// Dump a sample config to stdout
    Dump(Dump),
    /// List all available lints
    List(List),
}

#[derive(Parser, Debug)]
pub struct Check {
    /// File or directory to run check on
    #[arg(default_value = ".")]
    target: PathBuf,

    /// Globs of file patterns to skip
    #[arg(short, long)]
    ignore: Vec<String>,

    /// Don't respect .gitignore files
    #[arg(short, long)]
    unrestricted: bool,

    /// Output format.
    #[cfg_attr(feature = "json", doc = "Supported values: stderr, errfmt, json")]
    #[cfg_attr(not(feature = "json"), doc = "Supported values: stderr, errfmt")]
    #[arg(short = 'o', long, default_value_t)]
    pub format: OutFormat,

    /// Path to strictix.toml or its parent directory
    #[arg(short = 'c', long = "config", default_value = ".")]
    pub conf_path: PathBuf,

    /// Enable "streaming" mode, accept file on stdin, output diagnostics on stdout
    #[arg(short, long = "stdin")]
    pub streaming: bool,

    /// Enable all lints, including opt-in ones (`with_expression`, `single_use_let`, etc.)
    #[arg(long)]
    pub strict: bool,

    /// Enable specific opt-in lints by name (can be repeated)
    #[arg(short = 'e', long = "enable")]
    pub enable: Vec<String>,
}

impl Check {
    pub fn vfs(&self, extra_ignores: &[String]) -> Result<ReadOnlyVfs, ConfigErr> {
        project_vfs(
            self.streaming,
            &self.target,
            &self.ignore,
            extra_ignores,
            self.unrestricted,
        )
    }
}

#[derive(Parser, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct Fix {
    /// File or directory to run fix on
    #[arg(default_value = ".")]
    target: PathBuf,

    /// Globs of file patterns to skip
    #[arg(short, long)]
    ignore: Vec<String>,

    /// Don't respect .gitignore files
    #[arg(short, long)]
    unrestricted: bool,

    /// Do not fix files in place, display a diff instead
    #[arg(short, long = "dry-run")]
    pub diff_only: bool,

    /// Path to strictix.toml or its parent directory
    #[arg(short = 'c', long = "config", default_value = ".")]
    pub conf_path: PathBuf,

    /// Enable "streaming" mode, accept file on stdin, output diagnostics on stdout
    #[arg(short, long = "stdin")]
    pub streaming: bool,

    /// Enable all lints, including opt-in ones (`with_expression`, `single_use_let`, etc.)
    #[arg(long)]
    pub strict: bool,

    /// Enable specific opt-in lints by name (can be repeated)
    #[arg(short = 'e', long = "enable")]
    pub enable: Vec<String>,
}

pub enum FixOut {
    Diff,
    Stream,
    Write,
}

impl FixOut {
    fn from_flags(diff_only: bool, streaming: bool) -> Self {
        if diff_only {
            Self::Diff
        } else if streaming {
            Self::Stream
        } else {
            Self::Write
        }
    }
}

impl Fix {
    pub fn vfs(&self, extra_ignores: &[String]) -> Result<ReadOnlyVfs, ConfigErr> {
        project_vfs(
            self.streaming,
            &self.target,
            &self.ignore,
            extra_ignores,
            self.unrestricted,
        )
    }

    // i need this ugly helper because clap's data model
    // does not reflect what i have in mind
    #[must_use]
    pub fn out(&self) -> FixOut {
        FixOut::from_flags(self.diff_only, self.streaming)
    }
}

#[derive(Parser, Debug)]
pub struct Single {
    /// File to run single-fix on
    pub target: Option<PathBuf>,

    /// Position to attempt a fix at
    #[arg(short, long, value_parser = parse_line_col)]
    pub position: (usize, usize),

    /// Do not fix files in place, display a diff instead
    #[arg(short, long = "dry-run")]
    pub diff_only: bool,

    /// Enable "streaming" mode, accept file on stdin, output diagnostics on stdout
    #[arg(short, long = "stdin")]
    pub streaming: bool,

    /// Path to strictix.toml or its parent directory
    #[arg(short = 'c', long = "config", default_value = ".")]
    pub conf_path: PathBuf,
}

impl Single {
    pub fn vfs(&self) -> Result<ReadOnlyVfs, ConfigErr> {
        single_file_vfs(self.streaming, self.target.as_deref())
    }
    #[must_use]
    pub fn out(&self) -> FixOut {
        FixOut::from_flags(self.diff_only, self.streaming)
    }
}

#[derive(Parser, Debug)]
pub struct Explain {
    /// Warning code to explain
    #[arg(value_parser = parse_warning_code)]
    pub target: u32,
}

#[derive(Parser, Debug)]
pub struct Dump {}

#[derive(Parser, Debug)]
pub struct List {}

#[derive(Debug, Copy, Clone, Default)]
pub enum OutFormat {
    #[cfg(feature = "json")]
    Json,
    Errfmt,
    #[default]
    StdErr,
}

impl fmt::Display for OutFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                #[cfg(feature = "json")]
                Self::Json => "json",
                Self::Errfmt => "errfmt",
                Self::StdErr => "stderr",
            }
        )
    }
}

impl FromStr for OutFormat {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            #[cfg(feature = "json")]
            "json" => Ok(Self::Json),
            #[cfg(not(feature = "json"))]
            "json" => Err("strictix was not compiled with the `json` feature flag"),
            "errfmt" => Ok(Self::Errfmt),
            "stderr" => Ok(Self::StdErr),
            #[cfg(feature = "json")]
            _ => Err("unknown output format, try: stderr, errfmt, json"),
            #[cfg(not(feature = "json"))]
            _ => Err("unknown output format, try: stderr, errfmt"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct RepeatedKeysConf {
    /// Minimum number of repeated key occurrences before W20 fires (default: 3, minimum: 2).
    pub min_occurrences: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct LintConf {
    #[serde(default)]
    pub repeated_keys: RepeatedKeysConf,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ConfFile {
    #[serde(default = "Vec::new")]
    enabled: Vec<String>,

    #[serde(default = "Vec::new")]
    disabled: Vec<String>,

    #[serde(default = "Vec::new")]
    pub ignore: Vec<String>,

    /// Enable all lints, including those that are opt-in by default.
    #[serde(default)]
    strict: bool,

    #[serde(default)]
    pub lints: LintConf,
}

impl ConfFile {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, ConfigErr> {
        let path = path.as_ref();
        let config_file = fs::read_to_string(path).map_err(ConfigErr::InvalidPath)?;
        toml::de::from_str(&config_file).map_err(ConfigErr::ConfFileParse)
    }
    /// Discover config by walking ancestors of `path` (defaults to CWD via `--config`),
    /// not the lint target. This is intentional: config applies per-project, not per-file.
    pub fn discover<P: AsRef<Path>>(path: P) -> Result<Self, ConfigErr> {
        let canonical_path = fs::canonicalize(path.as_ref()).map_err(ConfigErr::InvalidPath)?;
        let mut config = Self::from_global_path()?;

        for p in canonical_path.ancestors() {
            let strictix_toml_path = if p.is_dir() {
                p.join("strictix.toml")
            } else {
                p.to_path_buf()
            };
            if strictix_toml_path.exists() {
                config.merge(Self::from_path(strictix_toml_path)?);
                return Ok(config);
            }
        }
        Ok(config)
    }
    #[must_use]
    pub fn dump(&self) -> String {
        let ideal_config = Self {
            enabled: vec![],
            disabled: vec![],
            ignore: vec![".direnv".into()],
            strict: false,
            lints: LintConf {
                repeated_keys: RepeatedKeysConf {
                    min_occurrences: Some(2),
                },
            },
        };
        toml::ser::to_string_pretty(&ideal_config)
            .expect("default config serialization should not fail")
    }
    /// Apply per-lint options from config to the global lint settings.
    /// Must be called before linting runs.
    pub fn apply_lint_options(&self) {
        if let Some(n) = self.lints.repeated_keys.min_occurrences {
            lib::set_repeated_keys_min_occurrences(n);
        }
    }

    #[must_use]
    pub fn lints(&self) -> LintMap {
        utils::lint_map_of(
            (*LINTS)
                .iter()
                .filter(|l| {
                    // Explicitly enabled lints are always included
                    if self.enabled.iter().any(|name| name == l.name()) {
                        return true;
                    }
                    // If enabled list is non-empty, only those lints run (allowlist mode)
                    if !self.enabled.is_empty() {
                        return false;
                    }
                    // Include if default_enabled or strict mode is on
                    l.default_enabled() || self.strict
                })
                .filter(|l| !self.disabled.iter().any(|check| check == l.name()))
                .copied()
                .collect::<Vec<_>>()
                .as_slice(),
        )
    }

    /// Enable strict mode (all lints, including opt-in ones).
    pub fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Explicitly enable specific lints by name.
    pub fn enable_lints(&mut self, names: &[String]) {
        self.enabled.extend(names.iter().cloned());
    }

    fn merge(&mut self, other: Self) {
        if !other.enabled.is_empty() {
            self.enabled = other.enabled;
        }
        self.disabled.extend(other.disabled);
        self.ignore.extend(other.ignore);
        // Project config overrides global strict setting.
        if other.strict {
            self.strict = true;
        }
        // Project config overrides global lint options when present.
        if let Some(n) = other.lints.repeated_keys.min_occurrences {
            self.lints.repeated_keys.min_occurrences = Some(n);
        }
    }

    fn from_global_path() -> Result<Self, ConfigErr> {
        let Some(path) = Self::global_path() else {
            return Ok(Self::default());
        };
        if path.exists() {
            Self::from_path(path)
        } else {
            Ok(Self::default())
        }
    }

    fn global_path() -> Option<PathBuf> {
        if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
            return Some(
                PathBuf::from(config_home)
                    .join("strictix")
                    .join("config.toml"),
            );
        }

        env::var_os("HOME").map(|home| {
            PathBuf::from(home)
                .join(".config")
                .join("strictix")
                .join("config.toml")
        })
    }
}

fn parse_line_col(src: &str) -> Result<(usize, usize), ConfigErr> {
    let Some((line, col)) = src.split_once(',') else {
        return Err(ConfigErr::InvalidPosition(src.to_owned()));
    };
    let do_parse = |val: &str| {
        val.parse::<usize>()
            .map_err(|_| ConfigErr::InvalidPosition(src.to_owned()))
    };
    Ok((do_parse(line)?, do_parse(col)?))
}

fn parse_warning_code(src: &str) -> Result<u32, ConfigErr> {
    let mut char_stream = src.chars();
    let severity = char_stream
        .next()
        .ok_or_else(|| ConfigErr::InvalidWarningCode(src.to_owned()))?
        .to_ascii_lowercase();
    match severity {
        'w' => char_stream
            .collect::<String>()
            .parse::<u32>()
            .map_err(|_| ConfigErr::InvalidWarningCode(src.to_owned())),
        _ => Err(ConfigErr::InvalidWarningCode(src.to_owned())),
    }
}

fn vfs(files: &[PathBuf]) -> vfs::ReadOnlyVfs {
    let mut vfs = ReadOnlyVfs::default();
    for file in files {
        if let Ok(data) = fs::read_to_string(file) {
            vfs.set_file_contents(file, data.as_bytes());
        } else {
            eprintln!("`{}` contains non-utf8 content", file.display());
        }
    }
    vfs
}

fn read_stdin() -> Result<String, ConfigErr> {
    use std::io::{self, BufRead};

    io::stdin()
        .lock()
        .lines()
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
        .map_err(ConfigErr::InvalidPath)
}

fn stdin_vfs() -> Result<ReadOnlyVfs, ConfigErr> {
    let src = read_stdin()?;
    Ok(ReadOnlyVfs::singleton("<stdin>", src.as_bytes()))
}

fn project_vfs(
    streaming: bool,
    target: &Path,
    ignore: &[String],
    extra_ignores: &[String],
    unrestricted: bool,
) -> Result<ReadOnlyVfs, ConfigErr> {
    if streaming {
        stdin_vfs()
    } else {
        filesystem_vfs(target, ignore, extra_ignores, unrestricted)
    }
}

fn single_file_vfs(streaming: bool, target: Option<&Path>) -> Result<ReadOnlyVfs, ConfigErr> {
    if streaming {
        return stdin_vfs();
    }

    let target = target.ok_or_else(|| {
        ConfigErr::InvalidPath(io::Error::new(
            io::ErrorKind::NotFound,
            "no target file provided",
        ))
    })?;
    let src = std::fs::read_to_string(target).map_err(ConfigErr::InvalidPath)?;
    let path = target.to_str().unwrap_or("<stdin>");
    Ok(ReadOnlyVfs::singleton(path, src.as_bytes()))
}

fn filesystem_vfs(
    target: &Path,
    ignore: &[String],
    extra_ignores: &[String],
    unrestricted: bool,
) -> Result<ReadOnlyVfs, ConfigErr> {
    let all_ignores = [ignore, extra_ignores].concat();
    let ignore = dirs::build_ignore_set(&all_ignores, target, unrestricted)?;
    let mut files: Vec<_> = dirs::walk_nix_files(ignore, target)?.collect();
    files.sort();
    Ok(vfs(&files))
}

#[cfg(test)]
mod tests {
    use super::ConfFile;

    use std::{
        env, fs,
        sync::{Mutex, MutexGuard},
    };

    use tempfile::tempdir;

    // Serialise all tests that touch process-global environment variables.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_env_var(key: &str, value: &std::path::Path) {
        unsafe { env::set_var(key, value) };
    }

    fn remove_env_var(key: &str) {
        unsafe { env::remove_var(key) };
    }

    #[test]
    fn discovers_global_config_from_xdg_path() {
        let _guard = lock_env();
        let temp = tempdir().expect("failed to create temporary directory");
        let config_dir = temp.path().join("xdg").join("strictix");
        fs::create_dir_all(&config_dir).expect("failed to create XDG config directory tree");
        fs::write(
            config_dir.join("config.toml"),
            "enabled = [\"with_expression\"]\ndisabled = [\"empty_pattern\"]\n",
        )
        .expect("failed to write global config.toml");

        set_env_var("XDG_CONFIG_HOME", &temp.path().join("xdg"));
        remove_env_var("HOME");

        let config = ConfFile::discover(temp.path()).expect("failed to discover global config from XDG path");
        let lints = config.lints();

        assert!(
            lints
                .values()
                .flatten()
                .any(|lint| lint.name() == "with_expression")
        );
        assert!(
            !lints
                .values()
                .flatten()
                .any(|lint| lint.name() == "empty_pattern")
        );
        assert_eq!(lints.values().flatten().count(), 1);
    }

    #[test]
    fn project_config_overrides_global_allowlist() {
        let _guard = lock_env();
        let temp = tempdir().expect("failed to create temporary directory");
        let xdg_home = temp.path().join("xdg");
        let config_dir = xdg_home.join("strictix");
        let project_dir = temp.path().join("project");
        fs::create_dir_all(&config_dir).expect("failed to create XDG config directory tree");
        fs::create_dir_all(&project_dir).expect("failed to create project directory");
        fs::write(
            config_dir.join("config.toml"),
            "enabled = [\"with_expression\"]\n",
        )
        .expect("failed to write global config.toml");
        fs::write(
            project_dir.join("strictix.toml"),
            "enabled = [\"empty_pattern\"]\n",
        )
        .expect("failed to write project strictix.toml");

        set_env_var("XDG_CONFIG_HOME", &xdg_home);
        remove_env_var("HOME");

        let config = ConfFile::discover(&project_dir).expect("failed to discover config with project override");
        let lints = config.lints();

        assert!(
            lints
                .values()
                .flatten()
                .any(|lint| lint.name() == "empty_pattern")
        );
        assert!(
            !lints
                .values()
                .flatten()
                .any(|lint| lint.name() == "with_expression")
        );
        assert_eq!(lints.values().flatten().count(), 1);
    }
}
