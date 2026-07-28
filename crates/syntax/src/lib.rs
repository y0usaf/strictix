//! Lexer, parser, lossless syntax tree, and typed AST for the Nix
//! expression language.
//!
//! Everything in this crate is owned: no rowan, no rnix. The tree is
//! lossless (whitespace and comments are preserved as trivia) so that
//! automatic fixes never destroy formatting.

#![forbid(unsafe_code)]

/// Placeholder for M1 (lexer). Returns the input unchanged so the
/// workspace links and the CLI smoke test can call into this crate.
#[must_use]
pub fn echo(source: &str) -> &str {
    source
}
