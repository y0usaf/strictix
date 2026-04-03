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
    let stdout = _utils::test_cli(
        indoc! {r#"
            let
              a = "${config.user.homeDirectory}/Tokens/id_rsa_${config.user.name}";
              b = "${config.user.homeDirectory}/Tokens/id_ed25519_${config.user.name}";
            in
              [ a a b b ]
        "#},
        &["check"],
    )
    .unwrap();

    assert!(
        stdout.trim().is_empty(),
        "expected no diagnostics for repeated selects inside string interpolation, got:\n{stdout}"
    );
}
