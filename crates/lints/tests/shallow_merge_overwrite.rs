use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::shallow_merge_overwrite::ShallowMergeOverwrite;
use strictix_syntax::parse;

fn run(source: &str, config: LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(ShallowMergeOverwrite) as Box<dyn Rule>],
        &tree,
        &model,
        &config,
        source,
        &mut diags,
    );
    diags
}

fn enabled() -> LintConfig {
    LintConfig::default().with_enabled(["shallow-merge-overwrite".into()])
}

#[test]
fn warns_when_dotted_nested_keys_are_replaced() {
    let diagnostics = run("{ a.x = 1; } // { a.y = 2; }", enabled());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity_str(), "warning");
    assert_eq!(diagnostics[0].range.start(), 0);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn warns_for_nested_literal_attrsets() {
    assert_eq!(
        run(
            "{ a = { x = 1; y = 2; }; } // { a = { x = 3; }; }",
            enabled()
        )
        .len(),
        1
    );
}

#[test]
fn skips_safe_overwrites_and_unknown_operands() {
    for source in [
        "{ a.x = 1; } // { a.x = 2; }",
        "left // { a.y = 2; }",
        "{ a.x = 1; } // right",
        "{ a = { x = 1; }; } // { a = { x = 2; }; }",
        "{ ${k}.x = 1; } // { a.y = 2; }",
    ] {
        assert!(run(source, enabled()).is_empty(), "{source}");
    }
}

#[test]
fn is_opt_in() {
    assert!(run("{ a.x = 1; } // { a.y = 2; }", LintConfig::default()).is_empty());
}

#[test]
fn equivalent_dotted_and_nested_forms_do_not_warn() {
    for source in [
        "{ a = { x = { y = 1; }; }; } // { a.x.y = 2; }",
        "{ a.x.y = 1; } // { a = { x = { y = 2; }; }; }",
        "{ a.x.y = 1; } // { a.x = unknown; }",
        "{ a.x = 1; } // { a = { ${key} = 2; }; }",
        "let true = {x=1;}; in {a.x=1;} // {a=true;}",
    ] {
        assert!(run(source, enabled()).is_empty(), "{source}");
    }
    assert_eq!(run("{ a.x = 1; } // { a = {}; }", enabled()).len(), 1);
}
