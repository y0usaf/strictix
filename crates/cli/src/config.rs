//! TOML lint configuration, parsed by hand (stdlib only).
//!
//! strictix dropped its last third-party dependency: the on-disk config
//! is a tiny, fixed subset of TOML, so a hand-rolled parser is smaller
//! to understand than the `toml` crate. The shape is:
//!
//! ```toml
//! [lint]
//! disabled = ["tautology"]
//! schema = "options.json"
//!
//! [schemas]
//! "modules/phone" = "phone-options.json"
//! "modules/flake" = ""
//! ```
//!
//! Supported: `#` comments (full line), blank lines, the `[lint]`
//! section, `disabled` and `enabled` (arrays of strings), `schema` (string),
//! and the `[schemas]` section of `"path prefix" = "options.json"` pairs
//! (`""` turns the schema rules off under that prefix). Unknown keys in
//! `[lint]` and unknown sections are ignored; missing keys are fine.
//!
//! ponytail: no trailing comments, no multiline arrays, no other TOML
//! features — the config never uses them; add them only if a real
//! config needs them.

use std::path::{Path, PathBuf};

use strictix_core::config::LintConfig;

/// Load the TOML config and merge in the command-line overrides.
///
/// `config_path` is the file to read (`--config`, or the default
/// `./strictix.toml` when it exists); `None` means no config file.
/// `flags` are the repeatable `--disable` codes; `schema_flag` is the
/// `--schema` path when given.
///
/// Merging rules: disabled = TOML `disabled` followed by the
/// `--disable` flags, deduplicated keeping first occurrence; schema =
/// `--schema` if given, else the TOML `schema`.
///
/// # Errors
///
/// - The config file cannot be read.
/// - The config file is not valid (for the supported subset).
pub fn load_config(
    config_path: Option<&Path>,
    flags: &[String],
    schema_flag: Option<&Path>,
) -> Result<LintConfig, String> {
    let mut config = match config_path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read config file {}: {e}", path.display()))?;
            parse_config(&text)
                .map_err(|e| format!("cannot parse config file {}: {e}", path.display()))?
        }
        None => LintConfig::default(),
    };

    for code in flags {
        if !config.disabled.contains(code) {
            config.disabled.push(code.clone());
        }
    }

    config.schema = schema_flag.map(PathBuf::from).or(config.schema);

    Ok(config)
}

/// Parse the strictix.toml subset.
fn parse_config(text: &str) -> Result<LintConfig, String> {
    let mut config = LintConfig::default();
    let mut section = String::new();

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_owned();
            continue;
        }
        if section != "lint" && section != "schemas" {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected `key = value`", line_no + 1));
        };
        let key = key.trim();
        let value = value.trim();
        if section == "schemas" {
            let err = |e: String| format!("line {}: {e}", line_no + 1);
            let prefix = if key.starts_with('"') {
                parse_toml_string(key).map_err(err)?
            } else {
                key.to_owned()
            };
            let schema = parse_toml_string(value).map_err(err)?;
            config.schemas.push((
                PathBuf::from(prefix),
                (!schema.is_empty()).then(|| PathBuf::from(schema)),
            ));
            continue;
        }
        match key {
            "disabled" => {
                config.disabled =
                    parse_string_array(value).map_err(|e| format!("line {}: {e}", line_no + 1))?;
            }
            "enabled" => {
                config.enabled =
                    parse_string_array(value).map_err(|e| format!("line {}: {e}", line_no + 1))?;
            }
            "schema" => {
                config.schema = Some(PathBuf::from(
                    parse_toml_string(value).map_err(|e| format!("line {}: {e}", line_no + 1))?,
                ));
            }
            _ => {} // unknown key ignored
        }
    }

    Ok(config)
}

/// Parse a TOML array of strings: `["a", "b"]`.
fn parse_string_array(value: &str) -> Result<Vec<String>, String> {
    let value = value.trim();
    if !value.starts_with('[') || !value.ends_with(']') {
        return Err("expected an array of strings".to_owned());
    }
    let inner = &value[1..value.len() - 1];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    // ponytail: naive comma split — rule codes and paths never contain
    // commas, so this is correct for the subset.
    inner
        .split(',')
        .map(|s| parse_toml_string(s.trim()))
        .collect()
}

/// Parse a TOML basic string: `"..."` with the usual escapes.
fn parse_toml_string(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !value.starts_with('"') || !value.ends_with('"') || value.len() < 2 {
        return Err("expected a quoted string".to_owned());
    }
    let inner = &value[1..value.len() - 1];
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('b') => out.push('\u{0008}'),
                Some('f') => out.push('\u{000C}'),
                Some(other) => return Err(format!("invalid escape '\\{other}'")),
                None => return Err("unterminated escape".to_owned()),
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}
