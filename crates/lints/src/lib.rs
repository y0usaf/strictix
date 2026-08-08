//! Builtin lint rules — the first customer of `strictix-core`'s public API.
//!
//! Every rule here is declared via `strictix_core::rules!` — the one
//! registry mechanism — and implements the `Rule` trait. Node rules
//! (syntax-level) live in [node_rules], file rules (semantic) in
//! [file_rules], and the options-schema rule in [schema].

pub mod file_rules;
pub mod node_rules;
pub mod schema;

use strictix_core::rules::Rule;

/// The full builtin registry, in declaration order.
///
/// Called once per run by the CLI; the boxed rules are shared across all
/// worker threads (rules are `Send + Sync`; the schema rule caches its
/// parsed options.json in a `OnceLock`).
#[must_use]
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    strictix_core::rules! {
        // --- node rules (syntax) ---
        node_rules::ConstantIf,
        node_rules::AssertTrue,
        node_rules::Tautology,
        // --- file rules (semantic) ---
        file_rules::UnusedLetBinding,
        file_rules::UnusedLambdaParam,
        file_rules::UnusedFormal,
        file_rules::ShadowedBinding,
        file_rules::RedundantWith,
        file_rules::SelfReferentialLet,
        // --- schema (M8) ---
        schema::UnknownOption,
    }
}
