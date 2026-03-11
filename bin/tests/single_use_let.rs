mod _utils;

use indoc::indoc;

use macros::generate_tests;

generate_tests! {
    rule: single_use_let,
    expressions: [
        // binding used exactly once in body — should be flagged
        indoc! {"
            let
              x = pkgs.hello;
            in
              x.meta.description
        "},
        // binding never used — should be flagged
        indoc! {"
            let
              x = 1;
            in
              null
        "},
        // both bindings single-use — both flagged
        indoc! {"
            let
              a = 1;
              b = 2;
            in
              a + b
        "},
        // binding used twice in body — should NOT be flagged
        indoc! {"
            let
              x = pkgs.hello;
            in
              x.name + x.version
        "},
        // binding used in bare inherit — diagnostic only, no auto-fix
        indoc! {"
            let
              x = 1;
            in
              { inherit x; }
        "},
        // binding used in string interpolation — diagnostic only, no auto-fix
        indoc! {"
            let
              dev = \"/dev/sda\";
            in
              ''mount ${dev} /mnt''
        "},
        // binding used as inherit-from source — fix without double parens
        indoc! {"
            let
              attrs = { a = 1; };
            in
              { inherit (attrs) a; }
        "},
    ],
}
