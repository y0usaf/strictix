use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, rules::Rule, semantic::SemanticModel,
};
use strictix_lints::flake_inputs::FlakeInputWithoutNixpkgsFollows;
use strictix_syntax::parse;

fn check_with(source: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    FlakeInputWithoutNixpkgsFollows {}.check_file(&model, config, &mut diags);
    diags
}

fn check(source: &str) -> Vec<Diagnostic> {
    check_with(source, &LintConfig::default())
}

#[test]
fn flags_an_attrset_input_that_pins_its_own_nixpkgs() {
    let diags = check(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            tomoe.url = "github:y0usaf/tomoe";
          };
          outputs = _: {};
        }"#,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, "flake-input-without-nixpkgs-follows");
    assert_eq!(diags[0].severity_str(), "warning");
    assert!(diags[0].message.contains("'tomoe'"), "{}", diags[0].message);
}

#[test]
fn flags_an_input_written_as_a_dotted_url() {
    let diags = check(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            cudaterm.url = "github:y0usaf/cudaterm";
          };
          outputs = _: {};
        }"#,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
}

#[test]
fn accepting_follows_in_both_shapes() {
    let diags = check(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            a = {
              url = "github:a/a";
              inputs.nixpkgs.follows = "nixpkgs";
            };
            b.url = "github:b/b";
            b.inputs.nixpkgs.follows = "nixpkgs";
          };
          outputs = _: {};
        }"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn a_source_only_input_is_exempt() {
    let diags = check(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            omp-bot = {
              url = "git+file:///srv/omp-bot";
              flake = false;
            };
          };
          outputs = _: {};
        }"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn the_root_nixpkgs_itself_is_never_flagged() {
    let diags = check(
        r#"{ inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable"; outputs = _: {}; }"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn a_file_without_a_nixpkgs_input_is_left_alone() {
    let diags = check(r#"{ inputs.foo.url = "github:a/b"; outputs = _: {}; }"#);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn the_allow_setting_suppresses_an_input() {
    let config =
        LintConfig::default().with_setting("flake-input-without-nixpkgs-follows", "allow", "tomoe");
    let diags = check_with(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            tomoe.url = "github:y0usaf/tomoe";
          };
          outputs = _: {};
        }"#,
        &config,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn a_nested_inputs_block_counts_as_following() {
    let diags = check(
        r#"{
          inputs = {
            nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
            durapi = {
              url = "github:y0usaf/durapi";
              inputs = {
                nixpkgs.follows = "nixpkgs";
                pi.follows = "pi";
              };
            };
          };
          outputs = _: {};
        }"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}
