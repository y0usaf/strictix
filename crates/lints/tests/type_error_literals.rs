use strictix_core::rules::Rule;
use strictix_core::{config::LintConfig, rules::lint};
use strictix_lints::{
    ill_typed_unary_op::IllTypedUnaryOp, non_callable_application::NonCallableApplication,
};

fn run(source: &str, rule: Box<dyn Rule>) -> Vec<String> {
    lint(&[rule], source, None, &LintConfig::default(), false)
        .diagnostics
        .into_iter()
        .map(|d| d.code.to_owned())
        .collect()
}

#[test]
fn literal_application_is_non_callable() {
    assert_eq!(
        run("1 2", Box::new(NonCallableApplication)),
        ["non-callable-application"]
    );
    assert_eq!(
        run("\"x\" 2", Box::new(NonCallableApplication)),
        ["non-callable-application"]
    );
    assert_eq!(
        run("[ 1 ] 2", Box::new(NonCallableApplication)),
        ["non-callable-application"]
    );
    assert_eq!(
        run("{} 2", Box::new(NonCallableApplication)),
        ["non-callable-application"]
    );
}

#[test]
fn callable_or_unknown_application_is_skipped() {
    assert!(run("{ __functor = f; } 2", Box::new(NonCallableApplication)).is_empty());
    assert!(run("f 2", Box::new(NonCallableApplication)).is_empty());
    assert!(run("(f x) 2", Box::new(NonCallableApplication)).is_empty());
}

#[test]
fn unary_literals_are_checked() {
    assert_eq!(
        run("! 1", Box::new(IllTypedUnaryOp)),
        ["ill-typed-unary-op"]
    );
    assert_eq!(
        run("- \"x\"", Box::new(IllTypedUnaryOp)),
        ["ill-typed-unary-op"]
    );
    assert!(run("- 1", Box::new(IllTypedUnaryOp)).is_empty());
    assert!(run("! true", Box::new(IllTypedUnaryOp)).is_empty());
}

#[test]
fn unary_unknown_and_compound_are_skipped() {
    assert!(run("! x", Box::new(IllTypedUnaryOp)).is_empty());
    assert!(run("- (x + 1)", Box::new(IllTypedUnaryOp)).is_empty());
    assert!(run("! (f x)", Box::new(IllTypedUnaryOp)).is_empty());
}
