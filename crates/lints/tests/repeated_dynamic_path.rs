use std::path::PathBuf;

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    project::{ProjectContext, ProjectFile},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::repeated_dynamic_path::RepeatedDynamicPath;
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
        RepeatedDynamicPath.check_file_project(&model, config, Some(&project), &mut diags);
        out.extend(diags.into_iter().map(|d| (path.display().to_string(), d)));
    }
    out
}

const A: &str = "{config, ...}: { manzil.users.${config.user.name}.files.a.text = \"a\"; }";
const B: &str = "{config, ...}: { manzil = { users.${config.user.name}.files.b.text = \"b\"; }; }";
const C: &str = "{config, lib, ...}: { config = lib.mkIf true { manzil.users.${config.user.name}.files.c.text = \"c\"; }; }";
const D: &str =
    "{config, ...}: { config.manzil.users.\"${config.user.name}\".files.d.text = \"d\"; }";

#[test]
fn a_dynamic_path_in_four_modules_is_reported_at_each() {
    let found = diagnostics(
        &[
            ("./a.nix", A),
            ("./b.nix", B),
            ("./c.nix", C),
            ("./d.nix", D),
        ],
        &LintConfig::default(),
    );
    let files: Vec<&str> = found.iter().map(|(file, _)| file.as_str()).collect();
    assert_eq!(
        files,
        ["./a.nix", "./b.nix", "./c.nix", "./d.nix"],
        "{found:?}"
    );
    assert!(
        found[0].1.message.starts_with("`manzil.users.${config.user.name}` is spelled out here and in 3 other modules (b.nix:1, c.nix:1, d.nix:1)"),
        "{found:?}"
    );
    assert!(found[0].1.message.contains("lib.mkAliasDefinitions"));
}

#[test]
fn three_modules_are_below_the_default_and_min_modules_lowers_it() {
    let files = [("./a.nix", A), ("./b.nix", B), ("./c.nix", C)];
    assert!(diagnostics(&files, &LintConfig::default()).is_empty());
    let three = LintConfig::default().with_setting("repeated-dynamic-path", "min-modules", "3");
    assert_eq!(diagnostics(&files, &three).len(), 3);
}

#[test]
fn a_module_counts_once_and_a_bare_name_is_no_shared_path() {
    let twice = "{config, ...}: { manzil.users.${config.user.name}.files.a.text = \"a\"; manzil.users.${config.user.name}.files.b.text = \"b\"; }";
    let bare = "{name, ...}: { users.${name}.files.a.text = \"a\"; }";
    let found = diagnostics(
        &[
            ("./a.nix", twice),
            ("./b.nix", A),
            ("./c.nix", A),
            ("./d.nix", A),
        ],
        &LintConfig::default(),
    );
    assert_eq!(found.len(), 4, "{found:?}");
    let found = diagnostics(
        &[
            ("./a.nix", bare),
            ("./b.nix", bare),
            ("./c.nix", bare),
            ("./d.nix", bare),
        ],
        &LintConfig::default(),
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn merged_bindings_are_modules() {
    let body = "{config, ...}: { users.users.${config.user.name}.extraGroups = [\"x\"]; }";
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
