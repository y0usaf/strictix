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
fn glsl_statements_are_word_lists_and_calls_are_nodes() {
    let source = "#define SCALE(c) ((c) / 255.0)\nconst float K = -2.0;\n\nfloat f(vec2 p, float k) {\n    float c = cos(k), s = sin(k);\n    if (k < 0.0) return -(p.x + k);\n    else if (k > 1.0) { c = 1.0; s = 0.0; }\n    for (int i = 0; i < 3; i++) {\n        c += p[i] * float(i);\n    }\n    return c * s;\n}\n";
    assert_eq!(
        expression("glsl", source, &[]).unwrap(),
        "[\n  (define (call \"SCALE\" [\"c\"]) [[[\"c\"] \"/\" \"255.0\"]])\n  [\"const\" \"float\" \"K\" \"=\" \"-2.0\"]\n  blank\n  (fn [\"float\" (call \"f\" [[\"vec2\" \"p\"] [\"float\" \"k\"]])] [\n    [\"float\" \"c\" \"=\" (call \"cos\" [\"k\"]) \",\" \"s\" \"=\" (call \"sin\" [\"k\"])]\n    (ifElse [\"k\" \"<\" \"0.0\"] (bare [\"return\" (unary \"-\" [\"p.x\" \"+\" \"k\"])]) (if' [\"k\" \">\" \"1.0\"] (inline [[\"c\" \"=\" \"1.0\"] [\"s\" \"=\" \"0.0\"]])))\n    (for [\"int\" \"i\" \"=\" 0] [\"i\" \"<\" 3] \"i++\" [\n      [\"c\" \"+=\" (index \"p\" \"i\") \"*\" (call \"float\" [\"i\"])]\n    ])\n    [\"return\" \"c\" \"*\" \"s\"]\n  ])\n]"
    );
}

#[test]
fn glsl_broken_lines_are_rows() {
    let source = "float a = b + c\n        - d;\nvec3 v = mix(x,\n             y);\nconst vec2 P[2] = vec2[2](\n    vec2(0.0), vec2(1.0)\n);\n";
    assert_eq!(
        expression("glsl", source, &[]).unwrap(),
        "[\n  (rows [\n    [\"float\" \"a\" \"=\" \"b\" \"+\" \"c\"]\n    [\"-\" \"d\"]\n  ])\n  [\"vec3\" \"v\" \"=\" (call \"mix\" (rows [\n    [\"x\"]\n    [\"y\"]\n  ]))]\n  [\"const\" \"vec2\" (index \"P\" 2) \"=\" (call (index \"vec2\" 2) (rows [\n    []\n    [(call \"vec2\" [\"0.0\"]) (call \"vec2\" [\"1.0\"])]\n  ]))]\n]"
    );
    assert!(expression("glsl", "float a = b\n  + c;\n", &[])
        .unwrap()
        .contains("(at 2 [\"+\" \"c\"])"));
}

#[test]
fn glsl_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("glsl", "float a  = 1.0;\n", &[]).is_err());
    assert!(expression("glsl", "float f() {\n  return 1.0;\n}\n", &[]).is_err());
    assert!(expression("glsl", "float a = f(b , c);\n", &[]).is_err());
    assert!(expression("glsl", "float a = 1.0; // one\n", &[]).is_err());
}
