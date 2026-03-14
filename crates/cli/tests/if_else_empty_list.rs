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
    let stdout = _utils::test_cli("if cond then [ foo ] else [ bar ]", &["check"])?;
    assert!(
        stdout.trim().is_empty(),
        "should not lint non-empty else: {stdout}"
    );
    Ok(())
}

#[test]
fn no_lint_then_not_a_list() -> anyhow::Result<()> {
    let stdout = _utils::test_cli("if cond then foo else []", &["check"])?;
    assert!(
        stdout.trim().is_empty(),
        "should not lint when then-body is not a list: {stdout}"
    );
    Ok(())
}

#[test]
fn no_lint_then_empty() -> anyhow::Result<()> {
    let stdout = _utils::test_cli("if cond then [] else [ foo ]", &["check"])?;
    assert!(
        stdout.trim().is_empty(),
        "should not lint when then-body is empty: {stdout}"
    );
    Ok(())
}

#[test]
fn no_lint_both_empty() -> anyhow::Result<()> {
    let stdout = _utils::test_cli("if cond then [] else []", &["check"])?;
    assert!(
        stdout.trim().is_empty(),
        "should not lint when both branches are empty: {stdout}"
    );
    Ok(())
}
