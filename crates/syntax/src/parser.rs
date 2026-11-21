use crate::green::{SyntaxNode, TextRange, TreeBuilder};
use crate::kind::SyntaxKind;
use crate::lexer::{lex, Token};

use SyntaxKind as K;

const UNARY_BP: u8 = 10;

#[must_use]
pub fn parse(source: &str) -> SyntaxNode {
    let events = {
        let tokens = lex(source);
        let mut next_sig = 0;
        while next_sig < tokens.len() && tokens[next_sig].kind.is_trivia() {
            next_sig += 1;
        }
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            next_sig,
            events: Vec::with_capacity(tokens.len() * 2),
        };
        parser.parse_root();
        parser.events
    };
    build_events(events)
}

enum Event {
    Start {
        kind: SyntaxKind,
        forward_parent: u32,
    },
    Placeholder {
        forward_parent: u32,
        tail_offset: u32,
    },
    Finish,
    Token(SyntaxKind, TextRange),
}

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
            | K::Minus
    )
}

fn build_events(mut events: Vec<Event>) -> SyntaxNode {
    let mut builder = TreeBuilder::with_capacity(events.len());
    let mut parents = Vec::with_capacity(8);
    for idx in 0..events.len() {
        match events[idx] {
            Event::Placeholder { forward_parent, .. } => {
                if forward_parent != 0 {
                    let mut cur = idx + forward_parent as usize;
                    loop {
                        let (kind, next_fp) = match &mut events[cur] {
                            Event::Start {
                                kind,
                                forward_parent,
                            } => {
                                let k = *kind;
                                *kind = K::Eof;
                                (k, *forward_parent)
                            }
                            _ => unreachable!(),
                        };
                        if kind != K::Eof {
                            parents.push(kind);
                        }
                        if next_fp == 0 {
                            break;
                        }
                        cur += next_fp as usize;
                    }
                    for kind in parents.drain(..).rev() {
                        builder.start(kind);
                    }
                }
            }
            Event::Start { kind, .. } => {
                if kind != K::Eof {
                    builder.start(kind);
                }
            }
            Event::Finish => builder.finish(),
            Event::Token(kind, range) => builder.token(kind, range),
        }
    }
    builder.finish_all()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    next_sig: usize,
    events: Vec<Event>,
}

impl<'a> Parser<'a> {
    fn current(&self) -> SyntaxKind {
        self.tokens
            .get(self.next_sig)
            .map_or(K::Eof, |t| t.kind)
    }

    fn peek_next(&self) -> SyntaxKind {
        self.tokens
            .get(self.next_sig + 1..)
            .and_then(|s| s.iter().find(|t| !t.kind.is_trivia()))
            .map_or(K::Eof, |t| t.kind)
    }

    fn at_eof(&self) -> bool {
        self.current() == K::Eof
    }

    fn bump(&mut self) {
        while self.pos < self.next_sig {
            self.emit_token(self.tokens[self.pos]);
            self.pos += 1;
        }
        if self.pos >= self.tokens.len() || self.tokens[self.pos].kind == K::Eof {
            return;
        }
        let token = self.tokens[self.pos];
        self.emit_token(token);
        self.pos += 1;
        self.next_sig = self.pos;
        while self.next_sig < self.tokens.len() && self.tokens[self.next_sig].kind.is_trivia() {
            self.next_sig += 1;
        }
    }

    fn emit_token(&mut self, token: Token) {
        let start = token.start;
        let end = start + token.len;
        self.events
            .push(Event::Token(token.kind, TextRange::new(start, end)));
    }

        fn expect(&mut self, kind: SyntaxKind) {
        if self.current() == kind {
            self.bump();
        } else {
            self.error_recover();
        }
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.events.push(Event::Start {
            kind,
            forward_parent: 0,
        });
    }

    fn finish(&mut self) {
        self.events.push(Event::Finish);
    }

    fn checkpoint(&mut self) -> usize {
        let idx = self.events.len();
        self.events.push(Event::Placeholder {
            forward_parent: 0,
            tail_offset: 0,
        });
        idx
    }

    fn start_at(&mut self, at: usize, kind: SyntaxKind) {
        let new_idx = self.events.len();
        self.events.push(Event::Start {
            kind,
            forward_parent: 0,
        });
        match &mut self.events[at] {
            Event::Placeholder {
                forward_parent,
                tail_offset,
            } => {
                if *forward_parent == 0 {
                    let delta = (new_idx - at) as u32;
                    *forward_parent = delta;
                    *tail_offset = delta;
                } else {
                    let prev_tail = at + *tail_offset as usize;
                    let delta = (new_idx - prev_tail) as u32;
                    *tail_offset = (new_idx - at) as u32;
                    if let Event::Start {
                        forward_parent: fp, ..
                    } = &mut self.events[prev_tail]
                    {
                        *fp = delta;
                    }
                }
            }
            _ => unreachable!(),
        }
    }


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

        fn error_recover(&mut self) {
        self.start(K::ErrorNode);
        while !self.at_eof() && !self.is_sync() {
            self.bump();
        }
        self.finish();
    }

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

    fn parse_root(&mut self) {
        self.start(K::Root);
        if !self.at_eof() {
            self.parse_expr();
        }
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

        fn parse_expr_bp(&mut self, min_bp: u8) {
        let checkpoint = self.checkpoint();
        if matches!(self.current(), K::Bang | K::Minus) && UNARY_BP >= min_bp {
            self.start(K::UnaryExpr);
            self.bump();
            self.parse_expr_bp(UNARY_BP);
            self.finish();
        } else {
            self.parse_app(checkpoint);
        }

        while let Some((lbp, right_assoc)) = bin_op_bp(self.current()) {
            if lbp < min_bp {
                break;
            }
            if self.current() == K::Question {
                self.start_at(checkpoint, K::HasAttrExpr);
                self.bump();
                self.parse_attrpath();
                self.finish();
                continue;
            }
            self.start_at(checkpoint, K::BinExpr);
            self.bump();
            let rbp = if right_assoc { lbp } else { lbp + 1 };
            self.parse_expr_bp(rbp);
            self.finish();
        }
    }

        fn parse_app(&mut self, checkpoint: usize) {
        match self.current() {
            K::Ident if self.peek_next() == K::Colon => self.parse_lambda_ident(),
            K::Ident if self.peek_next() == K::At => self.parse_lambda_named_formals(),
            K::LBrace if !self.brace_is_attrset() => self.parse_lambda_formals(),
            _ => {
                self.parse_select();
                while can_start_expr(self.current())
                    && !matches!(self.current(), K::Bang | K::Minus)
                {
                    self.start_at(checkpoint, K::ApplyExpr);
                    self.parse_select();
                    self.finish();
                }
            }
        }
    }

        fn parse_select(&mut self) {
        let operand_checkpoint = self.checkpoint();
        self.parse_primary();
        while self.current() == K::Dot {
            self.start_at(operand_checkpoint, K::SelectExpr);
            self.bump();
            self.parse_attrpath();
            if self.current() == K::KwOr {
                self.bump();
                self.parse_select();
            }
            self.finish();
        }
    }

    fn parse_lambda_ident(&mut self) {
        self.start(K::LambdaExpr);
        self.bump();
        self.expect(K::Colon);
        self.parse_expr();
        self.finish();
    }

    fn parse_lambda_named_formals(&mut self) {
        self.start(K::LambdaExpr);
        self.bump();
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

        fn parse_formals_brace(&mut self) {
        self.start(K::Formals);
        self.bump();
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

    fn parse_primary(&mut self) {
        match self.current() {
            K::KwLet => self.parse_let(),
            K::KwIf => self.parse_if(),
            K::KwWith => self.parse_with(),
            K::KwAssert => self.parse_assert(),
            K::KwRec => {
                self.start(K::RecAttrsetExpr);
                self.bump();
                self.parse_attrset_body();
                self.finish();
            }
            K::LBrace => {
                self.parse_attrset_body();
            }
            K::LBracket => self.parse_list(),
            K::LParen => self.parse_paren(),
            K::StringStart => self.parse_string(K::StringExpr, K::StringEnd),
            K::IndStringStart => self.parse_string(K::IndStringExpr, K::IndStringEnd),
            _ if can_start_expr(self.current()) => self.bump(),
            _ => self.error_recover(),
        }
    }

    fn parse_let(&mut self) {
        self.start(K::LetExpr);
        self.bump();
        self.start(K::LetBindings);
        self.parse_bindings(K::KwIn);
        self.finish();
        self.expect(K::KwIn);
        self.parse_expr();
        self.finish();
    }

    fn parse_if(&mut self) {
        self.start(K::IfExpr);
        self.bump();
        self.parse_expr();
        self.expect(K::KwThen);
        self.parse_expr();
        self.expect(K::KwElse);
        self.parse_expr();
        self.finish();
    }

    fn parse_with(&mut self) {
        self.start(K::WithExpr);
        self.bump();
        self.parse_expr();
        self.expect(K::Semicolon);
        self.parse_expr();
        self.finish();
    }

    fn parse_assert(&mut self) {
        self.start(K::AssertExpr);
        self.bump();
        self.parse_expr();
        self.expect(K::Semicolon);
        self.parse_expr();
        self.finish();
    }

    fn parse_attrset_body(&mut self) {
        self.start(K::AttrsetExpr);
        self.bump();
        self.parse_bindings(K::RBrace);
        self.expect(K::RBrace);
        self.finish();
    }

    fn parse_bindings(&mut self, term: SyntaxKind) {
        loop {
            match self.current() {
                k if k == term => break,
                K::KwInherit => self.parse_inherit(),
                K::Eof => break,
                _ if self.current().is_attr_name()
                    || matches!(self.current(), K::StringStart | K::InterpStart) =>
                {
                    self.parse_binding()
                }
                _ => self.loop_error(),
            }
        }
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
        self.bump();
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
                self.parse_interp();
            } else {
                self.start(K::ErrorNode);
                self.finish();
                break;
            }
            if self.current() != K::Dot {
                break;
            }
            self.bump();
        }
        self.finish();
    }

    fn parse_list(&mut self) {
        self.start(K::ListExpr);
        self.bump();
        loop {
            match self.current() {
                K::RBracket => break,
                K::Eof => break,
                _ if can_start_expr(self.current()) => self.parse_select(),
                _ => self.loop_error(),
            }
        }
        self.expect(K::RBracket);
        self.finish();
    }

    fn parse_paren(&mut self) {
        self.start(K::ParenExpr);
        self.bump();
        self.parse_expr();
        self.expect(K::RParen);
        self.finish();
    }

    fn parse_string(&mut self, node: SyntaxKind, end: SyntaxKind) {
        self.start(node);
        self.bump();
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
        self.bump();
        self.parse_expr();
        if self.current() == K::InterpEnd {
            self.bump();
        } else {
            self.error_recover();
        }
        self.finish();
    }

        fn brace_is_attrset(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while let Some(token) = self.tokens.get(i) {
            match token.kind {
                K::LBrace => depth += 1,
                K::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
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
        true
    }
}
