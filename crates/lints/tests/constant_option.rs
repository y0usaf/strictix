use std::path::PathBuf;

use strictix_core::{
    config::LintConfig,
    project::{ProjectContext, ProjectFile},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::constant_option::ConstantOption;
use strictix_syntax::parse;

const DECLARING: &str = "{config, lib, ...}: {\n  options.a = {\n    one = lib.mkOption {\n      type = lib.types.str;\n      default = \"x\";\n    };\n    three = lib.mkOption {\n      type = lib.types.int;\n      default = 3;\n    };\n  };\n  config.environment.variables.ONE = config.a.one;\n}\n";

fn project() -> ProjectContext {
    let files = [
        ("defaults.nix", DECLARING),
        (
            "b.nix",
            "{config, ...}: let\n  inherit (config.a) three;\nin {\n  b = three;\n}\n",
        ),
        (
            "c.nix",
            "{config, ...}: let\n  cfg = config.a;\nin {\n  c = cfg.three;\n}\n",
        ),
        ("d.nix", "{config, ...}: {\n  d = config.a.three;\n}\n"),
    ];
    ProjectContext::new(files.map(|(path, source)| ProjectFile {
        path: PathBuf::from(path),
        source: source.to_owned(),
    }))
}

fn messages(config: &LintConfig) -> Vec<String> {
    let project = project();
    let tree = parse(DECLARING);
    let model = SemanticModel::new(DECLARING, &tree);
    let mut diags = Vec::new();
    ConstantOption.check_file_project(&model, config, Some(&project), &mut diags);
    diags.into_iter().map(|d| d.message).collect()
}

#[test]
fn every_constant_option_is_reported_without_a_readers_setting() {
    let found = messages(&LintConfig::default());
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].starts_with("option 'a.one' is never set"),
        "{found:?}"
    );
    assert!(
        found[1].starts_with("option 'a.three' is never set"),
        "{found:?}"
    );
}

#[test]
fn readers_setting_keeps_an_option_enough_files_read() {
    let config = LintConfig::default().with_setting("constant-option", "readers", "3");
    let found = messages(&config);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].starts_with(
            "option 'a.one' is never set anywhere in the project, so it always has its default, and 1 file(s) read it where 3 would justify an option;"
        ),
        "{found:?}"
    );
}

#[test]
fn defined_setting_keeps_an_option_defined_outside_the_project() {
    let config = LintConfig::default()
        .with_setting("constant-option", "readers", "3")
        .with_setting("constant-option", "defined", "a.one");
    assert!(messages(&config).is_empty());
}
