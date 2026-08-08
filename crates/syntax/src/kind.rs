//! Token and (later) node kinds. One enum covers the whole syntax
//! vocabulary: tokens now, tree nodes in M2.

/// Every kind of token (and later, node) in the Nix syntax vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    // --- trivia ---
    Whitespace,
    Comment,

    // --- names and literals ---
    Ident,
    KwIf,
    KwThen,
    KwElse,
    KwAssert,
    KwWith,
    KwLet,
    KwIn,
    KwRec,
    KwInherit,
    KwOr,
    Int,
    Float,
    Path,
    SearchPath,
    Uri,

    // --- string parts ---
    /// `"`
    StringStart,
    /// `''`
    IndStringStart,
    StringContent,
    /// `${`
    InterpStart,
    /// `}` closing an interpolation
    InterpEnd,
    /// closing `"`
    StringEnd,
    /// closing `''`
    IndStringEnd,

    // --- punctuation and operators ---
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Semicolon,
    Comma,
    /// `=`
    Assign,
    /// `==`
    EqEq,
    /// `!=`
    Neq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    /// `&&`
    AndAnd,
    /// `||`
    OrOr,
    /// `!`
    Bang,
    /// `++`
    PlusPlus,
    Plus,
    Minus,
    Star,
    /// `/` (division)
    Slash,
    /// `//` (attrset merge)
    SlashSlash,
    /// `->`
    Arrow,
    Dot,
    /// `...`
    Ellipsis,
    /// `?`
    Question,
    /// `@`
    At,
    Colon,

    // --- tree nodes ---
    /// Top-level node wrapping one whole file.
    Root,
    /// A malformed region produced by error recovery.
    ErrorNode,

    /// `let bindings in body`
    LetExpr,
    /// The binding section of a `let`.
    LetBindings,
    /// One `attrpath = expr;` binding (also used in attrsets).
    Binding,
    /// A dotted path of attribute names, e.g. `a.b."c".d`.
    Attrpath,
    /// An `inherit (from) a b;` statement.
    InheritStmt,

    /// `with expr; body`
    WithExpr,
    /// `assert cond; body`
    AssertExpr,
    /// `if cond then then-e else else-e`
    IfExpr,

    /// `{ ... }`
    AttrsetExpr,
    /// `rec { ... }`
    RecAttrsetExpr,
    /// Lambda formal parameters `{ a, b ? 1, ... }`.
    Formals,
    /// `[ ... ]`
    ListExpr,
    /// `params: body`
    LambdaExpr,

    /// Function application `f x`.
    ApplyExpr,
    /// Prefix `! e`.
    UnaryExpr,
    /// Infix `a op b` (arithmetic, comparison, logic).
    BinExpr,
    /// `e.attrpath` (`or default` included).
    SelectExpr,
    /// `e ? attrpath`.
    HasAttrExpr,

    /// `"..."`
    StringExpr,
    /// `''...''`
    IndStringExpr,
    /// `\${expr}` inside a string.
    InterpExpr,
    /// `( expr )`
    ParenExpr,

    // --- bookkeeping ---
    Error,
    Eof,
}

impl SyntaxKind {
    /// Trivia tokens carry no meaning but must be preserved for
    /// lossless round-trips and faithful fixes.
    #[must_use]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comment)
    }

    /// Whether this token kind can serve as an attribute-name element:
    /// a name in a dotted path, a numeric key like 1 in { 1 = "x"; },
    /// or keyword names like if in { if = 1; }.
    #[must_use]
    pub fn is_attr_name(self) -> bool {
        matches!(
            self,
            Self::Ident
                | Self::Int
                | Self::KwIf
                | Self::KwThen
                | Self::KwElse
                | Self::KwAssert
                | Self::KwWith
                | Self::KwLet
                | Self::KwIn
                | Self::KwRec
                | Self::KwInherit
                | Self::KwOr
        )
    }

    /// Whether this token kind is an infix operator.
    #[must_use]
    pub fn is_binop(self) -> bool {
        matches!(
            self,
            Self::Arrow
                | Self::OrOr
                | Self::AndAnd
                | Self::EqEq
                | Self::Neq
                | Self::Lt
                | Self::LtEq
                | Self::Gt
                | Self::GtEq
                | Self::PlusPlus
                | Self::Plus
                | Self::Minus
                | Self::Star
                | Self::Slash
                | Self::SlashSlash
        )
    }
}
