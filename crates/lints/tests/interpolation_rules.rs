use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::interpolation_rules::{CoercedInterpolation, RedundantInterpolation};
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

// --- coerced-interpolation --------------------------------------------

#[test]
fn coerced_interpolation_flags_int_and_fixes_by_splicing_digits() {
    let source = r#""count: ${5}""#;
    let diagnostics = run(source, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "coerced-interpolation");
    assert_eq!(diagnostics[0].severity_str(), "error");
    let fix = diagnostics[0].fix.as_ref().expect("integer splice fix");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok(r#""count: 5""#.to_owned())
    );
}

#[test]
fn coerced_interpolation_whole_string_int_fix_round_trips() {
    let source = r#""${5}""#;
    let diagnostics = run(source, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    let fix = diagnostics[0].fix.as_ref().expect("integer splice fix");
    assert_eq!(apply_fixes(source, &fix.edits), Ok(r#""5""#.to_owned()));
}

#[test]
fn coerced_interpolation_leading_zero_int_flags_without_fix() {
    // `007` evaluates to 7, so splicing the raw digits would not match
    // what toString produces; the finding stands but no fix is offered.
    let diagnostics = run(r#""${007}""#, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn coerced_interpolation_flags_float_without_fix() {
    // toString 1.5 is "1.500000", so no splice is value-preserving.
    let diagnostics = run(r#""${1.5}""#, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "coerced-interpolation");
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn coerced_interpolation_flags_list_literal() {
    let diagnostics = run(r#""${[ 1 2 ]}""#, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn coerced_interpolation_flags_plain_attrset_literal() {
    let diagnostics = run(r#""${{ a = 1; }}""#, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn coerced_interpolation_attrset_with_outpath_or_tostring_is_silent() {
    for source in [
        r#""${{ outPath = "/nix/store/x"; }}""#,
        r#""${{ __toString = self: "x"; }}""#,
        r#""${{ "outPath" = "/nix/store/x"; }}""#,
        r#""${rec { outPath = "/x"; other = outPath; }}""#,
        r#"let drv = { outPath = "/x"; }; in "${{ inherit (drv) outPath; }}""#,
    ] {
        assert!(
            run(source, Box::new(CoercedInterpolation {})).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn coerced_interpolation_dynamic_attr_name_is_silent() {
    // A dynamic name could evaluate to outPath; nothing is proven.
    assert!(run(
        r#"let k = "outPath"; in "${{ ${k} = "/x"; }}""#,
        Box::new(CoercedInterpolation {})
    )
    .is_empty());
}

#[test]
fn coerced_interpolation_flags_rec_attrset_without_coercion_hooks() {
    let diagnostics = run(
        r#""${rec { a = 1; b = a; }}""#,
        Box::new(CoercedInterpolation {}),
    );
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn coerced_interpolation_flags_global_true_false_null() {
    for source in [r#""${true}""#, r#""${false}""#, r#""${null}""#] {
        let diagnostics = run(source, Box::new(CoercedInterpolation {}));
        assert_eq!(diagnostics.len(), 1, "{source}");
        assert_eq!(diagnostics[0].severity_str(), "error");
        assert!(diagnostics[0].fix.is_none(), "{source}");
    }
}

#[test]
fn coerced_interpolation_shadowed_true_is_silent() {
    // `true` is an ordinary ident; a lexical binding makes it somebody's
    // variable whose value the syntax cannot know.
    assert!(run(
        r#"let true = "y"; in "${true}""#,
        Box::new(CoercedInterpolation {})
    )
    .is_empty());
}

#[test]
fn coerced_interpolation_with_covered_null_is_silent() {
    // An enclosing `with` may supply the name; nothing is proven.
    assert!(run(
        r#"with { null = "n"; }; "${null}""#,
        Box::new(CoercedInterpolation {})
    )
    .is_empty());
}

#[test]
fn coerced_interpolation_unwraps_parens() {
    let diagnostics = run(r#""${(5)}""#, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "coerced-interpolation");
}

#[test]
fn coerced_interpolation_dynamic_interpolants_are_silent() {
    for source in [
        r#"x: "${x}""#,
        r#"f: "${f 1}""#,
        r#"s: "${s.a}""#,
        r#"x: "${toString x}""#,
        r#""${"nested"}""#,
    ] {
        assert!(
            run(source, Box::new(CoercedInterpolation {})).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn coerced_interpolation_fires_inside_indented_string() {
    let source = "''line ${5}''";
    let diagnostics = run(source, Box::new(CoercedInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    let fix = diagnostics[0].fix.as_ref().expect("integer splice fix");
    assert_eq!(apply_fixes(source, &fix.edits), Ok("''line 5''".to_owned()));
}

#[test]
fn coerced_interpolation_ignores_attrname_interpolation() {
    // `a.${...}` is an attrpath interpolation, not a string part.
    assert!(run(
        r#"let a = { "5" = 1; }; in a.${"5"}"#,
        Box::new(CoercedInterpolation {})
    )
    .is_empty());
}

// --- redundant-interpolation --------------------------------------------

#[test]
fn redundant_interpolation_flags_and_splices_plain_literal() {
    let source = r#""a ${"b"} c""#;
    let diagnostics = run(source, Box::new(RedundantInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "redundant-interpolation");
    assert_eq!(diagnostics[0].severity_str(), "warning");
    let fix = diagnostics[0].fix.as_ref().expect("safe splice");
    assert_eq!(apply_fixes(source, &fix.edits), Ok(r#""a b c""#.to_owned()));
}

#[test]
fn redundant_interpolation_whole_string_identity_round_trips() {
    let source = r#""${"foo"}""#;
    let diagnostics = run(source, Box::new(RedundantInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    let fix = diagnostics[0].fix.as_ref().expect("safe splice");
    assert_eq!(apply_fixes(source, &fix.edits), Ok(r#""foo""#.to_owned()));
}

#[test]
fn redundant_interpolation_escape_bearing_inner_is_fixless() {
    // `\n` means a newline inside `"..."` but two literal characters if
    // spliced into an indented string; and even in a `"..."` outer, the
    // rule conservatively refuses to move escapes.
    let diagnostics = run(r#""${"a\nb"}""#, Box::new(RedundantInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn redundant_interpolation_indented_outer_splices_quote_free_content() {
    let source = r#"''pre ${"mid"} post''"#;
    let diagnostics = run(source, Box::new(RedundantInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    let fix = diagnostics[0].fix.as_ref().expect("safe splice");
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("''pre mid post''".to_owned())
    );
}

#[test]
fn redundant_interpolation_indented_outer_refuses_quote_in_content() {
    // A spliced `'` could pair with a neighboring quote into `''`.
    let diagnostics = run(r#"''${"it's"}''"#, Box::new(RedundantInterpolation {}));
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].fix.is_none());
}

#[test]
fn redundant_interpolation_skips_nested_interpolation_and_empty_string() {
    for source in [r#"x: "${"a${x}"}""#, r#""${""}""#] {
        assert!(
            run(source, Box::new(RedundantInterpolation {})).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn redundant_interpolation_skips_non_string_interpolants() {
    for source in [r#"x: "${x}""#, "''${''ind''}''", r#"x: "${toString x}""#] {
        assert!(
            run(source, Box::new(RedundantInterpolation {})).is_empty(),
            "{source}"
        );
    }
}
