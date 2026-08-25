//! Integration tests for the schema rules: unknown-option (read and
//! write side, wildcard-aware) and option-type-mismatch.
//!
//! The parsed schema is cached in a process-wide OnceLock, so every
//! test in this binary must use the SAME schema file
//! (fixtures/options_typed.json) — the first load wins for the whole
//! process. Diagnostics are rendered with the one-line contract format
//! (`[code] severity start..end message`) and asserted exactly.

use std::path::PathBuf;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::schema::{OptionTypeMismatch, UnknownOption};
use strictix_syntax::parse;

/// Render diagnostics in the one-line contract format.
fn render(diags: &[Diagnostic]) -> Vec<String> {
    diags
        .iter()
        .map(|d| {
            format!(
                "[{}] {} {}..{} {}",
                d.code,
                d.severity_str(),
                d.range.start(),
                d.range.end(),
                d.message
            )
        })
        .collect()
}

/// Run both schema rules over `source` against the typed fixture schema.
fn run(source: &str) -> Vec<String> {
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(UnknownOption {}), Box::new(OptionTypeMismatch {})];
    let schema = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/options_typed.json");
    let config = LintConfig::default().with_schema(schema);
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);
    render(&diags)
}

/// The `start..end` of the first occurrence of `needle` in `source`,
/// so range assertions survive cosmetic source edits.
fn span(source: &str, needle: &str) -> String {
    let start = source.find(needle).expect("needle in source");
    format!("{}..{}", start, start + needle.len())
}

// --- unknown-option: read side ----------------------------------------

#[test]
fn read_side_accepts_wildcard_paths() {
    // Regression: `users.users.<name>.home` must match any user name.
    let src = "{ config, ... }: { services.example.dataDir = config.users.users.alice.home; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn read_side_still_flags_unknown_paths() {
    let src = "{ config, ... }: { services.example.name = config.users.uzers.alice.home; }";
    assert_eq!(
        run(src),
        [format!(
            "[unknown-option] error {} option 'users.uzers.alice.home' is not declared in options.json",
            span(src, "users.uzers.alice.home")
        )]
    );
}

#[test]
fn write_side_flags_unknown_path_with_empty_attrset_value() {
    // Regression: an empty attrset has no leaves to descend into, so
    // the accumulated path itself must be judged — otherwise a typo'd
    // interior (`services.exampel.settings = { }`) escapes entirely.
    let src = "{ config, ... }: { services.exampel.settings = { }; }";
    assert_eq!(
        run(src),
        [format!(
            "[unknown-option] error {} option 'services.exampel.settings' is not declared in options.json",
            span(src, "services.exampel.settings")
        )]
    );
}

#[test]
fn write_side_accepts_empty_attrset_on_declared_prefix() {
    // `services.example = { };` is a structural write to a real prefix
    // of declared options — legal, and judged by neither rule.
    let src = "{ config, ... }: { services.example = { }; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn write_side_accepts_empty_attrset_on_attrsof_option() {
    // An empty attrset assigned to a declared `attribute set of` option
    // is a matching literal, not a mismatch.
    let src = "{ config, ... }: { services.example.settings = { }; }";
    assert_eq!(run(src), Vec::<String>::new());
}

// --- unknown-option: write side ----------------------------------------

#[test]
fn write_side_flags_typoed_path() {
    let src = "{ config, lib, pkgs, ... }: { services.exmple.enable = true; }";
    assert_eq!(
        run(src),
        [format!(
            "[unknown-option] error {} option 'services.exmple.enable' is not declared in options.json",
            span(src, "services.exmple.enable")
        )]
    );
}

#[test]
fn write_side_accumulates_nested_attrsets() {
    let ok = "{ pkgs, ... }: { services = { example = { enable = true; port = 8080; }; }; }";
    assert_eq!(run(ok), Vec::<String>::new());
    // The diagnostic names the FULL accumulated path but points at the
    // leaf binding's own attrpath.
    let bad = "{ pkgs, ... }: { services = { example = { enabel = true; }; }; }";
    assert_eq!(
        run(bad),
        [format!(
            "[unknown-option] error {} option 'services.example.enabel' is not declared in options.json",
            span(bad, "enabel")
        )]
    );
}

#[test]
fn write_side_descends_mk_if() {
    let src = "{ lib, ... }: { config = lib.mkIf true { services.example.enable = true; }; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn write_side_descends_mk_merge_elements() {
    let src = "{ lib, ... }: { config = lib.mkMerge [ { services.example.enable = true; } { services.bogus.thing = 1; } ]; }";
    assert_eq!(
        run(src),
        [format!(
            "[unknown-option] error {} option 'services.bogus.thing' is not declared in options.json",
            span(src, "services.bogus.thing")
        )]
    );
}

#[test]
fn locally_declared_option_is_accepted() {
    let src = "{ lib, ... }: { options.mine.stuff.enable = lib.mkOption { }; config = { mine.stuff.enable = true; }; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn freeform_interior_write_is_accepted() {
    // `services.example.settings` is a declared attrset option; writing
    // below it (freeform/submodule interior) is legitimate.
    let src = "{ pkgs, ... }: { services.example.settings.whatever.deep = \"x\"; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn intermediate_attrset_write_is_structural() {
    let src = "{ pkgs, ... }: { services.example = { }; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn non_module_file_write_side_is_silent() {
    // No module formals, no imports/options/config key: not a module,
    // so the typo'd path is never judged.
    let src = "{ services.exmple.enable = true; }";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn dynamic_segments_abort_their_subtree() {
    let src = "{ pkgs, ... }: { services.${\"exmple\"}.enable = true; }";
    assert_eq!(run(src), Vec::<String>::new());
}

// --- option-type-mismatch -----------------------------------------------

#[test]
fn boolean_option_rejects_string_literal() {
    let src = "{ pkgs, ... }: { services.example.enable = \"true\"; }";
    assert_eq!(
        run(src),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.enable' expects boolean, got string",
            span(src, "\"true\"")
        )]
    );
}

#[test]
fn integer_option_rejects_string_literal() {
    let src = "{ pkgs, ... }: { services.example.port = \"8080\"; }";
    assert_eq!(
        run(src),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.port' expects 16 bit unsigned integer; between 0 and 65535 (both inclusive), got string",
            span(src, "\"8080\"")
        )]
    );
}

#[test]
fn string_option_rejects_integer_literal() {
    let src = "{ pkgs, ... }: { services.example.name = 42; }";
    assert_eq!(
        run(src),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.name' expects string, got integer",
            span(src, "42")
        )]
    );
}

#[test]
fn null_is_accepted_by_null_or_types_only() {
    assert_eq!(
        run("{ pkgs, ... }: { services.example.banner = null; }"),
        Vec::<String>::new()
    );
    let bad = "{ pkgs, ... }: { services.example.banner = 42; }";
    assert_eq!(
        run(bad),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.banner' expects null or string, got integer",
            span(bad, "42")
        )]
    );
}

#[test]
fn list_and_attrset_options_reject_wrong_shapes() {
    let bad_list = "{ pkgs, ... }: { services.example.extras = \"nope\"; }";
    assert_eq!(
        run(bad_list),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.extras' expects list of string, got string",
            span(bad_list, "\"nope\"")
        )]
    );
    let bad_attrs = "{ pkgs, ... }: { services.example.settings = [ ]; }";
    assert_eq!(
        run(bad_attrs),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.settings' expects attribute set of string, got list",
            span(bad_attrs, "[ ]")
        )]
    );
}

#[test]
fn wildcard_option_types_are_judged() {
    let src = "{ pkgs, ... }: { users.users.alice.home = 5; }";
    assert_eq!(
        run(src),
        [format!(
            "[option-type-mismatch] error {} option 'users.users.alice.home' expects path, got integer",
            span(src, "5")
        )]
    );
}

#[test]
fn mismatch_is_judged_through_modifiers() {
    // mkForce/mkIf unwrap to the payload literal before judging.
    let src = "{ lib, ... }: { services.example.enable = lib.mkForce \"yes\"; }";
    assert_eq!(
        run(src),
        [format!(
            "[option-type-mismatch] error {} option 'services.example.enable' expects boolean, got string",
            span(src, "\"yes\"")
        )]
    );
}

#[test]
fn matching_literals_are_clean() {
    let src = "{ pkgs, lib, ... }: {
  services.example.enable = true;
  services.example.port = 8080;
  services.example.name = \"svc\";
  services.example.dataDir = \"/var/lib/svc\";
  services.example.extras = [ \"a\" ];
  services.example.banner = null;
  users.users.alice.home = \"/home/alice\";
  environment.etc.\"my-config\".text = \"hello\";
}";
    assert_eq!(run(src), Vec::<String>::new());
}

#[test]
fn unjudgeable_values_and_types_stay_silent() {
    // Non-literal RHS: never judged, even against a typed option.
    assert_eq!(
        run("{ pkgs, ... }: { services.example.port = builtins.getEnv \"PORT\"; }"),
        Vec::<String>::new()
    );
    // `package`-typed option: never judged, even against a literal.
    assert_eq!(
        run("{ pkgs, ... }: { services.example.package = \"hello\"; }"),
        Vec::<String>::new()
    );
}
