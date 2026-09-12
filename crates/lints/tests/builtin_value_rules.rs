use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::builtin_value_rules::{
    InvalidBuiltinRange, InvalidListToAttrsEntry, ReplaceStringsLengthMismatch,
};
use strictix_syntax::parse;

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let rules: Vec<Box<dyn Rule>> = vec![
        Box::new(InvalidListToAttrsEntry),
        Box::new(ReplaceStringsLengthMismatch),
        Box::new(InvalidBuiltinRange),
    ];
    let mut out = Vec::new();
    run_rules(
        &rules,
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut out,
    );
    out
}

fn finding(source: &str, code: &str, span: &str, message: &str) {
    let out = diagnostics(source);
    assert_eq!(out.len(), 1, "unexpected findings for {source}: {out:?}");
    let diag = &out[0];
    assert_eq!(diag.code, code);
    assert_eq!(diag.severity, Severity::Error);
    assert_eq!(diag.message, message);
    assert_eq!(
        &source[diag.range.start() as usize..diag.range.end() as usize],
        span
    );
    assert!(diag.fix.is_none());
}

fn clean(sources: &[&str]) {
    for source in sources {
        let out = diagnostics(source);
        assert!(out.is_empty(), "unexpected findings for {source}: {out:?}");
    }
}

#[test]
fn reports_missing_list_to_attrs_fields_with_entry_spans() {
    finding(
        "builtins.listToAttrs [ { value = 1; } ]",
        "invalid-list-to-attrs-entry",
        "{ value = 1; }",
        "builtin 'listToAttrs' entry is missing the 'name' attribute",
    );
    finding(
        "builtins.listToAttrs [ { name = \"x\"; } ]",
        "invalid-list-to-attrs-entry",
        "{ name = \"x\"; }",
        "builtin 'listToAttrs' entry is missing the 'value' attribute",
    );
    finding(
        "builtins.listToAttrs [ {} ]",
        "invalid-list-to-attrs-entry",
        "{}",
        "builtin 'listToAttrs' entry is missing the 'name' attribute",
    );
}

#[test]
fn reports_non_sets_and_non_string_names() {
    for entry in [
        "42", "3.5", "[]", "\"x\"", "''x''", "./foo", "(x: x)", "true", "null", "(-1)", "(-1.5)",
    ] {
        finding(
            &format!("builtins.listToAttrs [ {entry} ]"),
            "invalid-list-to-attrs-entry",
            entry,
            "builtin 'listToAttrs' expects each entry to be an attribute set",
        );
    }
    for name in [
        "42", "3.5", "[]", "{}", "rec {}", "./foo", "(x: x)", "false", "null", "((-1))", "(-1.5)",
    ] {
        finding(
            &format!("builtins.listToAttrs [ {{ name = {name}; value = 1; }} ]"),
            "invalid-list-to-attrs-entry",
            name,
            "builtin 'listToAttrs' expects the 'name' attribute to be a string",
        );
    }
    finding(
        "builtins.listToAttrs [ { name.part = \"x\"; value = 1; } ]",
        "invalid-list-to-attrs-entry",
        "name.part",
        "builtin 'listToAttrs' expects the 'name' attribute to be a string",
    );
}

#[test]
fn accepts_valid_entries_and_unknown_values() {
    clean(&[
        "builtins.listToAttrs []",
        "builtins.listToAttrs [{ name = \"x\"; value = 1; }]",
        "builtins.listToAttrs [rec { name = \"x\"; value = name; }]",
        "builtins.listToAttrs [{ name = \"x\"; value.text = \"text\"; }]",
        "builtins.listToAttrs [{ \"name\" = ''x''; \"value\" = 1; }]",
        "builtins.listToAttrs [{ name = n; value = v; }]",
        "builtins.listToAttrs [{ name = \"${n}\"; value = v; }]",
        "builtins.listToAttrs [{ name = https://example.com; value = 1; }]",
        "builtins.listToAttrs [{ name = \"x\"; value = throw \"lazy\"; }]",
        "builtins.listToAttrs [entry]",
        "builtins.listToAttrs entries",
        "builtins.listToAttrs",
        "let false = \"x\"; in builtins.listToAttrs [{ name = false; value = 1; }]",
        "let xs = builtins.listToAttrs [{ name = false; value = 1; }]; false = \"x\"; in xs",
    ]);
}

#[test]
fn leaves_inherits_and_dynamic_attribute_names_unknown() {
    clean(&[
        "builtins.listToAttrs [{ inherit (entry) name value; }]",
        "builtins.listToAttrs [{ inherit name value; }]",
        "builtins.listToAttrs [{ inherit (entry) name; }]",
        "builtins.listToAttrs [{ ${field} = 1; }]",
        "builtins.listToAttrs [{ \"${field}\" = 1; }]",
        "builtins.listToAttrs [{ name = \"x\"; ${field} = 1; }]",
        "builtins.listToAttrs [{ name = 1; ${field} = 1; }]",
        "builtins.listToAttrs [{ name.${field} = 1; value = 1; }]",
    ]);
}

#[test]
fn duplicate_entries_do_not_require_a_value() {
    clean(&[
        "builtins.listToAttrs [{ name = \"x\"; value = 1; } { name = \"x\"; }]",
        "builtins.listToAttrs [{ name = \"x\"; value = 1; } { name = \"x\"; value = throw \"ignored\"; }]",
        r#"builtins.listToAttrs [{ name = "\q"; value = 1; } { name = "q"; }]"#,
        "builtins.listToAttrs [{ name = n; value = 1; } { name = \"x\"; }]",
        "builtins.listToAttrs [{ name = \"x\"; value = 1; } { name = n; }]",
        "builtins.listToAttrs [entry { name = \"x\"; }]",
        "builtins.listToAttrs [{ inherit (entry) name value; } { name = \"x\"; }]",
        "builtins.listToAttrs [{ name = ''x''; value = 1; } { name = \"x\"; }]",
        "builtins.listToAttrs [{ name = \"${n}\"; value = 1; } { name = \"x\"; }]",
    ]);
    finding(
        "builtins.listToAttrs [{ name = \"x\"; value = 1; } { name = \"y\"; }]",
        "invalid-list-to-attrs-entry",
        "{ name = \"y\"; }",
        "builtin 'listToAttrs' entry is missing the 'value' attribute",
    );
    finding(
        "builtins.listToAttrs [{ name = \"x\"; value = 1; } { name = 1; }]",
        "invalid-list-to-attrs-entry",
        "1",
        "builtin 'listToAttrs' expects the 'name' attribute to be a string",
    );
}

#[test]
fn reports_replace_strings_lengths_on_replacement_list() {
    finding(
        "builtins.replaceStrings [\"a\" \"b\"] [\"x\"] \"abc\"",
        "replace-strings-length-mismatch",
        "[\"x\"]",
        "builtin 'replaceStrings' search list has length 2 but replacement list has length 1",
    );
    finding(
        "builtins.replaceStrings [] [replacement] text",
        "replace-strings-length-mismatch",
        "[replacement]",
        "builtin 'replaceStrings' search list has length 0 but replacement list has length 1",
    );
}

#[test]
fn accepts_equal_lengths_and_skips_unknown_lists_and_partial_calls() {
    clean(&[
        "builtins.replaceStrings [] [] \"abc\"",
        "builtins.replaceStrings [\"a\"] [\"b\"] \"abc\"",
        "builtins.replaceStrings [from] [to] text",
        "builtins.replaceStrings from [] \"abc\"",
        "builtins.replaceStrings [] to \"abc\"",
        "builtins.replaceStrings ([\"a\"] ++ []) [] \"abc\"",
        "builtins.replaceStrings",
        "builtins.replaceStrings [\"a\"]",
        "builtins.replaceStrings [\"a\"] []",
        "(builtins.replaceStrings [\"a\"]) []",
    ]);
}

#[test]
fn reports_negative_builtin_ranges_with_argument_spans() {
    finding(
        "builtins.genList (i: i) (-1)",
        "invalid-builtin-range",
        "(-1)",
        "builtin 'genList' length must not be negative",
    );
    finding(
        "builtins.substring (-(2)) 3 \"abcd\"",
        "invalid-builtin-range",
        "(-(2))",
        "builtin 'substring' start position must not be negative",
    );
}

#[test]
fn accepts_range_boundaries_and_negative_substring_lengths() {
    clean(&[
        "builtins.genList (i: i) 0",
        "builtins.genList (i: i) (-0)",
        "builtins.genList (i: i) 2",
        "builtins.substring 0 0 \"abc\"",
        "builtins.substring 3 4 \"abc\"",
        "builtins.substring 0 (-1) \"abc\"",
        "builtins.substring 0 (-2) \"abc\"",
        "builtins.genList f length",
        "builtins.genList f (0 - 1)",
        "builtins.substring start 2 text",
        "builtins.substring (0 - 1) 2 text",
        "builtins.genList",
        "builtins.genList (i: i)",
        "builtins.substring (-1)",
        "builtins.substring (-1) 2",
    ]);
}

#[test]
fn recognizes_parentheses_currying_and_nested_calls_once() {
    for source in [
        "f (((builtins).listToAttrs) (([ (({ name = ((\"x\")); })) ])))",
        "((builtins.replaceStrings) ([\"a\"])) ([]) \"text\"",
        "(((builtins).genList) (i: i)) ((-(1)))",
        "((builtins.substring (-1)) 2) \"abc\"",
        "builtins.listToAttrs [{}] 1",
        "builtins.replaceStrings [\"x\"] [] \"abc\" 1",
        "builtins.genList (i: i) (-1) 1",
    ] {
        assert_eq!(
            diagnostics(source).len(),
            1,
            "unexpected findings for {source}"
        );
    }
}

#[test]
fn respects_builtin_shadowing_and_ambiguous_callees() {
    for call in [
        "builtins.listToAttrs [{}]",
        "builtins.replaceStrings [\"x\"] [] \"text\"",
        "builtins.genList (i: i) (-1)",
        "builtins.substring (-1) 2 \"text\"",
    ] {
        for source in [
            format!("let builtins = custom; in {call}"),
            format!("let result = {call}; builtins = custom; in result"),
            format!("let inherit (custom) builtins; in {call}"),
            format!("builtins: {call}"),
            format!("{{ builtins }}: {call}"),
            format!("{{ result ? {call}, builtins ? custom }}: result"),
            format!("rec {{ result = {call}; builtins = custom; }}"),
            format!("with custom; {call}"),
        ] {
            clean(&[&source]);
        }
    }
    clean(&[
        "listToAttrs [{}]",
        "replaceStrings [\"x\"] [] \"text\"",
        "genList (i: i) (-1)",
        "substring (-1) 2 \"text\"",
        "other.listToAttrs [{}]",
        "builtins.nested.listToAttrs [{}]",
        "builtins.${fn} [{}]",
        "(builtins.listToAttrs or fallback) [{}]",
    ]);
}
