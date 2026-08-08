//! TOML lint configuration.
//!
//! serde + toml are the CLI's blessed dependencies and only ever live
//! here: the CLI deserializes `strictix.toml` into its own DTO, then
//! converts it into [strictix_core::config::LintConfig], which is
//! deliberately serde-free. The on-disk shape is:
//!
//! ```toml
//! [lint]
//! disabled = ["tautology"]
//! schema = "options.json"
//! ```
//!
//! Unknown keys are ignored (plain serde, no deny_unknown_fields), and
//! missing keys are fine.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use strictix_core::config::LintConfig;

/// The on-disk TOML shape. All fields optional so a config file that
/// only sets one thing still loads.
#[derive(Debug, Default, Deserialize)]
struct TomlConfig {
    #[serde(default)]
    lint: Option<LintTable>,
}

/// The `[lint]` table.
#[derive(Debug, Default, Deserialize)]
struct LintTable {
    #[serde(default)]
    disabled: Option<Vec<String>>,
    #[serde(default)]
    schema: Option<String>,
}

/// Load the TOML config and merge in the command-line overrides.
///
/// `config_path` is the file to read (`--config`, or the default
/// `./strictix.toml` when it exists); `None` means no config file —
/// the disabled list is then just the `--disable` flags. `flags` are
/// the repeatable `--disable` codes; `schema_flag` is the
/// `--schema` path when given.
///
/// Merging rules: disabled = TOML `disabled` followed by the
/// `--disable` flags, deduplicated keeping first occurrence; schema =
/// `--schema` if given, else the TOML `schema`.
///
/// # Errors
///
/// - The config file cannot be read.
/// - The config file is not valid TOML.
pub fn load_config(
    config_path: Option<&Path>,
    flags: &[String],
    schema_flag: Option<&Path>,
) -> Result<LintConfig, String> {
    let toml_config = match config_path {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read config file {}: {e}", path.display()))?;
            toml::from_str::<TomlConfig>(&text)
                .map_err(|e| format!("cannot parse config file {}: {e}", path.display()))?
        }
        None => TomlConfig::default(),
    };

    let mut disabled: Vec<String> = Vec::new();
    if let Some(lint) = &toml_config.lint {
        if let Some(codes) = &lint.disabled {
            for code in codes {
                if !disabled.contains(code) {
                    disabled.push(code.clone());
                }
            }
        }
    }
    for code in flags {
        if !disabled.contains(code) {
            disabled.push(code.clone());
        }
    }

    let schema = schema_flag.map(PathBuf::from).or_else(|| {
        toml_config
            .lint
            .as_ref()
            .and_then(|l| l.schema.clone())
            .map(PathBuf::from)
    });

    Ok(LintConfig { disabled, schema })
}
