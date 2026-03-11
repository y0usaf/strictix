mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: manual_inherit_from,
    expressions: [
        // depth 1 (original cases)
        "let a.b = 2; in { b = a.b; }",
        "let a.b = 2; in { c = a.c; }",
        "let a.b = 2; in { b = a.c; }",
        // depth 2: z = x.y.z -> inherit (x.y) z
        "let x.y.z = 2; in { z = x.y.z; }",
        // depth 3: w = x.y.z.w -> inherit (x.y.z) w
        "let x.y.z.w = 2; in { w = x.y.z.w; }",
        // no lint: key != last attr
        "let x.y.z = 2; in { a = x.y.z; }",
    ],
}
