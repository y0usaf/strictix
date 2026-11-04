use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::rules::Rule;
use strictix_syntax::TextRange;

pub struct ScatteredDocumentation {}

impl Rule for ScatteredDocumentation {
    fn code(&self) -> &'static str {
        "scattered-documentation"
    }

    fn name(&self) -> &'static str {
        "Scattered documentation"
    }

    fn description(&self) -> &'static str {
        "Flags a Markdown file other than the project's one document, `file` in `[lint.scattered-documentation]` (default `README.md`), so the project's prose lives in one place and the tree carries no file that only documents a directory. Opt-in."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_path(
        &self,
        path: &Path,
        _source: &str,
        config: &LintConfig,
        diags: &mut Vec<Diagnostic>,
    ) {
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            return;
        }
        let document = config
            .setting_str(self.code(), "file")
            .unwrap_or("README.md");
        let bare = |p: &'_ Path| p.strip_prefix(".").unwrap_or(p).to_path_buf();
        if bare(path) == bare(Path::new(document)) {
            return;
        }
        diags.push(
            Diagnostic::new(
                self.code(),
                Severity::Warning,
                format!("documentation outside {document}"),
                TextRange::new(0, 0),
            )
            .with_help(format!(
                "fold this file into {document} under a heading naming its directory and delete it"
            )),
        );
    }
}
