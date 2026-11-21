use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::deprecated_package_alias::DeprecatedPackageAlias;
use strictix_syntax::parse;

fn run(source: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();
    DeprecatedPackageAlias.check_file(&model, config, &mut diags);
    let mut through = Vec::new();
    run_rules(
        &[Box::new(DeprecatedPackageAlias)],
        &tree,
        &model,
        config,
        source,
        &mut through,
    );
    assert_eq!(diags.len(), through.len());
    diags
}

fn alone() -> LintConfig {
    LintConfig::default().with_disabled(["unsafe-with-shadowing".to_owned()])
}

fn fixed(source: &str) -> String {
    let diags = run(source, &alone());
    let edits: Vec<_> = diags
        .iter()
        .flat_map(|d| d.fix.as_ref().expect("fix").edits.clone())
        .collect();
    let out = apply_fixes(source, &edits).unwrap();
    assert!(run(&out, &alone()).is_empty(), "{out}");
    out
}

#[test]
fn xorg_members_under_one_with_become_top_level_names() {
    let source = "{pkgs, ...}: {
  buildInputs = with pkgs; [
    xorg.libX11 xorg.libxcb
    xorg.xcbutilwm
    xorg.libXext.dev
  ];
}";
    let diags = run(source, &LintConfig::default());
    assert_eq!(diags.len(), 4, "{diags:?}");
    assert_eq!(
        diags[0].message,
        "`xorg.libX11` is a deprecated nixpkgs alias of `libx11`"
    );
    assert_eq!(
        fixed(source),
        "{pkgs, ...}: {
  buildInputs = with pkgs; [
    libx11 libxcb
    libxcb-wm
    libxext.dev
  ];
}"
    );
}

#[test]
fn selections_keep_their_base() {
    let source = "{pkgs, final, inputs, system, ...}: [
  pkgs.xorg.libX11
  final.xorg.xcbutilwm.dev
  inputs.nixpkgs.legacyPackages.${system}.xorg.libXext
  inputs.nixpkgs.legacyPackages.x86_64-linux.swww
  (pkgs.xorg.libXi or null)
  pkgs.swww
]";
    assert_eq!(
        fixed(source),
        "{pkgs, final, inputs, system, ...}: [
  pkgs.libx11
  final.libxcb-wm.dev
  inputs.nixpkgs.legacyPackages.${system}.libxext
  inputs.nixpkgs.legacyPackages.x86_64-linux.awww
  (pkgs.libxi or null)
  pkgs.awww
]"
    );
}

#[test]
fn a_receiver_not_known_as_nixpkgs_gets_no_fix() {
    let source = "{pkgs, custom, ...}: [
  (custom: custom.xorg.libX11)
  custom.xorg.libX11
  pkgs.pkgsCross.aarch64-multiplatform.xorg.libXext
  (import ./x.nix).xorg.libX11
  (with custom; xorg.libX11)
  (with custom.xorg; libX11)
  (with custom; with xorg; libX11)
  custom.swww
  (with custom; swww)
]";
    for config in [LintConfig::default(), alone()] {
        let diags = run(source, &config);
        assert_eq!(diags.len(), 7, "{diags:?}");
        assert!(diags.iter().all(|d| d.fix.is_none()), "{diags:?}");
        assert!(diags[0]
            .help
            .as_deref()
            .is_some_and(|help| help.starts_with("`custom` is not known to be nixpkgs")));
    }
}

#[test]
fn naming_a_receiver_in_sets_lets_fix_rewrite_it() {
    let source = "{custom, ...}: [ custom.xorg.libX11 (custom.swww) (with custom; xorg.libXext) ]";
    let config = alone().with_setting("deprecated-package-alias", "sets", "custom");
    let diags = run(source, &config);
    let edits: Vec<_> = diags
        .iter()
        .flat_map(|d| d.fix.as_ref().expect("fix").edits.clone())
        .collect();
    assert_eq!(
        apply_fixes(source, &edits).unwrap(),
        "{custom, ...}: [ custom.libx11 (custom.awww) (with custom; libxext) ]"
    );
}

#[test]
fn with_the_xorg_set_selects_from_its_parent() {
    let source = "{pkgs, ...}: with pkgs.xorg; [ libX11 libXext.dev ]";
    assert_eq!(
        fixed(source),
        "{pkgs, ...}: with pkgs.xorg; [ pkgs.libx11 pkgs.libxext.dev ]"
    );
    let legacy = "{inputs, ...}: with inputs.nixpkgs.legacyPackages.x86_64-linux.xorg; [ libX11 ]";
    assert_eq!(
        fixed(legacy),
        "{inputs, ...}: with inputs.nixpkgs.legacyPackages.x86_64-linux.xorg; [ inputs.nixpkgs.legacyPackages.x86_64-linux.libx11 ]"
    );
}

#[test]
fn one_segment_aliases_need_a_package_set() {
    let source = "{config, pkgs, inputs, ...}: [
  config.services.swww
  inputs.swww
  (with config; swww)
  (with pkgs; swww)
]";
    let diags = run(source, &LintConfig::default());
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(
        fixed(source),
        "{config, pkgs, inputs, ...}: [
  config.services.swww
  inputs.swww
  (with config; swww)
  (with pkgs; awww)
]"
    );
    let custom = alone().with_setting("deprecated-package-alias", "sets", "inputs");
    assert_eq!(run(source, &custom).len(), 1);
}

#[test]
fn unclear_resolution_gets_help_only() {
    let source = "{lib, pkgs, xorg, ...}: [
  xorg.libX11
  (with pkgs; with lib; xorg.libXext)
  (let libxi = 1; in with pkgs; xorg.libXi)
  (with pkgs; with xorg; libXtst)
]";
    let diags = run(source, &LintConfig::default());
    assert_eq!(diags.len(), 4, "{diags:?}");
    assert!(diags.iter().all(|d| d.fix.is_none()), "{diags:?}");
    assert!(diags.iter().all(|d| d.help.is_some()));
}

#[test]
fn inner_with_of_a_package_set_is_unambiguous() {
    let source = "{lib, pkgs, ...}: with lib; with pkgs; [ xorg.libX11 ]";
    assert_eq!(
        fixed(source),
        "{lib, pkgs, ...}: with lib; with pkgs; [ libx11 ]"
    );
}

#[test]
fn current_names_and_other_paths_are_left_alone() {
    let source = "{pkgs, ...}: {
  xorg.libX11 = 1;
  a = pkgs.libx11;
  b = pkgs ? xorg.libX11;
  c = with pkgs; [ libx11 xorg ];
  inherit (pkgs.xorg) libX11;
}";
    assert!(run(source, &LintConfig::default()).is_empty());
}

#[test]
fn a_with_that_unsafe_with_shadowing_removes_is_renamed_on_the_next_pass() {
    for (source, expected) in [
        (
            "{pkgs, ...}: with pkgs.xorg; [ libX11 libXext ]",
            "{pkgs, ...}: [ pkgs.libx11 pkgs.libxext ]",
        ),
        (
            "{pkgs, ...}: with pkgs; [ xorg.libX11 swww ]",
            "{pkgs, ...}: [ pkgs.libx11 pkgs.awww ]",
        ),
    ] {
        let diags = run(source, &LintConfig::default());
        assert!(!diags.is_empty());
        assert!(diags.iter().all(|d| d.fix.is_none()), "{diags:?}");
        let run = strictix_core::rules::lint(
            &strictix_lints::all_rules(),
            source,
            None,
            &LintConfig::default(),
            true,
        );
        assert!(run.error.is_none(), "{:?}", run.error);
        assert_eq!(run.fixed.as_deref(), Some(expected));
    }
}

#[test]
fn a_parenthesized_receiver_keeps_its_parentheses() {
    let source = "pkgs: with pkgs; [ (xorg).libX11 (pkgs).xorg.libXext ]";
    let out = fixed(source);
    assert_eq!(out, "pkgs: with pkgs; [ (xorg).libX11 (pkgs).libxext ]");
    assert!(strictix_syntax::parse(&out).error_nodes().next().is_none());
    let diags = run(source, &alone());
    assert_eq!(diags.len(), 1, "{diags:?}");
}
