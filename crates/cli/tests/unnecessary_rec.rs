mod _utils;

use indoc::indoc;
use macros::generate_tests;

generate_tests! {
    rule: unnecessary_rec,
    expressions: [
        indoc! {"
            rec {
              a = 1;
              b = 2;
            }
        "},
    ],
}
