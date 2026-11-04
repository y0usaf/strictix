use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, rules::Rule, semantic::SemanticModel,
};
use strictix_lints::machine_local_default::MachineLocalDefault;
use strictix_syntax::parse;

fn check(source: &str) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    MachineLocalDefault {}.check_file(&model, &LintConfig::default(), &mut diags);
    diags
}

#[test]
fn flags_a_default_that_names_one_machine() {
    let diags = check(
        r#"{ lib, ... }: {
          options.user.dev.paseo.environmentFiles = lib.mkOption {
            type = lib.types.listOf lib.types.str;
            default = ["/home/y0usaf/Tokens/AI_GATEWAY_API_KEY.txt"];
          };
        }"#,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, "machine-local-default");
    assert!(diags[0].message.contains("/home/y0usaf/Tokens"), "{}", diags[0].message);
}

#[test]
fn a_bare_string_default_is_flagged() {
    let diags = check(
        r#"{ lib, ... }: { options.x = lib.mkOption { default = "/root/key"; }; }"#,
    );
    assert_eq!(diags.len(), 1, "{diags:?}");
}

#[test]
fn interpolated_and_relative_defaults_are_silent() {
    for source in [
        r#"{ lib, ... }: { options.x = lib.mkOption { default = "${config.user.homeDirectory}/Tokens/k"; }; }"#,
        r#"{ lib, ... }: { options.x = lib.mkOption { default = "./k"; }; }"#,
        r#"{ lib, ... }: { options.x = lib.mkOption { default = "/nix/store/abc-k"; }; }"#,
        r#"{ lib, ... }: { options.x = lib.mkOption { default = 3; }; }"#,
    ] {
        assert!(check(source).is_empty(), "{source}");
    }
}

#[test]
fn a_string_outside_a_default_is_silent() {
    let diags = check(
        r#"{ lib, ... }: { options.x = lib.mkOption { description = "/home/y0usaf/x"; }; }"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn the_allow_setting_matches_by_prefix() {
    let config = LintConfig::default().with_setting(
        "machine-local-default",
        "allow",
        "/home/y0usaf/Tokens/",
    );
    let tree = parse(r#"{ lib, ... }: { options.x = lib.mkOption { default = "/home/y0usaf/Tokens/k"; }; }"#);
    let model = SemanticModel::new(r#"{ lib, ... }: { options.x = lib.mkOption { default = "/home/y0usaf/Tokens/k"; }; }"#, &tree);
    let mut diags = Vec::new();
    MachineLocalDefault {}.check_file(&model, &config, &mut diags);
    assert!(diags.is_empty(), "{diags:?}");
}
