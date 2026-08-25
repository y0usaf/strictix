//! Builtin lint rules — the first customer of strictix-core's public API.
//!
//! Every rule here is declared via the `rules!` macro — the one
//! registry mechanism — and implements the `Rule` trait. Node rules
//! (syntax-level) live in [node_rules], file rules (semantic) in
//! [file_rules], and the options-schema rule in [schema].

pub mod bare_import_in_list;
pub mod call_simplification;
pub mod duplicate_attr;
pub mod duplicate_formal;
pub mod file_rules;
pub mod ill_typed_binop;
pub mod interpolation_rules;
pub mod lib_type_rules;
pub mod node_rules;
pub mod non_boolean_condition;
pub mod path_rules;
pub mod reference_rules;
pub mod schema;
pub mod simplification_rules;
pub mod style_rules;
pub mod suggested_rules;

use bare_import_in_list::BareImportInList;
use call_simplification::{
    DeprecatedIsNull, ManualGetattr, ManualHasattr, ManualOptional, OptionalListArgument,
};
use duplicate_attr::DuplicateAttribute;
use duplicate_formal::DuplicateFormal;
use file_rules::{
    CircularLet, ReboundConstant, RedundantWith, SelfReferentialLet, ShadowedBinding,
    UnusedFormal, UnusedLambdaParam, UnusedLetBinding,
};
use ill_typed_binop::IllTypedBinop;
use interpolation_rules::{CoercedInterpolation, RedundantInterpolation};
use lib_type_rules::UnknownLibType;
use non_boolean_condition::NonBooleanCondition;
use path_rules::{AccidentalPathDivision, DanglingPath, SearchPathReference};
use reference_rules::{UndefinedVariable, UnknownBuiltin};
use node_rules::{AssertTrue, ConstantIf, Tautology};
use schema::{OptionTypeMismatch, UnknownOption};
use simplification_rules::{BooleanIf, NegationSimplification, TrivialLet};
use suggested_rules::{
    AssertFalse, DuplicateLiteralListItem, LiteralDivisionByZero, RedundantBooleanComparison,
    UnnecessaryRec, UnusedRecBinding,
};
use style_rules::{
    CollapsibleLetIn, DeprecatedToPath, EmptyInherit, EmptyLetIn, EmptyListConcat, EmptyPattern,
    EtaReduction, ManualInherit, ManualInheritFrom, RedundantPatternBind, RepeatedKeys,
    UnquotedUri, UselessHasAttr, UselessParens,
};
use strictix_core::rules::Rule;

/// The full builtin registry, in declaration order.
///
/// Called once per run by the CLI; the boxed rules are shared across all
/// worker threads (rules are `Sync`; the schema rule caches its
/// parsed options.json in a `OnceLock`).
#[must_use]
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    strictix_core::rules! {
        ConstantIf,
        AssertTrue,
        Tautology,
        UnusedLetBinding,
        UnusedLambdaParam,
        UnusedFormal,
        ShadowedBinding,
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
        UselessParens,
        RepeatedKeys,
        UnquotedUri,
        DuplicateAttribute,
        DuplicateFormal,
        NonBooleanCondition,
        IllTypedBinop,
        BareImportInList,
        UnnecessaryRec,
        UnusedRecBinding,
        AssertFalse,
        LiteralDivisionByZero,
        RedundantBooleanComparison,
        DuplicateLiteralListItem,
        CircularLet,
        ReboundConstant,
        CoercedInterpolation,
        RedundantInterpolation,
        DanglingPath,
        AccidentalPathDivision,
        SearchPathReference,
        OptionTypeMismatch,
        UnknownLibType,
        BooleanIf,
        NegationSimplification,
        TrivialLet,
        ManualHasattr,
        ManualGetattr,
        DeprecatedIsNull,
        ManualOptional,
        OptionalListArgument,
    }
}
