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
