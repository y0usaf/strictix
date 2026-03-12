mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: with_expression,
    expressions: [
        "with pkgs; [ git curl ]",
    ],
}
