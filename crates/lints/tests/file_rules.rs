//! Integration tests for the file rules (semantic) and the schema rule.
//!
//! Each rule gets its positive case plus 2-3 false-positive guards:
//! well-formed code that must produce zero diagnostics. Diagnostics are
//! rendered with the one-line contract format
//! (`[code] severity start..end message`) and asserted exactly.

use std::path::PathBuf;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::file_rules::{
    RedundantWith, SelfReferentialLet, ShadowedBinding, UnusedFormal, UnusedLambdaParam,
    UnusedLetBinding,
};
use strictix_lints::schema::UnknownOption;
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

/// Run `rules` over `source` with a fresh semantic model per source.
fn run(source: &str, rules: &[Box<dyn Rule>], config: LintConfig) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(rules, &tree, &model, &config, source, &mut diags);
    render(&diags)
}

fn one(rule: Box<dyn Rule>) -> Vec<Box<dyn Rule>> {
    vec![rule]
}

/// All file rules plus the schema rule — used for the module fixtures'
/// "clean for ALL file rules" guarantee.
fn all_file_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(UnusedLetBinding {}),
        Box::new(UnusedLambdaParam {}),
        Box::new(UnusedFormal {}),
        Box::new(ShadowedBinding {}),
        Box::new(RedundantWith {}),
        Box::new(SelfReferentialLet {}),
        Box::new(UnknownOption {}),
    ]
}

/// Read a fixture from tests/fixtures.
fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture exists")
}

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/options.json")
}

// --- unused-let-binding ---------------------------------------------

#[test]
fn unused_let_binding_flags_unused() {
    let rules = one(Box::new(UnusedLetBinding {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let x = 1; in 2", &rules, cfg),
        ["[unused-let-binding] warning 4..5 binding 'x' is never used"]
    );
}

#[test]
fn unused_let_binding_clean_when_used() {
    let rules = one(Box::new(UnusedLetBinding {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let x = 1; in x", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    assert_eq!(
        run("let x = 1; y = x; in y", &rules, cfg),
        Vec::<String>::new()
    );
}

#[test]
fn unused_let_binding_offers_removal_fix() {
    let source = "let x = 1; in 2";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &one(Box::new(UnusedLetBinding {})),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    let fix = diags[0].fix.as_ref().expect("unused binding gets a fix");
    assert_eq!(fix.label, "remove unused binding");
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].replacement, "");
    assert_eq!(fix.edits[0].range.start(), 3);
    assert_eq!(fix.edits[0].range.end(), 10);
}

// --- unused-lambda-param --------------------------------------------

#[test]
fn unused_lambda_param_flags_unused() {
    let rules = one(Box::new(UnusedLambdaParam {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("x: 1", &rules, cfg),
        ["[unused-lambda-param] warning 0..1 parameter 'x' is never used"]
    );
}

#[test]
fn unused_lambda_param_clean_cases() {
    let rules = one(Box::new(UnusedLambdaParam {}));
    let cfg = LintConfig::default();
    assert_eq!(run("_x: 1", &rules, cfg.clone()), Vec::<String>::new());
    assert_eq!(run("x: x", &rules, cfg.clone()), Vec::<String>::new());
    // Only the inner y is unused; the outer x is referenced.
    assert_eq!(
        run("x: y: x", &rules, cfg),
        ["[unused-lambda-param] warning 3..4 parameter 'y' is never used"]
    );
}

// --- unused-formal ---------------------------------------------------

#[test]
fn unused_formal_flags_unused() {
    let rules = one(Box::new(UnusedFormal {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("{ a, b }: b", &rules, cfg),
        ["[unused-formal] warning 2..3 parameter 'a' is never used"]
    );
}

#[test]
fn unused_formal_clean_cases() {
    let rules = one(Box::new(UnusedFormal {}));
    let cfg = LintConfig::default();
    // Ellipsis means the extra formals are an open interface.
    assert_eq!(
        run("{ a, b, ... }: b", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    // Underscore names opt out.
    assert_eq!(
        run("{ _a, b }: b", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    // Used formals are fine.
    assert_eq!(run("{ a }: a", &rules, cfg), Vec::<String>::new());
}

// --- shadowed-binding ------------------------------------------------

#[test]
fn shadowed_binding_flags_inner() {
    let rules = one(Box::new(ShadowedBinding {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let a = 1; in let a = 2; in a", &rules, cfg),
        ["[shadowed-binding] warning 18..19 binding 'a' shadows an outer binding"]
    );
}

#[test]
fn shadowed_binding_clean_cases() {
    let rules = one(Box::new(ShadowedBinding {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let a = 1; in let b = 2; in a + b", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    // A rec attr sees its own name at its position: not a shadow.
    assert_eq!(run("rec { a = 1; }", &rules, cfg), Vec::<String>::new());
}

// --- redundant-with --------------------------------------------------

#[test]
fn redundant_with_flags_when_body_is_lexical() {
    let rules = one(Box::new(RedundantWith {}));
    let cfg = LintConfig::default();
    // pkgs resolves lexically (to the let binding), so the with is dead.
    assert_eq!(
        run("let pkgs = 1; in with pkgs; pkgs.hello", &rules, cfg),
        ["[redundant-with] warning 16..38 with-scope is never used"]
    );
}

#[test]
fn redundant_with_flags_let_body() {
    let rules = one(Box::new(RedundantWith {}));
    let cfg = LintConfig::default();
    // x resolves to the let binding; the with attrset is never consulted.
    assert_eq!(
        run("let x = 1; in with { y = 2; }; x", &rules, cfg),
        ["[redundant-with] warning 13..32 with-scope is never used"]
    );
}

#[test]
fn redundant_with_clean_when_with_is_needed() {
    let rules = one(Box::new(RedundantWith {}));
    let cfg = LintConfig::default();
    // map/f/xs are unbound — the with scope is a real fallback.
    assert_eq!(
        run("with lib; map f xs", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    // A single unbound reference needs the with too.
    assert_eq!(run("with lib; f", &rules, cfg), Vec::<String>::new());
}

// --- self-referential-let -------------------------------------------

#[test]
fn self_referential_let_flags_infinite_recursion() {
    let rules = one(Box::new(SelfReferentialLet {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let x = x; in x", &rules, cfg.clone()),
        ["[self-referential-let] error 4..5 binding 'x' references itself: infinite recursion"]
    );
    assert_eq!(
        run("let x = x + 1; in x", &rules, cfg),
        ["[self-referential-let] error 4..5 binding 'x' references itself: infinite recursion"]
    );
}

#[test]
fn self_referential_let_clean_cases() {
    let rules = one(Box::new(SelfReferentialLet {}));
    let cfg = LintConfig::default();
    assert_eq!(
        run("let x = 1; in x", &rules, cfg.clone()),
        Vec::<String>::new()
    );
    // y is a different name — not self-reference.
    assert_eq!(run("let x = y; in x", &rules, cfg), Vec::<String>::new());
}

// --- unknown-option (schema rule) ------------------------------------

#[test]
fn unknown_option_flags_hallucinated_path() {
    let rules = all_file_rules();
    let cfg = LintConfig::default().with_schema(schema_path());
    let bad = fixture("module_bad.nix");
    assert_eq!(
        run(&bad, &rules, cfg),
        ["[unknown-option] error 24..47 option 'services.exmaple.enable' is not declared in options.json"]
    );
}

#[test]
fn unknown_option_clean_for_valid_module() {
    let rules = all_file_rules();
    let cfg = LintConfig::default().with_schema(schema_path());
    let ok = fixture("module_ok.nix");
    assert_eq!(run(&ok, &rules, cfg), Vec::<String>::new());
}

#[test]
fn unknown_option_off_without_schema() {
    let rules = all_file_rules();
    let cfg = LintConfig::default(); // schema None
    let bad = fixture("module_bad.nix");
    assert_eq!(run(&bad, &rules, cfg), Vec::<String>::new());
}
