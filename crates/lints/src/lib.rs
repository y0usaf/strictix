//! Builtin lint rules — the first customer of strictix-core's public API.
//!
//! Every rule here is declared via the `rules!` macro — the one
//! registry mechanism — and implements the `Rule` trait. Node rules
//! (syntax-level) live in [node_rules], file rules (semantic) in
//! [file_rules], and the options-schema rule in [schema].

pub mod advanced_rules;
pub mod bare_import_in_list;
pub mod call_simplification;
pub mod duplicate_attr;
pub mod duplicate_formal;
pub mod duplicate_inherit;
pub mod file_rules;
pub mod ill_typed_binop;
pub mod ill_typed_unary_op;
pub mod interpolation_rules;
pub mod lib_type_rules;
pub mod node_rules;
pub mod non_boolean_condition;
pub mod non_callable_application;
pub mod path_rules;
pub mod reference_rules;
pub mod schema;
pub mod simplification_rules;
pub mod style_rules;
pub mod suggested_rules;

use advanced_rules::{
    BuiltinArity, DuplicateFunctionArgument, DynamicImport, SuspiciousImportArgument,
    SuspiciousRecursion, UnreachableBranch, UnsafeWithShadowing,
};
use bare_import_in_list::BareImportInList;
use call_simplification::{
    DeprecatedIsNull, ManualGetattr, ManualHasattr, ManualOptional, OptionalListArgument,
};
use duplicate_attr::DuplicateAttribute;
use duplicate_formal::DuplicateFormal;
use duplicate_inherit::DuplicateInherit;
use file_rules::{
    CircularLet, CyclomaticComplexity, ImportCycle, MissingImport, ReboundConstant, RedundantWith,
    SelfReferentialLet, ShadowedBinding, ShadowedFormal, UnnecessaryOr, UnusedFormal,
    UnusedInherit, UnusedLambdaParam, UnusedLetBinding,
};
use ill_typed_binop::IllTypedBinop;
use ill_typed_unary_op::IllTypedUnaryOp;
use interpolation_rules::{CoercedInterpolation, RedundantInterpolation};
use lib_type_rules::UnknownLibType;
use node_rules::{AssertTrue, ConstantIf, ConstantIfBranches, Tautology};
use non_boolean_condition::NonBooleanCondition;
use non_callable_application::NonCallableApplication;
use path_rules::{AccidentalPathDivision, DanglingPath, SearchPathReference};
use reference_rules::{UndefinedVariable, UnknownBuiltin};
use schema::{OptionTypeMismatch, UnknownOption};
use simplification_rules::{
    BooleanIf, ConstantBooleanBinop, ConstantBooleanNot, EmptyAttrsetMerge, IdentityLambda,
    NegationSimplification, TrivialLet,
};
use strictix_core::rules::Rule;
use style_rules::{
    CollapsibleLetIn, DeprecatedToPath, EmptyInherit, EmptyLetIn, EmptyListConcat, EmptyPattern,
    EtaReduction, ManualInherit, ManualInheritFrom, RedundantPatternBind, RepeatedKeys,
    SingletonListConcat, UnquotedUri, UselessHasAttr, UselessParens,
};
use suggested_rules::{
    AssertFalse, DuplicateLiteralListItem, LiteralDivisionByZero, RedundantBooleanComparison,
    UnnecessaryRec, UnusedRecBinding,
};

/// The full builtin registry, in declaration order.
///
/// Called once per run by the CLI; the boxed rules are shared across all
/// worker threads (rules are `Sync`; the schema rule caches its
/// parsed options.json in a `OnceLock`).
#[must_use]
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    strictix_core::rules! {
        BuiltinArity,
        DuplicateFunctionArgument,
        DynamicImport,
        SuspiciousImportArgument,
        SuspiciousRecursion,
        UnreachableBranch,
        UnsafeWithShadowing,
        ConstantIf,
        ConstantIfBranches,
        AssertTrue,
        Tautology,
        UnusedLetBinding,
        UnusedLambdaParam,
        UnusedFormal,
        UnusedInherit,
        ShadowedBinding,
        ShadowedFormal,
        UnnecessaryOr,
        RedundantWith,
        SelfReferentialLet,
        UndefinedVariable,
        UnknownBuiltin,
        UnknownOption,
        EmptyLetIn,
        ManualInherit,
        ManualInheritFrom,
        CollapsibleLetIn,
        EtaReduction,
        EmptyPattern,
        RedundantPatternBind,
        EmptyInherit,
        DeprecatedToPath,
        UselessHasAttr,
        EmptyListConcat,
        SingletonListConcat,
        UselessParens,
        RepeatedKeys,
        UnquotedUri,
        DuplicateAttribute,
        DuplicateFormal,
        DuplicateInherit,
        NonBooleanCondition,
        IllTypedBinop,
        IllTypedUnaryOp,
        NonCallableApplication,
        MissingImport,
        ImportCycle,
        BareImportInList,
        UnnecessaryRec,
        UnusedRecBinding,
        AssertFalse,
        LiteralDivisionByZero,
        RedundantBooleanComparison,
        DuplicateLiteralListItem,
        CircularLet,
        CyclomaticComplexity,
        ReboundConstant,
        CoercedInterpolation,
        RedundantInterpolation,
        DanglingPath,
        AccidentalPathDivision,
        SearchPathReference,
        OptionTypeMismatch,
        UnknownLibType,
        BooleanIf,
        ConstantBooleanNot,
        ConstantBooleanBinop,
        IdentityLambda,
        EmptyAttrsetMerge,
        NegationSimplification,
        TrivialLet,
        ManualHasattr,
        ManualGetattr,
        DeprecatedIsNull,
        ManualOptional,
        OptionalListArgument,
    }
}
