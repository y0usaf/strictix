mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: unused_lambda_param,
    expressions: [
        // match
        "x: 42",
        "drv: { name = \"demo\"; }",

        // don't match
        "x: x",
        "x: let y = 1; in x + y",
        "_: 42",
        "{ x, ... }: 42",
    ],
}
