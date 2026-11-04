use std::path::Path;

use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, fix::apply_fixes, rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::singleton_list::SingletonList;
use strictix_syntax::parse;

fn check(source: &str, path: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree).with_path(Some(Path::new(path)));
    let mut diags = Vec::new();
    SingletonList.check_file(&model, config, &mut diags);
    diags
}

#[test]
fn a_one_item_list_over_lines_is_written_on_one() {
    let source = "{\n  a = [\n    pkgs.x\n  ];\n  b = [\n    (f \"y\")\n  ];\n}\n";
    let diags = check(source, "m.nix", &LintConfig::default());
    assert_eq!(diags.len(), 2);
    assert_eq!(
        diags[0].message,
        "this list holds one item; write `[pkgs.x]`"
    );
    let edits: Vec<_> = diags
        .into_iter()
        .filter_map(|d| d.fix)
        .flat_map(|f| f.edits)
        .collect();
    assert_eq!(
        apply_fixes(source, &edits).unwrap(),
        "{\n  a = [pkgs.x];\n  b = [(f \"y\")];\n}\n"
    );
}

#[test]
fn lists_the_rewrite_would_change_are_left_alone() {
    for source in [
        "[x]\n",
        "[\n  x\n  y\n]\n",
        "[\n  # why\n  x\n]\n",
        "[\n  {\n    a = 1;\n  }\n]\n",
        "[\n]\n",
    ] {
        assert!(
            check(source, "m.nix", &LintConfig::default()).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn exempt_paths_are_skipped() {
    let source = "[\n  x\n]\n";
    let config = LintConfig::default().with_setting("singleton-list", "exempt", "flake.nix");
    assert!(check(source, "flake.nix", &config).is_empty());
    assert_eq!(check(source, "modules/x.nix", &config).len(), 1);
}
