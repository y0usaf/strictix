mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: empty_attrset_merge,
    expressions: [
        "{} // { a = 1; }",
        "{ a = 1; } // {}",
    ],
}
