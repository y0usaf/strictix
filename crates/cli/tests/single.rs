mod _utils;

use std::fs;

use anyhow::Context;
use tempfile::tempdir;

use self::_utils::{run_cli, run_cli_in_dir};

#[test]
fn single_respects_config_disabled_lint() -> anyhow::Result<()> {
    let dir = tempdir()?;
    let file_path = dir.path().join("test.nix");
    let config_path = dir.path().join("strictix.toml");

    fs::write(&file_path, "if x == true then y else z\n")?;
    fs::write(&config_path, "disabled = [\"bool_comparison\"]\n")?;

    let check = run_cli(
        &file_path,
        &[
            "check",
            "--config",
            dir.path().to_str().context("utf-8 path")?,
        ],
    )?;
    assert!(
        check.success,
        "stdout:\n{}\nstderr:\n{}",
        check.stdout, check.stderr
    );
    assert!(
        check.stdout.trim().is_empty(),
        "stdout:\n{}\nstderr:\n{}",
        check.stdout,
        check.stderr
    );

    let output = run_cli(
        &file_path,
        &[
            "single",
            "--config",
            dir.path().to_str().context("utf-8 path")?,
            "--position",
            "1,5",
        ],
    )?;

    assert!(
        !output.success,
        "stdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(
        output.stderr.contains("nothing to fix"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        fs::read_to_string(&file_path)?,
        "if x == true then y else z\n"
    );

    Ok(())
}

#[test]
fn single_discovers_config_from_target_when_invoked_elsewhere() -> anyhow::Result<()> {
    let caller = tempdir()?;
    let project = tempdir()?;
    let file_path = project.path().join("test.nix");

    fs::write(&file_path, "if x == true then y else z\n")?;
    fs::write(
        project.path().join("strictix.toml"),
        "disabled = [\"bool_comparison\"]\n",
    )?;

    let output = run_cli_in_dir(caller.path(), &file_path, &["single", "--position", "1,5"])?;

    assert!(
        !output.success,
        "stdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(
        output.stderr.contains("nothing to fix"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        fs::read_to_string(&file_path)?,
        "if x == true then y else z\n"
    );

    Ok(())
}
