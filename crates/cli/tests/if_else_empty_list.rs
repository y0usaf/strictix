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

#[test]
fn no_lint_non_empty_else() -> anyhow::Result<()> {
    _utils::assert_check_clean("if cond then [ foo ] else [ bar ]")?;
    Ok(())
}

#[test]
fn no_lint_then_not_a_list() -> anyhow::Result<()> {
    _utils::assert_check_clean("if cond then foo else []")?;
    Ok(())
}

#[test]
fn no_lint_then_empty() -> anyhow::Result<()> {
    _utils::assert_check_clean("if cond then [] else [ foo ]")?;
    Ok(())
}

#[test]
fn no_lint_both_empty() -> anyhow::Result<()> {
    _utils::assert_check_clean("if cond then [] else []")?;
    Ok(())
}
