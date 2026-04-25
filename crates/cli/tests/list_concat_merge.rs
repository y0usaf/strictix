mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: list_concat_merge,
    expressions: [
        // adjacent list literals
        "[ a b ] ++ [ c d ]",

        // adjacent trailing lists after a dynamic prefix
        "base ++ [ a ] ++ [ b c ]",

        // adjacent leading lists before a dynamic suffix
        "[ a ] ++ [ b c ] ++ tail",

        // adjacent middle lists keep surrounding expressions in place
        "base ++ [ a b ] ++ [ c d ] ++ tail",

        // multiline source is still rewritten safely
        r"
        base
        ++ [ a b ]
        ++ [ c d ]
        ++ tail
        ",
    ],
}

#[test]
fn separated_unconditional_lists_are_not_reordered_across_optional() -> anyhow::Result<()> {
    _utils::assert_check_clean(
        r#"
        [ "source aliases" ]
        ++ lib.optional cfg.enable "source optional"
        ++ [ pluginSettings historySettings ]
        "#,
    )
}
