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
fn python_statements_expressions_and_layout() {
    let source = "import os\n\n\ndef f(a, b=1):\n    if a:\n        return os.path.join(a, f\"{b:03d}\")\n    elif b:\n        x = g(a,\n              b)\n    else:\n        pass\n";
    assert_eq!(
        expression("python", source, &[]).unwrap(),
        "[\n  [\"import\" \"os\"]\n  blank\n  blank\n  (def \"f\" [\"a\" (kw \"b\" 1)] [\n    (ifElse \"a\" [\n      [\"return\" (call \"os.path.join\" [\"a\" (str [(spec \"b\" \"03d\")])])]\n    ] (ifElse \"b\" [\n      [\"x\" \"=\" (call \"g\" (aligned [\"a\" \"b\"]))]\n    ] [\n      \"pass\"\n    ]))\n  ])\n]"
    );
}

#[test]
fn python_hanging_brackets_slices_and_continuations() {
    let source =
        "x = f(\n    a,\n    b,\n)\ny = g(\n    a, b\n)\nz = s[a + 1 :] + s[1:] \\\n    + t\n";
    assert_eq!(
        expression("python", source, &[]).unwrap(),
        "[\n  [\"x\" \"=\" (call \"f\" (rows [\"a\" \"b\"]))]\n  [\"y\" \"=\" (call \"g\" (rows [[\"a\" \"b\"]]))]\n  (rows [[\"z\" \"=\" (index \"s\" (slice [\"a\" \"+\" 1] null)) \"+\" (index \"s\" (slice 1 null))] [\"+\" \"t\"]])\n]"
    );
}

#[test]
fn python_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("python", "x  = 1\n", &[])
        .unwrap_err()
        .contains("not one space"));
    assert!(expression("python", "f(a, b,)\n", &[])
        .unwrap_err()
        .contains("trailing comma"));
    assert!(expression("python", "x = 1  # note\n", &[]).is_err());
    assert!(expression("python", "s[a+1:]\n", &[]).is_err());
}

#[test]
fn python_template_placeholders_stay_names() {
    assert_eq!(
        data_file("python", "x = @owners@.get(k, '')\n", &[]).unwrap(),
        "{\n  call,\n  sq,\n  ...\n}: [\n  [\"x\" \"=\" (call \"@owners@.get\" [\"k\" (sq \"\")])]\n]\n"
    );
}
