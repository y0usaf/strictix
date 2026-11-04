//! Per-run lint configuration.
//!
//! A plain data struct: no serde here (the CLI hand-parses its own TOML
//! subset and converts it into this type). The schema field backs the
//! M8 `unknown-option` rule; `None` means that
//! rule is off for this run.

use std::path::{Path, PathBuf};

/// Configuration for one lint run: which rule codes are skipped and
/// whether the options schema rule is active.
#[derive(Debug, Clone, Default)]
pub struct LintConfig {
    /// Opt-in rule codes explicitly enabled. `disabled` takes precedence.
    pub enabled: Vec<String>,
    /// Rule codes to skip. A rule whose code appears here never fires.
    pub disabled: Vec<String>,
    /// options.json path (M8); `None` = schema rule off.
    pub schema: Option<PathBuf>,
    /// Per-path schema overrides as `(path prefix, schema)`; a `None`
    /// schema turns the schema rules off under that prefix. The longest
    /// matching prefix wins over [Self::schema].
    pub schemas: Vec<(PathBuf, Option<PathBuf>)>,
    /// Path prefixes of generated data files, exempt from
    /// `hand-maintained-data-file`.
    pub generated: Vec<PathBuf>,
    /// Codecs from `[codecs.<id>]` as `(id, extensions)`; non-empty
    /// enables `codec-candidate`.
    pub codecs: Vec<(String, Vec<String>)>,
    pub codec_generators: Vec<(String, String)>,
    pub settings: Vec<(String, String, String)>,
}

impl LintConfig {
    /// Builder: replace the list of explicitly enabled rules.
    pub fn with_enabled(mut self, codes: impl IntoIterator<Item = String>) -> Self {
        self.enabled = codes.into_iter().collect();
        self
    }

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

    /// The options.json that applies to `file`: the longest matching
    /// [Self::schemas] prefix, else [Self::schema]. A leading `./` on
    /// either side is ignored.
    #[must_use]
    pub fn schema_for(&self, file: Option<&Path>) -> Option<&Path> {
        let bare = |p: &'_ Path| p.strip_prefix(".").unwrap_or(p).to_path_buf();
        file.map(bare)
            .and_then(|file| {
                self.schemas
                    .iter()
                    .filter(|(prefix, _)| file.starts_with(bare(prefix)))
                    .max_by_key(|(prefix, _)| bare(prefix).components().count())
            })
            .map_or(self.schema.as_deref(), |(_, schema)| schema.as_deref())
    }

    /// Whether `file` lies under a [Self::generated] prefix. A leading
    /// `./` on either side is ignored.
    #[must_use]
    pub fn is_generated(&self, file: &Path) -> bool {
        let bare = |p: &'_ Path| p.strip_prefix(".").unwrap_or(p).to_path_buf();
        let file = bare(file);
        self.generated
            .iter()
            .any(|prefix| file.starts_with(bare(prefix)))
    }

    /// Builder: set the schema path, enabling the schema rule.
    pub fn with_schema(mut self, path: impl Into<PathBuf>) -> Self {
        self.schema = Some(path.into());
        self
    }

    pub fn with_setting(mut self, code: &str, key: &str, value: impl Into<String>) -> Self {
        self.settings.retain(|(c, k, _)| !(c == code && k == key));
        self.settings
            .push((code.to_owned(), key.to_owned(), value.into()));
        self
    }

    #[must_use]
    pub fn setting_str(&self, code: &str, key: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|(c, k, _)| c == code && k == key)
            .map(|(_, _, value)| value.as_str())
    }

    #[must_use]
    pub fn setting_usize(&self, code: &str, key: &str) -> Option<usize> {
        self.setting_str(code, key)?.parse().ok()
    }

    pub fn setting_list<'s>(
        &'s self,
        code: &'s str,
        key: &'s str,
    ) -> impl Iterator<Item = &'s str> {
        self.settings
            .iter()
            .filter(move |(c, k, _)| c == code && k == key)
            .map(|(_, _, value)| value.as_str())
    }
}
