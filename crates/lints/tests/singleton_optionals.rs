use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::singleton_optionals::SingletonOptionals;
use strictix_syntax::{parse, SyntaxKind};

fn run(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    assert!(!tree
        .descendants()
        .any(|node| node.kind() == SyntaxKind::ErrorNode));
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(SingletonOptionals)],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags
}

fn fixed(source: &str) -> String {
    let diags = run(source);
    assert_eq!(diags.len(), 1);
    let result = apply_fixes(source, &diags[0].fix.as_ref().expect("safe fix").edits).unwrap();
    assert!(
        run(&result).is_empty(),
        "fix is parseable and idempotent: {result}"
    );
    result
}

#[test]
fn fixes_qualified_calls_and_library_aliases() {
    assert_eq!(
        fixed("{ lib }: lib.optionals flag [\"arg\"]"),
        "{ lib }: lib.optional flag \"arg\""
    );
    assert_eq!(
        fixed("{ lib }: let l = lib; in l.optionals flag [value]"),
        "{ lib }: let l = lib; in l.optional flag value"
    );
    assert_eq!(
        fixed("{ lib }: (lib.optionals flag) [value] extra"),
        "{ lib }: (lib.optional flag) value extra"
    );
}

#[test]
fn reports_each_call_in_the_discord_pattern() {
    let source = r#"{lib, cfg}: let inherit (lib) optionals; in
        optionals (enabled != []) ["--enabled=${value}"]
        ++ optionals (disabled != []) ["--disabled=${value}"]
        ++ optionals (!cfg.smoothScroll) ["--disable-smooth-scrolling"]
        ++ cfg.extraArgs"#;
    let diags = run(source);
    assert_eq!(diags.len(), 3);
    assert!(diags
        .iter()
        .all(|diag| diag.code == "singleton-optionals" && diag.fix.is_none()));
    for diag in diags {
        assert!(
            source[diag.range.start() as usize..diag.range.end() as usize].starts_with("optionals")
        );
    }
}

#[test]
fn recognizes_inherited_with_and_aliased_helpers_without_unsafe_renames() {
    for source in [
        "{ lib }: let inherit (lib) optionals; in optionals a [x]",
        "{ lib }: with lib; optionals a [x]",
        "{ lib }: let opts = lib.optionals; in opts a [x]",
        "{ lib }: let inherit (lib) optionals; optional = x: x; in optionals a [x]",
    ] {
        let diags = run(source);
        assert_eq!(diags.len(), 1, "{source}");
        assert!(
            diags[0].fix.is_none(),
            "must not assume optional is in scope"
        );
    }
}

#[test]
fn avoids_nonmatching_lists_partial_calls_and_shadowed_names() {
    for source in [
        "{ lib }: lib.optionals a []",
        "{ lib }: lib.optionals a [x y]",
        "{ lib }: lib.optionals a xs",
        "{ lib }: lib.optionals a",
        "{ lib }: lib.optional a x",
        "{ lib }: lib.optionalAttrs a {x=1;}",
        "{ lib }: lib.optionals a [[x]]",
        "{ lib }: lib.optionals a [([x])]",
        "{ optionals }: optionals a [x]",
        "{ lib }: let lib = { optionals = a: b: b; }; in lib.optionals a [x]",
        "{ lib }: let result = lib.optionals a [x]; lib = {}; in result",
        "{ lib, other }: with lib; with other; optionals a [x]",
        "{ lib }: (lib.optionals or fallback) a [x]",
    ] {
        assert!(run(source).is_empty(), "{source}");
    }
}

#[test]
fn preserves_comments_and_expression_grouping() {
    assert_eq!(
        fixed("{ lib }: lib.optionals a [ # before\n (f x) /* after */ ]"),
        "{ lib }: lib.optional a ( # before\n (f x) /* after */ )"
    );
    assert_eq!(
        fixed("{ lib }: lib.optionals a [(x: x)]"),
        "{ lib }: lib.optional a (x: x)"
    );
    assert_eq!(
        fixed("{ lib }: lib.optionals a [attrs.value]"),
        "{ lib }: lib.optional a attrs.value"
    );
    assert_eq!(
        fixed("{ lib }: lib.optionals a [(attrs.value or fallback)]"),
        "{ lib }: lib.optional a (attrs.value or fallback)"
    );
    assert_eq!(
        fixed("{ lib }: lib.optionals a [ { value = 1; } ]"),
        "{ lib }: lib.optional a { value = 1; }"
    );
}

#[test]
fn registered_enabled_by_default_and_suppressible() {
    let rules: Vec<Box<dyn Rule>> = strictix_lints::all_rules()
        .into_iter()
        .filter(|rule| rule.code() == "singleton-optionals")
        .collect();
    assert_eq!(rules.len(), 1);
    let check = |source: &str, config: LintConfig| {
        let tree = parse(source);
        let model = SemanticModel::new(source, &tree);
        let mut diags = Vec::new();
        run_rules(&rules, &tree, &model, &config, source, &mut diags);
        diags
    };
    let source = "{ lib }: lib.optionals a [x]";
    assert_eq!(check(source, LintConfig::default()).len(), 1);
    assert!(check(
        source,
        LintConfig::default().with_disabled(["singleton-optionals".into()])
    )
    .is_empty());
    assert!(check(
        &format!("# strictix: disable-next-line=singleton-optionals\n{source}"),
        LintConfig::default()
    )
    .is_empty());
}
