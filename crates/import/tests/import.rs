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
