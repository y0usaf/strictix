use strictix_core::{
    config::LintConfig,
    diagnostic::Diagnostic,
    fix::apply_fixes,
    rules::{run_rules, Rule},
    semantic::SemanticModel,
};
use strictix_lints::simplification_rules::{
    BooleanIf, ConstantBooleanBinop, ConstantBooleanNot, EmptyAttrsetMerge, IdentityLambda,
    NegationSimplification, TrivialLet,
};
use strictix_syntax::parse;

fn run(source: &str, rule: Box<dyn Rule>) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diagnostics = Vec::new();
    run_rules(
        &[rule],
        &tree,
        &model,
        &LintConfig::default(),
        source,
        &mut diagnostics,
    );
    diagnostics
}

/// Run a rule expecting exactly one diagnostic with a fix, and return
/// the fixed source.
fn fixed(source: &str, rule: Box<dyn Rule>) -> String {
    let diagnostics = run(source, rule);
    assert_eq!(diagnostics.len(), 1, "one diagnostic for {source}");
    let fix = diagnostics[0].fix.as_ref().expect("fix present");
    apply_fixes(source, &fix.edits).expect("fix applies")
}

// --- constant-boolean-not -------------------------------------------

#[test]
fn constant_boolean_not_reduces_literals() {
    assert_eq!(fixed("!true", Box::new(ConstantBooleanNot {})), "false");
    assert_eq!(fixed("!(false)", Box::new(ConstantBooleanNot {})), "true");
    assert!(run("!x", Box::new(ConstantBooleanNot {})).is_empty());
}

// --- constant-boolean-binop -----------------------------------------

#[test]
fn constant_boolean_binop_reduces_boolean_literals() {
    for (source, expected) in [
        ("true && false", "false"),
        ("false && true", "false"),
        ("true || false", "true"),
        ("false || false", "false"),
    ] {
        assert_eq!(fixed(source, Box::new(ConstantBooleanBinop {})), expected);
    }
    assert!(run("true && x", Box::new(ConstantBooleanBinop {})).is_empty());
}

// --- identity-lambda -------------------------------------------------

#[test]
fn identity_lambda_is_diagnostic_only() {
    let diagnostics = run("x: x", Box::new(IdentityLambda {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "identity-lambda");
    assert!(diagnostics[0].fix.is_none());
    assert!(run("x: y", Box::new(IdentityLambda {})).is_empty());
    assert!(run("{ x }: x", Box::new(IdentityLambda {})).is_empty());
}

// --- empty-attrset-merge --------------------------------------------

#[test]
fn empty_attrset_merge_removes_empty_operand() {
    assert_eq!(
        fixed("{} // attrs", Box::new(EmptyAttrsetMerge {})),
        "attrs"
    );
    assert_eq!(
        fixed("attrs // {}", Box::new(EmptyAttrsetMerge {})),
        "attrs"
    );
    assert_eq!(
        fixed("{} // (a // b)", Box::new(EmptyAttrsetMerge {})),
        "(a // b)"
    );
    assert!(run("a // b", Box::new(EmptyAttrsetMerge {})).is_empty());
}

// --- boolean-if ------------------------------------------------------

#[test]
fn boolean_if_literal_branches_reduce_to_condition() {
    assert_eq!(
        fixed("if c then true else false", Box::new(BooleanIf {})),
        "c"
    );
    // A non-atom condition is parenthesized so the spliced text cannot
    // re-associate with surrounding operators.
    assert_eq!(
        fixed("if a == b then true else false", Box::new(BooleanIf {})),
        "(a == b)"
    );
}

#[test]
fn boolean_if_flipped_literal_branches_negate_condition() {
    assert_eq!(
        fixed("if c then false else true", Box::new(BooleanIf {})),
        "!c"
    );
    assert_eq!(
        fixed("if a == b then false else true", Box::new(BooleanIf {})),
        "!(a == b)"
    );
}

#[test]
fn boolean_if_mixed_branches_use_boolean_operators() {
    assert_eq!(
        fixed("if c then true else a == b", Box::new(BooleanIf {})),
        "c || (a == b)"
    );
    assert_eq!(
        fixed("if c then a == b else false", Box::new(BooleanIf {})),
        "c && (a == b)"
    );
    assert_eq!(
        fixed("if c then false else a == b", Box::new(BooleanIf {})),
        "!c && (a == b)"
    );
    assert_eq!(
        fixed("if c then a == b else true", Box::new(BooleanIf {})),
        "!c || (a == b)"
    );
}

#[test]
fn boolean_if_fix_splices_inside_surrounding_code() {
    assert_eq!(
        fixed("x: if c then true else false", Box::new(BooleanIf {})),
        "x: c"
    );
}

#[test]
fn boolean_if_skips_branch_of_unproven_type() {
    // `x` could evaluate to anything; `c || x` would force it to a
    // boolean where the original if accepted any type in that branch.
    assert!(run("if c then true else x", Box::new(BooleanIf {})).is_empty());
    assert!(run("if c then x else false", Box::new(BooleanIf {})).is_empty());
}

#[test]
fn boolean_if_skips_literal_condition_owned_by_constant_if() {
    // constant-if owns literal conditions; a second fix here would
    // overlap with its branch-inlining fix.
    assert!(run("if true then true else false", Box::new(BooleanIf {})).is_empty());
    assert!(run("if (false) then true else false", Box::new(BooleanIf {})).is_empty());
}

#[test]
fn boolean_if_flags_identical_atom_branches_without_fix() {
    for source in [
        "if c then x else x",
        "if c then 1 else 1",
        "if c then true else true",
    ] {
        let diagnostics = run(source, Box::new(BooleanIf {}));
        assert_eq!(diagnostics.len(), 1, "{source}");
        assert_eq!(diagnostics[0].code, "boolean-if");
        assert_eq!(diagnostics[0].message, "both branches are identical");
        // No fix: inlining a branch would skip forcing the condition,
        // changing error/divergence behavior.
        assert!(diagnostics[0].fix.is_none(), "{source}");
    }
}

#[test]
fn boolean_if_identical_check_requires_same_kind_atoms() {
    // Different texts, different kinds (1 vs 1.0), and non-atom
    // branches all stay silent.
    for source in [
        "if c then x else y",
        "if c then 1 else 1.0",
        "if c then f x else f x",
    ] {
        assert!(run(source, Box::new(BooleanIf {})).is_empty(), "{source}");
    }
}

// --- negation-simplification -----------------------------------------

#[test]
fn negation_simplification_cancels_double_negation() {
    assert_eq!(fixed("!(!x)", Box::new(NegationSimplification {})), "x");
    let diagnostics = run("!(!x)", Box::new(NegationSimplification {}));
    assert_eq!(diagnostics[0].code, "negation-simplification");
    assert_eq!(diagnostics[0].message, "double negation cancels out");
}

#[test]
fn negation_simplification_flips_every_comparison() {
    // The flipped comparison keeps its parentheses: without a parent
    // pointer the rule cannot prove the negation is not itself a binop
    // operand, and `(a != b)` is precedence-inert everywhere `!(…)`
    // was legal.
    for (source, expected) in [
        ("!(a == b)", "(a != b)"),
        ("!(a != b)", "(a == b)"),
        ("!(a < b)", "(a >= b)"),
        ("!(a > b)", "(a <= b)"),
        ("!(a <= b)", "(a > b)"),
        ("!(a >= b)", "(a < b)"),
    ] {
        assert_eq!(
            fixed(source, Box::new(NegationSimplification {})),
            expected,
            "{source}"
        );
    }
}

#[test]
fn negation_simplification_fix_splices_inside_surrounding_code() {
    assert_eq!(
        fixed("!(a == b) && c", Box::new(NegationSimplification {})),
        "(a != b) && c"
    );
    // Non-atom comparison operands travel verbatim.
    assert_eq!(
        fixed("!(f x == g y)", Box::new(NegationSimplification {})),
        "(f x != g y)"
    );
}

#[test]
fn negation_simplification_skips_plain_negation_and_other_operators() {
    // `!x` is already minimal; `&&`/`||`/arithmetic under `!` have no
    // single flipped operator.
    for source in ["!x", "!(a && b)", "!(a || b)", "!(a + b)"] {
        assert!(
            run(source, Box::new(NegationSimplification {})).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn negation_simplification_skips_tautology_owned_operands() {
    // `x == x` is tautology's finding; two fixes on the same text
    // would overlap.
    for source in ["!(x == x)", "!(1 == 1)", "!(x < x)"] {
        assert!(
            run(source, Box::new(NegationSimplification {})).is_empty(),
            "{source}"
        );
    }
}

// --- trivial-let ------------------------------------------------------

#[test]
fn trivial_let_reduces_to_bound_value() {
    let diagnostics = run("let x = 1; in x", Box::new(TrivialLet {}));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "trivial-let");
    assert_eq!(diagnostics[0].message, "let binds 'x' only to return it");
    assert_eq!(fixed("let x = 1; in x", Box::new(TrivialLet {})), "1");
    // Compound values splice verbatim, trivia-trimmed.
    assert_eq!(fixed("let x = f y; in x", Box::new(TrivialLet {})), "f y");
}

#[test]
fn trivial_let_fix_splices_inside_surrounding_code() {
    assert_eq!(
        fixed("{ a = let x = 1; in x; }", Box::new(TrivialLet {})),
        "{ a = 1; }"
    );
}

#[test]
fn trivial_let_skips_multiple_bindings_and_inherits() {
    for source in [
        "let x = 1; y = 2; in x",
        "let x = 1; inherit y; in x",
        "let inherit x; in x",
    ] {
        assert!(run(source, Box::new(TrivialLet {})).is_empty(), "{source}");
    }
}

#[test]
fn trivial_let_skips_dotted_attrpath() {
    // `x.y = 1` builds a nested attrset under x; the body returns that
    // attrset, not the bound value.
    assert!(run("let x.y = 1; in x", Box::new(TrivialLet {})).is_empty());
}

#[test]
fn trivial_let_skips_body_naming_something_else() {
    assert!(run("let x = 1; in y", Box::new(TrivialLet {})).is_empty());
}

#[test]
fn trivial_let_skips_self_referential_values() {
    // Choice: `let x = x; in x` does NOT fire. Let bindings are
    // recursive, so the value's `x` resolves to the binding itself;
    // splicing the value text into the enclosing scope would rebind it
    // (or unbind it entirely). The same holds for any value mentioning
    // its own name, e.g. `let f = g f; in f`.
    assert!(run("let x = x; in x", Box::new(TrivialLet {})).is_empty());
    assert!(run("let f = g f; in f", Box::new(TrivialLet {})).is_empty());
}
