use std::path::PathBuf;

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    project::{ProjectContext, ProjectFile},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::repeated_let_value::RepeatedLetValue;
use strictix_syntax::parse;

fn diagnostics(files: &[(&str, &str)], config: &LintConfig) -> Vec<(String, Diagnostic)> {
    let project = ProjectContext::new(files.iter().map(|(path, source)| ProjectFile {
        path: PathBuf::from(path),
        source: (*source).to_owned(),
    }));
    let mut out = Vec::new();
    for (path, source) in files {
        let tree = parse(source);
        let path = PathBuf::from(path);
        let model = SemanticModel::new(source, &tree).with_path(Some(&path));
        let mut diags = Vec::new();
        RepeatedLetValue.check_file_project(&model, config, Some(&project), &mut diags);
        out.extend(diags.into_iter().map(|d| (path.display().to_string(), d)));
    }
    out
}

const RUNTIME: &str = "{config, ...}: let\n  user = config.users.users.me;\n  runtimeDir = \"/run/user/${toString user.uid}\";\nin {x = runtimeDir;}";
const RENAMED: &str = "{config, ...}: let user = config.users.users.me; dir = \"/run/user/${toString  user.uid}\"; in dir";

#[test]
fn a_value_computed_in_four_modules_is_reported_at_each_whatever_its_name() {
    let files = [
        ("./a.nix", RUNTIME),
        ("./b.nix", RUNTIME),
        ("./c.nix", RUNTIME),
        ("./d.nix", RENAMED),
    ];
    let found = diagnostics(&files, &LintConfig::default());
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found[0].1.message.starts_with(
        "`runtimeDir = \"/run/user/${toString user.uid}\"` is computed here and in 3 other modules (b.nix:3, c.nix:3, d.nix:1)"
    ), "{found:?}");
    assert!(found[3].1.message.starts_with("`dir = "), "{found:?}");
    let three = &files[..3];
    assert!(diagnostics(three, &LintConfig::default()).is_empty());
    let low = LintConfig::default().with_setting("repeated-let-value", "min-modules", "3");
    assert_eq!(diagnostics(three, &low).len(), 3);
}

#[test]
fn aliases_literals_and_imports_are_not_computations() {
    let source = "{config, lib, ...}: let\n  name = config.user.name;\n  n = 3;\n  s = \"plain\";\n  shared = import ../shared.nix {inherit lib;};\n  prelude = import ./prelude.nix lib config;\n  data = import ./x.nix;\nin [name n s shared prelude data]";
    let files = [
        ("./a.nix", source),
        ("./b.nix", source),
        ("./c.nix", source),
        ("./d.nix", source),
    ];
    assert!(diagnostics(&files, &LintConfig::default()).is_empty());
}

#[test]
fn merged_bindings_are_modules_and_generated_ones_are_left_out() {
    let body = "{lib, ...}: let json = lib.generators.toJSON {}; in json";
    let merged = format!(
        "let\n  a = {body};\n  b = {body};\n  c = {body};\n  d = {body};\n  merge_manifest = {{\n    files = {{\n      a = \"a.nix\";\n      b = \"b.nix\";\n      c = \"c.nix\";\n      d = \"gen/d.nix\";\n    }};\n  }};\nin\n  builtins.seq merge_manifest [a b c d]\n"
    );
    let found = diagnostics(&[("./flake.nix", &merged)], &LintConfig::default());
    assert_eq!(found.len(), 4, "{found:?}");
    assert!(found[0].1.message.contains("b (line 3)"), "{found:?}");
    let generated = LintConfig {
        generated: vec![PathBuf::from("gen")],
        ..LintConfig::default()
    };
    assert!(diagnostics(&[("./flake.nix", &merged)], &generated).is_empty());
}
