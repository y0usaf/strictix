use strictix_import::{data_file, expression};

#[test]
fn lisp_lists_are_nix_lists_and_breaks_are_rows() {
    let source = "(in-package #:user)\n\n(defun f (x)\n  \"Doc.\"\n  (list 'x #'car -1 2.5))\n";
    assert_eq!(
        expression("lisp", source, &[]).unwrap(),
        "[\n  [\"in-package\" \"#:user\"]\n  blank\n  (rows [\n    [\"defun\" \"f\" [\"x\"]]\n    (str \"Doc.\")\n    [[\"list\" (quote \"x\") (function \"car\") (-1) \"2.5\"]]\n  ])\n]"
    );
}

#[test]
fn lisp_indentation_off_the_rules_is_kept_with_at() {
    let source = "(foo a\n       b)\n";
    assert_eq!(
        expression("lisp", source, &[]).unwrap(),
        "[\n  (rows [\n    [\"foo\" \"a\"]\n    (at 7 \"b\")\n  ])\n]"
    );
}

#[test]
fn lisp_spacing_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("lisp", "(a  b)\n", &[])
        .unwrap_err()
        .contains("separated by"));
    assert!(expression("lisp", "(a) (b)\n", &[])
        .unwrap_err()
        .contains("share a line"));
}

#[test]
fn shell_statements_words_and_layout() {
    let source = "set -eu\n\nf() { echo \"$1\" >&2; exit 1; }\nif [ -n \"$x\" ]; then\n  f \"$(date)\"\nfi\n";
    assert_eq!(
        expression("shell", source, &[]).unwrap(),
        "[\n  [\"set\" \"-eu\"]\n  blank\n  (fn \"f\" (inline [[\"echo\" (dq \"$1\") (redir \">&\" \"2\")] [\"exit\" \"1\"]]))\n  (if' [\"[\" \"-n\" (dq \"$x\") \"]\"] [\n[\"f\" (dq [(sub [[\"date\"]])])]\n])\n]"
    );
}

#[test]
fn holes_become_interpolations() {
    let hole = char::from_u32(strictix_import::HOLE_BASE).unwrap();
    let source = format!("printf '{hole}'\n");
    assert_eq!(
        data_file("shell", &source, &["values.foreground".to_owned()]).unwrap(),
        "{\n  sq,\n  ...\n}: [\n  [\"printf\" (sq \"${values.foreground}\")]\n]\n"
    );
}

#[test]
fn a_data_file_takes_the_constructors_it_uses() {
    assert_eq!(
        data_file("lisp", "(a b)\n", &[]).unwrap(),
        "_: [\n  [\"a\" \"b\"]\n]\n"
    );
    assert_eq!(
        data_file("lisp", "(a\n \"b\")\n", &[]).unwrap(),
        "{\n  rows,\n  str,\n  ...\n}: [\n  (rows [\n    \"a\"\n    (str \"b\")\n  ])\n]\n"
    );
}

#[test]
fn lua_calls_are_nix_lists_and_blocks_keep_their_layout() {
    let source = "local t = { a = 1, \"x\", -1 }\nif not t then return end\n\nfor i = 1, #t do f:write(t[i]); break end\n";
    assert_eq!(
        expression("lua", source, &[]).unwrap(),
        "[\n  (local \"t\" [(table [(field \"a\" 1) (str \"x\") (-1)])])\n  (if' (not \"t\") (inline [(return [])]))\n  blank\n  (forNum \"i\" [1 (len \"t\")] (inline [[\"f:write\" (index \"t\" \"i\")] \";\" \"break\"]))\n]"
    );
}

#[test]
fn lua_broken_conditions_and_tables_are_rows() {
    let source = "if a\n  and b == nil then\n  x = {\n    { 1 }, { 2 },\n    p{ q = true },\n  }\nelseif c then y() else z() end\n";
    assert_eq!(
        expression("lua", source, &[]).unwrap(),
        "[\n  (ifElse (op (rows [[\"a\"] [\"and\" (op [\"b\" \"==\" null])]])) [\n(assign \"x\" [(table (rows [\n[(table [1]) (table [2])]\n(apply \"p\" (table [(field \"q\" true)]))\n]))])\n] (ifElse \"c\" (inline [[\"y\"]]) (inline [[\"z\"]])))\n]"
    );
}

#[test]
fn lua_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("lua", "f(a,b)\n", &[])
        .unwrap_err()
        .contains("expected"));
    assert!(expression("lua", "f() g()\n", &[])
        .unwrap_err()
        .contains("share a line"));
    assert!(expression("lua", "x = [[long]]\n", &[])
        .unwrap_err()
        .contains("long bracket"));
    assert!(expression("lua", "local t = { a = 1 }\n", &[]).is_ok());
    assert!(expression("lua", "local t = {\n  a = 1\n}\n", &[]).is_err());
}

#[test]
fn lua_data_file_formals_leave_out_nix_literals() {
    assert_eq!(
        data_file("lua", "x = true\n", &[]).unwrap(),
        "{\n  assign,\n  ...\n}: [\n  (assign \"x\" [true])\n]\n"
    );
}
