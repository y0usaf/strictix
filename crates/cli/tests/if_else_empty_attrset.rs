mod _utils;

use indoc::indoc;
use macros::generate_tests;

generate_tests! {
    rule: if_else_empty_attrset,
    expressions: [
        indoc! {"
            base // (if config.foo.enable then { bar = 1; } else {})
        "},
    ],
}
