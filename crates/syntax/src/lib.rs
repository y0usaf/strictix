//! Lexer, parser, lossless syntax tree, and typed AST for the Nix
//! expression language.
//!
//! Everything in this crate is owned: no rowan, no rnix. The lexer
//! produces a flat token stream; the parser assembles it into a
//! lossless concrete syntax tree (whitespace and comments are preserved
//! as trivia) so that automatic fixes never destroy formatting. The
//! typed AST layer ([ast]) adds shape-aware wrappers over the tree.
//!
//! Error recovery is built in: malformed input yields a tree with
//! [SyntaxKind::ErrorNode] regions, never a panic or an abort.

#![forbid(unsafe_code)]

mod ast;
mod green;
mod kind;
mod lexer;
mod parser;

pub use ast::*;
pub use green::{NodeOrToken, SyntaxNode, SyntaxToken, TextRange};
pub use kind::SyntaxKind;
pub use lexer::{lex, Token};
pub use parser::parse;
