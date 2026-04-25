mod _utils;

use std::fs;

use anyhow::Result;
use clap::Parser;
use strictix::config::{Opts, SubCommand};
use tempfile::tempdir;

#[test]
fn nested_gitignore_files_are_respected() -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path();
    let nested = root.join("nested");
    fs::create_dir_all(&nested)?;

    fs::write(
        root.join("strictix.toml"),
        "enabled = [\"with_expression\"]\n",
    )?;
    fs::write(root.join("keep.nix"), "with pkgs; foo\n")?;
    fs::write(root.join(".gitignore"), "ignored-root.nix\n")?;
    fs::write(root.join("ignored-root.nix"), "with pkgs; foo\n")?;

    fs::write(nested.join(".gitignore"), "ignored-nested.nix\n")?;
    fs::write(nested.join("ignored-nested.nix"), "with pkgs; foo\n")?;
    fs::write(nested.join("kept-nested.nix"), "with pkgs; foo\n")?;

    let output = _utils::run_cli(root, &["check"])?;

    assert!(
        !output.success,
        "expected lint failures, got success\nstdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(output.stdout.contains("keep.nix"));
    assert!(output.stdout.contains("kept-nested.nix"));
    assert!(!output.stdout.contains("ignored-root.nix"));
    assert!(!output.stdout.contains("ignored-nested.nix"));

    Ok(())
}

#[test]
fn diagnostics_are_emitted_in_deterministic_path_order() -> Result<()> {
    let temp = tempdir()?;
    let root = temp.path();

    fs::write(
        root.join("strictix.toml"),
        "enabled = [\"with_expression\"]\n",
    )?;
    fs::write(root.join("z-last.nix"), "with pkgs; foo\n")?;
    fs::write(root.join("a-first.nix"), "with pkgs; foo\n")?;

    let first = _utils::run_cli(root, &["check"])?;
    let second = _utils::run_cli(root, &["check"])?;

    assert_eq!(first.stderr, second.stderr);
    assert_eq!(first.stdout, second.stdout);
    let first_idx = first
        .stdout
        .find("a-first.nix")
        .expect("missing a-first.nix in stdout");
    let second_idx = first
        .stdout
        .find("z-last.nix")
        .expect("missing z-last.nix in stdout");
    assert!(
        first_idx < second_idx,
        "expected a-first.nix before z-last.nix\nstdout:\n{}",
        first.stdout
    );

    Ok(())
}

#[test]
fn dry_run_conflicts_with_stdin_for_fix() {
    let err = Opts::try_parse_from(["strictix", "fix", "--dry-run", "--stdin"])
        .expect_err("expected clap conflict for fix");
    let message = err.to_string();
    assert!(message.contains("--dry-run"));
    assert!(message.contains("--stdin"));
}

#[test]
fn dry_run_conflicts_with_stdin_for_single() {
    let err = Opts::try_parse_from([
        "strictix",
        "single",
        "--position",
        "1,1",
        "--dry-run",
        "--stdin",
    ])
    .expect_err("expected clap conflict for single");
    let message = err.to_string();
    assert!(message.contains("--dry-run"));
    assert!(message.contains("--stdin"));
}

#[test]
fn single_requires_target_or_stdin() {
    let err = Opts::try_parse_from(["strictix", "single", "--position", "1,1"])
        .expect_err("expected clap validation error for missing input source");
    let message = err.to_string();
    assert!(message.contains("--stdin") || message.contains("<TARGET>"));
}

#[test]
fn single_target_conflicts_with_stdin() {
    let err = Opts::try_parse_from([
        "strictix",
        "single",
        "target.nix",
        "--position",
        "1,1",
        "--stdin",
    ])
    .expect_err("expected clap conflict for single target + stdin");
    let message = err.to_string();
    assert!(message.contains("target.nix") || message.contains("<TARGET>"));
    assert!(message.contains("--stdin"));
}

#[test]
fn fix_discovers_config_from_target_when_invoked_elsewhere() -> Result<()> {
    let caller = tempdir()?;
    let project = tempdir()?;
    let file_path = project.path().join("test.nix");

    fs::write(&file_path, "if x == true then y else z\n")?;
    fs::write(
        project.path().join("strictix.toml"),
        "disabled = [\"bool_comparison\"]\n",
    )?;

    let output = _utils::run_cli_in_dir(caller.path(), &file_path, &["fix", "--dry-run"])?;

    assert!(
        output.success,
        "expected fix --dry-run to succeed\nstdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(
        output.stdout.trim().is_empty(),
        "expected no fix because the lint is disabled\nstdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );

    Ok(())
}

#[test]
fn check_defaults_config_discovery_to_target_path() {
    let opts = Opts::parse_from(["strictix", "check", "some/target"]);
    let SubCommand::Check(check) = opts.cmd else {
        panic!("expected check subcommand");
    };

    assert_eq!(check.target, std::path::PathBuf::from("some/target"));
    assert_eq!(check.conf_path, None);
}
