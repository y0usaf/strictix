#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    Whitespace,
    Comment,

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

        StringStart,
        IndStringStart,
    StringContent,
        InterpStart,
        InterpEnd,
        StringEnd,
        IndStringEnd,

    LBrace,
    RBrace,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Semicolon,
    Comma,
        Assign,
        EqEq,
        Neq,
    Lt,
    LtEq,
    Gt,
    GtEq,
        AndAnd,
        OrOr,
        Bang,
        PlusPlus,
    Plus,
    Minus,
    Star,
        Slash,
        SlashSlash,
        Arrow,
    Dot,
        Ellipsis,
        Question,
        At,
    Colon,

        Root,
        ErrorNode,

        LetExpr,
        LetBindings,
        Binding,
        Attrpath,
        InheritStmt,

        WithExpr,
        AssertExpr,
        IfExpr,

        AttrsetExpr,
        RecAttrsetExpr,
        Formals,
        ListExpr,
        LambdaExpr,

        ApplyExpr,
        UnaryExpr,
        BinExpr,
        SelectExpr,
        HasAttrExpr,

        StringExpr,
        IndStringExpr,
        InterpExpr,
        ParenExpr,

    Error,
    Eof,
}

impl SyntaxKind {
        #[must_use]
    pub fn is_trivia(self) -> bool {
        matches!(self, Self::Whitespace | Self::Comment)
    }

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
