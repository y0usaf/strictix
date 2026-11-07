//! Diagnostics: one rule finding, optionally with help and an auto fix.
//!
//! A diagnostic is the unit of linter output: a rule produces them, the
//! CLI renders them. The fix rides along here instead of being returned
//! separately so a rule emits everything the user needs to act in one
//! object — rendering and fixing then share a single path over the same
//! data, and there is no second channel for rules to forget.

use crate::fix::Fix;
use strictix_syntax::TextRange;

/// How severe a finding is.
///
/// `Warning` flags smell; `Error` flags things that are wrong enough to
/// block acceptance. The distinction is purely informational for the
/// renderer — both kinds carry fixes the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// A single finding from a rule, anchored to a byte range in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Rule code, kebab-case, e.g. "unused-let-binding".
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub range: TextRange,
    /// Optional human hint, rendered below the message by the CLI.
    pub help: Option<String>,
    /// Optional automatic fix; `None` when the rule has no safe rewrite.
    pub fix: Option<Fix>,
}

impl Diagnostic {
    /// Start a diagnostic with the fields every rule must provide.
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

    /// Attach a help hint. Consumes and returns `self` so rules can
    /// build the diagnostic in one expression.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attach an automatic fix.
    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }

    /// The severity as the lowercase string the rendering contract uses
    /// ("warning" / "error"), shared by the one-line, human, and
    /// JSON renderers so they cannot drift apart.
    #[must_use]
    pub const fn severity_str(&self) -> &'static str {
        match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}
