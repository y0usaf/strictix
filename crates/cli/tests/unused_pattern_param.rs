mod _utils;

use std::fs;

use proptest::prelude::*;
use tempfile::tempdir;

const LINT: &str = "unused_pattern_param";

#[test]
fn disabled_by_default() -> anyhow::Result<()> {
    _utils::assert_check_clean("({ config, lib, pkgs, ... }: config)")
}

#[test]
fn fixes_unused_variadic_pattern_params_when_enabled() -> anyhow::Result<()> {
    let stdout = _utils::test_cli(
        "({ config, lib, pkgs, ... }: config)",
        &["fix", "--dry-run", "-e", LINT],
    )?;

    assert!(stdout.contains("-({ config, lib, pkgs, ... }: config)"));
    assert!(stdout.contains("+({ config, ... }: config)"));
    Ok(())
}

#[test]
fn keeps_ellipsis_after_removal() -> anyhow::Result<()> {
    let stdout = _utils::test_cli(
        "({ lib, ... }: { imports = []; })",
        &["fix", "--dry-run", "-e", LINT],
    )?;

    assert!(stdout.contains("-({ lib, ... }: { imports = []; })"));
    assert!(stdout.contains("+({ ... }: { imports = []; })"));
    Ok(())
}

#[test]
fn skips_closed_patterns() -> anyhow::Result<()> {
    let stdout = _utils::test_cli("({ lib, pkgs }: pkgs)", &["fix", "--dry-run", "-e", LINT])?;

    assert!(stdout.trim().is_empty());
    Ok(())
}

#[test]
fn skips_patterns_when_outer_bind_is_used() -> anyhow::Result<()> {
    let stdout = _utils::test_cli(
        "(args @ { lib, pkgs, ... }: args.pkgs)",
        &["fix", "--dry-run", "-e", LINT],
    )?;

    assert!(stdout.trim().is_empty());
    Ok(())
}

#[test]
fn keeps_params_referenced_from_defaults() -> anyhow::Result<()> {
    let stdout = _utils::test_cli(
        "({ lib, pkgs ? lib.defaultPkgs, ... }: pkgs)",
        &["fix", "--dry-run", "-e", LINT],
    )?;

    assert!(stdout.trim().is_empty());
    Ok(())
}

#[test]
fn config_can_remove_ellipsis() -> anyhow::Result<()> {
    let dir = tempdir()?;
    fs::write(
        dir.path().join("strictix.toml"),
        r#"enabled = ["unused_pattern_param"]

[lints.unused_pattern_param]
remove_ellipsis = true
"#,
    )?;
    let file = dir.path().join("case.nix");
    fs::write(&file, "({ config, lib, pkgs, ... }: config)\n")?;

    let stdout = _utils::run_cli(&file, &["fix", "--dry-run"])?.stdout;

    assert!(stdout.contains("-({ config, lib, pkgs, ... }: config)"));
    assert!(stdout.contains("+({ config }: config)"));
    Ok(())
}

#[test]
fn config_does_not_remove_ellipsis_without_unused_named_params() -> anyhow::Result<()> {
    let dir = tempdir()?;
    fs::write(
        dir.path().join("strictix.toml"),
        r#"enabled = ["unused_pattern_param"]

[lints.unused_pattern_param]
remove_ellipsis = true
"#,
    )?;
    let file = dir.path().join("case.nix");
    fs::write(&file, "({ config, lib, ... }: lib.mkIf config.enable {})\n")?;

    let stdout = _utils::run_cli(&file, &["fix", "--dry-run"])?.stdout;

    assert!(stdout.trim().is_empty());
    Ok(())
}

proptest! {
    #![proptest_config(proptest::test_runner::Config {
        failure_persistence: None,
        .. proptest::test_runner::Config::default()
    })]

    #[test]
    fn unused_pattern_param_fix_properties(
        expression in prop_oneof![
            Just("({ config, lib, pkgs, ... }: config)"),
            Just("({ lib, ... }: { imports = []; })"),
            Just("({ config, lib, pkgs, ... }: lib.mkIf true { package = pkgs.hello; })"),
            Just("(args @ { config, lib, pkgs, ... }: config)"),
        ],
        prefix in _utils::trivia_strategy(),
        suffix in _utils::trivia_strategy(),
    ) {
        _utils::assert_rewrite_invariants(LINT, expression, &prefix, &suffix)
            .expect("rewrite invariant violated");
    }
}
