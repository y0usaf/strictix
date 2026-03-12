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
        // bare inherit — fix: replace `inherit x;` with `x = 1;`
        indoc! {"
            let
              x = 1;
            in
              { inherit x; }
        "},
        // string interpolation — fix: replace `${dev}` with the literal content
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
        // multiline binding used once in body — fix should still inline
        indoc! {"
            let
              gpuArgs =
                optionals (disableFeatures != []) [
                  \"--disable-features=${concatStringsSep \",\" disableFeatures}\"
                ]
                ++ optionals (!smoothScroll) [
                  \"--disable-smooth-scrolling\"
                ];
            in
              concatStringsSep \" \" (gpuArgs ++ extraArgs)
        "},
        // multiline binding used in bare inherit — fix should rewrite to assignment
        indoc! {"
            let
              settings =
                {
                  a = 1;
                  b = 2;
                };
            in
              { inherit settings; }
        "},
        // binding referenced from a sibling `inherit (x) ...;` entry — should NOT be flagged
        indoc! {"
            let
              data = { version = 5; pins = {}; };
              inherit (data) version;
            in
              if version == 5 then data.pins else {}
        "},
        // multi-attr inherit — fix only the targeted attr and preserve the others
        indoc! {"
            let
              x = 1;
            in
              { inherit x y; }
        "},
        // string interpol with non-string value — diagnostic only
        indoc! {"
            let
              x = someExpr;
            in
              \"value: ${x}\"
        "},
        // single use in sibling binding — fix inlines before removing the source binding
        indoc! {"
            let
              base = pkgs.hello;
              wrapped = base.override { };
            in
              wrapped
        "},
        // recursive self-reference but no external uses — safe to drop as dead code
        indoc! {"
            let
              loop = loop;
            in
              1
        "},
    ],
}
