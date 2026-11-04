use strictix_core::diagnostic::Severity;
use strictix_core::rules::Rule;

pub struct UnformattedFile {}

impl Rule for UnformattedFile {
    fn code(&self) -> &'static str {
        "unformatted-file"
    }

    fn name(&self) -> &'static str {
        "Unformatted file"
    }

    fn description(&self) -> &'static str {
        "Flags a .nix file the project's formatter would change, at the first line it changes. `command` in `[lint.unformatted-file]` is the formatter's command line (split on spaces), which reads Nix on stdin and prints it formatted, such as `alejandra --quiet -`; the CLI runs it on every linted file, after fixes, and skips files with syntax errors. Opt-in, and inert without `command`; no automatic fix."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }
}
