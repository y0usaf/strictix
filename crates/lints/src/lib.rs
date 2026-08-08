//! Builtin lint rules — the first customer of 's public API.
//!
//! Every rule here is declared via  — the one
//! registry mechanism — and implements the  trait. Node rules
//! (syntax-level) live in [node_rules], file rules (semantic) in
//! [file_rules], and the options-schema rule in [schema].

pub mod file_rules;
pub mod node_rules;
pub mod schema;

use file_rules::{RedundantWith, SelfReferentialLet, ShadowedBinding, UnusedFormal, UnusedLambdaParam, UnusedLetBinding};
use node_rules::{AssertTrue, ConstantIf, Tautology};
use schema::UnknownOption;
use strictix_core::rules::Rule;

/// The full builtin registry, in declaration order.
///
/// Called once per run by the CLI; the boxed rules are shared across all
/// worker threads (rules are ; the schema rule caches its
/// parsed options.json in a ).
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
        UnknownOption,
    }
}
