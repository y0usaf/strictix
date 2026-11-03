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
fn javascript_statements_expressions_and_layout() {
    let source = "#!/usr/bin/env node\n\nimport { join } from \"node:path\";\n\nconst { a, b } = x;\nlet n;\nasync function f(s) {\n  if (!s) return;\n  try { n = await g(s, -1); } catch {}\n  const o = { k: [1, \"v\"], \"q-r\": {\n    m: (n = s.t?.u) >= 0\n  }};\n  return o.k.length +\n    typeof n;\n}\n";
    assert_eq!(
        expression("javascript", source, &[]).unwrap(),
        "[\n  (comment \"#!/usr/bin/env node\")\n  blank\n  (import' [\"join\"] \"node:path\")\n  blank\n  (const (obj [\"a\" \"b\"]) \"x\")\n  (let' \"n\" null)\n  (async (function \"f\" [\"s\"] [\n(if' (not \"s\") (return null))\n(try' (inline [(op [\"n\" \"=\" (await [\"g\" \"s\" (-1)])])]) null (inline []))\n(const \"o\" (obj [[\"k\" (array [1 (str \"v\")])] [(str \"q-r\") (obj (rows [\n[\"m\" (op [(paren (op [\"n\" \"=\" \"s.t?.u\"])) \">=\" 0])]\n]))]]))\n(return (op (rows [\n\"o.k.length\"\n\"+\"\n(typeof \"n\")\n])))\n]))\n]"
    );
}

#[test]
fn javascript_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("javascript", "f(a,b);\n", &[])
        .unwrap_err()
        .contains("line 1"));
    assert!(expression("javascript", "const o = {a: 1};\n", &[]).is_err());
    assert!(expression("javascript", "if (a) f();\n", &[])
        .unwrap_err()
        .contains("without braces"));
    assert!(expression("javascript", "const s = 'x';\n", &[])
        .unwrap_err()
        .contains("single-quoted"));
}

#[test]
fn javascript_data_file_leaves_null_to_nix() {
    assert_eq!(
        data_file("javascript", "let n;\n", &[]).unwrap(),
        "{\n  let',\n  ...\n}: [\n  (let' \"n\" null)\n]\n"
    );
}
