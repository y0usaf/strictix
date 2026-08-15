//! Per-run lint configuration.
//!
//! A plain data struct: no serde here (the CLI hand-parses its own TOML
//! subset and converts it into this type). The schema field backs the
//! M8 `unknown-option` rule; `None` means that
//! rule is off for this run.

use std::path::PathBuf;

/// Configuration for one lint run: which rule codes are skipped and
/// whether the options schema rule is active.
#[derive(Debug, Clone, Default)]
pub struct LintConfig {
    /// Rule codes to skip. A rule whose code appears here never fires.
    pub disabled: Vec<String>,
    /// options.json path (M8); `None` = schema rule off.
    pub schema: Option<PathBuf>,
}

impl LintConfig {
    /// Whether the rule with `code` should run: true unless `disabled`
    /// contains `code`.
    #[must_use]
    pub fn is_enabled(&self, code: &str) -> bool {
        !self.disabled.iter().any(|d| d == code)
    }

    /// Builder: replace the disabled list with `codes`.
    pub fn with_disabled(mut self, codes: impl IntoIterator<Item = String>) -> Self {
        self.disabled = codes.into_iter().collect();
        self
    }

    /// Builder: set the schema path, enabling the schema rule.
    pub fn with_schema(mut self, path: impl Into<PathBuf>) -> Self {
        self.schema = Some(path.into());
        self
    }
}
