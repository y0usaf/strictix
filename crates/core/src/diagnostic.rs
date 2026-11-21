use crate::fix::Fix;
use strictix_syntax::TextRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
        pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub range: TextRange,
        pub help: Option<String>,
        pub fix: Option<Fix>,
}

impl Diagnostic {
        #[must_use]
    pub fn new(
        code: &'static str,
        severity: Severity,
        message: impl Into<String>,
        range: TextRange,
    ) -> Self {
        Self {
            code,
            severity,
            message: message.into(),
            range,
            help: None,
            fix: None,
        }
    }

        #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

        #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

        #[must_use]
    pub const fn severity_str(&self) -> &'static str {
        match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}
