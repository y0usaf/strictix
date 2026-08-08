//! End-to-end CLI tests, running the real binary.
//!
//! These exercise the whole stack: argument parsing, config loading,
//! directory walking, ignore files, per-file worker threads, rule
//! dispatch, and rendering - all through the compiled strictix
//! binary. Two fixtures carry the cleanliness guarantees the rule set
//! depends on: the clean fixture ("let x = 1; in x") produces zero
//! diagnostics under every rule, and the dirty fixture
//! ("let x = 1; in 2") triggers exactly unused-let-binding (when the
//! registry is populated). Tests that depend on a populated registry
//! detect it first via the list command and skip their assertions when
//! the registry is still an empty stub (sibling milestones land in any
//! order).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Run the binary with the given args from the workspace directory.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strictix"))
        .args(args)
        .output()
        .expect("binary starts")
}

/// Run the binary with the given args from cwd.
fn run_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_strictix"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("binary starts")
}

/// Absolute path to a fixture under crates/cli/tests/fixtures.
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

/// Whether the rule registry has rules yet (detected by running
/// 'strictix list' and looking for a real rule code). The registry is
/// built by a sibling milestone and may be an empty stub while that
/// work is still landing.
fn rules_available() -> bool {
    let out = run(&["list"]);
    out.status.code() == Some(0)
        && String::from_utf8_lossy(&out.stdout).contains("unused-let-binding")
}

// --- help / version -------------------------------------------------

#[test]
fn help_exits_zero_and_mentions_commands() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("check"), "help mentions check");
    assert!(stdout.contains("fix"), "help mentions fix");
}

#[test]
fn version_prints_crate_version() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "strictix 0.1.0"
    );
}

// --- list -----------------------------------------------------------

#[test]
fn list_exits_zero_and_prints_rules() {
    let out = run(&["list"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.is_empty(), "list output is non-empty");
    // Loose format assertion: a real rule line carries its severity.
    if stdout.lines().any(|l| l.contains("unused-let-binding")) {
        let line = stdout
            .lines()
            .find(|l| l.contains("unused-let-binding"))
            .expect("line exists");
        assert!(
            line.contains("warning"),
            "rule line carries a severity: {line}"
        );
    }
}

// --- check: clean and dirty -----------------------------------------

#[test]
fn check_clean_fixture_is_clean() {
    let out = run(&["check", fixture("clean.nix").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("0 diagnostics"),
        "summary says clean: {stdout}"
    );
}

#[test]
fn check_clean_fixture_json_summary() {
    let out = run(&[
        "check",
        "--format",
        "json",
        fixture("clean.nix").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("output is valid JSON");
    assert_eq!(
        value["summary"]["diagnostics"].as_u64(),
        Some(0),
        "json summary reports zero diagnostics"
    );
}

#[test]
fn check_dirty_fixture_fails_when_rules_exist() {
    let out = run(&["check", fixture("dirty.nix").to_str().unwrap()]);
    let code = out.status.code();
    if !rules_available() {
        assert_eq!(code, Some(0), "empty registry: exit 0 only");
        return;
    }
    assert_eq!(code, Some(1), "diagnostics found -> exit 1");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("unused-let-binding"),
        "output names the fired rule: {stdout}"
    );
}

#[test]
fn disable_rule_cleans_dirty_fixture() {
    if !rules_available() {
        return;
    }
    let out = run(&[
        "check",
        "--disable",
        "unused-let-binding",
        fixture("dirty.nix").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0 diagnostics"), "rule disabled: {stdout}");
}

// --- fix ------------------------------------------------------------

#[test]
fn fix_rewrites_dirty_copy() {
    if !rules_available() {
        return;
    }
    // Work on a copy so the checked-in fixture stays dirty.
    let dir = std::env::temp_dir().join(format!(
        "strictix-cli-fix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let copy = dir.join("dirty-copy.nix");
    std::fs::copy(fixture("dirty.nix"), &copy).expect("copy fixture");

    let before = std::fs::read_to_string(&copy).expect("read before");
    let out = run(&["fix", copy.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let after = std::fs::read_to_string(&copy).expect("read after");

    assert_ne!(before, after, "fix changed the file");
    assert!(after.contains("in 2"), "body survives: {after}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(copy.to_str().unwrap()) || stdout.contains("fix"),
        "output mentions the file or a fix count: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// --- usage errors ---------------------------------------------------

#[test]
fn unknown_flag_is_an_error() {
    let out = run(&["check", "--definitely-not-a-flag"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error:"),
        "stderr carries error: prefix"
    );
}

#[test]
fn unknown_command_is_an_error() {
    let out = run(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("error:"));
}

// --- ignore file ----------------------------------------------------

#[test]
fn ignore_file_prunes_subdir() {
    let dir = fixture("ignore");
    let out = run_in(&dir, &["check", "."]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("dirty"),
        "ignored subdir is pruned: {stdout}"
    );
    assert!(
        stdout.contains("0 diagnostics"),
        "only the clean file: {stdout}"
    );
}

// --- directory walking ----------------------------------------------

#[test]
fn walk_counts_nested_files() {
    let out = run(&["check", fixture("walk").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("2 file(s) linted"),
        "both nested files linted: {stdout}"
    );
}

// --- bad path -------------------------------------------------------

#[test]
fn missing_path_is_an_error() {
    let out = run(&["check", "/definitely/not/a/real/path.nix"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error:"),
        "stderr carries error: prefix"
    );
}
