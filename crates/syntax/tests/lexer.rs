//! Lexer tests: token kinds, interpolation mode-stack, Nix lexing
//! quirks, and the lossless round-trip property.

use strictix_syntax::{lex, SyntaxKind};

use SyntaxKind as K;

/// Token kinds for `src`, trivia and Eof excluded.
fn kinds(src: &str) -> Vec<K> {
    lex(src)
        .into_iter()
        .filter(|t| !t.kind.is_trivia() && t.kind != K::Eof)
        .map(|t| t.kind)
        .collect()
}

/// Concatenating every token's text must reproduce the source exactly.
fn assert_round_trip(src: &str) {
    let tokens = lex(src);
    let mut rebuilt = String::new();
    for token in &tokens {
        rebuilt.push_str(token.text(src));
    }
    assert_eq!(rebuilt, src, "round trip failed");
    // Tokens must tile the source with no gaps or overlaps.
    let mut offset = 0;
    for token in &tokens {
        assert_eq!(token.start as usize, offset);
        offset += token.len as usize;
    }
    assert_eq!(offset, src.len());
}

#[test]
fn simple_let_in() {
    assert_eq!(
        kinds("let x = 1; in x"),
        [
            K::KwLet,
            K::Ident,
            K::Assign,
            K::Int,
            K::Semicolon,
            K::KwIn,
            K::Ident
        ]
    );
}

#[test]
fn keywords_and_lookalikes() {
    assert_eq!(
        kinds("if then else assert with let in rec inherit or"),
        [
            K::KwIf,
            K::KwThen,
            K::KwElse,
            K::KwAssert,
            K::KwWith,
            K::KwLet,
            K::KwIn,
            K::KwRec,
            K::KwInherit,
            K::KwOr
        ]
    );
    // Idents that merely start with keywords stay idents.
    assert_eq!(
        kinds("inheritance oracle letter"),
        [K::Ident, K::Ident, K::Ident]
    );
}

#[test]
fn numbers() {
    assert_eq!(kinds("42"), [K::Int]);
    assert_eq!(kinds("1.5"), [K::Float]);
    assert_eq!(kinds("2.0e3"), [K::Float]);
    assert_eq!(kinds("1.5e-3"), [K::Float]);
    // No digits after the dot: Int then Dot.
    assert_eq!(kinds("1."), [K::Int, K::Dot]);
    // Nix floats require a decimal point: `1e3` is Int + Ident.
    assert_eq!(kinds("1e3"), [K::Int, K::Ident]);
}

#[test]
fn paths() {
    assert_eq!(kinds("./foo/bar.nix"), [K::Path]);
    assert_eq!(kinds("../up/dir"), [K::Path]);
    assert_eq!(kinds("/abs/path"), [K::Path]);
    assert_eq!(kinds("~/home/dir"), [K::Path]);
    assert_eq!(kinds("foo/bar"), [K::Path]);
    assert_eq!(kinds("<nixpkgs>"), [K::SearchPath]);
    assert_eq!(kinds("<nixpkgs/nixos>"), [K::SearchPath]);
}

#[test]
fn division_needs_spaces() {
    // `a/b` is a path; `a / b` is division; `a // b` is attrset merge.
    assert_eq!(kinds("a/b"), [K::Path]);
    assert_eq!(kinds("a / b"), [K::Ident, K::Slash, K::Ident]);
    assert_eq!(kinds("a // b"), [K::Ident, K::SlashSlash, K::Ident]);
    assert_eq!(
        kinds("{ } // { }"),
        [K::LBrace, K::RBrace, K::SlashSlash, K::LBrace, K::RBrace]
    );
}

#[test]
fn operators() {
    assert_eq!(
        kinds("== != <= >= < > && || ! ++ + - * -> . ... ? @ : ; , ="),
        [
            K::EqEq,
            K::Neq,
            K::LtEq,
            K::GtEq,
            K::Lt,
            K::Gt,
            K::AndAnd,
            K::OrOr,
            K::Bang,
            K::PlusPlus,
            K::Plus,
            K::Minus,
            K::Star,
            K::Arrow,
            K::Dot,
            K::Ellipsis,
            K::Question,
            K::At,
            K::Colon,
            K::Semicolon,
            K::Comma,
            K::Assign,
        ]
    );
}

#[test]
fn comments() {
    assert_eq!(kinds("1 # trailing\n2"), [K::Int, K::Int]);
    assert_eq!(kinds("/* block */ 1"), [K::Int]);
    assert_eq!(kinds("/* multi\nline */ 1"), [K::Int]);
}

#[test]
fn uri_quirk() {
    // `x: body` is a lambda; `x:1` is a URI (longest match wins).
    assert_eq!(kinds("x: body"), [K::Ident, K::Colon, K::Ident]);
    assert_eq!(kinds("http://example.com/x"), [K::Uri]);
    assert_eq!(kinds("x:1"), [K::Uri]);
}

#[test]
fn plain_string() {
    assert_eq!(
        kinds("\"hello\""),
        [K::StringStart, K::StringContent, K::StringEnd]
    );
}

#[test]
fn string_interpolation() {
    let src = "\"hello ${name}!\"";
    assert_eq!(
        kinds(src),
        [
            K::StringStart,
            K::StringContent,
            K::InterpStart,
            K::Ident,
            K::InterpEnd,
            K::StringContent,
            K::StringEnd
        ]
    );
    let texts: Vec<String> = lex(src)
        .into_iter()
        .filter(|t| !t.kind.is_trivia() && t.kind != K::Eof)
        .map(|t| t.text(src).to_string())
        .collect();
    assert_eq!(texts, ["\"", "hello ", "${", "name", "}", "!", "\""]);
}

#[test]
fn string_escaped_interpolation_is_content() {
    let src = "\"a \\${b}\"";
    assert_eq!(kinds(src), [K::StringStart, K::StringContent, K::StringEnd]);
}

#[test]
fn nested_interpolation() {
    // Interpolation inside an attrset inside a string inside an interp.
    let src = "\"${ { a = \"x${y}z\"; }.a }\"";
    assert_eq!(
        kinds(src),
        [
            K::StringStart,
            K::InterpStart,
            K::LBrace,
            K::Ident,
            K::Assign,
            K::StringStart,
            K::StringContent,
            K::InterpStart,
            K::Ident,
            K::InterpEnd,
            K::StringContent,
            K::StringEnd,
            K::Semicolon,
            K::RBrace,
            K::Dot,
            K::Ident,
            K::InterpEnd,
            K::StringEnd
        ]
    );
}

#[test]
fn indented_string() {
    let src = "''\n  hello ${name}\n  ''";
    assert_eq!(
        kinds(src),
        [
            K::IndStringStart,
            K::StringContent,
            K::InterpStart,
            K::Ident,
            K::InterpEnd,
            K::StringContent,
            K::IndStringEnd
        ]
    );
}

#[test]
fn indented_string_escapes() {
    // `'''` is a literal quote, `''$` a literal dollar, `''\n` an
    // escape: none of them end the string or start interpolation.
    // Verified against real Nix: `''a'''b''${c}''\n d''` evaluates to
    // `a''b${c}\n d` with no interpolation.
    let src = "''a'''b''${c}''\\n d''";
    assert_eq!(
        kinds(src),
        [K::IndStringStart, K::StringContent, K::IndStringEnd]
    );
}

#[test]
fn indented_string_single_quote_content() {
    let src = "''it's fine''";
    assert_eq!(
        kinds(src),
        [K::IndStringStart, K::StringContent, K::IndStringEnd]
    );
}

#[test]
fn unterminated_string_yields_error() {
    let tokens = lex("\"open");
    assert!(tokens.iter().any(|t| t.kind == K::Error));
}

#[test]
fn unterminated_block_comment_is_comment_to_eof() {
    let src = "/* never closed";
    assert_eq!(kinds(src), []);
    assert_round_trip(src);
}

#[test]
fn interpolation_brace_tracking() {
    // Braces inside the interpolated attrset must not close the interp.
    let src = "\"${{ a = 1; }.a}\"";
    assert_eq!(
        kinds(src),
        [
            K::StringStart,
            K::InterpStart,
            K::LBrace,
            K::Ident,
            K::Assign,
            K::Int,
            K::Semicolon,
            K::RBrace,
            K::Dot,
            K::Ident,
            K::InterpEnd,
            K::StringEnd
        ]
    );
}

#[test]
fn round_trip_mixed_source() {
    let src = "{ a, b ? 1, ... }@args: let x = ./foo/bar; in { c = \"${x}\"; /* c */ d = a or 2; }";
    assert_round_trip(src);
}

#[test]
fn round_trip_corpus() {
    for entry in
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus")).expect("corpus dir")
    {
        let path = entry.expect("entry").path();
        let src = std::fs::read_to_string(&path).expect("read corpus file");
        assert_round_trip(&src);
        let tokens = lex(&src);
        assert!(
            !tokens.iter().any(|t| t.kind == K::Error),
            "lexer error in {}",
            path.display()
        );
    }
}
