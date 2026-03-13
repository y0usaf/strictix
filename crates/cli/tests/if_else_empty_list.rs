mod _utils;

use indoc::indoc;
use macros::generate_tests;

generate_tests! {
    rule: if_else_empty_list,
    expressions: [
        indoc! {"
            if config.foo.enable then [ foo bar ] else []
        "},
        indoc! {"
            if a && b then [ foo ] else []
        "},
        indoc! {"
            if cond then [
              foo
              bar
              baz
            ] else []
        "},
    ],
}
