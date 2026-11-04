use std::path::PathBuf;

use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    project::{ProjectContext, ProjectFile},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::duplicate_definitions::DuplicateDefinitions;
use strictix_syntax::parse;

const RECORD: &str = "{ a = 1; b = 2; c = 3; d = 4; e = 5; f = 6; g = 7; h = 8; }";

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
        DuplicateDefinitions.check_file_project(&model, config, Some(&project), &mut diags);
        out.extend(diags.into_iter().map(|d| (path.display().to_string(), d)));
    }
    out
}

fn generated(prefixes: &[&str]) -> LintConfig {
    LintConfig {
        generated: prefixes.iter().map(PathBuf::from).collect(),
        ..LintConfig::default()
    }
}

#[test]
fn identical_records_in_two_files_are_reported_at_both() {
    let found = diagnostics(
        &[("./a.nix", RECORD), ("./b.nix", RECORD)],
        &LintConfig::default(),
    );
    let files: Vec<&str> = found.iter().map(|(file, _)| file.as_str()).collect();
    assert_eq!(files, ["./a.nix", "./b.nix"], "{found:?}");
    assert!(found[0].1.message.contains("these 8 leaf bindings"));
}

#[test]
fn generated_files_are_left_out_of_the_comparison() {
    let found = diagnostics(
        &[
            ("./a.nix", RECORD),
            ("./locks/b.data.nix", RECORD),
            ("./locks/c.data.nix", RECORD),
        ],
        &generated(&["locks"]),
    );
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_generated_prefix_spares_only_the_files_under_it() {
    let found = diagnostics(
        &[
            ("./a.nix", RECORD),
            ("./b.nix", RECORD),
            ("./locks/c.data.nix", RECORD),
        ],
        &generated(&["./locks/c.data.nix"]),
    );
    let files: Vec<&str> = found.iter().map(|(file, _)| file.as_str()).collect();
    assert_eq!(files, ["./a.nix", "./b.nix"], "{found:?}");
    assert!(!found[0].1.message.contains("c.data.nix"), "{found:?}");
}

#[test]
fn a_merged_binding_whose_manifest_path_is_generated_is_left_out() {
    let merged = format!(
        "let\n  a = {RECORD};\n  b = {RECORD};\n  c = {RECORD};\n  merge_manifest = {{\n    files = {{\n      a = \"a.nix\";\n      b = \"locks/b.data.nix\";\n      c = \"locks/c.data.nix\";\n    }};\n  }};\nin\n  builtins.seq merge_manifest (a b c)\n"
    );
    let all = diagnostics(&[("./flake.nix", &merged)], &LintConfig::default());
    assert_eq!(all.len(), 3, "{all:?}");
    let found = diagnostics(&[("./flake.nix", &merged)], &generated(&["locks"]));
    assert!(found.is_empty(), "{found:?}");
    let one = diagnostics(
        &[("./flake.nix", &merged)],
        &generated(&["locks/c.data.nix"]),
    );
    assert_eq!(one.len(), 2, "{one:?}");
    assert!(
        one.iter().all(|(_, d)| !d.message.contains("(line 4)")),
        "{one:?}"
    );
}
