use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct LintConfig {
        pub enabled: Vec<String>,
        pub disabled: Vec<String>,
        pub schema: Option<PathBuf>,
        pub schemas: Vec<(PathBuf, Option<PathBuf>)>,
        pub generated: Vec<PathBuf>,
    pub settings: Vec<(String, String, String)>,
}

impl LintConfig {
        #[must_use]
    pub fn is_enabled(&self, code: &str) -> bool {
        !self.disabled.iter().any(|d| d == code)
    }

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

        #[must_use]
    pub fn is_generated(&self, file: &Path) -> bool {
        let bare = |p: &'_ Path| p.strip_prefix(".").unwrap_or(p).to_path_buf();
        let file = bare(file);
        self.generated
            .iter()
            .any(|prefix| file.starts_with(bare(prefix)))
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
