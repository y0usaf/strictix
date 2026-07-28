//! Diagnostics, fix machinery, rule traits, and the semantic model.
//!
//! Depends only on `strictix-syntax`. Builtin rules live in
//! `strictix-lints` and consume only this crate's public API.

#![forbid(unsafe_code)]

/// Placeholder for M5 (rules engine). Reports crate version so the CLI
/// smoke test can call into this crate.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
