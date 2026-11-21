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
