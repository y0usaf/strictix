mod _utils;

use indoc::indoc;

use macros::generate_tests;

generate_tests! {
    rule: repeated_expression,
    expressions: [
        // 3-part common prefix: pkgs.hello.meta is repeated
        indoc! {"
            let
              a = pkgs.hello.meta.description;
              b = pkgs.hello.meta.license;
            in
              null
        "},
        // three occurrences of the same 4-part select
        indoc! {"
            let
              a = nixpkgs.lib.types.str;
              b = nixpkgs.lib.types.str;
              c = nixpkgs.lib.types.str;
            in
              null
        "},
        // repeat in body only (no bindings reference it) - 3-part prefix
        indoc! {"
            let
              x = 1;
            in
              pkgs.stdenv.mkDerivation { } // pkgs.stdenv.mkDerivation { }
        "},
    ],
}

#[test]
fn repeated_expression_ignores_selects_inside_string_interpolation() {
    _utils::assert_check_clean(indoc! {r#"
        let
          a = "${config.user.homeDirectory}/Tokens/id_rsa_${config.user.name}";
          b = "${config.user.homeDirectory}/Tokens/id_ed25519_${config.user.name}";
        in
          [ a a b b ]
    "#})
    .expect("CLI 'check' should succeed for selects inside string interpolation");
}

#[test]
fn repeated_expression_ignores_dots_inside_application_arguments() {
    _utils::assert_check_clean(indoc! {r#"
        let
          a = builtins.fetchTarball {
            url = "https://api.github.com/repos/foo/bar/tarball/v1";
            sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
          };
          b = builtins.fetchTarball {
            url = "https://api.github.com/repos/baz/qux/tarball/v2";
            sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
          };
        in
          [ a b ]
    "#})
    .expect("CLI 'check' should succeed when dots only repeat inside application arguments");
}
