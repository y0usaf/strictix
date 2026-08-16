//! Builtin lint rules — the first customer of strictix-core's public API.
//!
//! Every rule here is declared via the `rules!` macro — the one
//! registry mechanism — and implements the `Rule` trait. Node rules
//! (syntax-level) live in [node_rules], file rules (semantic) in
//! [file_rules], and the options-schema rule in [schema].

pub mod file_rules;
pub mod node_rules;
pub mod reference_rules;
pub mod schema;
pub mod style_rules;

use file_rules::{
    RedundantWith, SelfReferentialLet, ShadowedBinding, UnusedFormal, UnusedLambdaParam,
    UnusedLetBinding,
};
use reference_rules::{UndefinedVariable, UnknownBuiltin};
use node_rules::{AssertTrue, ConstantIf, Tautology};
use schema::UnknownOption;
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
    }
}
