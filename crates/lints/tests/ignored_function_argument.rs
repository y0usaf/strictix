use std::path::PathBuf;

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    project::{ProjectContext, ProjectFile},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::ignored_function_argument::IgnoredFunctionArgument;
use strictix_syntax::parse;

fn run_in(files: &[(&str, &str)], target: &str) -> Vec<Diagnostic> {
    let project = ProjectContext::new(files.iter().map(|(path, source)| ProjectFile {
        path: PathBuf::from(path),
        source: (*source).to_owned(),
    }));
    let source = files.iter().find(|(p, _)| *p == target).unwrap().1;
    let tree = parse(source);
    let path = PathBuf::from(target);
    let model = SemanticModel::new(source, &tree).with_path(Some(&path));
    let mut diags = Vec::new();
    IgnoredFunctionArgument.check_file_project(
        &model,
        &LintConfig::default(),
        Some(&project),
        &mut diags,
    );
    diags
}

fn run(source: &str) -> Vec<Diagnostic> {
    run_in(&[("./a.nix", source)], "./a.nix")
}

fn fixed(source: &str) -> String {
    let diags = run(source);
    assert_eq!(diags.len(), 1, "{diags:?}");
    let out = apply_fixes(source, &diags[0].fix.as_ref().expect("fix").edits).unwrap();
    assert!(run(&out).is_empty(), "{out}");
    out
}

#[test]
fn an_inherited_name_the_pattern_ignores_is_dropped() {
    assert_eq!(
        fixed("{lib, pkgs}: let f = {lib, ...}: lib; in f {inherit lib pkgs;}"),
        "{lib, pkgs}: let f = {lib, ...}: lib; in f {inherit lib;}"
    );
    let diags = run("{lib, pkgs}: let f = {lib, ...}: lib; in f {inherit lib pkgs;}");
    assert!(diags[0]
        .message
        .starts_with("the function ignores 'pkgs': its pattern neither names it"));
}

#[test]
fn bindings_and_whole_inherits_go_with_their_space() {
    assert_eq!(
        fixed("{s}: let f = {a, ...}: a; in f {\n  a = 1;\n  b = 2;\n  inherit (s) c d;\n}"),
        "{s}: let f = {a, ...}: a; in f {\n  a = 1;\n}"
    );
    assert_eq!(
        fixed("{s}: let f = {a, ...}: a; in f {inherit (s) a c;}"),
        "{s}: let f = {a, ...}: a; in f {inherit (s) a;}"
    );
}

#[test]
fn closed_and_named_patterns_and_declared_names_are_left_alone() {
    assert!(run("let f = {a}: a; in f {a = 1;}").is_empty());
    assert!(run("let f = args@{a, ...}: args.b; in f {a = 1; b = 2;}").is_empty());
    assert!(run("let f = {a, b ? 1, ...}: a; in f {a = 1; b = 2;}").is_empty());
    assert!(run("let f = x: x; in f {a = 1;}").is_empty());
}

#[test]
fn a_comment_in_a_dropped_value_keeps_the_report_without_a_fix() {
    let diags = run("let f = {a, ...}: a; in f {a = 1; b = 2 # why\n;}");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].fix.is_none());
}

#[test]
fn an_imported_project_file_is_read_for_its_pattern() {
    let files = [
        ("./hosts/shared.nix", "{lib, ...}: {x = lib;}"),
        (
            "./hosts/a/config.nix",
            "{lib, pkgs, ...}: (import ../shared.nix {inherit lib pkgs;}).x",
        ),
        ("./hosts/b/default.nix", "{a, ...}: a"),
        ("./c.nix", "{pkgs}: import ./hosts/b {a = 1; inherit pkgs;}"),
    ];
    let diags = run_in(&files, "./hosts/a/config.nix");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("'pkgs'"));
    assert_eq!(run_in(&files, "./c.nix").len(), 1);
    let unknown = [("./d.nix", "{lib}: import ./missing.nix {inherit lib;}")];
    assert!(run_in(&unknown, "./d.nix").is_empty());
}
