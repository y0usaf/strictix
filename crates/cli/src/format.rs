use std::io::Write as _;
use std::process::{Command, Stdio};

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_syntax::TextRange;

const CODE: &str = "unformatted-file";

pub fn command(config: &LintConfig) -> Option<&str> {
    (config.is_enabled(CODE) && config.enabled.iter().any(|code| code == CODE))
        .then(|| config.setting_str(CODE, "command"))
        .flatten()
}

pub fn apply(command: &str, input: &crate::Input, result: &mut crate::FileResult) {
    let Ok(source) = &input.source else {
        return;
    };
    if result.diagnostics.iter().any(|d| d.code == "syntax-error") {
        return;
    }
    match check(command, result.fixed.as_deref().unwrap_or(source)) {
        Ok(Some(diagnostic)) => {
            result.diagnostics.push(diagnostic);
            result.diagnostics.sort_by_key(|d| d.range.start());
        }
        Ok(None) => {}
        Err(message) => result.read_error = Some(format!("{}: {message}", input.path.display())),
    }
}

fn check(command: &str, source: &str) -> Result<Option<Diagnostic>, String> {
    let mut words = command.split_whitespace();
    let program = words
        .next()
        .ok_or_else(|| format!("[lint.{CODE}] command is empty"))?;
    let mut child = Command::new(program)
        .args(words)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("running the formatter {program}: {e}"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = source.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let output = child
        .wait_with_output()
        .map_err(|e| format!("running the formatter {program}: {e}"))?;
    writer
        .join()
        .expect("formatter input thread panicked")
        .map_err(|e| format!("writing to the formatter {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "the formatter {program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let formatted = output.stdout;
    let source = source.as_bytes();
    if formatted == source {
        return Ok(None);
    }
    let differs = source
        .iter()
        .zip(&formatted)
        .position(|(a, b)| a != b)
        .unwrap_or(source.len().min(formatted.len()));
    let start = source[..differs]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1);
    let start = u32::try_from(start).expect("source fits in u32");
    Ok(Some(
        Diagnostic::new(
            CODE,
            Severity::Warning,
            "the formatter would change this file from this line on",
            TextRange::new(start, start),
        )
        .with_help(format!("format the file with {program}")),
    ))
}
