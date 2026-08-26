//! Integration tests for the `bare-import-in-list` file rule.
//!
//! Diagnostics are rendered in the one-line contract format
//! (`[code] severity start..end message`) and asserted exactly. The rule
//! needs the SemanticModel (to skip a shadowed local `import`), so the
//! harness builds one per source, same as `file_rules.rs`.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::Diagnostic;
use strictix_core::rules::run_rules;
use strictix_core::semantic::SemanticModel;
use strictix_lints::bare_import_in_list::BareImportInList;
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

/// Run the single rule over `source` with a fresh semantic model.
fn run(source: &str, config: LintConfig) -> Vec<String> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(BareImportInList {})],
        &tree,
        &model,
        &config,
        source,
        &mut diags,
    );
    render(&diags)
}

/// Collect the fix edits (without rendering) for `source`.
fn fixes(source: &str) -> Vec<(u32, u32, String)> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(BareImportInList {})],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags
        .iter()
        .filter_map(|d| d.fix.as_ref())
        .map(|f| {
            let e = &f.edits[0];
            (e.range.start(), e.range.end(), e.replacement.clone())
        })
        .collect()
}

// `{ imports = [ import ./hw.nix ]; }`
//
// Layout (0-based):
//   0  {       14..20  import     21..29 ./hw.nix
//   13 [        30 ]
// The bare `import` is a separate element from the path → one finding.
#[test]
fn bare_import_in_list_triggers_once_with_exact_offsets() {
    let source = "{ imports = [ import ./hw.nix ]; }";
    assert_eq!(
        run(source, LintConfig::default()),
        ["[bare-import-in-list] warning 14..20 'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)"]
    );
    // The fix lifts the path into a paren frame spanning import..path:
    let fix = fixes(source);
    assert_eq!(fix, [(14, 29, "(import ./hw.nix)".to_string())]);
}

// The same source, expressed as rendered diagnostics (contract format),
// and the applied fix yields the parenthesized form.
#[test]
fn bare_import_in_list_applies_fix_in_module() {
    let source = "{ imports = [ import ./hw.nix ]; }";
    let diags = fixes(source);
    assert_eq!(diags, [(14, 29, "(import ./hw.nix)".to_string())]);
    let result = strictix_core::fix::apply_fixes(
        source,
        &[strictix_core::fix::TextEdit::new(
            strictix_syntax::TextRange::new(14, 29),
            "(import ./hw.nix)".to_string(),
        )],
    )
    .expect("fix applies");
    assert_eq!(result, "{ imports = [ (import ./hw.nix) ]; }");
}

// `[ import ./a.nix ]` — one finding, fix yields the parenthesized form.
#[test]
fn bare_import_in_list_triggers_and_fixes_bare_list() {
    let source = "[ import ./a.nix ]";
    assert_eq!(
        run(source, LintConfig::default()),
        ["[bare-import-in-list] warning 2..8 'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)"]
    );
    let diags = fixes(source);
    assert_eq!(diags, [(2, 16, "(import ./a.nix)".to_string())]);
    let result = strictix_core::fix::apply_fixes(
        source,
        &[strictix_core::fix::TextEdit::new(
            strictix_syntax::TextRange::new(2, 16),
            "(import ./a.nix)".to_string(),
        )],
    )
    .expect("fix applies");
    assert_eq!(result, "[ (import ./a.nix) ]");
}

// Already parenthesized → the item is a Paren, not a bare import ident.
#[test]
fn bare_import_in_list_clean_when_parenthesized() {
    assert_eq!(
        run("[ (import ./a.nix) ]", LintConfig::default()),
        Vec::<String>::new()
    );
}

// Bare `import` as the LAST element has nothing to import — no finding.
#[test]
fn bare_import_in_list_clean_when_last_item() {
    assert_eq!(
        run("[ import ]", LintConfig::default()),
        Vec::<String>::new()
    );
}

// A shadowed `import` (local binding) is the user's own function, not the
// builtin — this is exactly why the rule is a FILE rule.
#[test]
fn bare_import_in_list_clean_when_shadowed() {
    assert_eq!(
        run(
            "let import = x: y; in [ import ./a.nix ]",
            LintConfig::default()
        ),
        Vec::<String>::new()
    );
}

// Two paths after one bare `import`: both elements would be caught if the
// author meant a single import; the rule fires exactly once at the import.
#[test]
fn bare_import_in_list_fires_once_with_extra_path() {
    let source = "[ import ./a ./b ]";
    assert_eq!(
        run(source, LintConfig::default()),
        ["[bare-import-in-list] warning 2..8 'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)"]
    );
    // Fix still targets just the first following path.
    assert_eq!(fixes(source), [(2, 12, "(import ./a)".to_string())]);
}

// Interpolated strings are not touchable → diagnostic but no auto-fix.
#[test]
fn bare_import_in_list_no_fix_on_interpolated_string() {
    let source = r#"[ import "../../${name}/x" ]"#;
    assert_eq!(
        run(source, LintConfig::default()),
        ["[bare-import-in-list] warning 2..8 'import' in a list applies only to the next item; wrap the imported expression in parens: (import …)"]
    );
    assert_eq!(fixes(source), Vec::<(u32, u32, String)>::new());
}
