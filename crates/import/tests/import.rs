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
fn luau_statements_expressions_and_layout() {
    let source = "--!strict\nlocal last: number? = nil\n\nlocal function f(cx: WindowCx): { [string]: boolean }\n\tif cx then\n\t\treturn { a = 1, [k] = \"x\" }\n\telseif not cx then\n\t\tcx:go(#t + 1) -- note\n\telse\n\t\treturn\n\tend\n\treturn {}\nend\n";
    assert_eq!(
        expression("luau", source, &[]).unwrap(),
        "[\n  (comment \"--!strict\")\n  (local (typed \"last\" (opt \"number\")) \"nil\")\n  blank\n  (localFunction \"f\" (typed [(typed \"cx\" \"WindowCx\")] (table [(pair \"string\" \"boolean\")])) [\n(ifElse \"cx\" [\n(return (table [(field \"a\" 1) (pair \"k\" (str \"x\"))]))\n] (ifElse (unary \"not\" \"cx\") [\n(trailing [\"cx:go\" (infix [(unary \"#\" \"t\") \"+\" 1])] \"-- note\")\n] [\n\"return\"\n]))\n(return (table []))\n])\n]"
    );
}

#[test]
fn luau_line_breaks_are_rows() {
    let source = "f(\n\tx,\n\t\"a\"\n\t\t.. b\n)\nlocal t = {\n\ta = '\"',\n\n\t\"\\u{2061}\",\n}\nlocal s = if a\n\tthen b\n\telse c\n";
    assert_eq!(
        expression("luau", source, &[]).unwrap(),
        "[\n  (rows [\"f\" \"x\" (rows (infix [(str \"a\") \"..\" \"b\"]))])\n  (local \"t\" (rows (table [\n(field \"a\" (str \"\\\"\"))\nblank\n(str [(unicode \"2061\")])\n])))\n  (local \"s\" (rows (ifExpr [[\"a\"] [\"b\"] [\"c\"]])))\n]"
    );
}

#[test]
fn luau_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("luau", "local x  = 1\n", &[])
        .unwrap_err()
        .contains("the printer writes"));
    assert!(expression("luau", "f \"x\"\n", &[])
        .unwrap_err()
        .contains("a call without parentheses"));
    assert!(expression("luau", "local s = 'x'\n", &[])
        .unwrap_err()
        .contains("the printer writes"));
    assert_eq!(expression("luau", "\n", &[]).unwrap(), "[blank]");
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

#[test]
fn markdown_blocks_inlines_and_blank_lines() {
    let source = "# Title\n\n<!-- note -->\n\n- one `x`\n- **two** [a](b)\n\n| A | B |\n|---|---|\n| *c* | d |\n";
    assert_eq!(
        expression("markdown", source, &[]).unwrap(),
        "[\n  (heading 1 \"Title\")\n  blank\n  (comment \" note \")\n  blank\n  (bullets [\n    [\"one \" (code \"x\")]\n    [(strong \"two\") \" \" (link \"a\" \"b\")]\n  ])\n  blank\n  (table [\n    [\"A\" \"B\"]\n    [[(em \"c\")] \"d\"]\n  ])\n]"
    );
}

#[test]
fn markdown_paragraphs_filled_to_the_width_are_wrap() {
    let words = "word ".repeat(20);
    let source = format!("{}\n{}\n", words[..74].trim_end(), words[75..].trim_end());
    assert_eq!(
        expression("markdown", &source, &[]).unwrap(),
        format!("[\n  (wrap {:?})\n]", words.trim_end())
    );
    assert!(expression("markdown", "a\nb\n", &[])
        .unwrap_err()
        .contains("not a fill"));
}

#[test]
fn markdown_it_cannot_reproduce_is_an_error() {
    assert!(expression("markdown", "Title\n=====\n", &[]).is_err());
    assert!(expression("markdown", "* star\n", &[]).is_err());
    assert!(expression("markdown", "- a\n\n- b\n", &[])
        .unwrap_err()
        .contains("loose"));
    assert!(expression("markdown", "__a__\n", &[]).is_err());
    assert!(expression("markdown", "a\n\n\n", &[]).is_err());
}
