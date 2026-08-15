//! Typed AST layer over the lossless CST.
//!
//! The CST is untyped: every node is a SyntaxNode with a SyntaxKind and
//! a flat child list. This layer adds one typed wrapper per node kind,
//! with accessors that know the shape: a Binding has an attrpath and a
//! value, a BinExpr has a left operand, an operator, and a right
//! operand. Rules (M5+) consume this layer, never the raw tree.
//!
//! Wrappers are zero-copy views over the CST: casting is a kind check,
//! accessors filter children. No allocation, no separate tree.

use crate::green::{NodeOrToken, SyntaxNode, SyntaxToken, TextRange};
use crate::kind::SyntaxKind;

use SyntaxKind as K;

/// A typed view over a SyntaxNode of one specific kind.
pub trait AstNode<'a>: Copy {
    /// The node kind this wrapper accepts.
    fn kind() -> SyntaxKind;

    /// Wrap a node if its kind matches; None otherwise.
    fn cast(node: &'a SyntaxNode) -> Option<Self>;

    /// The underlying node.
    fn syntax(&self) -> &'a SyntaxNode;

    /// The byte range of the underlying node.
    fn range(&self) -> TextRange {
        self.syntax().range()
    }
}

/// One wrapper per node kind: struct + AstNode impl.
macro_rules! ast_node {
    ($(#[$doc:meta])* $name:ident, $kind:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct $name<'a>(&'a SyntaxNode);

        impl<'a> AstNode<'a> for $name<'a> {
            fn kind() -> SyntaxKind {
                SyntaxKind::$kind
            }
            fn cast(node: &'a SyntaxNode) -> Option<Self> {
                (node.kind() == SyntaxKind::$kind).then_some(Self(node))
            }
            fn syntax(&self) -> &'a SyntaxNode {
                self.0
            }
        }
    };
}

ast_node!(Root, Root);
ast_node!(ErrorNode, ErrorNode);
ast_node!(LetExpr, LetExpr);
ast_node!(LetBindings, LetBindings);
ast_node!(Binding, Binding);
ast_node!(Attrpath, Attrpath);
ast_node!(InheritStmt, InheritStmt);
ast_node!(WithExpr, WithExpr);
ast_node!(AssertExpr, AssertExpr);
ast_node!(IfExpr, IfExpr);
ast_node!(AttrsetExpr, AttrsetExpr);
ast_node!(RecAttrsetExpr, RecAttrsetExpr);
ast_node!(Formals, Formals);
ast_node!(ListExpr, ListExpr);
ast_node!(LambdaExpr, LambdaExpr);
ast_node!(ApplyExpr, ApplyExpr);
ast_node!(UnaryExpr, UnaryExpr);
ast_node!(BinExpr, BinExpr);
ast_node!(SelectExpr, SelectExpr);
ast_node!(HasAttrExpr, HasAttrExpr);
ast_node!(StringExpr, StringExpr);
ast_node!(IndStringExpr, IndStringExpr);
ast_node!(InterpExpr, InterpExpr);
ast_node!(ParenExpr, ParenExpr);

// --- expressions ----------------------------------------------------

/// A Nix expression: an atom token or a compound node.
///
/// Atoms (idents, literals, paths, uris) are tokens in the CST, not
/// nodes, so the enum has one variant per atom kind plus one per
/// expression node kind. This is what rules match on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expr<'a> {
    Ident(&'a SyntaxToken),
    Int(&'a SyntaxToken),
    Float(&'a SyntaxToken),
    Path(&'a SyntaxToken),
    SearchPath(&'a SyntaxToken),
    Uri(&'a SyntaxToken),
    Let(LetExpr<'a>),
    With(WithExpr<'a>),
    Assert(AssertExpr<'a>),
    If(IfExpr<'a>),
    Attrset(AttrsetExpr<'a>),
    RecAttrset(RecAttrsetExpr<'a>),
    List(ListExpr<'a>),
    Lambda(LambdaExpr<'a>),
    Apply(ApplyExpr<'a>),
    Unary(UnaryExpr<'a>),
    Bin(BinExpr<'a>),
    Select(SelectExpr<'a>),
    HasAttr(HasAttrExpr<'a>),
    String(StringExpr<'a>),
    IndString(IndStringExpr<'a>),
    Paren(ParenExpr<'a>),
}

impl<'a> Expr<'a> {
    /// Cast a node to an expression; None for structural nodes
    /// (Attrpath, Formals, ...) and error nodes.
    #[must_use]
    pub fn cast(node: &'a SyntaxNode) -> Option<Self> {
        Some(match node.kind() {
            K::LetExpr => Self::Let(LetExpr::cast(node)?),
            K::WithExpr => Self::With(WithExpr::cast(node)?),
            K::AssertExpr => Self::Assert(AssertExpr::cast(node)?),
            K::IfExpr => Self::If(IfExpr::cast(node)?),
            K::AttrsetExpr => Self::Attrset(AttrsetExpr::cast(node)?),
            K::RecAttrsetExpr => Self::RecAttrset(RecAttrsetExpr::cast(node)?),
            K::ListExpr => Self::List(ListExpr::cast(node)?),
            K::LambdaExpr => Self::Lambda(LambdaExpr::cast(node)?),
            K::ApplyExpr => Self::Apply(ApplyExpr::cast(node)?),
            K::UnaryExpr => Self::Unary(UnaryExpr::cast(node)?),
            K::BinExpr => Self::Bin(BinExpr::cast(node)?),
            K::SelectExpr => Self::Select(SelectExpr::cast(node)?),
            K::HasAttrExpr => Self::HasAttr(HasAttrExpr::cast(node)?),
            K::StringExpr => Self::String(StringExpr::cast(node)?),
            K::IndStringExpr => Self::IndString(IndStringExpr::cast(node)?),
            K::ParenExpr => Self::Paren(ParenExpr::cast(node)?),
            _ => return None,
        })
    }

    /// Cast an atom token to an expression.
    #[must_use]
    pub fn cast_token(token: &'a SyntaxToken) -> Option<Self> {
        Some(match token.kind() {
            K::Ident => Self::Ident(token),
            K::Int => Self::Int(token),
            K::Float => Self::Float(token),
            K::Path => Self::Path(token),
            K::SearchPath => Self::SearchPath(token),
            K::Uri => Self::Uri(token),
            _ => return None,
        })
    }

    /// The byte range of this expression.
    #[must_use]
    pub fn range(&self) -> TextRange {
        match self {
            Self::Ident(t)
            | Self::Int(t)
            | Self::Float(t)
            | Self::Path(t)
            | Self::SearchPath(t)
            | Self::Uri(t) => t.range(),
            Self::Let(e) => e.range(),
            Self::With(e) => e.range(),
            Self::Assert(e) => e.range(),
            Self::If(e) => e.range(),
            Self::Attrset(e) => e.range(),
            Self::RecAttrset(e) => e.range(),
            Self::List(e) => e.range(),
            Self::Lambda(e) => e.range(),
            Self::Apply(e) => e.range(),
            Self::Unary(e) => e.range(),
            Self::Bin(e) => e.range(),
            Self::Select(e) => e.range(),
            Self::HasAttr(e) => e.range(),
            Self::String(e) => e.range(),
            Self::IndString(e) => e.range(),
            Self::Paren(e) => e.range(),
        }
    }

    /// The source text of this expression.
    #[must_use]
    pub fn text<'s>(&self, source: &'s str) -> &'s str {
        let r = self.range();
        &source[r.start() as usize..r.end() as usize]
    }
}

/// Cast a child (node or token) to an expression.
fn expr_of<'a>(child: &'a NodeOrToken) -> Option<Expr<'a>> {
    match child {
        NodeOrToken::Node(n) => Expr::cast(n),
        NodeOrToken::Token(t) => Expr::cast_token(t),
    }
}

/// The first expression child, if any.
fn first_expr<'a>(node: &'a SyntaxNode) -> Option<Expr<'a>> {
    node.children().iter().find_map(expr_of)
}

/// The last expression child, if any.
fn last_expr<'a>(node: &'a SyntaxNode) -> Option<Expr<'a>> {
    node.children().iter().rev().find_map(expr_of)
}

// --- binding blocks (let / attrset) ---------------------------------

/// One entry in a binding block: a binding or an inherit statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrItem<'a> {
    Binding(Binding<'a>),
    Inherit(InheritStmt<'a>),
}

/// The entries of a binding block, in source order.
fn block_items<'a>(node: &'a SyntaxNode) -> impl Iterator<Item = AttrItem<'a>> + 'a {
    node.child_nodes().filter_map(|n| match n.kind() {
        K::Binding => Some(AttrItem::Binding(Binding(n))),
        K::InheritStmt => Some(AttrItem::Inherit(InheritStmt(n))),
        _ => None,
    })
}

impl<'a> Root<'a> {
    /// The top-level expression, if any.
    #[must_use]
    pub fn expr(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }
}

impl<'a> LetExpr<'a> {
    /// The binding section between let and in.
    #[must_use]
    pub fn bindings(&self) -> Option<LetBindings<'a>> {
        self.syntax().child_node(K::LetBindings).map(LetBindings)
    }

    /// The expression after in.
    #[must_use]
    pub fn body(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

impl<'a> LetBindings<'a> {
    /// The entries of this binding block, in source order.
    pub fn items(&self) -> impl Iterator<Item = AttrItem<'a>> + 'a {
        block_items(self.syntax())
    }
}

impl<'a> AttrsetExpr<'a> {
    /// The entries of this attrset, in source order.
    pub fn items(&self) -> impl Iterator<Item = AttrItem<'a>> + 'a {
        block_items(self.syntax())
    }
}

impl<'a> Binding<'a> {
    /// The bound attribute path.
    #[must_use]
    pub fn attrpath(&self) -> Option<Attrpath<'a>> {
        self.syntax().child_node(K::Attrpath).map(Attrpath)
    }

    /// The bound value expression.
    #[must_use]
    pub fn value(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }
}

/// One element of an attribute path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrName<'a> {
    /// A plain name token: a in a.b, or a keyword/number.
    Ident(&'a SyntaxToken),
    /// A quoted segment: the string in etc."x/y".text.
    Str(StringExpr<'a>),
    /// An unquoted dynamic segment: the expr in a.${b}.c.
    Interp(InterpExpr<'a>),
}

impl<'a> Attrpath<'a> {
    /// The path elements in order, dots excluded.
    pub fn elements(&self) -> impl Iterator<Item = AttrName<'a>> + 'a {
        self.syntax().children().iter().filter_map(|c| match c {
            NodeOrToken::Token(t) if t.kind().is_attr_name() => Some(AttrName::Ident(t)),
            NodeOrToken::Node(n) if n.kind() == K::StringExpr => Some(AttrName::Str(StringExpr(n))),
            NodeOrToken::Node(n) if n.kind() == K::InterpExpr => {
                Some(AttrName::Interp(InterpExpr(n)))
            }
            _ => None,
        })
    }
}

impl<'a> InheritStmt<'a> {
    /// The (from) expression, if present.
    #[must_use]
    pub fn source(&self) -> Option<Expr<'a>> {
        let node = self.syntax();
        let lp = node.children().iter().position(|c| c.kind() == K::LParen)?;
        node.children()[lp + 1..].iter().find_map(expr_of)
    }

    /// The inherited names, source expression excluded.
    pub fn names(&self) -> impl Iterator<Item = &'a SyntaxToken> + 'a {
        let node = self.syntax();
        // For `inherit (from) a b;` the names come after the RParen;
        // otherwise they come after the leading `inherit` keyword. The
        // keyword is located explicitly: unlike an RParen, it does not
        // delimit the names, but the node may begin with leading trivia,
        // so its index cannot be assumed to be 1.
        let start = if let Some(rp) = node.children().iter().position(|c| c.kind() == K::RParen) {
            rp + 1
        } else {
            node.children()
                .iter()
                .position(|c| c.kind() == K::KwInherit)
                .map_or(1, |i| i + 1)
        };
        node.children()[start..].iter().filter_map(|c| match c {
            NodeOrToken::Token(t) if t.kind().is_attr_name() => Some(t),
            _ => None,
        })
    }
}

impl<'a> WithExpr<'a> {
    /// The scope expression.
    #[must_use]
    pub fn scope(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The body after the semicolon.
    #[must_use]
    pub fn body(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

impl<'a> AssertExpr<'a> {
    /// The asserted condition.
    #[must_use]
    pub fn cond(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The body after the semicolon.
    #[must_use]
    pub fn body(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

impl<'a> IfExpr<'a> {
    /// The condition.
    #[must_use]
    pub fn cond(&self) -> Option<Expr<'a>> {
        self.exprs().nth(0)
    }

    /// The then branch.
    #[must_use]
    pub fn then_branch(&self) -> Option<Expr<'a>> {
        self.exprs().nth(1)
    }

    /// The else branch.
    #[must_use]
    pub fn else_branch(&self) -> Option<Expr<'a>> {
        self.exprs().nth(2)
    }

    fn exprs(&self) -> impl Iterator<Item = Expr<'a>> + '_ {
        self.syntax().children().iter().filter_map(expr_of)
    }
}

impl<'a> RecAttrsetExpr<'a> {
    /// The inner attrset.
    #[must_use]
    pub fn attrset(&self) -> Option<AttrsetExpr<'a>> {
        self.syntax().child_node(K::AttrsetExpr).map(AttrsetExpr)
    }
}

/// One formal parameter: a name and an optional default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormalParam<'a> {
    pub name: &'a SyntaxToken,
    pub default: Option<Expr<'a>>,
}

impl<'a> Formals<'a> {
    /// The parameters in order; b ? 1 yields a default.
    pub fn params(&self) -> impl Iterator<Item = FormalParam<'a>> + 'a {
        let mut pending: Option<&'a SyntaxToken> = None;
        let mut expect_default = false;
        self.syntax().children().iter().filter_map(move |c| {
            // flush a pending plain param at a separator
            let sep = matches!(c.kind(), K::Comma | K::Ellipsis | K::RBrace);
            if sep && !expect_default {
                let out = pending.map(|name| FormalParam {
                    name,
                    default: None,
                });
                pending = None;
                if out.is_some() {
                    return out;
                }
            }
            match c {
                NodeOrToken::Token(t) if expect_default => {
                    if let Some(default) = Expr::cast_token(t) {
                        expect_default = false;
                        Some(FormalParam {
                            name: pending.take()?,
                            default: Some(default),
                        })
                    } else if t.kind().is_trivia() {
                        None // whitespace inside the default expr
                    } else {
                        // stray significant token: drop the incomplete param
                        expect_default = false;
                        pending = None;
                        None
                    }
                }
                NodeOrToken::Node(n) if expect_default => {
                    expect_default = false;
                    Some(FormalParam {
                        name: pending.take()?,
                        default: Expr::cast(n),
                    })
                }
                NodeOrToken::Token(t) if t.kind() == K::Ident => {
                    pending = Some(t);
                    None
                }
                NodeOrToken::Token(t) if t.kind() == K::Question => {
                    expect_default = true;
                    None
                }
                _ => None,
            }
        })
    }

    /// Whether the ellipsis is present.
    #[must_use]
    pub fn has_ellipsis(&self) -> bool {
        self.syntax()
            .child_tokens()
            .any(|t| t.kind() == K::Ellipsis)
    }
}

impl<'a> ListExpr<'a> {
    /// The list elements in order.
    pub fn items(&self) -> impl Iterator<Item = Expr<'a>> + 'a {
        self.syntax().children().iter().filter_map(expr_of)
    }
}

/// A lambda parameter: a bare name or a formal set (with an optional
/// at-name binding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LambdaParam<'a> {
    Ident(&'a SyntaxToken),
    Formals(Formals<'a>, Option<&'a SyntaxToken>),
}

impl<'a> LambdaExpr<'a> {
    /// The parameter.
    #[must_use]
    pub fn param(&self) -> LambdaParam<'a> {
        let node = self.syntax();
        match node.child_node(K::Formals) {
            Some(f) => {
                // The at-name, when present, is the ident beside the at.
                let name = node
                    .children()
                    .iter()
                    .enumerate()
                    .find_map(|(i, c)| match c {
                        NodeOrToken::Token(t) if t.kind() == K::At => {
                            let before = node.children()[..i].iter().rev().find_map(|c| match c {
                                NodeOrToken::Token(t) if t.kind() == K::Ident => Some(t),
                                _ => None,
                            });
                            let after = node.children()[i + 1..].iter().find_map(|c| match c {
                                NodeOrToken::Token(t) if t.kind() == K::Ident => Some(t),
                                _ => None,
                            });
                            before.or(after)
                        }
                        _ => None,
                    });
                LambdaParam::Formals(Formals(f), name)
            }
            None => LambdaParam::Ident(
                node.child_tokens()
                    .find(|t| t.kind() == K::Ident)
                    .expect("lambda ident param"),
            ),
        }
    }

    /// The lambda body.
    #[must_use]
    pub fn body(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

impl<'a> ApplyExpr<'a> {
    /// The function.
    #[must_use]
    pub fn func(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The argument.
    #[must_use]
    pub fn arg(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

impl<'a> UnaryExpr<'a> {
    /// The operand after the bang.
    #[must_use]
    pub fn operand(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }
}

impl<'a> BinExpr<'a> {
    /// The left operand.
    #[must_use]
    pub fn lhs(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The right operand.
    #[must_use]
    pub fn rhs(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }

    /// The operator token kind (Plus, Arrow, ...).
    #[must_use]
    pub fn op(&self) -> Option<SyntaxKind> {
        self.syntax()
            .child_tokens()
            .map(|t| t.kind())
            .find(|k| k.is_binop())
    }
}

impl<'a> SelectExpr<'a> {
    /// The base expression.
    #[must_use]
    pub fn base(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The selected attribute path.
    #[must_use]
    pub fn attrpath(&self) -> Option<Attrpath<'a>> {
        self.syntax().child_node(K::Attrpath).map(Attrpath)
    }

    /// The or-default expression, if present.
    #[must_use]
    pub fn default(&self) -> Option<Expr<'a>> {
        if self.syntax().child_tokens().any(|t| t.kind() == K::KwOr) {
            last_expr(self.syntax())
        } else {
            None
        }
    }
}

impl<'a> HasAttrExpr<'a> {
    /// The base expression.
    #[must_use]
    pub fn base(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }

    /// The attribute path on the right, parsed as an expression
    /// (an ident, a string, or a select chain for dotted paths).
    #[must_use]
    pub fn attrpath(&self) -> Option<Expr<'a>> {
        last_expr(self.syntax())
    }
}

/// One part of a string: literal text or an interpolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringPart<'a> {
    Content(&'a SyntaxToken),
    Interp(InterpExpr<'a>),
}

fn string_parts<'a>(node: &'a SyntaxNode) -> impl Iterator<Item = StringPart<'a>> + 'a {
    node.children().iter().filter_map(|c| match c {
        NodeOrToken::Token(t) if t.kind() == K::StringContent => Some(StringPart::Content(t)),
        NodeOrToken::Node(n) if n.kind() == K::InterpExpr => {
            Some(StringPart::Interp(InterpExpr(n)))
        }
        _ => None,
    })
}

impl<'a> StringExpr<'a> {
    /// The string parts in order: literal text and interpolations.
    pub fn parts(&self) -> impl Iterator<Item = StringPart<'a>> + 'a {
        string_parts(self.syntax())
    }
}

impl<'a> IndStringExpr<'a> {
    /// The string parts in order: literal text and interpolations.
    pub fn parts(&self) -> impl Iterator<Item = StringPart<'a>> + 'a {
        string_parts(self.syntax())
    }
}

impl<'a> InterpExpr<'a> {
    /// The interpolated expression.
    #[must_use]
    pub fn expr(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }
}

impl<'a> ParenExpr<'a> {
    /// The wrapped expression.
    #[must_use]
    pub fn expr(&self) -> Option<Expr<'a>> {
        first_expr(self.syntax())
    }
}
