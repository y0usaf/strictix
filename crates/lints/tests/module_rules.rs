use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::module_rules::{ConfigDependentImports, MixedModuleSyntax};
use strictix_syntax::parse;

fn check(source: &str, rule: impl Rule + 'static) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    run_rules(
        &[Box::new(rule)],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diags,
    );
    diags
}

#[test]
fn mixed_module_sections_flag_option_definitions() {
    for source in [
        "{ imports = []; config.services.ssh.enable = true; networking.hostName = \"box\"; }",
        "{ config, ... }: { config.services.ssh.enable = true; networking.hostName = \"box\"; }",
        "{ lib, ... }: { options.example = lib.mkOption {}; networking.hostName = \"box\"; }",
        "{ config, ... }: let name = \"box\"; in { config = {}; networking.hostName = name; }",
        "{ imports = []; \"config\" = {}; \"networking\".hostName = \"box\"; }",
        "{ imports = []; config = {}; inherit networking; }",
        "{ imports = []; config = {}; _module.args.foo = 1; }",
    ] {
        let diags = check(source, MixedModuleSyntax);
        assert_eq!(diags.len(), 1, "{source}: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Error);
        assert!(diags[0].fix.is_none());
    }
}

#[test]
fn mixed_module_syntax_allows_metadata_and_consistent_sections() {
    for source in [
        "{ imports = []; config = {}; options = {}; meta = {}; freeformType = null; _class = \"nixos\"; _file = \"module\"; key = \"module\"; disabledModules = []; }",
        "{ imports = []; services.ssh.enable = true; networking.hostName = \"box\"; }",
        "{ config, ... }: { config = { services.ssh.enable = true; networking.hostName = \"box\"; }; }",
        "{ lib, ... }: { options.foo = lib.mkOption {}; config.foo = true; }",
    ] {
        assert!(check(source, MixedModuleSyntax).is_empty(), "{source}");
    }
}

#[test]
fn mixed_module_syntax_skips_ambiguous_data_packages_and_nested_modules() {
    for source in [
        "{ config = {}; value = 1; }",
        "{ options = {}; value = 1; }",
        "{ lib, pkgs, ... }: { config = {}; pname = \"package\"; }",
        "{ lib, ... }: lib.makeDerivation { config = {}; value = 1; }",
        "{ nixpkgs, system }: import nixpkgs { inherit system; config = { allowUnfree = true; }; overlays = []; }",
        "{ imports = [ { config = {}; data = 1; } ]; config = {}; }",
        "{ config, ... }: value: { config = {}; value = value; }",
    ] {
        assert!(check(source, MixedModuleSyntax).is_empty(), "{source}");
    }
}

#[test]
fn imports_flag_config_demands_in_conditions_paths_and_aliases() {
    for source in [
        "{ config, ... }: { imports = if config.feature then [ ./feature.nix ] else []; }",
        "{ config, lib, ... }: { imports = lib.optionals config.feature [ ./feature.nix ]; }",
        "{ config, lib, ... }: { imports = lib.optional config.feature ./feature.nix; }",
        "{ config, ... }: { imports = [ config.module ]; }",
        "{ config, ... }: { imports = [ (./. + config.module) ]; }",
        "{ config, ... }: { imports = [ \"${config.module}\" ]; }",
        "{ config, ... }: let cfg = config.feature; in { imports = if cfg then [] else []; }",
        "{ config, lib, ... }: let cfg = config; in { imports = lib.optionals cfg.feature []; }",
        "{ config, lib, ... }: let choose = lib.optionals; cfg = config.feature; in { imports = choose cfg []; }",
        "{ config, lib, ... }: let inherit (lib) optionals; in { imports = optionals config.feature []; }",
        "{ config, lib, ... }: with lib; { imports = optionals config.feature []; }",
        "{ config, ... }: { imports = [] ++ (if config.feature then [] else []); }",
        "{ config, ... }: { imports = if true then [ config.module ] else []; }",
        "{ config, lib, ... }: { imports = lib.optionals true [ config.module ]; }",
        "{ config, ... }: { imports = if true && config.feature then [] else []; }",
    ] {
        let diags = check(source, ConfigDependentImports);
        assert_eq!(diags.len(), 1, "{source}: {diags:?}");
        assert_eq!(diags[0].severity, Severity::Warning);
        assert_eq!(&source[diags[0].range.start() as usize..diags[0].range.end() as usize], "config");
        assert!(diags[0].fix.is_none());
    }
}

#[test]
fn imports_preserve_lazy_module_definitions_and_unknown_calls() {
    for source in [
        "{ config, ... }: { imports = [ { services.foo.enable = config.feature; } ]; }",
        "{ config, ... }: { imports = [ ({ ... }: { services.foo.enable = config.feature; }) ]; }",
        "{ config, lib, ... }: { imports = [ (lib.mkIf config.feature { services.foo.enable = true; }) ]; }",
        "{ config, ... }: { imports = let unused = config.feature; in []; }",
        "{ config, ... }: let cfg = config.feature; in { imports = [ ./feature.nix ]; config.other = cfg; }",
        "{ config, ... }: let cycle = cycle; in { imports = cycle; }",
        "{ config, ... }: { imports = with config; []; }",
        "{ config, ... }: { imports = unknown config; }",
        "{ config, lib, ... }: { imports = lib.optional false config.module; }",
        "{ config, ... }: { imports = if false then [ config.module ] else []; }",
        "{ config, flag, ... }: { imports = if flag then [ config.module ] else []; }",
        "{ config, ... }: { imports = if false && config.feature then [] else []; }",
        "{ config, ... }: { imports = if true || config.feature then [] else []; }",
        "{ config, ... }: { imports = if [ config.feature ] == [] then [] else []; }",
        "{ config, lib, ... }: { imports = lib.optionals true [ { value = config.feature; } ]; }",
    ] {
        assert!(check(source, ConfigDependentImports).is_empty(), "{source}");
    }
}

#[test]
fn imports_respect_shadowing_special_arguments_and_forward_lets() {
    for source in [
        "{ config, lib, ... }: let config = { feature = true; }; in { imports = lib.optionals config.feature []; }",
        "{ config, lib, ... }: { imports = let result = lib.optionals config.feature []; config = { feature = true; }; in result; }",
        "{ config, lib, ... }: let lib = { optionals = a: b: b; }; in { imports = lib.optionals config.feature []; }",
        "{ config, lib, ... }: { imports = let result = lib.optionals config.feature []; lib = { optionals = a: b: b; }; in result; }",
        "{ config, lib, ... }: let optionals = a: b: b; in { imports = optionals config.feature []; }",
        "{ config, lib, ... }: with lib; let result = optionals config.feature []; optionals = a: b: b; in { imports = result; }",
        "{ config, other, ... }: { imports = other.optionals config.feature []; }",
        "{ config, lib, fallback, ... }: { imports = (lib.optionals or fallback) config.feature []; }",
        "{ config, lib, feature, ... }: { imports = lib.optionals feature [ ./feature.nix ]; }",
        "{ specialArgs, lib, ... }: { imports = lib.optionals specialArgs.feature [ ./feature.nix ]; }",
        "{ config, ... }: { imports = let true = false; in if true then [ config.module ] else []; }",
        "let config = { module = ./feature.nix; }; in { imports = [ config.module ]; }",
    ] {
        assert!(check(source, ConfigDependentImports).is_empty(), "{source}");
    }
}

#[test]
fn imports_respect_quoted_config_and_library_shadowing() {
    for source in [
        "{ config, lib, ... }: let \"config\" = { feature = false; }; in { imports = lib.optionals config.feature []; }",
        "{ config, lib, ... }: let \"config\" = { feature = false; }; cfg = config; in { imports = lib.optionals cfg.feature []; }",
        "{ config, lib, ... }: { imports = (rec { \"config\" = { feature = false; }; selected = lib.optionals config.feature []; }).selected; }",
        "{ config, lib, ... }: let \"lib\" = { optionals = _: modules: modules; }; in { imports = lib.optionals config.feature []; }",
        "{ config, lib, ... }: let \"lib\" = { optionals = _: modules: modules; }; choose = lib.optionals; in { imports = choose config.feature []; }",
    ] {
        assert!(check(source, ConfigDependentImports).is_empty(), "{source}");
    }
}
