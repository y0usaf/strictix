//! Integration tests for the reference/builtin rules:
//! `undefined-variable` and `unknown-builtin`.
//!
//! Diagnostics use the one-line contract format
//! (`[code] severity start..end message`) and are asserted exactly,
//! mirroring the harness in file_rules.rs.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::reference_rules::{UndefinedVariable, UnknownBuiltin};
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

fn undefined_rules() -> Vec<Box<dyn Rule>> {
    one(Box::new(UndefinedVariable {}))
}

fn builtin_rules() -> Vec<Box<dyn Rule>> {
    one(Box::new(UnknownBuiltin {}))
}

// --- undefined-variable ---------------------------------------------

#[test]
fn undefined_variable_flags_unbound_name() {
    let cfg = LintConfig::default();
    assert_eq!(
        run("let a = 1; in a + b", &undefined_rules(), cfg),
        ["[undefined-variable] error 18..19 undefined variable 'b' in expression position is never bound"]
    );
}

#[test]
fn undefined_variable_offers_escape_fix_in_indented_string() {
    // `'' text ${HOME} ''` — a shell variable meant literally inside an
    // indented string. The escape fix rewrites `${HOME}` to `''${HOME}`.
    let source = "'' text ${HOME} ''";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &undefined_rules(),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    assert_eq!(
        render(&diags),
        ["[undefined-variable] error 10..14 undefined variable 'HOME' in expression position is never bound"]
    );
    let fix = diags[0].fix.as_ref().expect("indented-string case gets a fix");
    assert_eq!(fix.label, "escape literal with ''$ (only in indented string)");
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].range.start(), 8);
    assert_eq!(fix.edits[0].range.end(), 15);
    let result = strictix_core::fix::apply_fixes(source, &fix.edits).expect("fix applies");
    assert_eq!(result, "'' text ''${HOME} ''");
}

#[test]
fn undefined_variable_no_fix_in_plain_string() {
    // A normal double-quoted string has no safe generic fix, so the
    // error still fires but no fix rides along.
    let source = "\"${HOME}\"";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &undefined_rules(),
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    assert_eq!(
        render(&diags),
        ["[undefined-variable] error 3..7 undefined variable 'HOME' in expression position is never bound"]
    );
    assert!(diags[0].fix.is_none(), "no fix for a plain string");
}

#[test]
fn undefined_variable_clean_cases() {
    let cfg = LintConfig::default();
    // with-provided name: resolves via the with fallback.
    assert_eq!(
        run("let pkgs = 1; in with pkgs; hello", &undefined_rules(), cfg.clone()),
        Vec::<String>::new()
    );
    // a select field is not a reference; builtins is a global.
    assert_eq!(
        run("builtins.attrNames {}", &undefined_rules(), cfg.clone()),
        Vec::<String>::new()
    );
    // rec attr: mutually visible.
    assert_eq!(
        run("rec { x = 1; y = x; }", &undefined_rules(), cfg.clone()),
        Vec::<String>::new()
    );
    // a straightforward let-bound name.
    assert_eq!(
        run("let a = 1; in a", &undefined_rules(), cfg.clone()),
        Vec::<String>::new()
    );
    // lambda param used.
    assert_eq!(
        run("map (x: x) [1]", &undefined_rules(), cfg.clone()),
        Vec::<String>::new()
    );
    // globals gate: map is a Nix global, x is a param.
    assert_eq!(
        run("\"hello ${map (x: x) [1]}\"", &undefined_rules(), cfg),
        Vec::<String>::new()
    );
}

// --- unknown-builtin ------------------------------------------------

#[test]
fn unknown_builtin_flags_filter_attrs() {
    let cfg = LintConfig::default();
    assert_eq!(
        run("builtins.filterAttrs", &builtin_rules(), cfg),
        ["[unknown-builtin] warning 9..20 builtin 'filterAttrs' does not exist"]
    );
}

#[test]
fn unknown_builtin_flags_another_invented_name() {
    let cfg = LintConfig::default();
    assert_eq!(
        run("builtins.mapAttrs_to_string", &builtin_rules(), cfg),
        ["[unknown-builtin] warning 9..27 builtin 'mapAttrs_to_string' does not exist"]
    );
}

#[test]
fn unknown_builtin_clean_for_real_builtins() {
    let cfg = LintConfig::default();
    for src in [
        "builtins.map (x: x) [1]",
        "builtins.attrNames {}",
        "builtins.toString 1",
        "builtins.elem 1 [1]",
        "builtins.concatStringsSep \": \" [\"a\"]",
        "builtins.head [1]",
        "builtins.fromJSON \"{}\"",
        "builtins.removeAttrs { a = 1; } [\"a\"]",
        "builtins.intersectAttrs {} {}",
        "builtins.typeOf 1",
        "builtins.genList (x: x) 1",
        "builtins.elemAt [10 20] 0",
        "builtins.all (x: x) [true]",
        "builtins.any (x: x) [false]",
        "builtins.seq 1 2",
        "builtins.throw \"x\"",
        "builtins.trace \"x\" 1",
        "builtins.fetchTree {}",
    ] {
        assert_eq!(
            run(src, &builtin_rules(), cfg.clone()),
            Vec::<String>::new(),
            "no finding for {src}"
        );
    }
}

#[test]
fn unknown_builtin_hasattr_probe_guard() {
    let cfg = LintConfig::default();
    // `builtins ? fetchTree` is a feature probe (HasAttr), not a
    // select; the actual `builtins.fetchTree` is a valid builtin.
    assert_eq!(
        run(
            "if builtins ? fetchTree then builtins.fetchTree {} else {}",
            &builtin_rules(),
            cfg,
        ),
        Vec::<String>::new()
    );
}

#[test]
fn unknown_builtin_skips_shadowed_builtins() {
    let cfg = LintConfig::default();
    // `builtins` is renamed (let-bound) here, so `builtins.foo` is a
    // select off a real binding, not the constant — no diagnostic.
    assert_eq!(
        run("let builtins = 1; in builtins.foo", &builtin_rules(), cfg),
        Vec::<String>::new()
    );
}

#[test]
fn unknown_builtin_skips_deep_attrpaths() {
    let cfg = LintConfig::default();
    // `builtins.map.anything` — only the first hop (map) is a builtin;
    // past the first hop is a user namespace, not this rule's business.
    assert_eq!(
        run("builtins.map.clearCache", &builtin_rules(), cfg),
        Vec::<String>::new()
    );
}