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
fn vbs_statements_expressions_and_header_comments() {
    let source = "' Play.\n\nOption Explicit\nDim a, b\nSet a = CreateObject(\"X.Y\")\nIf a.Count <> 0 Then\n\t' Go.\n\tb = \"say \"\"hi\"\"\" & a.Run(1, True)\nElse\n\tWScript.Quit 1\nEnd If\n";
    assert_eq!(
        expression("vbs", source, &[]).unwrap(),
        "[\n  (comment \"' Play.\")\n  blank\n  (option \"Explicit\")\n  (dim [\"a\" \"b\"])\n  (set \"a\" [\"CreateObject\" (str \"X.Y\")])\n  (ifElse (op [\"a.Count\" \"<>\" \"0\"]) [\n(comment \"' Go.\")\n(assign \"b\" (op [(str \"say \\\"hi\\\"\") \"&\" [\"a.Run\" \"1\" \"True\"]]))\n] [\n[\"WScript.Quit\" \"1\"]\n])\n]"
    );
}

#[test]
fn vbs_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("vbs", "x  = 1\n", &[])
        .unwrap_err()
        .contains("would not give this layout back"));
    assert!(expression("vbs", "x = 1 ' one\n", &[])
        .unwrap_err()
        .contains("would not give this layout back"));
    assert!(expression("vbs", "If x Then\n    y\nEnd If\n", &[])
        .unwrap_err()
        .contains("line 2"));
}
