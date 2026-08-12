//! Hand-written recursive-descent parser producing a lossless CST.
//!
//! The parser is intentionally simple: one method per production, no
//! tables beyond the operator-precedence one, and error recovery from
//! day one. When the input cannot be parsed, an [`ErrorNode`] is
//! produced and the parser resynchronizes at `;`, `}`, `]`, `)`, `in`,
//! or end of input — the parser never panics and never aborts.
//!
//! Tokens are consumed through [`Parser::bump`], which emits both the
//! significant token and any trivia (whitespace/comments) so the tree
//! tiles the source exactly.

use crate::green::{SyntaxNode, TextRange, TreeBuilder};
use crate::kind::SyntaxKind;
use crate::lexer::{lex, Token};

use SyntaxKind as K;

/// Binding power of unary `!`. It binds tighter than `//` but looser
/// than `?` (verified against Nix evaluation).
const UNARY_BP: u8 = 10;

/// Parse `source` into a lossless CST. Never panics; malformed input
/// yields a tree containing [`SyntaxKind::ErrorNode`] regions.
#[must_use]
pub fn parse(source: &str) -> SyntaxNode {
    let tokens = lex(source);
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        events: Vec::new(),
    };
    parser.parse_root();
    parser.build()
}

/// One event in the parser output stream, consumed by [`TreeBuilder`].
enum Event {
    Start(SyntaxKind),
    Finish,
    Token(SyntaxKind, TextRange),
}

/// Binding power (higher binds tighter) for each infix operator, plus
/// whether it is right-associative. Order was verified against Nix
/// evaluation; see tests.
fn bin_op_bp(kind: SyntaxKind) -> Option<(u8, bool)> {
    Some(match kind {
        K::Arrow => (1, true),
        K::OrOr => (2, false),
        K::AndAnd => (3, false),
        K::EqEq | K::Neq => (4, false),
        K::Lt | K::LtEq | K::Gt | K::GtEq => (5, false),
        K::PlusPlus => (6, false),
        K::Plus | K::Minus => (7, false),
        K::Star | K::Slash => (8, false),
        K::SlashSlash => (9, false),
        K::Question => (11, false),
        _ => return None,
    })
}

/// Whether `kind` can begin an expression.
fn can_start_expr(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        K::Ident
            | K::Int
            | K::Float
            | K::Path
            | K::SearchPath
            | K::Uri
            | K::KwLet
            | K::KwIf
            | K::KwWith
            | K::KwAssert
            | K::KwRec
            | K::LBrace
            | K::LBracket
            | K::LParen
            | K::StringStart
            | K::IndStringStart
            | K::Bang
    )
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    events: Vec<Event>,
}

impl<'a> Parser<'a> {
    // --- token access ---

    /// Kind of the next significant (non-trivia) token.
    fn current(&self) -> SyntaxKind {
        self.tokens[self.pos..]
            .iter()
            .find(|t| !t.kind.is_trivia())
            .map(|t| t.kind)
            .unwrap_or(K::Eof)
    }

    /// Kind of the `offset`-th significant token after the current one.
    fn peek(&self, offset: usize) -> SyntaxKind {
        self.tokens[self.pos..]
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .nth(offset)
            .map(|t| t.kind)
            .unwrap_or(K::Eof)
    }

    fn at_eof(&self) -> bool {
        self.current() == K::Eof
    }

    /// Emit the current token, advancing past it. Any leading trivia is
    /// flushed first so the tree tiles the source exactly. Never emits
    /// the Eof sentinel and never advances past it, so the parser
    /// cannot panic even on wildly malformed input.
    fn bump(&mut self) {
        while let Some(token) = self.tokens.get(self.pos) {
            if token.kind.is_trivia() {
                self.emit_token(*token);
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos >= self.tokens.len() || self.tokens[self.pos].kind == K::Eof {
            return;
        }
        let token = self.tokens[self.pos];
        self.emit_token(token);
        self.pos += 1;
    }

    fn emit_token(&mut self, token: Token) {
        let start = token.start;
        let end = start + token.len;
        self.events
            .push(Event::Token(token.kind, TextRange::new(start, end)));
    }

    /// Consume the current token if it has `kind`; otherwise emit an
    /// error node and resynchronize.
    fn expect(&mut self, kind: SyntaxKind) {
        if self.current() == kind {
            self.bump();
        } else {
            self.error_recover();
        }
    }

    // --- tree events ---

    fn start(&mut self, kind: SyntaxKind) {
        self.events.push(Event::Start(kind));
    }

    fn finish(&mut self) {
        self.events.push(Event::Finish);
    }

    /// Start a node whose opening lands at an earlier position in the
    /// event stream, wrapping everything emitted since that position —
    /// the already-parsed operand — inside a node of the given kind.
    /// The matching finish() closes it after the operator's right side.
    fn start_at(&mut self, at: usize, kind: SyntaxKind) {
        self.events.insert(at, Event::Start(kind));
    }

    /// Run the accumulated events through the tree builder.
    fn build(self) -> SyntaxNode {
        let mut builder = TreeBuilder::default();
        for event in self.events {
            match event {
                Event::Start(kind) => builder.start(kind),
                Event::Finish => builder.finish(),
                Event::Token(kind, range) => builder.token(kind, range),
            }
        }
        builder.finish_all()
    }

    // --- error recovery ---

    /// Whether the current token is a safe place to resume parsing.
    fn is_sync(&self) -> bool {
        matches!(
            self.current(),
            K::Semicolon
                | K::RBrace
                | K::RBracket
                | K::RParen
                | K::KwIn
                | K::KwThen
                | K::KwElse
                | K::Eof
        )
    }

    /// Wrap the unexpected tokens (up to but *not including* the next
    /// sync point) in an error node. The caller resumes at the sync
    /// token, which usually belongs to the enclosing construct.
    fn error_recover(&mut self) {
        self.start(K::ErrorNode);
        while !self.at_eof() && !self.is_sync() {
            self.bump();
        }
        self.finish();
    }

    /// Handle an unexpected token inside a repetition loop. Garbage is
    /// consumed by [`Self::error_recover`]; a stray sync token (one the
    /// loop does not handle) is wrapped in a small error node so the
    /// loop always makes progress.
    fn loop_error(&mut self) {
        if self.at_eof() {
            return;
        }
        if self.is_sync() {
            self.start(K::ErrorNode);
            self.bump();
            self.finish();
        } else {
            self.error_recover();
        }
    }

    // --- entry ---

    fn parse_root(&mut self) {
        self.start(K::Root);
        if !self.at_eof() {
            self.parse_expr();
        }
        // Leftover significant tokens mean the expression was followed
        // by garbage: wrap them in an error node. Trivia is flushed in
        // either case so the tree still tiles the source exactly.
        let leftover = self.current() != K::Eof;
        if leftover {
            self.start(K::ErrorNode);
        }
        while self.pos < self.tokens.len() && self.tokens[self.pos].kind != K::Eof {
            self.bump();
        }
        if leftover {
            self.finish();
        }
        self.finish();
    }

    // --- expressions ---

    fn parse_expr(&mut self) {
        if self.at_eof() {
            return;
        }
        if !can_start_expr(self.current()) {
            self.error_recover();
            return;
        }
        self.parse_expr_bp(0);
    }

    /// Precedence-climbing expression parser. min_bp is the lowest
    /// binding power the caller will accept.
    fn parse_expr_bp(&mut self, min_bp: u8) {
        let checkpoint = self.events.len();
        if self.current() == K::Bang && UNARY_BP >= min_bp {
            self.start(K::UnaryExpr);
            self.bump(); // bang
            self.parse_expr_bp(UNARY_BP);
            self.finish();
        } else {
            self.parse_app(checkpoint);
        }

        while let Some((lbp, right_assoc)) = bin_op_bp(self.current()) {
            if lbp < min_bp {
                break;
            }
            let kind = if self.current() == K::Question {
                K::HasAttrExpr
            } else {
                K::BinExpr
            };
            // Wrap the already-parsed left operand inside the operator
            // node: '1 + 2 * 3' nests as BinExpr(1, +, BinExpr(2, *, 3)),
            // not as a flat spine with the lhs as a preceding sibling.
            self.start_at(checkpoint, kind);
            self.bump(); // the operator
            let rbp = if right_assoc { lbp } else { lbp + 1 };
            self.parse_expr_bp(rbp);
            self.finish();
        }
    }

    /// An application chain: `f a.b c` = `((f (a.b)) c)`, left-assoc.
    /// A lambda is a complete expression (its body parses its own app).
    /// The checkpoint is the expression-level event index (the left edge
    /// of the whole chain); each select records its own operand start.
    fn parse_app(&mut self, checkpoint: usize) {
        match self.current() {
            // x: body
            K::Ident if self.peek(1) == K::Colon => self.parse_lambda_ident(),
            // args@{ ... }: body
            K::Ident if self.peek(1) == K::At => self.parse_lambda_named_formals(),
            // { formals }: body
            K::LBrace if !self.brace_is_attrset() => self.parse_lambda_formals(),
            _ => {
                self.parse_select();
                while can_start_expr(self.current()) {
                    self.start_at(checkpoint, K::ApplyExpr);
                    self.parse_select();
                    self.finish();
                }
            }
        }
    }

    /// A primary with postfix selection: `a.b.c`, `a.b or default`.
    /// Selection binds tighter than application (verified against Nix):
    /// `f a.b` is `f (a.b)`, and `a.b c` is `(a.b) c`. List items
    /// and application arguments are selects, not applications — Nix
    /// rejects `[ 1 + 2 ]` and `[ f x ]` is two elements.
    ///
    /// The select node is opened *around* the already-parsed primary via
    /// start_at, so the base ends up as a child, not a sibling.
    fn parse_select(&mut self) {
        let operand_checkpoint = self.events.len();
        self.parse_primary();
        while self.current() == K::Dot {
            self.start_at(operand_checkpoint, K::SelectExpr);
            self.bump(); // dot
            self.parse_attrpath();
            if self.current() == K::KwOr {
                self.bump();
                // or default — the default is itself a select,
                // and application continues outside.
                self.parse_select();
            }
            self.finish();
        }
    }
    // --- lambdas ---

    fn parse_lambda_ident(&mut self) {
        self.start(K::LambdaExpr);
        self.bump(); // parameter ident
        self.expect(K::Colon);
        self.parse_expr();
        self.finish();
    }

    fn parse_lambda_named_formals(&mut self) {
        self.start(K::LambdaExpr);
        self.bump(); // args ident
        self.expect(K::At);
        self.parse_formals_brace();
        self.expect(K::Colon);
        self.parse_expr();
        self.finish();
    }

    fn parse_lambda_formals(&mut self) {
        self.start(K::LambdaExpr);
        self.parse_formals_brace();
        if self.current() == K::At {
            self.bump();
            if self.current() == K::Ident {
                self.bump();
            } else {
                self.error_recover();
            }
        }
        self.expect(K::Colon);
        self.parse_expr();
        self.finish();
    }

    /// Parse `{ a, b ? 1, ... }` as lambda formal parameters.
    fn parse_formals_brace(&mut self) {
        self.start(K::Formals);
        self.bump(); // `{`
        loop {
            match self.current() {
                K::RBrace => break,
                K::Ellipsis => self.bump(),
                K::Ident => {
                    self.bump();
                    if self.current() == K::Question {
                        self.bump();
                        self.parse_expr();
                    }
                }
                K::Comma => self.bump(),
                _ => self.loop_error(),
            }
        }
        self.expect(K::RBrace);
        self.finish();
    }

    // --- primaries ---

    fn parse_primary(&mut self) {
        match self.current() {
            K::KwLet => self.parse_let(),
            K::KwIf => self.parse_if(),
            K::KwWith => self.parse_with(),
            K::KwAssert => self.parse_assert(),
            K::KwRec => {
                self.start(K::RecAttrsetExpr);
                self.bump(); // `rec`
                self.parse_attrset_body();
                self.finish();
            }
            K::LBrace => {
                // Only reachable when the brace is an attrset (formals
                // are intercepted by parse_operand).
                self.parse_attrset_body();
            }
            K::LBracket => self.parse_list(),
            K::LParen => self.parse_paren(),
            K::StringStart => self.parse_string(K::StringExpr, K::StringEnd),
            K::IndStringStart => self.parse_string(K::IndStringExpr, K::IndStringEnd),
            // Plain atoms: ident, literals, paths, uris.
            _ if can_start_expr(self.current()) => self.bump(),
            _ => self.error_recover(),
        }
    }

    fn parse_let(&mut self) {
        self.start(K::LetExpr);
        self.bump(); // `let`
        self.start(K::LetBindings);
        loop {
            match self.current() {
                K::KwIn => break,
                K::KwInherit => self.parse_inherit(),
                K::Eof => break,
                _ if self.current().is_attr_name() || self.current() == K::StringStart => {
                    self.parse_binding()
                }
                _ => self.loop_error(),
            }
        }
        self.finish(); // LetBindings
        self.expect(K::KwIn);
        self.parse_expr();
        self.finish(); // LetExpr
    }

    fn parse_if(&mut self) {
        self.start(K::IfExpr);
        self.bump(); // `if`
        self.parse_expr();
        self.expect(K::KwThen);
        self.parse_expr();
        self.expect(K::KwElse);
        self.parse_expr();
        self.finish();
    }

    fn parse_with(&mut self) {
        self.start(K::WithExpr);
        self.bump(); // `with`
        self.parse_expr();
        self.expect(K::Semicolon);
        self.parse_expr();
        self.finish();
    }

    fn parse_assert(&mut self) {
        self.start(K::AssertExpr);
        self.bump(); // `assert`
        self.parse_expr();
        self.expect(K::Semicolon);
        self.parse_expr();
        self.finish();
    }

    fn parse_attrset_body(&mut self) {
        self.start(K::AttrsetExpr);
        self.bump(); // `{`
        loop {
            match self.current() {
                K::RBrace => break,
                K::KwInherit => self.parse_inherit(),
                K::Eof => break,
                _ if self.current().is_attr_name() || self.current() == K::StringStart => {
                    self.parse_binding()
                }
                _ => self.loop_error(),
            }
        }
        self.expect(K::RBrace);
        self.finish();
    }

    fn parse_binding(&mut self) {
        self.start(K::Binding);
        self.parse_attrpath();
        self.expect(K::Assign);
        self.parse_expr();
        self.expect(K::Semicolon);
        self.finish();
    }

    fn parse_inherit(&mut self) {
        self.start(K::InheritStmt);
        self.bump(); // `inherit`
        if self.current() == K::LParen {
            self.bump();
            self.parse_expr();
            self.expect(K::RParen);
        }
        loop {
            if self.current().is_attr_name() {
                self.bump();
            } else {
                break;
            }
        }
        self.expect(K::Semicolon);
        self.finish();
    }

    fn parse_attrpath(&mut self) {
        self.start(K::Attrpath);
        loop {
            if self.current().is_attr_name() {
                self.bump();
            } else if self.current() == K::StringStart {
                self.parse_string(K::StringExpr, K::StringEnd);
            } else if self.current() == K::InterpStart {
                // Unquoted dynamic segment: the expr in a.${b}.c.
                self.parse_interp();
            } else {
                break;
            }
            if self.current() != K::Dot {
                break;
            }
            self.bump(); // `.`
        }
        self.finish();
    }

    fn parse_list(&mut self) {
        self.start(K::ListExpr);
        self.bump(); // [
        loop {
            match self.current() {
                K::RBracket => break,
                K::Eof => break,
                // Nix list items are selects: no application folding
                // ([ f x ] is two elements), no binops ([ 1 + 2 ] errors).
                _ if can_start_expr(self.current()) => self.parse_select(),
                _ => self.loop_error(),
            }
        }
        self.expect(K::RBracket);
        self.finish();
    }

    fn parse_paren(&mut self) {
        self.start(K::ParenExpr);
        self.bump(); // `(`
        self.parse_expr();
        self.expect(K::RParen);
        self.finish();
    }

    fn parse_string(&mut self, node: SyntaxKind, end: SyntaxKind) {
        self.start(node);
        self.bump(); // opening quote
        loop {
            match self.current() {
                K::InterpStart => self.parse_interp(),
                k if k == end => break,
                K::StringContent | K::InterpEnd => self.bump(),
                K::Eof => break,
                _ => self.loop_error(),
            }
        }
        self.expect(end);
        self.finish();
    }

    fn parse_interp(&mut self) {
        self.start(K::InterpExpr);
        self.bump(); // `${`
        self.parse_expr();
        if self.current() == K::InterpEnd {
            self.bump();
        } else {
            self.error_recover();
        }
        self.finish();
    }

    // --- lookahead ---

    /// Decide whether the brace block starting at the current position
    /// is an attrset (bindings with `=`) or lambda formals.
    ///
    /// A block is an attrset if it contains `=`, `;`, or `inherit` at
    /// its own top level; it is formals if it contains `,` or `...`.
    /// An empty block is an attrset unless followed by `:` or `@`.
    fn brace_is_attrset(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while let Some(token) = self.tokens.get(i) {
            match token.kind {
                K::LBrace => depth += 1,
                K::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        // No separator seen: empty block, or a block
                        // that contains only invalid-for-formals content.
                        return !matches!(
                            self.tokens.get(i + 1).map(|t| t.kind),
                            Some(K::Colon | K::At)
                        );
                    }
                }
                K::Assign | K::Semicolon | K::KwInherit if depth == 1 => return true,
                K::Comma | K::Ellipsis if depth == 1 => return false,
                _ => {}
            }
            i += 1;
        }
        // Unterminated block: treat as an attrset to avoid a bogus
        // lambda interpretation.
        true
    }
}
