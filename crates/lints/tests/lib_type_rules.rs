use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::lib_type_rules::UnknownLibType;
use strictix_syntax::parse;

fn run(source: &str, rule: Box<dyn Rule>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    run_rules(
        &[rule],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics
}

#[test]
fn removed_string_type_flags_with_str_fix() {
    let source = "{ lib, ... }: lib.types.string";
    let diagnostics = run(source, Box::new(UnknownLibType {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "unknown-lib-type");
    assert_eq!(diagnostics[0].message, "'string' is not a lib.types member");
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("types.string was removed; use types.str")
    );
    let fix = diagnostics[0].fix.as_ref().expect("safe fix");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("{ lib, ... }: lib.types.str".to_owned())
    );
}

#[test]
fn real_member_is_silent() {
    assert!(run("{ lib, ... }: lib.types.str", Box::new(UnknownLibType {})).is_empty());
}

#[test]
fn hallucinated_member_flags_without_fix() {
    let diagnostics = run(
        "{ lib, ... }: lib.types.frobnicate",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "'frobnicate' is not a lib.types member"
    );
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn never_existed_list_gets_curated_help() {
    let diagnostics = run("{ lib, ... }: lib.types.list", Box::new(UnknownLibType {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("use types.listOf <elem>")
    );
}

#[test]
fn function_gets_curated_help() {
    let diagnostics = run(
        "{ lib, ... }: lib.types.function",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].help.as_deref(),
        Some("use types.functionTo <return>")
    );
}

#[test]
fn let_bound_lib_is_a_custom_lib_and_stays_silent() {
    assert!(run(
        "let lib = import ./mine.nix; in lib.types.whatever",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn unbound_lib_still_flags() {
    // Top-level bare `lib` (file evaluated with lib in scope) is
    // treated as nixpkgs lib.
    let diagnostics = run("lib.types.string", Box::new(UnknownLibType {}));
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn inherited_types_from_formal_lib_flags() {
    let diagnostics = run(
        "{ lib, ... }: let inherit (lib) types; in types.list",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "'list' is not a lib.types member");
}

#[test]
fn inherited_types_from_let_bound_lib_is_silent() {
    assert!(run(
        "let lib = import ./mine.nix; inherit (lib) types; in types.list",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn inherit_from_lib_types_binds_members_not_the_set() {
    // `inherit (lib.types) str` binds individual members; a later
    // `str.whatever`-style use is a different shape entirely and a
    // plain `inherit types;` proves nothing — both stay silent.
    assert!(run(
        "{ lib, ... }: let inherit (lib.types) str; in str",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
    assert!(run(
        "{ types, ... }: let inherit types; in types.bogus",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn with_lib_over_formal_flags_bare_types() {
    let diagnostics = run(
        "{ lib, ... }: with lib; types.bogus",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "'bogus' is not a lib.types member");
}

#[test]
fn with_let_bound_lib_is_silent() {
    assert!(run(
        "let lib = import ./mine.nix; in with lib; types.bogus",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn with_non_lib_subject_is_silent() {
    assert!(run(
        "{ pkgs, ... }: with pkgs; types.bogus",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn nested_ints_member_validates_second_hop() {
    assert!(run(
        "{ lib, ... }: lib.types.ints.u8",
        Box::new(UnknownLibType {})
    )
    .is_empty());
    let diagnostics = run(
        "{ lib, ... }: lib.types.ints.u42",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "'u42' is not a lib.types.ints member"
    );
}

#[test]
fn nested_numbers_member_validates_second_hop() {
    assert!(run(
        "{ lib, ... }: lib.types.numbers.positive",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
    let diagnostics = run(
        "{ lib, ... }: lib.types.numbers.negative",
        Box::new(UnknownLibType {}),
    );
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].message,
        "'negative' is not a lib.types.numbers member"
    );
}

#[test]
fn has_attr_probe_never_fires() {
    assert!(run(
        "{ lib, ... }: lib.types ? string",
        Box::new(UnknownLibType {})
    )
    .is_empty());
    assert!(run(
        "{ lib, ... }: lib ? types.string",
        Box::new(UnknownLibType {})
    )
    .is_empty());
}

#[test]
fn or_default_probe_never_fires() {
    assert!(run(
        "{ lib, ... }: lib.types.string or lib.types.str",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}

#[test]
fn dynamic_segment_is_silent() {
    assert!(run(
        "{ lib, x, ... }: lib.types.${x}",
        Box::new(UnknownLibType {}),
    )
    .is_empty());
}
