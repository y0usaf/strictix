//! Diagnostic rendering: the shared one-line format, the human format
//! with source excerpts, and the machine-readable JSON document.
//!
//! The one-line format is the deterministic contract shared with
//! snapshot tests in core and lints:
//!
//! ```text
//! [unused-let-binding] warning 10..23 binding 'x' is never used
//! ```
//!
//! The human format adds the file path, 1-based byte line/column, the
//! offending source line with a caret under the diagnostic's range, and
//! an optional help line. The JSON format mirrors the same fields as a
//! serde_json document, one entry per file plus a summary.

use strictix_core::diagnostic::Diagnostic;
use strictix_core::fix::Fix;
use strictix_syntax::TextRange;

/// The deterministic one-line rendering shared by CLI and tests:
/// `[code] severity start..end message`.
#[must_use]
#[allow(dead_code)] // contract rendering kept for core/lints snapshot tests
pub fn one_line(diag: &Diagnostic) -> String {
    format!(
        "[{}] {} {}..{} {}",
        diag.code,
        diag.severity_str(),
        diag.range.start(),
        diag.range.end(),
        diag.message
    )
}

/// Render one diagnostic in the human format (path, line/col, message,
/// source excerpt with caret, help) with the source it was found in.
#[must_use]
pub fn human(diag: &Diagnostic, path: &str, source: &str) -> String {
    let (line, col) = line_col(source, diag.range.start());
    let mut out = format!(
        "{path}:{line}:{col}: {}[{}]: {}",
        diag.severity_str(),
        diag.code,
        diag.message
    );
    if let Some((text, caret)) = source_excerpt(source, diag.range) {
        out.push('\n');
        out.push_str(text);
        out.push('\n');
        out.push_str(&caret);
    }
    if let Some(help) = &diag.help {
        out.push_str("\n  help: ");
        out.push_str(help);
    }
    out
}

/// The 1-based (line, byte column) of a byte offset in source.
fn line_col(source: &str, offset: u32) -> (u32, u32) {
    let offset = (offset as usize).min(source.len());
    let mut line = 1u32;
    let mut line_start = 0usize;
    for (i, b) in source.bytes().enumerate() {
        if i >= offset {
            break;
        }
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    (line, (offset - line_start) as u32 + 1)
}

/// The source line containing `range.start` and a caret line marking
/// the range's width, clamped to the line. Returns `None` when the
/// offset sits past the end of the source.
fn source_excerpt(source: &str, range: TextRange) -> Option<(&str, String)> {
    let start = range.start() as usize;
    if start > source.len() {
        return None;
    }
    // Line start: after the last newline before start (or 0).
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    // Line end: the next newline after start, or EOF.
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
    let text = &source[line_start..line_end];
    let col = start - line_start;
    let available = line_end.saturating_sub(start);
    let width = (range.end() as usize - start).min(available);
    let caret = format!("{}{}", " ".repeat(col), "^".repeat(width));
    Some((text, caret))
}

/// Build the JSON document for a whole run: one entry per file holding
/// that file's diagnostics, plus a summary object. `fix` is `null`
/// when a diagnostic carries no fix; `help` likewise.
pub fn json(files: &[JsonFile]) -> serde_json::Value {
    let mut file_values = Vec::with_capacity(files.len());
    let mut diag_total = 0usize;
    for file in files {
        let mut diags = Vec::with_capacity(file.diagnostics.len());
        for diag in &file.diagnostics {
            diag_total += 1;
            diags.push(serde_json::json!({
                "code": diag.code,
                "severity": diag.severity_str(),
                "message": diag.message,
                "range": { "start": diag.range.start(), "end": diag.range.end() },
                "help": diag.help,
                "fix": diag.fix.as_ref().map(fix_json),
            }));
        }
        file_values.push(serde_json::json!({
            "path": file.path,
            "diagnostics": diags,
        }));
    }
    serde_json::json!({
        "files": file_values,
        "summary": { "files": files.len(), "diagnostics": diag_total },
    })
}

/// The JSON shape of one fix: label plus its edits.
fn fix_json(fix: &Fix) -> serde_json::Value {
    let edits: Vec<serde_json::Value> = fix
        .edits
        .iter()
        .map(|edit| {
            serde_json::json!({
                "range": { "start": edit.range.start(), "end": edit.range.end() },
                "replacement": edit.replacement,
            })
        })
        .collect();
    serde_json::json!({ "label": fix.label, "edits": edits })
}

/// One file's worth of results, as the JSON renderer consumes it.
pub struct JsonFile {
    pub path: String,
    pub diagnostics: Vec<Diagnostic>,
}

/// The human summary line: `N file(s) linted, M diagnostic(s) found`,
/// with a clean run reading `0 diagnostics found`.
#[must_use]
pub fn summary_line(files: usize, diagnostics: usize) -> String {
    let diag_part = if diagnostics == 0 {
        "0 diagnostics found".to_owned()
    } else {
        format!("{diagnostics} diagnostic(s) found")
    };
    format!("{files} file(s) linted, {diag_part}")
}
