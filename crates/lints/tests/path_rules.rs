use std::path::{Path, PathBuf};

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::path_rules::{AccidentalPathDivision, DanglingPath, SearchPathReference};
use strictix_syntax::parse;

/// Run one file rule with an explicit (optional) on-disk path. The
/// shared run_rules helper never attaches a path, so path-sensitive
/// rules are exercised through check_file on a hand-built model.
fn check(rule: &dyn Rule, source: &str, path: Option<&Path>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree).with_path(path);
    let mut diags = Vec::new();
    rule.check_file(&model, &LintConfig::default(), &mut diags);
    diags
}

/// The committed fixture file the dangling-path tests pretend to lint;
/// relative paths in test sources resolve against its directory.
fn lintme() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paths/lintme.nix")
}

// --- dangling-path ----------------------------------------------------

#[test]
fn dangling_path_flags_missing_relative_path_with_resolved_absolute() {
    let anchor = lintme();
    let diags = check(&DanglingPath {}, "./missing.nix", Some(&anchor));
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "dangling-path");
    assert_eq!(diags[0].severity_str(), "error");
    let resolved = anchor.parent().unwrap().join("./missing.nix");
    assert_eq!(
        diags[0].message,
        format!(
            "path './missing.nix' does not exist (resolved to {})",
            resolved.display()
        )
    );
    assert!(diags[0].fix.is_none());
}

#[test]
fn dangling_path_flags_missing_absolute_path() {
    let anchor = lintme();
    let diags = check(
        &DanglingPath {},
        "/no/such/strictix/fixture.nix",
        Some(&anchor),
    );
    assert_eq!(diags.len(), 1);
    assert!(diags[0]
        .message
        .contains("resolved to /no/such/strictix/fixture.nix"));
}

#[test]
fn dangling_path_is_silent_for_existing_path() {
    let anchor = lintme();
    assert!(check(&DanglingPath {}, "./exists.nix", Some(&anchor)).is_empty());
}

#[test]
fn dangling_path_is_silent_for_path_exists_guarded_path() {
    let anchor = lintme();
    for source in [
        "builtins.pathExists ./missing.nix",
        "lib.filesystem.pathExists ./missing.nix",
        // Every occurrence of a probed path is skipped, not just the
        // probe argument itself.
        "if builtins.pathExists ./missing.nix then ./missing.nix else null",
    ] {
        assert!(
            check(&DanglingPath {}, source, Some(&anchor)).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn dangling_path_flags_directory_import_without_default_nix() {
    let anchor = lintme();
    let diags = check(&DanglingPath {}, "import ./nodefault", Some(&anchor));
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].message,
        "directory './nodefault' has no default.nix"
    );
}

#[test]
fn dangling_path_is_silent_for_directory_import_with_default_nix() {
    let anchor = lintme();
    assert!(check(&DanglingPath {}, "import ./module", Some(&anchor)).is_empty());
}

#[test]
fn dangling_path_is_silent_for_non_import_directory_use() {
    let anchor = lintme();
    assert!(check(&DanglingPath {}, "./nodefault", Some(&anchor)).is_empty());
}

#[test]
fn dangling_path_is_silent_without_a_model_path() {
    assert!(check(&DanglingPath {}, "./missing.nix", None).is_empty());
    assert!(check(&DanglingPath {}, "import ./nodefault", None).is_empty());
}

#[test]
fn dangling_path_skips_home_relative_paths() {
    let anchor = lintme();
    assert!(check(
        &DanglingPath {},
        "~/definitely-not-a-real-strictix-file.nix",
        Some(&anchor)
    )
    .is_empty());
}

#[test]
fn dangling_path_never_fires_on_interpolated_path_fragments() {
    let anchor = lintme();
    // The lexer splits interpolated paths into fragments (Path, Slash,
    // InterpStart, ..., InterpEnd, Path); none of the fragments is a
    // whole path literal, so none may be existence-checked.
    for source in [
        "let v = \"a\"; in ./x/${v}",
        "let v = \"a\"; in /abs/${v}/tail",
        "let v = \"a\"; in ./x${v}",
    ] {
        assert!(
            check(&DanglingPath {}, source, Some(&anchor)).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn dangling_path_skips_numeric_division_trap_fragments() {
    // `4/2` lexes as Int + Path `/2`; existence-checking the `/2`
    // fragment would be nonsense (and would double-report the
    // accidental-path-division finding).
    let anchor = lintme();
    assert!(check(&DanglingPath {}, "4/2", Some(&anchor)).is_empty());
}

// --- accidental-path-division ------------------------------------------

#[test]
fn accidental_path_division_flags_digits_slash_digits_and_fixes_spacing() {
    let source = "4/2";
    let diags = check(&AccidentalPathDivision {}, source, None);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "accidental-path-division");
    assert_eq!(diags[0].severity_str(), "warning");
    assert_eq!(
        diags[0].message,
        "Nix parses '4/2' as a relative path, not division"
    );
    let fix = diags[0].fix.as_ref().expect("division fix");
    assert_eq!(apply_fixes(source, &fix.edits), Ok("4 / 2".to_owned()));
}

#[test]
fn accidental_path_division_fix_round_trips_inside_larger_expression() {
    let source = "[ 40/8 1 ]";
    let diags = check(&AccidentalPathDivision {}, source, None);
    assert_eq!(diags.len(), 1);
    let fix = diags[0].fix.as_ref().expect("division fix");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("[ 40 / 8 1 ]".to_owned())
    );
}

#[test]
fn accidental_path_division_is_silent_for_real_paths_and_real_division() {
    for source in [
        "./4/2",
        "foo/2",
        "4/bar",
        "4 / 2",
        // Interpolated composite: the `4/2` prefix is a path fragment,
        // not a completed literal.
        "let v = \"a\"; in 4/2/${v}",
    ] {
        assert!(
            check(&AccidentalPathDivision {}, source, None).is_empty(),
            "{source}"
        );
    }
}

// --- search-path-reference ----------------------------------------------

#[test]
fn search_path_reference_flags_every_search_path() {
    let diags = check(&SearchPathReference {}, "import <nixpkgs> { }", None);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "search-path-reference");
    assert_eq!(diags[0].severity_str(), "warning");
    assert_eq!(
        diags[0].message,
        "'<nixpkgs>' resolves through NIX_PATH at evaluation time; impure and incompatible with pure flake evaluation"
    );
    assert!(diags[0].fix.is_none());

    let diags = check(&SearchPathReference {}, "<nixpkgs/lib>", None);
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.starts_with("'<nixpkgs/lib>'"));
}

#[test]
fn search_path_reference_is_silent_for_comparisons() {
    assert!(check(&SearchPathReference {}, "a < b", None).is_empty());
}
