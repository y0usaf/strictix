mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: redundant_if_bool,
    expressions: [
        "if cond then true else false",
        "if cond then false else true",
    ],
}
