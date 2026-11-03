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
fn ts_statements_expressions_and_layout() {
    let source = "import { a, type B } from \"x\";\n\nexport function f(p: B): number {\n\tif (!p) return 0;\n\tconst n = p.items\n\t\t.map((i) => i * 2)\n\t\t.filter(Boolean);\n\treturn n.length > 0 ? n[0] : -1;\n}\n";
    assert_eq!(
        expression("ts", source, &[]).unwrap(),
        "[\n  (import' [\"a\" (pre \"type\" \"B\")] (str \"x\"))\n  blank\n  (pre \"export\" (fn \"f\" (typed [(typed \"p\" \"B\")] \"number\") [\n(if' (pre \"!\" \"p\") (return 0))\n(const \"n\" (chain [\n\"p.items\"\n(call \"map\" [(arrow [\"i\"] (infix [\"i\" \"*\" 2]))])\n(call \"filter\" [\"Boolean\"])\n]))\n(return (ternary (infix [\"n.length\" \">\" 0]) (idx \"n\" 0) \"-1\"))\n]))\n]"
    );
}

#[test]
fn ts_broken_lists_operators_and_indentation_are_kept() {
    let source = "const x =\n\ta &&\n\tb;\nf(\n\t1,\n\t2,\n);\n";
    assert_eq!(
        expression("ts", source, &[]).unwrap(),
        "[\n  (const \"x\" (below (infix (rows [\n\"a\"\n\"&&\"\n\"b\"\n]))))\n  (call \"f\" (rows [\n1\n2\n]))\n]"
    );
    assert_eq!(
        expression("ts", "if (a) {\n  b();\n}\n", &[]).unwrap(),
        "(spaces 2 [\n  (if' \"a\" [\n(call \"b\" [])\n])\n])"
    );
}

#[test]
fn ts_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("ts", "f(a,  b);\n", &[])
        .unwrap_err()
        .contains("expected"));
    assert!(expression("ts", "a();  // note\n", &[]).is_err());
    assert!(expression("ts", "const x = a &&\n\tb;\n", &[]).is_err());
    assert!(expression("ts", "f(a,\n\tb);\n", &[]).is_err());
}
