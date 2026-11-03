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
fn patch_files_hunks_and_changes() {
    let source =
        "--- a/x.c\n+++ b/x.c\n@@ -1,2 +1,2 @@\n a\n-b\n+c\n@@ -9 +9,2 @@ int f() {\n \n+d\n";
    assert_eq!(
        data_file("patch", source, &[]).unwrap(),
        "{\n  add,\n  del,\n  file,\n  hunk,\n  section,\n  ...\n}: [\n  (file \"x.c\" [\n    (hunk 1 [\n      \"a\"\n      (del \"b\")\n      (add \"c\")\n    ])\n    (section 9 \"int f() {\" [\n      \"\"\n      (add \"d\")\n    ])\n  ])\n]\n"
    );
}

#[test]
fn patch_git_headers_are_kept() {
    let source = "diff --git a/y b/y\nindex 0123abc..4567def 100644\n--- a/y\n+++ b/y\n@@ -3 +3 @@\n-a\n+b\n";
    assert_eq!(
        expression("patch", source, &[]).unwrap(),
        "[\n  (git \"y\" \"0123abc..4567def 100644\" [\n    (hunk 3 [\n      (del \"a\")\n      (add \"b\")\n    ])\n  ])\n]"
    );
}

#[test]
fn patch_layout_the_printer_cannot_reproduce_is_an_error() {
    let hunk = "@@ -1 +1 @@\n-a\n+b\n";
    assert!(
        expression("patch", &format!("--- a/x\n+++ b/y\n{hunk}"), &[])
            .unwrap_err()
            .contains("paths differ")
    );
    assert!(
        expression("patch", "--- a/x\n+++ b/x\n@@ -1,1 +1 @@\n-a\n+b\n", &[])
            .unwrap_err()
            .contains("counts")
    );
    assert!(
        expression("patch", "--- a/x\n+++ b/x\n@@ -1 +2 @@\n-a\n+b\n", &[])
            .unwrap_err()
            .contains("new start")
    );
    assert!(expression(
        "patch",
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n\\ No newline at end of file\n+b\n",
        &[]
    )
    .unwrap_err()
    .contains("not context"));
    assert!(expression("patch", &format!("text\n--- a/x\n+++ b/x\n{hunk}"), &[]).is_err());
}
