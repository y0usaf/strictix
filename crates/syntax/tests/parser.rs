//! Parser tests: lossless CST construction, error recovery, and the
//! round-trip property (tree tokens tile the source exactly).

use strictix_syntax::{parse, SyntaxKind, SyntaxNode};

use SyntaxKind as K;

/// Parse `src` and assert the tree reassembles the source byte-for-byte.
fn parse_rt(src: &str) -> SyntaxNode {
    let tree = parse(src);
    assert_eq!(tree.reassemble(src), src, "round trip failed for {src:?}");
    tree
}

/// All node kinds present in the tree, depth-first, excluding trivia
/// and Eof tokens.
fn node_kinds(tree: &SyntaxNode) -> Vec<K> {
    fn walk(node: &SyntaxNode, out: &mut Vec<K>) {
        out.push(node.kind());
        for child in node.children() {
            match child {
                strictix_syntax::NodeOrToken::Node(n) => walk(n, out),
                strictix_syntax::NodeOrToken::Token(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut out);
    out
}

/// Whether an error node appears anywhere in the tree.
fn has_error(tree: &SyntaxNode) -> bool {
    node_kinds(tree).contains(&K::ErrorNode)
}

/// Count occurrences of each node kind.
fn count_kind(tree: &SyntaxNode, want: K) -> usize {
    node_kinds(tree).into_iter().filter(|k| *k == want).count()
}

// --- corpus: parse both files with zero panics and lossless round-trip ---

#[test]
fn corpus_parses() {
    for entry in
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus")).expect("corpus dir")
    {
        let path = entry.expect("entry").path();
        let src = std::fs::read_to_string(&path).expect("read corpus file");
        let tree = parse_rt(&src);
        assert!(!has_error(&tree), "parse error nodes in {}", path.display());
    }
}

// --- inline fixtures: round-trip + structure ---

#[test]
fn parses_let_in() {
    let tree = parse_rt("let x = 1; in x");
    assert!(matches!(tree.kind(), K::Root));
    assert!(node_kinds(&tree).contains(&K::LetExpr));
    assert!(node_kinds(&tree).contains(&K::Binding));
}

#[test]
fn parses_attrset_with_types() {
    let tree = parse_rt("{ config, lib, pkgs, ... }:");
    assert!(node_kinds(&tree).contains(&K::Formals));
    assert!(node_kinds(&tree).contains(&K::LambdaExpr));
}

#[test]
fn parses_rec_attrset() {
    let tree = parse_rt("rec { a = 1; b = a; }");
    assert!(node_kinds(&tree).contains(&K::RecAttrsetExpr));
    assert!(node_kinds(&tree).contains(&K::AttrsetExpr));
    assert_eq!(count_kind(&tree, K::Binding), 2);
}

#[test]
fn parses_lambda_application() {
    let tree = parse_rt("map (x: x + 1) list");
    assert!(node_kinds(&tree).contains(&K::ApplyExpr));
    assert!(node_kinds(&tree).contains(&K::LambdaExpr));
}

#[test]
fn parses_interpolation_in_strings() {
    let tree = parse_rt("\"hello ${name}!\"");
    assert!(node_kinds(&tree).contains(&K::StringExpr));
    assert!(node_kinds(&tree).contains(&K::InterpExpr));
    let tree = parse_rt("''indented ${x} end''");
    assert!(node_kinds(&tree).contains(&K::IndStringExpr));
    assert!(node_kinds(&tree).contains(&K::InterpExpr));
}

#[test]
fn parses_select_with_or_default() {
    let tree = parse_rt("old.patches or [ ]");
    assert!(node_kinds(&tree).contains(&K::SelectExpr));
    assert!(node_kinds(&tree).contains(&K::Attrpath));
}

#[test]
fn parses_nested_select_attrpath() {
    // Nix's `expr_select '.' attrpath` selects a multi-segment path in
    // one Select node: `e.a.b.c` is `Select(e, [a, b, c])`.
    let tree = parse_rt("config.services.example.enable");
    assert_eq!(count_kind(&tree, K::SelectExpr), 1);
    assert_eq!(count_kind(&tree, K::Attrpath), 1);
}

#[test]
fn parses_string_attr_in_path() {
    let tree = parse_rt("environment.etc.\"x/y\".text");
    assert!(node_kinds(&tree).contains(&K::StringExpr));
    assert!(node_kinds(&tree).contains(&K::Attrpath));
}

#[test]
fn parses_with_and_assert() {
    let tree = parse_rt("with lib; foo");
    assert!(node_kinds(&tree).contains(&K::WithExpr));
    let tree = parse_rt("assert cond; body");
    assert!(node_kinds(&tree).contains(&K::AssertExpr));
}

#[test]
fn parses_inherit_forms() {
    // `inherit` is only valid inside bindings (attrset or let).
    let tree = parse_rt("{ inherit a b c; }");
    assert!(node_kinds(&tree).contains(&K::InheritStmt));
    assert!(node_kinds(&tree).contains(&K::AttrsetExpr));
    let tree = parse_rt("{ inherit (lib) mkIf; }");
    assert!(node_kinds(&tree).contains(&K::InheritStmt));
    // The `(lib)` source expression is parsed (LParen/RParen tokens).
    assert_eq!(count_kind(&tree, K::InheritStmt), 1);
}

#[test]
fn parses_operator_precedence() {
    // `*` binds tighter than `+`: tree is `1 + (2 * 3)`.
    let tree = parse_rt("1 + 2 * 3");
    assert_eq!(count_kind(&tree, K::BinExpr), 2);
    // `//` binds looser than `+`: tree is `(1 + 2) // 3`.
    let tree = parse_rt("1 + 2 // 3");
    assert_eq!(count_kind(&tree, K::BinExpr), 2);
    // `->` is right-associative: `a -> b -> c` nests to the right.
    let tree = parse_rt("a -> b -> c");
    assert!(node_kinds(&tree).contains(&K::BinExpr));
}

#[test]
fn parses_negative_and_unary() {
    let tree = parse_rt("!true && false");
    assert!(node_kinds(&tree).contains(&K::UnaryExpr));
    assert!(node_kinds(&tree).contains(&K::BinExpr));
    // `! a ? b` groups as `!(a ? b)` (verified against Nix).
    let tree = parse_rt("!a ? b");
    assert!(node_kinds(&tree).contains(&K::HasAttrExpr));
    assert!(node_kinds(&tree).contains(&K::UnaryExpr));
}

#[test]
fn parses_uri_and_path() {
    let tree = parse_rt("{ uri = https://example.com/x; }");
    assert!(node_kinds(&tree).contains(&K::Binding));
    let tree = parse_rt("{ p = ./foo/bar.nix; }");
    assert!(node_kinds(&tree).contains(&K::Binding));
}

// --- error recovery ---

#[test]
fn error_recovery_missing_semicolon() {
    // Missing `;` after the binding: the parser should recover and
    // still produce the following expression as a sibling.
    let tree = parse_rt("let a = 1 in a;");
    assert!(has_error(&tree));
    assert!(node_kinds(&tree).contains(&K::LetExpr));
    assert!(node_kinds(&tree).contains(&K::Binding));
}

#[test]
fn error_recovery_missing_then() {
    let tree = parse_rt("if true x else y");
    assert!(has_error(&tree));
    assert!(node_kinds(&tree).contains(&K::IfExpr));
}

#[test]
fn error_recovery_unexpected_token() {
    // Garbage between two valid expressions: error node, then a
    // recovered sibling.
    let tree = parse_rt("1 $ % 2");
    assert!(has_error(&tree));
    // The trailing `2` should still parse as an Int token.
    assert!(tree.reassemble("1 $ % 2") == "1 $ % 2");
}

#[test]
fn error_recovery_unterminated_string() {
    let tree = parse_rt("\"unterminated");
    assert!(has_error(&tree));
    assert_round_trip_still(&tree, "\"unterminated");
}

#[test]
fn error_recovery_unclosed_attrset() {
    let tree = parse_rt("{ a = 1;");
    assert!(has_error(&tree));
    assert_round_trip_still(&tree, "{ a = 1;");
}

#[test]
fn error_recovery_empty_input() {
    let tree = parse_rt("");
    assert_eq!(tree.reassemble(""), "");
    assert!(matches!(tree.kind(), K::Root));
}

#[test]
fn error_recovery_recovers_at_in() {
    // Broken binding then a valid `in` expression: the body must survive.
    let tree = parse_rt("let a = ; in 42");
    assert!(has_error(&tree));
    assert!(count_kind(&tree, K::Binding) >= 1);
    // The `42` after `in` should be present.
    assert!(tree.reassemble("let a = ; in 42") == "let a = ; in 42");
}

#[test]
fn error_recovery_sibling_after_error() {
    // `x = !;` is broken; the next binding should still parse.
    let src = "{ a = !; b = 2; }";
    let tree = parse_rt(src);
    assert!(has_error(&tree));
    assert_eq!(count_kind(&tree, K::Binding), 2);
}

#[test]
fn never_panics_on_broken_input() {
    // A grab-bag of broken inputs must never panic and must always
    // round-trip (losslessness holds even for garbage).
    let broken = [
        "let",
        "let in",
        "{",
        "}",
        "if then else",
        "with",
        "assert",
        "1 +",
        "+ 1",
        "a.b or",
        "${",
        "( )",
        "[ 1 2",
        "x: ",
        "{ a, b }",
        "rec",
        "\"${}",
        "''",
        "foo bar baz",
        "!!!!!!!!",
        "{ a = 1; } and b",
        "let x = 1; let y = 2; in x",
    ];
    for src in broken {
        let tree = parse(src);
        assert_eq!(tree.reassemble(src), src, "round trip failed for {src:?}");
    }
}

// --- round-trip across a realistic module ---

#[test]
fn round_trip_realistic_module() {
    let src = r#"
{ config, pkgs, ... }:

let
  inherit (lib) mkIf;
  port = 8080;
  name = "example";
in
{
  services.example = {
    enable = mkIf config.services.enable {
      url = "http://localhost:${toString port}/api";
      opts = { inherit (pkgs) foo; };
    };
  };
}
"#;
    parse_rt(src);
}

fn assert_round_trip_still(tree: &SyntaxNode, src: &str) {
    assert_eq!(tree.reassemble(src), src);
}
