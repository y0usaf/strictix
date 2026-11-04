use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::rules::Rule;
use strictix_syntax::TextRange;

pub struct WrappedProse {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    Fixed,
    IndentedCode,
    Item,
    Text,
}

enum Block {
    Fence(u8, usize),
    Raw,
    Html,
}

const RAW: [&str; 4] = ["pre", "script", "style", "textarea"];

fn raw_open(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    RAW.iter().any(|tag| {
        lower
            .strip_prefix('<')
            .and_then(|rest| rest.strip_prefix(tag))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t', '>']))
    })
}

fn raw_close(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    RAW.iter().any(|tag| lower.contains(&format!("</{tag}>")))
}

fn fence(line: &str) -> Option<(u8, usize)> {
    let marker = *line.as_bytes().first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let run = line.bytes().take_while(|&b| b == marker).count();
    (run >= 3).then_some((marker, run))
}

fn item(line: &str) -> bool {
    let bytes = line.as_bytes();
    let marker = match bytes.first() {
        Some(b'-' | b'*' | b'+') => 1,
        Some(b'0'..=b'9') => {
            let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || !matches!(bytes.get(digits), Some(b'.' | b')')) {
                return false;
            }
            digits + 1
        }
        _ => return false,
    };
    matches!(bytes.get(marker), None | Some(b' ' | b'\t'))
}

fn rule_line(line: &str) -> bool {
    let line = line.trim_end();
    let Some(&first) = line.as_bytes().first() else {
        return false;
    };
    let marks = line.bytes().filter(|&b| b == first).count();
    (first == b'=' && marks == line.len())
        || (matches!(first, b'-' | b'*' | b'_')
            && marks >= 3
            && line.bytes().all(|b| b == first || b == b' ' || b == b'\t'))
}

fn reference(line: &str) -> bool {
    line.starts_with('[')
        && line
            .find("]:")
            .is_some_and(|end| end > 1 && !line[1..end].contains(']'))
}

fn classify(lines: &[&str]) -> Vec<Kind> {
    let mut kinds = Vec::with_capacity(lines.len());
    let mut block = None;
    for line in lines {
        let start = line.trim_start();
        let indent = line.len() - start.len();
        let kind = match block {
            Some(Block::Fence(marker, run)) => {
                let closing = start.bytes().take_while(|&b| b == marker).count();
                if closing >= run && start[closing..].trim().is_empty() {
                    block = None;
                }
                Kind::Fixed
            }
            Some(Block::Raw) => {
                if raw_close(line) {
                    block = None;
                }
                Kind::Fixed
            }
            Some(Block::Html) => {
                if start.is_empty() {
                    block = None;
                    Kind::Blank
                } else {
                    Kind::Fixed
                }
            }
            None if start.is_empty() => Kind::Blank,
            None if indent >= 4
                && matches!(kinds.last(), None | Some(Kind::Blank | Kind::IndentedCode)) =>
            {
                Kind::IndentedCode
            }
            None => {
                if let Some(open) = fence(start) {
                    block = Some(Block::Fence(open.0, open.1));
                    Kind::Fixed
                } else if raw_open(start) {
                    if !raw_close(start) {
                        block = Some(Block::Raw);
                    }
                    Kind::Fixed
                } else if start.starts_with('<') {
                    block = Some(Block::Html);
                    Kind::Fixed
                } else if start.starts_with(['#', '|', '>']) || rule_line(start) || reference(start)
                {
                    Kind::Fixed
                } else if item(start) {
                    Kind::Item
                } else {
                    Kind::Text
                }
            }
        };
        kinds.push(kind);
    }
    kinds
}

fn hard_break(line: &str) -> bool {
    line.ends_with("  ") || line.ends_with('\\')
}

impl Rule for WrappedProse {
    fn code(&self) -> &'static str {
        "wrapped-prose"
    }

    fn name(&self) -> &'static str {
        "Wrapped prose"
    }

    fn description(&self) -> &'static str {
        "Flags a Markdown paragraph or list item that continues on the next line through a soft line break, which renders as a space: write each paragraph and item on one line. Code blocks, HTML blocks, tables, headings, quotes, rules and link reference definitions are left as they are, and a hard break (two trailing spaces or a backslash) is kept. Opt-in; no automatic fix."
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
        source: &str,
        _config: &LintConfig,
        diags: &mut Vec<Diagnostic>,
    ) {
        if !path.extension().is_some_and(|ext| {
            ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
        }) {
            return;
        }
        let lines: Vec<&str> = source.split('\n').collect();
        let kinds = classify(&lines);
        let mut offset = 0;
        let mut reported = false;
        for (index, line) in lines.iter().enumerate() {
            let start = offset;
            offset += line.len() + 1;
            let continues = index > 0
                && kinds[index] == Kind::Text
                && matches!(kinds[index - 1], Kind::Text | Kind::Item)
                && !hard_break(lines[index - 1]);
            if !continues {
                reported = false;
                continue;
            }
            if reported {
                continue;
            }
            reported = true;
            let start = u32::try_from(start).expect("source fits in u32");
            let end = start + u32::try_from(line.len()).expect("line fits in u32");
            diags.push(
                Diagnostic::new(
                    self.code(),
                    Severity::Warning,
                    "this paragraph continues on the next line",
                    TextRange::new(start, end),
                )
                .with_help("join the paragraph's lines into one line"),
            );
        }
    }
}
