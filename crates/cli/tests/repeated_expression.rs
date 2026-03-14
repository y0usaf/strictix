mod _utils;

use indoc::indoc;

use macros::generate_tests;

generate_tests! {
    rule: repeated_expression,
    expressions: [
        // same select appears in two binding values
        indoc! {"
            let
              a = pkgs.hello.meta.description;
              b = pkgs.hello.bin;
            in
              { inherit a b; }
        "},
        // same select appears in a binding value and in the body
        indoc! {"
            let
              a = pkgs.hello.meta;
            in
              pkgs.hello.bin
        "},
        // longer common prefix: pkgs.hello.meta is the maximal repeat,
        // pkgs.hello should NOT be separately reported
        indoc! {"
            let
              a = pkgs.hello.meta.description;
              b = pkgs.hello.meta.license;
            in
              null
        "},
        // three occurrences of the same select
        indoc! {"
            let
              a = nixpkgs.lib.types.str;
              b = nixpkgs.lib.types.str;
              c = nixpkgs.lib.types.str;
            in
              null
        "},
        // repeat in body only (no bindings reference it)
        indoc! {"
            let
              x = 1;
            in
              pkgs.stdenv.mkDerivation { } // pkgs.stdenv.mkDerivation { }
        "},
    ],
}
