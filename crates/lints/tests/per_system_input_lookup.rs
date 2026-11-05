use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::per_system_input_lookup::PerSystemInputLookup;
use strictix_syntax::parse;

fn enabled() -> LintConfig {
    LintConfig::default().with_enabled(["per-system-input-lookup".to_owned()])
}

fn run(source: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    PerSystemInputLookup.check_file(&model, config, &mut diags);
    let mut through = Vec::new();
    run_rules(
        &[Box::new(PerSystemInputLookup)],
        &tree,
        &model,
        config,
        source,
        &mut through,
    );
    assert_eq!(diags.len(), through.len());
    diags
}

fn argument() -> LintConfig {
    enabled().with_setting("per-system-input-lookup", "argument", "inputPackages")
}

#[test]
fn every_platform_spelling_is_reported() {
    let source = "{flakeInputs, inputs, pkgs, system, ...}: [
  flakeInputs.a.packages.${pkgs.stdenv.hostPlatform.system}.default
  inputs.b.packages.${system}.default
  inputs.c.outputs.packages.${pkgs.system}.c
  inputs.nixpkgs.legacyPackages.${system}.hello
]";
    let diags = run(source, &enabled());
    assert_eq!(diags.len(), 4, "{diags:?}");
    assert!(diags[0].message.starts_with(
        "`flakeInputs.a.packages.${pkgs.stdenv.hostPlatform.system}` looks the platform up"
    ));
    assert!(diags.iter().all(|d| d.fix.is_none()));
}

#[test]
fn other_bases_and_fixed_platforms_are_left_alone() {
    let source = "{self, inputs, ...}: [
  self.packages.${system}.default
  inputs.a.packages.x86_64-linux.default
  inputs.a.packages.${name}.default
  inputs.a.lib.${system}
  deps.a.packages.${system}.default
]";
    assert!(run(source, &enabled()).is_empty());
    let custom = enabled().with_setting("per-system-input-lookup", "inputs", "deps");
    assert_eq!(run(source, &custom).len(), 1);
}

#[test]
fn a_bound_argument_gets_the_fix() {
    let source = "{flakeInputs, inputPackages, pkgs, ...}: {
  a = flakeInputs.a.packages.${pkgs.stdenv.hostPlatform.system}.default;
  b = flakeInputs.\"b-c\".outputs.packages.${pkgs.stdenv.hostPlatform.system};
}";
    let diags = run(source, &argument());
    assert_eq!(diags.len(), 2, "{diags:?}");
    let edits: Vec<_> = diags
        .iter()
        .flat_map(|d| d.fix.as_ref().expect("fix").edits.clone())
        .collect();
    let fixed = apply_fixes(source, &edits).unwrap();
    assert_eq!(
        fixed,
        "{flakeInputs, inputPackages, pkgs, ...}: {
  a = inputPackages.a.default;
  b = inputPackages.\"b-c\";
}"
    );
    assert!(run(&fixed, &argument()).is_empty());
}

#[test]
fn an_unbound_argument_or_legacy_packages_gets_help_only() {
    let source = "{inputs, inputPackages ? {}, system, ...}: [
  inputs.nixpkgs.legacyPackages.${system}.hello
  (let inputs' = inputs; in inputs.a.packages.${system})
]";
    let diags = run(source, &argument());
    assert_eq!(diags.len(), 2);
    assert!(diags[0].fix.is_none());
    assert!(diags[1].fix.is_some());
    let unbound = "{inputs, system, ...}: inputs.a.packages.${system}.default";
    let diags = run(unbound, &argument());
    assert!(diags[0].fix.is_none());
    assert!(diags[0]
        .help
        .as_deref()
        .is_some_and(|help| help.contains("take `inputPackages` as an argument")));
}
