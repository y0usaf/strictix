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

#[test]
fn glsl_locals_and_parameters_are_nix_bindings() {
    let source = "#define SCALE(c) ((c) / 255.0)\nconst float A = 0.50;\nconst vec2 P[2] = vec2[2](vec2(0.0), vec2(1.0, -1.0));\nconst float B = A * 2.0;\n// the falloff\nfloat f(in vec2 p, float k) {\n    float c = cos(k), s[2];\n    if (k < 0.0) return -(p.x + k);\n    else if (k > 1e-5) { float d = c - k; s[0] = d - (k - c); }\n    for (int i = 0; i < 3; i++) c += P[i].x * float(i) > 0.0 ? f(p.yx, k) : SCALE(1.0);\n    return c * (s[1] / 2.0);\n}\n";
    assert_eq!(
        expression("glsl", source, &[]).unwrap(),
        "{\n  array,\n  commented,\n  cond,\n  cos,\n  float,\n  fn,\n  for,\n  if',\n  ifElse,\n  in',\n  inc,\n  index,\n  int,\n  local,\n  macro,\n  neg,\n  return,\n  set,\n  sized,\n  typed,\n  update,\n  vec2,\n  ...\n}: self: let\n  inherit (self) A P SCALE f;\nin {\n  defines = {\n    SCALE = (macro ({c}: [c \"/\" 255.0]));\n  };\n  consts = {\n    A = 0.5;\n    B = (typed float [A \"*\" 2.0]);\n    P = (array vec2 [(vec2 0.0) (vec2 1.0 (-1.0))]);\n  };\n  fns = {\n    f = (commented \"the falloff\" (fn float [{p = (in' vec2);} {k = float;}] ({p, k, c, s}: [\n(local c float (cos k))\n(local s (sized float 2) null)\n(ifElse [k \"<\" 0.0] [\n(return (neg [p.x \"+\" k]))\n] (if' [k \">\" 1.0e-5] ({d}: [\n(local d float [c \"-\" k])\n(set (index s 0) [d \"-\" [k \"-\" c]])\n])))\n(for ({i}: {\n init = (local i int 0);\n test = [i \"<\" 3];\n step = (inc i);\n body = [\n(update c \"+=\" (cond [(index P i).x \"*\" (float i) \">\" 0.0] (f p.yx k) (SCALE 1.0)))\n];\n}))\n(return [c \"*\" [(index s 1) \"/\" 2.0]])\n])));\n  };\n}"
    );
}

#[test]
fn glsl_data_file_is_the_shader_function() {
    let file = data_file("glsl", "const float E = 1e-5;\n", &[]).unwrap();
    assert_eq!(file, "_: _: {\n  consts = {\n    E = 1.0e-5;\n  };\n}\n");
}

#[test]
fn glsl_what_a_keyed_shader_cannot_express_is_an_error() {
    assert!(expression("glsl", "uniform float x;\n", &[])
        .unwrap_err()
        .contains("const"));
    assert!(expression(
        "glsl",
        "float f(float a) {\n    return a;\n}\nfloat f(int a) {\n    return 1.0;\n}\n",
        &[]
    )
    .unwrap_err()
    .contains("declared twice"));
    assert!(expression("glsl", "const float a = 1.0f;\n", &[]).is_err());
    assert!(expression("glsl", "const float a = 1.0;\n// trailing\n", &[]).is_err());
    assert!(expression("glsl", "void f() {\n    { a = 1.0; }\n}\n", &[]).is_err());
    assert!(
        expression("glsl", "float f() {\n    return undeclared;\n}\n", &[])
            .unwrap_err()
            .contains("neither bound")
    );
    assert!(expression(
        "glsl",
        "float f() {\n    float set = 1.0;\n    return set;\n}\n",
        &[]
    )
    .unwrap_err()
    .contains("already gives a meaning"));
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
