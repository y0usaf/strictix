use std::{fs, io::Write, path::Path, process::Command};

use tempfile::NamedTempFile;

pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn create_fixture(expression: &str) -> anyhow::Result<NamedTempFile> {
    let mut fixture = NamedTempFile::with_suffix(".nix")?;
    fixture.write_all(expression.as_bytes())?;
    fixture.write_all(b"\n")?; // otherwise diff says there's no newline at end of file
    Ok(fixture)
}

fn sanitize_output(output: Vec<u8>, path: &Path) -> anyhow::Result<String> {
    let output = strip_ansi_escapes::strip(output)?;
    let output = String::from_utf8(output)?;
    Ok(output.replace(path.to_str().unwrap(), "<temp_file_path>"))
}

pub fn run_cli(path: &Path, args: &[&str]) -> anyhow::Result<CliOutput> {
    let output = Command::new("cargo")
        .arg("run")
        .arg("--")
        .args(args)
        .arg(path)
        .output()?;

    Ok(CliOutput {
        success: output.status.success(),
        stdout: sanitize_output(output.stdout, path)?,
        stderr: sanitize_output(output.stderr, path)?,
    })
}

pub fn test_cli(expression: &str, args: &[&str]) -> anyhow::Result<String> {
    let fixture = create_fixture(expression)?;
    Ok(run_cli(fixture.path(), args)?.stdout)
}

pub fn assert_fix_roundtrip(expression: &str) -> anyhow::Result<()> {
    let fixture = create_fixture(expression)?;
    let original = fs::read_to_string(fixture.path())?;

    let first_fix = run_cli(fixture.path(), &["fix"])?;
    assert!(
        first_fix.success,
        "strictix fix failed\nstdout:\n{}\nstderr:\n{}",
        first_fix.stdout, first_fix.stderr,
    );

    let first_after = fs::read_to_string(fixture.path())?;

    if first_after != original {
        let check = run_cli(fixture.path(), &["check"])?;
        assert!(
            check.success,
            "strictix check failed after fix\nstdout:\n{}\nstderr:\n{}",
            check.stdout, check.stderr,
        );
    }

    let second_fix = run_cli(fixture.path(), &["fix"])?;
    assert!(
        second_fix.success,
        "second strictix fix failed\nstdout:\n{}\nstderr:\n{}",
        second_fix.stdout, second_fix.stderr,
    );

    let second_after = fs::read_to_string(fixture.path())?;
    assert_eq!(
        first_after, second_after,
        "strictix fix is not idempotent\nfirst run stdout:\n{}\nfirst run stderr:\n{}\nsecond run stdout:\n{}\nsecond run stderr:\n{}",
        first_fix.stdout, first_fix.stderr, second_fix.stdout, second_fix.stderr,
    );

    let dry_run = run_cli(fixture.path(), &["fix", "--dry-run"])?;
    assert!(
        dry_run.success,
        "strictix fix --dry-run failed after convergence\nstdout:\n{}\nstderr:\n{}",
        dry_run.stdout, dry_run.stderr,
    );
    assert!(
        dry_run.stdout.trim().is_empty(),
        "strictix fix --dry-run still reports changes after convergence\nstdout:\n{}\nstderr:\n{}",
        dry_run.stdout,
        dry_run.stderr,
    );

    Ok(())
}
