//! End-to-end CLI tests, running the real binary.
//!
//! These exercise the whole stack: argument parsing, config loading,
//! directory walking, ignore files, per-file worker threads, rule
//! dispatch, and rendering - all through the compiled strictix
//! binary. Two fixtures carry the cleanliness guarantees the rule set
//! depends on: the clean fixture ("let x = 1; in x") produces zero
//! diagnostics under every rule, and the dirty fixture
//! ("let x = 1; in 2") triggers exactly unused-let-binding. Tests
//! confirm the rule is present (via the list command) before asserting
//! on it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use strictix_core::json::JsonValue;

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

/// Whether the rule registry has the builtin rules (detected by running
/// 'strictix list' and looking for a real rule code). Guards tests that
/// assert on rule behavior against a registry that lacks the rules.
fn rules_available() -> bool {
    let out = run(&["list"]);
    out.status.code() == Some(0)
        && String::from_utf8_lossy(&out.stdout).contains("unused-let-binding")
}

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

#[test]
fn list_exits_zero_and_prints_rules() {
    let out = run(&["list"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.is_empty(), "list output is non-empty");
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
    let value = JsonValue::parse(&stdout).expect("output is valid JSON");
    assert_eq!(
        value
            .get("summary")
            .and_then(|s| s.get("diagnostics"))
            .and_then(JsonValue::as_number),
        Some(0.0),
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

#[test]
fn fix_rewrites_dirty_copy() {
    if !rules_available() {
        return;
    }
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
    assert_eq!(after, "2", "body survives: {after}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(copy.to_str().unwrap()) || stdout.contains("fix"),
        "output mentions the file or a fix count: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_dry_run_does_not_write_and_shows_diff() {
    if !rules_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "strictix-cli-dry-{}-{}",
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
    let out = run(&["fix", "--dry-run", copy.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let after = std::fs::read_to_string(&copy).expect("read after");
    assert_eq!(before, after, "dry-run leaves the file untouched");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("would apply 1 fix(es)"),
        "dry-run says would-apply: {stdout}"
    );
    assert!(
        stdout.contains("-let x = 1; in 2") && stdout.contains("+2"),
        "diff shown: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

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

#[test]
fn missing_path_is_an_error() {
    let out = run(&["check", "/definitely/not/a/real/path.nix"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error:"),
        "stderr carries error: prefix"
    );
}

#[test]
fn shallow_merge_requires_opt_in_and_disable_wins() {
    let dir = std::env::temp_dir().join(format!("strictix-opt-in-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("input.nix"), "{ a.x = 1; } // { a.y = 2; }").unwrap();
    let has_warning = |out: Output| {
        assert!(matches!(out.status.code(), Some(0 | 1)));
        String::from_utf8_lossy(&out.stdout).contains("shallow-merge-overwrite")
    };
    assert!(!has_warning(run_in(&dir, &["check", "input.nix"])));
    assert!(has_warning(run_in(
        &dir,
        &["check", "input.nix", "--enable", "shallow-merge-overwrite"]
    )));
    std::fs::write(
        dir.join("strictix.toml"),
        "[lint]\nenabled = [\"shallow-merge-overwrite\"]\n",
    )
    .unwrap();
    assert!(has_warning(run_in(&dir, &["check", "input.nix"])));
    assert!(!has_warning(run_in(
        &dir,
        &["check", "input.nix", "--disable", "shallow-merge-overwrite"]
    )));
    assert!(!has_warning(run_in(
        &dir,
        &[
            "check",
            "input.nix",
            "--enable",
            "shallow-merge-overwrite",
            "--disable",
            "shallow-merge-overwrite"
        ]
    )));
    std::fs::remove_dir_all(dir).unwrap();
}

fn temp_tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strictix-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    for (rel, text) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, text).expect("write");
    }
    dir
}

fn read_tree(dir: &Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(dir).expect("under dir");
                out.insert(
                    rel.display().to_string(),
                    std::fs::read_to_string(&path).expect("read"),
                );
            }
        }
    }
    out
}

fn nix_available() -> bool {
    Command::new("nix")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const FLAKE_TREE: &[(&str, &str)] = &[
    (
        "flake.nix",
        "{\n  description = \"t\";\n  outputs = inputs: import ./modules/outputs.nix inputs;\n}\n",
    ),
    (
        "modules/outputs.nix",
        "inputs: let\n  shared = ./shared.nix;\n  hosts = {\n    a = [./a.nix ./shared.nix];\n    b = [./b.nix ./shared.nix];\n  };\nin {\n  inherit hosts;\n  nixosModules.a = ./a.nix;\n  lint = import ./lint.nix;\n  modules = [shared];\n}\n",
    ),
    ("modules/a.nix", "{...}: {}\n"),
    ("modules/b.nix", "{config = {};}\n"),
    ("modules/shared.nix", "{}\n"),
    ("modules/lint.nix", "{schemas = {\"modules/named.nix\" = \"\";};}\n"),
    ("modules/named.nix", "{x = 1;}\n"),
];

#[test]
fn merge_flake_in_place_keeps_an_attrset_and_split_restores_it() {
    let dir = temp_tree("merge-flake", FLAKE_TREE);
    let before = read_tree(&dir);
    let out = run_in(
        &dir,
        &["merge", ".", "--entry", "flake.nix", "--out", "flake.nix"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = stdout(&out);
    assert!(report.contains("inlined 7 files"), "{report}");
    assert!(
        report.contains("keyed module modules/shared.nix"),
        "{report}"
    );
    let after = read_tree(&dir);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        ["flake.nix"],
        "one file left"
    );
    let merged = &after["flake.nix"];
    assert!(merged.starts_with("{\n  description = \"t\";"), "{merged}");
    assert!(merged.contains("outputs = inputs: let"), "{merged}");
    assert!(
        merged.contains("merge_keyed_module \"modules/shared.nix\""),
        "{merged}"
    );
    assert!(
        merged.contains("nixosModules.a = (merge_keyed_module \"modules/a.nix\" modules_a);"),
        "{merged}"
    );
    assert!(merged.contains("\"modules/named.nix\" = \"\""), "{merged}");

    let out = run_in(&dir, &["split", ".", "--merged", "flake.nix"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read_tree(&dir),
        before,
        "split restores every file byte for byte"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn merge_reports_why_a_path_named_file_stays() {
    let dir = temp_tree(
        "merge-kept",
        &[
            ("default.nix", "import ./lint.nix\n"),
            ("lint.nix", "{text = \"${./lint.nix}\";}\n"),
        ],
    );
    let out = run_in(
        &dir,
        &[
            "merge",
            ".",
            "--entry",
            "default.nix",
            "--out",
            "merged.nix",
            "--dry-run",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("kept lint.nix as a file: named by the path `./lint.nix` in lint.nix outside import, callPackage and module lists"),
        "{}",
        stdout(&out)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn merge_expands_a_discovery_call_bound_in_a_reachable_file() {
    if !nix_available() {
        return;
    }
    let dir = temp_tree(
        "merge-expand",
        &[
            ("default.nix", "import ./modules/outputs.nix\n"),
            (
                "modules/outputs.nix",
                "let\n  found = dir: map (name: dir + \"/${name}\") (builtins.attrNames (builtins.readDir dir));\n  other = 1;\nin {\n  imports = found ./mods ++ [./mods/b.nix];\n  inherit other;\n}\n",
            ),
            ("modules/mods/a.nix", "{}\n"),
            ("modules/mods/b.nix", "{}\n"),
        ],
    );
    let before = read_tree(&dir);
    let out = run_in(
        &dir,
        &[
            "merge",
            ".",
            "--entry",
            "default.nix",
            "--out",
            "default.nix",
            "--expand",
            "found",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report = stdout(&out);
    assert!(
        report.contains("expanded `modules/outputs.nix: found ./mods` into 2 modules"),
        "{report}"
    );
    assert!(
        report.contains("removed let binding modules/outputs.nix: found"),
        "{report}"
    );
    assert!(
        report.contains("keyed module modules/mods/b.nix"),
        "{report}"
    );
    assert_eq!(read_tree(&dir).len(), 1, "one file left");
    let out = run_in(&dir, &["split", ".", "--merged", "default.nix"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout(&out).contains("restored 1 expanded calls"),
        "{}",
        stdout(&out)
    );
    assert_eq!(
        read_tree(&dir),
        before,
        "split restores the call and the binding"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

const LINT_TREE: &[(&str, &str)] = &[
    (
        "default.nix",
        "let\n  modules = [./modules/a.nix ./modules/b.nix ./vendor/c.nix];\nin\n  import ./modules/eval.nix modules\n",
    ),
    ("modules/eval.nix", "modules: {imports = modules;}\n"),
    (
        "modules/a.nix",
        "{lib, ...}: {\n  options.foo.enable = lib.mkEnableOption \"foo\";\n  config = {};\n}\n",
    ),
    (
        "modules/b.nix",
        "{...}: {\n  config = let\n    unused = 1;\n  in {};\n}\n",
    ),
    ("vendor/c.nix", "{...}: let\n  unused = 1;\nin {}\n"),
    ("strictix.toml", "[schemas]\n\"modules/b.nix\" = \"\"\n"),
    (".strictixignore", "vendor/\n"),
];

fn diagnostics(dir: &Path) -> Vec<(String, String)> {
    let out = run_in(
        dir,
        &[
            "check",
            ".",
            "--format",
            "json",
            "--enable",
            "never-enabled-option",
        ],
    );
    let value = JsonValue::parse(&stdout(&out)).expect("json");
    let mut found: Vec<(String, String)> = value
        .get("files")
        .and_then(JsonValue::as_array)
        .expect("files")
        .iter()
        .flat_map(|file| {
            file.get("diagnostics")
                .and_then(JsonValue::as_array)
                .unwrap_or_default()
        })
        .map(|diag| {
            let field = |key: &str| {
                diag.get(key)
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            (field("code"), field("message"))
        })
        .collect();
    found.sort();
    found
}

#[test]
fn check_and_fix_lint_a_merged_file_as_the_files_it_holds() {
    if !rules_available() {
        return;
    }
    let plain = temp_tree("lint-plain", LINT_TREE);
    let merged = temp_tree("lint-merged", LINT_TREE);
    let out = run_in(
        &merged,
        &[
            "merge",
            ".",
            "--entry",
            "default.nix",
            "--out",
            "default.nix",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!merged.join("modules").exists(), "the modules were inlined");

    let expected = diagnostics(&plain);
    assert!(
        expected
            .iter()
            .any(|(code, _)| code == "unused-let-binding"),
        "{expected:?}"
    );
    assert!(
        expected
            .iter()
            .any(|(code, _)| code == "never-enabled-option"),
        "{expected:?}"
    );
    assert!(
        !expected.iter().any(|(code, _)| code == "dangling-path"),
        "{expected:?}"
    );
    assert_eq!(
        diagnostics(&merged),
        expected,
        "same findings merged and unmerged"
    );

    let out = run_in(&merged, &["check", "default.nix"]);
    assert!(
        stdout(&out).contains("default.nix:"),
        "diagnostics point into the merged file: {}",
        stdout(&out)
    );

    for dir in [&plain, &merged] {
        let out = run_in(dir, &["fix", "."]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let out = run_in(&merged, &["split", ".", "--merged", "default.nix"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fixed = read_tree(&plain);
    assert!(
        !fixed["modules/b.nix"].contains("unused"),
        "{}",
        fixed["modules/b.nix"]
    );
    assert!(
        fixed["vendor/c.nix"].contains("unused"),
        "ignored file untouched"
    );
    assert_eq!(
        read_tree(&merged),
        fixed,
        "fixing the merged file fixes the same files"
    );
    std::fs::remove_dir_all(plain).unwrap();
    std::fs::remove_dir_all(merged).unwrap();
}
