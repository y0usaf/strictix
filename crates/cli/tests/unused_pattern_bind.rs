mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: unused_pattern_bind,
    expressions: [
        // match
        "args @ { x, y }: x + y",
        "attrs @ { x, ... }: x",

        // don't match
        "args @ { x, y }: args.x + y",
        "{ x, y }: x + y",
        "args @ { x ? 1, y }: args.x + y",
    ],
}
