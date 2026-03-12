mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: with_expression,
    expressions: [
        "with pkgs; [ git curl ]",
        "with { git = pkgs.git; curl = pkgs.curl; }; [ git curl ]",
        "with { hello = pkgs.hello; }; 1",
    ],
}
