mod _utils;

use indoc::indoc;

use macros::generate_tests;

generate_tests! {
    rule: empty_let_in,
    expressions: [
        "let in null",
        indoc! {"
            let
              # preserve the comment while fixing
            in
            null
        "},
        indoc! {"
            let # keep trailing comment
            in
              null
        "},
    ],
}
