mod _utils;

use indoc::indoc;
use macros::generate_tests;

generate_tests! {
    rule: collapsible_inherit_from,
    expressions: [
        indoc! {"
            {
              inherit (spec) command;
              inherit (spec) args;
            }
        "},
    ],
}
