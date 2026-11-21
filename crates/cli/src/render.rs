use std::fmt::Display;
use std::io::IsTerminal;

use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::json::write_json_str;
use strictix_syntax::TextRange;

struct Theme {
    error: &'static str,
    warning: &'static str,
    rule: &'static str,
    path: &'static str,
    help: &'static str,
    added: &'static str,
    removed: &'static str,
    clean: &'static str,
}

const TERMINAL: Theme = Theme {
    error: "31;1",
    warning: "33",
    rule: "35",
    path: "36;1",
    help: "36",
    added: "32",
    removed: "31",
    clean: "32",
};

#[must_use]
pub fn color_enabled() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

impl Theme {
    fn severity(&self, severity: Severity) -> &'static str {
        match severity {
            Severity::Warning => self.warning,
            Severity::Error => self.error,
        }
    }
}

fn paint<T: Display>(enabled: bool, code: &str, value: T) -> String {
    if enabled {
        format!("\x1b[{code}m{value}\x1b[0m")
    } else {
        value.to_string()
    }
}

#[must_use]
pub fn human(diag: &Diagnostic, path: &str, source: &str, color: bool) -> String {
    let (line, col) = line_col(source, diag.range.start());
    let severity = paint(color, TERMINAL.severity(diag.severity), diag.severity_str());
    let code = paint(color, TERMINAL.rule, diag.code);
    let path = paint(color, TERMINAL.path, path);
    let mut out = format!("{path}:{line}:{col}: {severity}[{code}]: {}", diag.message);
    if let Some((text, caret)) = source_excerpt(source, diag.range) {
        out.push('\n');
        out.push_str(text);
        out.push('\n');
        out.push_str(&paint(color, TERMINAL.severity(diag.severity), &caret));
    }
    if let Some(help) = &diag.help {
        out.push_str("\n  ");
        out.push_str(&paint(color, TERMINAL.help, "help:"));
        out.push(' ');
        out.push_str(help);
    }
    out
}

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

fn source_excerpt(source: &str, range: TextRange) -> Option<(&str, String)> {
    let start = range.start() as usize;
    if start > source.len() || !source.is_char_boundary(start) {
        return None;
    }
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
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

pub fn write_json_doc(
    files: &[(&std::path::Path, &[Diagnostic])],
    total_diags: usize,
    out: &mut impl std::io::Write,
) -> std::io::Result<()> {
    out.write_all(b"{\n")?;
    if files.is_empty() {
        out.write_all(b"  \"files\": [],\n")?;
    } else {
        out.write_all(b"  \"files\": [\n")?;
        for (fi, &(path, diags)) in files.iter().enumerate() {
            if fi > 0 {
                out.write_all(b",\n")?;
            }
            out.write_all(b"    {\n      \"path\": ")?;
            match path.to_str() {
                Some(s) => write_json_str(s, out)?,
                None => write_json_str(&path.to_string_lossy(), out)?,
            }
            if diags.is_empty() {
                out.write_all(b",\n      \"diagnostics\": []\n    }")?;
            } else {
                out.write_all(b",\n      \"diagnostics\": [\n")?;
                for (di, diag) in diags.iter().enumerate() {
                    if di > 0 {
                        out.write_all(b",\n")?;
                    }
                    out.write_all(b"        {\n          \"code\": ")?;
                    write_json_str(diag.code, out)?;
                    out.write_all(b",\n          \"severity\": ")?;
                    write_json_str(diag.severity_str(), out)?;
                    out.write_all(b",\n          \"message\": ")?;
                    write_json_str(&diag.message, out)?;
                    writeln!(
                        out,
                        ",\n          \"range\": {{\n            \"start\": {},\n            \"end\": {}\n          }},",
                        diag.range.start(),
                        diag.range.end()
                    )?;
                    out.write_all(b"          \"help\": ")?;
                    match &diag.help {
                        Some(h) => write_json_str(h, out)?,
                        None => out.write_all(b"null")?,
                    }
                    out.write_all(b",\n          \"fix\": ")?;
                    match &diag.fix {
                        None => out.write_all(b"null")?,
                        Some(fix) => {
                            out.write_all(b"{\n            \"label\": ")?;
                            write_json_str(&fix.label, out)?;
                            if fix.edits.is_empty() {
                                out.write_all(b",\n            \"edits\": []\n          }")?;
                            } else {
                                out.write_all(b",\n            \"edits\": [\n")?;
                                for (ei, edit) in fix.edits.iter().enumerate() {
                                    if ei > 0 {
                                        out.write_all(b",\n")?;
                                    }
                                    writeln!(
                                        out,
                                        "              {{\n                \"range\": {{\n                  \"start\": {},\n                  \"end\": {}\n                }},",
                                        edit.range.start(),
                                        edit.range.end()
                                    )?;
                                    out.write_all(b"                \"replacement\": ")?;
                                    write_json_str(&edit.replacement, out)?;
                                    out.write_all(b"\n              }")?;
                                }
                                out.write_all(b"\n            ]\n          }")?;
                            }
                        }
                    }
                    out.write_all(b"\n        }")?;
                }
                out.write_all(b"\n      ]\n    }")?;
            }
        }
        out.write_all(b"\n  ],\n")?;
    }
    writeln!(
        out,
        "  \"summary\": {{\n    \"files\": {},\n    \"diagnostics\": {}\n  }}\n}}",
        files.len(),
        total_diags
    )
}

#[must_use]
pub fn summary_line(files: usize, diagnostics: usize, color: bool) -> String {
    let diag_part = if diagnostics == 0 {
        paint(color, TERMINAL.clean, "0 diagnostics found")
    } else {
        paint(
            color,
            TERMINAL.warning,
            format!("{diagnostics} diagnostic(s) found"),
        )
    };
    format!("{files} file(s) linted, {diag_part}")
}

#[must_use]
pub fn diff(before: &str, after: &str, color: bool) -> String {
    let a: Vec<&str> = before.split('\n').collect();
    let b: Vec<&str> = after.split('\n').collect();

    let mut start = 0usize;
    while start < a.len() && start < b.len() && a[start] == b[start] {
        start += 1;
    }
    let mut a_end = a.len();
    let mut b_end = b.len();
    while a_end > start && b_end > start && a[a_end - 1] == b[b_end - 1] {
        a_end -= 1;
        b_end -= 1;
    }

    let mut out = String::new();
    for line in &a[start..a_end] {
        out.push_str(&paint(color, TERMINAL.removed, format!("-{}\n", line)));
    }
    for line in &b[start..b_end] {
        out.push_str(&paint(color, TERMINAL.added, format!("+{}\n", line)));
    }
    out
}
