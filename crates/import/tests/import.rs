use strictix_import::{data_file, expression};

#[test]
fn lisp_bound_names_are_nix_lambdas_and_free_symbols_are_formals() {
    let source = "(in-package #:user)\n\n(defun f (x)\n  \"Doc.\"\n  (list 'x #'car -1 2.5))\n";
    assert_eq!(
        expression("lisp", source, &[]).unwrap(),
        "{\n  blank,\n  car,\n  defun,\n  function,\n  in-package,\n  list,\n  quote,\n  rows,\n  str,\n  uninterned,\n  ...\n}: ({f}: [\n  [in-package (uninterned \"user\")]\n  blank\n  ({x}: rows [\n    [defun f [x]]\n    (str \"Doc.\")\n    [[list (quote x) (function car) (-1) 2.5]]\n  ])\n])"
    );
}

#[test]
fn lisp_a_function_name_and_a_variable_of_the_same_symbol_are_two_nix_names() {
    assert_eq!(
        expression("lisp", "(defun f (list) (list list))\n", &[]).unwrap(),
        "{\n  defun,\n  list,\n  ...\n}: ({f}: [\n  ({list'}: [defun f [list'] [list list']])\n])"
    );
    assert_eq!(
        expression("lisp", "(flet ((g (y &optional (z 1)) (list y z))) (g 1))\n", &[]).unwrap(),
        "{\n  and-optional,\n  flet,\n  list,\n  ...\n}: [\n  ({g}: [flet [({y, z}: [g [y and-optional [z 1]] [list y z]])] [g 1]])\n]"
    );
}

#[test]
fn lisp_a_rebinding_read_from_outside_its_form_is_primed() {
    assert_eq!(
        expression("lisp", "(defun f (m)\n  (let ((m (list m)))\n    m))\n", &[]).unwrap(),
        "{\n  defun,\n  let',\n  list,\n  rows,\n  ...\n}: ({f}: [\n  ({m}: rows [\n    [defun f [m]]\n    ({m'}: rows [\n      [let' [[m' [list m]]]]\n      m'\n    ])\n  ])\n])"
    );
}

#[test]
fn lisp_loop_variables_earmuffs_and_reader_literals_are_typed() {
    assert_eq!(
        expression("lisp", "(loop for (a . b) on nil collect a)\n", &[]).unwrap(),
        "{\n  collect,\n  dot,\n  for,\n  loop,\n  nil,\n  on,\n  ...\n}: [\n  ({a, b}: [loop for [a dot b] on nil collect a])\n]"
    );
    assert_eq!(
        expression("lisp", "(defvar *x* 1)\n(defun f () (list *x* +gap+ (+ 1 2)))\n", &[]).unwrap(),
        "{\n  constant,\n  defun,\n  defvar,\n  gap,\n  list,\n  special,\n  ...\n}: ({x, f}: [\n  [defvar (special x) 1]\n  [defun f [] [list (special x) (constant gap) [\"+\" 1 2]]]\n])"
    );
    assert_eq!(
        expression("lisp", "(list :a #\\Space 1/2 1d0 #xFF)\n", &[]).unwrap(),
        "{\n  char,\n  double,\n  hex,\n  kw,\n  list,\n  ratio,\n  ...\n}: [\n  [list (kw \"a\") (char \"Space\") (ratio 1 2) (double 1) (hex 255)]\n]"
    );
}

#[test]
fn lisp_indentation_off_the_rules_is_kept_with_at() {
    assert_eq!(
        expression("lisp", "(list nil\n   t)\n", &[]).unwrap(),
        "{\n  at,\n  list,\n  nil,\n  rows,\n  t,\n  ...\n}: [\n  (rows [\n    [list nil]\n    (at 3 t)\n  ])\n]"
    );
}

#[test]
fn lisp_names_nothing_declares_and_names_nix_cannot_hold_are_errors() {
    assert!(expression("lisp", "(frobnicate x)\n", &[])
        .unwrap_err()
        .contains("`frobnicate`, `x` are bound nowhere"));
    assert!(expression("lisp", "(defun f (a$b) a$b)\n", &[])
        .unwrap_err()
        .contains("cannot spell"));
    assert!(expression("lisp", "(cl:car nil)\n", &[])
        .unwrap_err()
        .contains("reader syntax"));
    assert!(expression("lisp", "(defun f (char) (char char 0))\n", &[])
        .unwrap_err()
        .contains("shadows"));
    assert!(expression("lisp", "(let ((a 1) (a 2)) a)\n", &[])
        .unwrap_err()
        .contains("declared twice"));
}

#[test]
fn lisp_spacing_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("lisp", "(list  t)\n", &[])
        .unwrap_err()
        .contains("separated by"));
    assert!(expression("lisp", "(list) (list)\n", &[])
        .unwrap_err()
        .contains("share a line"));
}

#[test]
fn shell_names_are_nix_bindings_and_expansions_are_nodes() {
    let source = "set -eu\n\nf() { local x=${1:-} y; echo \"$x/$HOME\" >&2; read -r y; [ -n \"$y\" ] || exit 1; }\nfor for in a \"$@\"; do\n  n=${for%.sh}\ndone\nif [ $# -gt 0 ]; then\n  f \"$(date)\" \"${n}x\"\nfi\n";
    assert_eq!(
        expression("shell", source, &[]).unwrap(),
        "{\n  HOME,\n  argCount,\n  args,\n  assign,\n  blank,\n  chain,\n  default,\n  dq,\n  dropSuffix,\n  fn,\n  for,\n  if',\n  inline,\n  nonEmpty,\n  redir,\n  sub,\n  test,\n  var,\n  ...\n}: ({f, for', n}: [\n  [\"set\" \"-eu\"]\n  blank\n  (fn f ({x, y}: inline [[\"local\" (assign x (default 1 \"\")) y] [\"echo\" (dq [(var x) \"/\" (var HOME)]) (redir \">&\" 2)] [\"read\" \"-r\" y] (chain [(test (nonEmpty (dq (var y)))) \"||\" [\"exit\" 1]])]))\n  (for for' [\"a\" (dq (var args))] [\n(assign n (dropSuffix for' \".sh\"))\n])\n  (if' (test [(var argCount) \"-gt\" 0]) [\n[f (dq (sub [[\"date\"]])) (dq [(var n) \"x\"])]\n])\n])"
    );
}

#[test]
fn shell_a_local_shadowing_a_script_name_is_primed() {
    assert_eq!(
        expression("shell", "x=1\nf() { local x=2; echo \"$x\"; }\n", &[]).unwrap(),
        "{\n  assign,\n  dq,\n  fn,\n  inline,\n  var,\n  ...\n}: ({x, f}: [\n  (assign x 1)\n  (fn f ({x'}: inline [[\"local\" (assign x' 2)] [\"echo\" (dq (var x'))]]))\n])"
    );
}

#[test]
fn shell_what_the_printer_would_not_write_or_nothing_declares_is_an_error() {
    let refused = |source: &str, reason: &str| {
        let error = expression("shell", source, &[]).unwrap_err();
        assert!(error.contains(reason), "{source:?}: {error}");
    };
    refused("echo \"$nothing\"\n", "does not declare");
    refused(
        "x=1\necho \"${x}/\"\n",
        "braces the printer would not write",
    );
    refused("local x\n", "local outside a function");
    refused("export X=1\n", "a declaration other than local");
    refused("cat <<EOF\n$HOME\nEOF\n", "not single-quoted");
    refused("echo \"a\\b\"\n", "escapes nothing");
    refused("x=$(y=1; echo)\n", "inside $(...)");
    refused("[[ -n x ]]\n", "not `[[ ... ]]`");
    refused("echo $((1 + 2))\n", "no shell word");
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
        data_file("lisp", "(1 2)\n", &[]).unwrap(),
        "_: [\n  [1 2]\n]\n"
    );
    assert_eq!(
        data_file("lisp", "(list\n \"b\")\n", &[]).unwrap(),
        "{\n  list,\n  rows,\n  str,\n  ...\n}: [\n  (rows [\n    list\n    (str \"b\")\n  ])\n]\n"
    );
}

#[test]
fn luau_names_are_nix_bindings() {
    let source = "--!strict\nlocal last: number? = nil\n\nlocal function f(cx: WindowCx, rows): { [string]: boolean }\n\tif cx then\n\t\treturn { a = 1, [last] = \"x\" }\n\telseif not cx then\n\t\tcx.session:go(#rows + 1) -- note\n\telse\n\t\treturn\n\tend\n\tfor _, type in ipairs(rows) do\n\t\tlocal with = type\n\t\tlast = with\n\tend\n\treturn {}\nend\n";
    assert_eq!(
        expression("luau", source, &[]).unwrap(),
        "{\n  WindowCx,\n  assign,\n  blank,\n  boolean,\n  comment,\n  dot,\n  field,\n  for,\n  ifElse,\n  infix,\n  ipairs,\n  len,\n  local,\n  localFunction,\n  nil,\n  not,\n  number,\n  opt,\n  pair,\n  return,\n  str,\n  string,\n  table,\n  trailing,\n  typed,\n  ...\n}: ({last, f}: [\n  (comment \"--!strict\")\n  (local (typed last (opt number)) nil)\n  blank\n  (localFunction f (typed [{cx = WindowCx;} {rows' = null;}] (table [(pair string boolean)])) ({cx, rows'}: [\n(ifElse cx [\n(return (table [(field \"a\" 1) (pair last (str \"x\"))]))\n] (ifElse (not cx) [\n(trailing [(dot cx \"session:go\") (infix [(len rows') \"+\" 1])] \"-- note\")\n] [\nreturn\n]))\n(for [{_ = null;} {type' = null;}] [ipairs rows'] ({_, type', with'}: [\n(local with' type')\n(assign last with')\n]))\n(return (table []))\n]))\n])"
    );
}

#[test]
fn luau_operators_parse_by_precedence() {
    let source = "local t = (1 + 2) * -3 .. (if tern then \"\" else \"b\") .. (tern :: number)\nlocal u = (t == 1 and t) or t ~= 0x1F and (tostring(t)) and not (t or t) and -t ^ 2\n";
    assert_eq!(
        expression("luau", source, &[]).unwrap(),
        "{\n  cast,\n  hex,\n  ifExpr,\n  infix,\n  local,\n  neg,\n  not,\n  number,\n  paren,\n  str,\n  tern,\n  tostring,\n  ...\n}: ({t, u}: [\n  (local t (infix [(infix [1 \"+\" 2]) \"*\" (-3) \"..\" (ifExpr [tern (str \"\") (str \"b\")]) \"..\" (cast tern number)]))\n  (local u (infix [(paren (infix [t \"==\" 1 \"and\" t])) \"or\" t \"~=\" (hex 31) \"and\" (paren [tostring t]) \"and\" (not (infix [t \"or\" t])) \"and\" (neg (infix [t \"^\" 2]))]))\n])"
    );
}

#[test]
fn luau_line_breaks_are_rows() {
    let source = "tern.on(\n\t\"x\",\n\t\"a\"\n\t\t.. tern.y\n)\nlocal t = {\n\ta = '\"',\n\n\t\"\\u{2061}\",\n}\nlocal s = if t\n\tthen t\n\telse t\n";
    assert_eq!(
        expression("luau", source, &[]).unwrap(),
        "{\n  blank,\n  dot,\n  field,\n  ifExpr,\n  infix,\n  local,\n  rows,\n  str,\n  table,\n  tern,\n  unicode,\n  ...\n}: ({t, s}: [\n  (rows [(dot tern \"on\") (str \"x\") (rows (infix [(str \"a\") \"..\" (dot tern \"y\")]))])\n  (local t (rows (table [\n(field \"a\" (str \"\\\"\"))\nblank\n(str [(unicode \"2061\")])\n])))\n  (local s (rows (ifExpr [[t] [t] [t]])))\n])"
    );
}

#[test]
fn luau_what_the_bound_form_cannot_express_is_an_error() {
    let refused = |source: &str, why: &str| {
        assert!(
            expression("luau", source, &[]).unwrap_err().contains(why),
            "{source}"
        )
    };
    refused("local x = x\n", "before the local of its block");
    refused("print(1)\n", "neither declared nor a global");
    refused("function f() end\n", "a global function");
    refused(
        "local x = (tern)\n",
        "parentheses the printer does not write",
    );
    refused(
        "local x = 1 + if tern then 1 else 2\n",
        "without parentheses",
    );
    refused("local x  = 1\n", "the printer writes");
    refused("tern \"x\"\n", "a call without parentheses");
    refused("local s = 'x'\n", "the printer writes");
    assert_eq!(
        data_file("luau", "\n", &[]).unwrap(),
        "{\n  blank,\n  ...\n}: [blank]\n"
    );
}

#[test]
fn vbs_names_are_nix_bindings_and_globals_are_formals() {
    let source = "' Play.\n\nOption Explicit\nDim a, b\nSet a = CreateObject(\"X.Y\")\nIf a.Count <> 0 Then\n\t' Go.\n\tb = \"say \"\"hi\"\"\" & a.Run(1, True)\nElse\n\tWScript.Quit 1\nEnd If\n";
    assert_eq!(
        expression("vbs", source, &[]).unwrap(),
        "{\n  CreateObject,\n  WScript,\n  assign,\n  blank,\n  call,\n  comment,\n  dim,\n  dot,\n  ifElse,\n  optionExplicit,\n  set,\n  str,\n  ...\n}: ({a, b}: [\n  (comment \"Play.\")\n  blank\n  optionExplicit\n  (dim [a b])\n  (set a (call CreateObject [(str \"X.Y\")]))\n  (ifElse [(dot a \"Count\") \"<>\" 0] [\n    (comment \"Go.\")\n    (assign b [(str \"say \\\"hi\\\"\") \"&\" (call (dot a \"Run\") [1 true])])\n  ] [\n    (call (dot WScript \"Quit\") [1])\n  ])\n])"
    );
}

#[test]
fn vbs_procedures_bind_their_parameters_and_locals() {
    let source = "Dim str\nSub Greet(who, ByVal n)\n\tDim line\n\tline = who & n\n\tWScript.Echo line\nEnd Sub\nFunction Twice(n)\n\tTwice = n * 2\nEnd Function\nGreet str, Twice(1)\n";
    assert_eq!(
        expression("vbs", source, &[]).unwrap(),
        "{\n  WScript,\n  assign,\n  byVal,\n  call,\n  dim,\n  dot,\n  function,\n  sub,\n  ...\n}: ({str', Greet, Twice}: [\n  (dim [str'])\n  (sub Greet [{who = null;} {n = byVal;}] ({who, n, line}: [\n    (dim [line])\n    (assign line [who \"&\" n])\n    (call (dot WScript \"Echo\") [line])\n  ]))\n  (function Twice [{n = null;}] ({n}: [\n    (assign Twice [n \"*\" 2])\n  ]))\n  (call Greet [str' (call Twice [1])])\n])"
    );
}

#[test]
fn vbs_operators_are_infix_lists_by_vbscript_precedence() {
    let source =
        "Dim a, b, c, x\nx = Not a = b And c\nx = (a + b) * -c\nx = a - (b - c)\nx = -a ^ 2\n";
    assert_eq!(
        expression("vbs", source, &[]).unwrap(),
        "{\n  assign,\n  dim,\n  neg,\n  not,\n  ...\n}: ({a, b, c, x}: [\n  (dim [a b c x])\n  (assign x [(not [a \"=\" b]) \"And\" c])\n  (assign x [[a \"+\" b] \"*\" (neg c)])\n  (assign x [a \"-\" [b \"-\" c]])\n  (assign x (neg [a \"^\" 2]))\n])"
    );
}

#[test]
fn vbs_what_the_printer_cannot_reproduce_or_nothing_declares_is_an_error() {
    for source in [
        "Dim a, b\na = (a * b) + b\n",
        "Dim x\nx  = 1\n",
        "Dim x\nx = 1 ' one\n",
    ] {
        assert!(expression("vbs", source, &[])
            .unwrap_err()
            .contains("would not give this layout back"));
    }
    assert!(
        expression("vbs", "Dim x\nIf x Then\n    x = 1\nEnd If\n", &[])
            .unwrap_err()
            .contains("line 3")
    );
    assert!(expression("vbs", "Dim x\nx = y\n", &[])
        .unwrap_err()
        .contains("nothing declares"));
    assert!(expression("vbs", "Dim x, x\n", &[])
        .unwrap_err()
        .contains("declared twice"));
    assert!(expression("vbs", "WScript.Quit(1)\n", &[])
        .unwrap_err()
        .contains("in parentheses"));
    assert!(expression("vbs", "Dim x\nx = 007\n", &[])
        .unwrap_err()
        .contains("plain integer"));
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
        "[\n  (heading 1 \"Title\")\n  blank\n  (comment \" note \")\n  blank\n  (bullets [\n    [\"one \" (code \"x\")]\n    [(strong \"two\") \" \" (link \"a\" \"b\")]\n  ])\n  blank\n  (table [\"A\" \"B\"] [\n    [[(em \"c\")] \"d\"]\n  ])\n]"
    );
}

#[test]
fn markdown_brackets_that_are_no_link_are_literal_text_around_typed_inlines() {
    assert_eq!(
        expression("markdown", "box [3,0,9] and [*x* `y`] new_tab\n", &[]).unwrap(),
        "[\n  [\"box [3,0,9] and [\" (em \"x\") \" \" (code \"y\") \"] new_tab\"]\n]"
    );
}

#[test]
fn markdown_text_that_would_read_as_markup_is_an_error() {
    for source in ["a * b\n", "a _b\n", "a_ b\n", "a & b\n"] {
        assert!(expression("markdown", source, &[])
            .unwrap_err()
            .contains("would read as markup"));
    }
    assert!(expression("markdown", "\\*a\n", &[])
        .unwrap_err()
        .contains("backslash_escape"));
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
fn lua_locals_parameters_and_loop_variables_are_nix_bindings() {
    let source = "local t = { a = 1, \"x\", -1, 0.5 }\nlocal function f(self, ...)\n  local n = select(\"#\", ...)\n  for i = 1, #t do t[i] = nil; break end\n  for _, v in ipairs(t) do print(v) end\n  return n\nend\nif not t then return end\n";
    assert_eq!(
        expression("lua", source, &[]).unwrap(),
        "{\n  assign,\n  break,\n  field,\n  forIn,\n  forNum,\n  if',\n  index,\n  inline,\n  ipairs,\n  len,\n  local,\n  localFunction,\n  nil,\n  not,\n  print,\n  return,\n  select,\n  semicolon,\n  str,\n  table,\n  vararg,\n  ...\n}: ({t, f}: [\n  (local t [(table [(field \"a\" 1) (str \"x\") (-1) 0.5])])\n  (localFunction f ({self, n}: {params = [self vararg]; body = [\n(local n [[select (str \"#\") vararg]])\n(forNum ({i}: {name = i; range = [1 (len t)]; body = inline [(assign (index t i) [nil]) semicolon break];}))\n(forIn ({_, v}: {names = [_ v]; values = [[ipairs t]]; body = inline [[print v]];}))\n(return [n])\n];}))\n  (if' (not t) (inline [(return [])]))\n])"
    );
}

#[test]
fn lua_infix_lists_follow_precedence_and_nest_where_the_source_parenthesizes() {
    let source = "local a, b = 1, 2\nlocal x = (a or b) and a .. b .. (a + b) * 2 or not (a == b)\nreturn (print(x)), x:gsub(\"y\", \"z\"), -a ^ 2\n";
    assert_eq!(
        expression("lua", source, &[]).unwrap(),
        "{\n  local,\n  method,\n  neg,\n  not,\n  paren,\n  print,\n  return,\n  str,\n  ...\n}: ({a, b, x}: [\n  (local [a b] [1 2])\n  (local x [[[a \"or\" b] \"and\" a \"..\" b \"..\" [a \"+\" b] \"*\" 2 \"or\" (not [a \"==\" b])]])\n  (return [(paren [print x]) [(method x \"gsub\") (str \"y\") (str \"z\")] (neg [a \"^\" 2])])\n])"
    );
}

#[test]
fn lua_broken_conditions_and_tables_are_rows() {
    let source = "local x\nif x\n  and x == nil then\n  x = {\n    { 1 }, { 2 },\n    print{ q = true },\n  }\nelseif x then print() else error() end\n";
    assert_eq!(
        expression("lua", source, &[]).unwrap(),
        "{\n  apply,\n  assign,\n  error,\n  field,\n  ifElse,\n  inline,\n  local,\n  nil,\n  print,\n  rows,\n  table,\n  ...\n}: ({x}: [\n  (local x [])\n  (ifElse (rows [[x] [\"and\" x \"==\" nil]]) [\n(assign x [(table (rows [\n[(table [1]) (table [2])]\n(apply print (table [(field \"q\" true)]))\n]))])\n] (ifElse x (inline [[print]]) (inline [[error]])))\n])"
    );
}

#[test]
fn lua_names_that_collide_with_nix_or_the_generator_take_a_prime() {
    assert_eq!(
        data_file("lua", "local print, let = print, 1\nprint(let)\n", &[]).unwrap(),
        "{\n  local,\n  print,\n  ...\n}: ({print', let'}: [\n  (local [print' let'] [print 1])\n  [print' let']\n])\n"
    );
    assert!(expression("lua", "local table = 1\n", &[])
        .unwrap_err()
        .contains("already ends in '"));
}

#[test]
fn lua_names_nothing_binds_are_an_error() {
    assert!(expression("lua", "print(undeclared)\n", &[])
        .unwrap_err()
        .contains("nothing declares"));
    assert!(expression("lua", "x = true\n", &[])
        .unwrap_err()
        .contains("nothing declares"));
    assert!(expression("lua", "function f() end\n", &[])
        .unwrap_err()
        .contains("global function"));
    assert!(expression(
        "lua",
        "local x = 1\ndo\n  print(x)\n  local x = 2\nend\n",
        &[]
    )
    .unwrap_err()
    .contains("capture"));
}

#[test]
fn lua_what_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("lua", "print(1,2)\n", &[])
        .unwrap_err()
        .contains("expected"));
    assert!(expression("lua", "print() print()\n", &[])
        .unwrap_err()
        .contains("share a line"));
    assert!(expression("lua", "print([[long]])\n", &[])
        .unwrap_err()
        .contains("long bracket"));
    assert!(expression("lua", "print((1 + 2))\n", &[])
        .unwrap_err()
        .contains("would not write"));
    assert!(expression("lua", "print(0x10)\n", &[])
        .unwrap_err()
        .contains("does not spell"));
    assert!(expression("lua", "repeat print() until true\n", &[]).is_err());
    assert!(expression("lua", "local t = { a = 1 }\n", &[]).is_ok());
    assert!(expression("lua", "local t = {\n  a = 1\n}\n", &[]).is_err());
}

#[test]
fn python_names_are_bound_by_their_scope_and_builtins_are_formals() {
    let source = "import os\n\n\ndef f(a, b=1):\n    if a:\n        return os.path.join(a, f\"{b:03d}\")\n    elif b:\n        x = print(a,\n                  b)\n    else:\n        pass\n";
    assert_eq!(
        data_file("python", source, &[]).unwrap(),
        "{\n  aligned,\n  assign,\n  blank,\n  call,\n  def,\n  default,\n  dot,\n  ifElse,\n  import',\n  pass,\n  print,\n  return,\n  spec,\n  str,\n  ...\n}: ({os, f}: [\n(import' os)\nblank\nblank\n(def f ({a, b, x}: {\nparams = [a (default b 1)];\nbody = [\n(ifElse a [\n(return (call (dot os \"path.join\") [a (str [(spec b \"03d\")])]))\n] (ifElse b [\n(assign x (call print (aligned [a b])))\n] [\npass\n]))\n];\n}))\n])\n"
    );
}

#[test]
fn python_hanging_brackets_slices_and_continuations() {
    let source = "from m import f, s, t\nx = f(\n    s,\n    t,\n)\ny = f(\n    s, t\n)\nz = s[t + 1 :] + s[1:] \\\n    + t\n";
    assert_eq!(
        expression("python", source, &[]).unwrap(),
        "{\n  assign,\n  call,\n  from,\n  index,\n  rows,\n  slice,\n  ...\n}: ({f, s, t, x, y, z}: [\n(from \"m\" [f s t])\n(assign x (call f (rows [s t])))\n(assign y (call f (rows [[s t]])))\n(assign z (rows [[(index s (slice [t \"+\" 1] null)) \"+\" (index s (slice 1 null))] [\"+\" t]]))\n])"
    );
}

#[test]
fn python_infix_lists_keep_only_the_grouping_precedence_does_not_give() {
    let source = "from m import a, b, c\nx = (a + b) * c - (a - b)\ny = not (a < b) < c\nz = [str(k) for k in x if k]\n";
    assert_eq!(
        expression("python", source, &[]).unwrap(),
        "{\n  assign,\n  call,\n  compFor,\n  compIf,\n  from,\n  listComp,\n  not,\n  str',\n  ...\n}: ({a, b, c, x, y, z}: [\n(from \"m\" [a b c])\n(assign x [[a \"+\" b] \"*\" c \"-\" [a \"-\" b]])\n(assign y (not [[a \"<\" b] \"<\" c]))\n(assign z (listComp ({k}: [(call str' [k]) (compFor k x) (compIf k)])))\n])"
    );
}

#[test]
fn python_names_colliding_with_constructors_builtins_or_nix_take_a_prime() {
    let source = "def index(list):\n    str = [x for x in list]\n    return lambda y=str: y\n";
    assert_eq!(
        expression("python", source, &[]).unwrap(),
        "{\n  assign,\n  compFor,\n  def,\n  default,\n  lambda,\n  listComp,\n  return,\n  ...\n}: ({index'}: [\n(def index' ({list', str'}: {\nparams = [list'];\nbody = [\n(assign str' (listComp ({x}: [x (compFor x list')])))\n(return (lambda ({y}: {\nparams = [(default y str')];\nbody = y;\n})))\n];\n}))\n])"
    );
}

#[test]
fn python_layout_the_printer_cannot_reproduce_is_an_error() {
    assert!(expression("python", "x  = 1\n", &[])
        .unwrap_err()
        .contains("where the printer writes"));
    assert!(expression("python", "print(1, 2,)\n", &[])
        .unwrap_err()
        .contains("trailing comma"));
    assert!(expression("python", "x = 1  # note\n", &[]).is_err());
    assert!(expression("python", "s = 1\ns[s+1:]\n", &[]).is_err());
    assert!(expression("python", "x = (1)\n", &[])
        .unwrap_err()
        .contains("neither the grouping nor a line break"));
}

#[test]
fn python_names_nothing_declares_or_nix_would_bind_differently_are_errors() {
    assert!(expression("python", "print(y)\n", &[])
        .unwrap_err()
        .contains("`y` is not declared"));
    assert!(expression("python", "x = [(y := 1)]\n", &[])
        .unwrap_err()
        .contains("assignment expression"));
    assert!(expression(
        "python",
        "class A:\n    x = 1\n\n    def f(self):\n        return x\n",
        &[]
    )
    .unwrap_err()
    .contains("class attribute"));
    assert!(expression("python", "x = [x for x in x]\n", &[])
        .unwrap_err()
        .contains("outside the scope that declares it"));
    assert!(expression("python", "\u{e9} = 1\n", &[])
        .unwrap_err()
        .contains("not a name Nix can spell"));
}

#[test]
fn python_template_placeholders_are_a_constructor() {
    assert_eq!(
        data_file("python", "k = 'a'\nx = @owners@.get(k, '')\n", &[]).unwrap(),
        "{\n  assign,\n  call,\n  dot,\n  placeholder,\n  sq,\n  ...\n}: ({k, x}: [\n(assign k (sq \"a\"))\n(assign x (call (dot (placeholder \"owners\") \"get\") [k (sq \"\")]))\n])\n"
    );
}

#[test]
fn javascript_names_are_nix_bindings_and_operators_infix_lists() {
    let source = "#!/usr/bin/env node\n\nimport { join } from \"node:path\";\n\nconst { a, b: [c] } = process;\nlet n;\nasync function f(s, obj) {\n  if (!s) return;\n  try { n = await join(s, -1, 0.5); } catch {}\n  const o = { k: [1, \"v\", null], \"q-r\": {\n    m: (n = s.t?.u) >= 0 && (a || c)\n  }};\n  return o.k.length +\n    typeof n;\n}\n";
    assert_eq!(
        expression("javascript", source, &[]).unwrap(),
        "{\n  array,\n  async,\n  await,\n  blank,\n  call,\n  comment,\n  const,\n  dot,\n  function,\n  if',\n  import',\n  inline,\n  let',\n  not,\n  null',\n  obj,\n  optional,\n  process,\n  return,\n  rows,\n  str,\n  try',\n  typeof,\n  ...\n}: ({join, a, c, n, f}: [\n(comment \"#!/usr/bin/env node\")\nblank\n(import' [join] \"node:path\")\nblank\n(const (obj [a [\"b\" (array [c])]]) process)\n(let' n null)\n(async (function f ({s, obj', o}: {params = [s obj']; body = [\n(if' (not s) (return null))\n(try' (inline [[n \"=\" (await (call join [s (-1) 0.5]))]]) (inline []))\n(const o (obj [[\"k\" (array [1 (str \"v\") null'])] [(str \"q-r\") (obj (rows [\n[\"m\" [[n \"=\" (optional (dot s \"t\") \"u\")] \">=\" 0 \"&&\" [a \"||\" c]]]\n]))]]))\n(return (rows [\n(dot o \"k.length\")\n\"+\"\n(typeof n)\n]))\n];})))\n])"
    );
}

#[test]
fn javascript_what_the_tree_cannot_hold_is_an_error() {
    assert!(expression("javascript", "const f = 1;\nf(f,f);\n", &[])
        .unwrap_err()
        .contains("line 2"));
    assert!(expression("javascript", "const o = {a: 1};\n", &[]).is_err());
    assert!(expression("javascript", "let a;\nif (a) a = 1;\n", &[])
        .unwrap_err()
        .contains("without braces"));
    assert!(expression("javascript", "const s = 'x';\n", &[])
        .unwrap_err()
        .contains("single-quoted"));
    assert!(expression("javascript", "g(1);\n", &[])
        .unwrap_err()
        .contains("declared nowhere"));
    assert!(expression("javascript", "const $x = 1;\n", &[])
        .unwrap_err()
        .contains("cannot spell"));
    assert!(
        expression("javascript", "const a = 1;\nconst b = (a) + 1;\n", &[])
            .unwrap_err()
            .contains("(a) + 1")
    );
    assert!(
        expression("javascript", "function f() {}\nfunction f() {}\n", &[])
            .unwrap_err()
            .contains("declared twice")
    );
    assert!(
        expression("javascript", "const a = 1;\nconst b = a +\n  a + a;\n", &[])
            .unwrap_err()
            .contains("some operators")
    );
}

#[test]
fn javascript_data_file_is_the_program_function() {
    assert_eq!(
        data_file("javascript", "let n;\n", &[]).unwrap(),
        "{\n  let',\n  ...\n}: ({n}: [\n(let' n null)\n])\n"
    );
}

#[test]
fn ts_names_are_bound_by_the_block_that_declares_them() {
    let source = "import { a, type B } from \"x\";\n\nexport function f(p: B): number {\n\tif (!p) return 0;\n\tconst n = p.items\n\t\t.map((i) => i * 2)\n\t\t.filter(Boolean);\n\treturn n.length > 0 ? n[0] : -1;\n}\n";
    assert_eq!(expression("ts", source, &[]).unwrap(), "{\n  Boolean,\n  arrow,\n  blank,\n  call,\n  chain,\n  const,\n  dot,\n  export,\n  fn,\n  idx,\n  if',\n  import',\n  infix,\n  not,\n  number,\n  return,\n  str,\n  ternary,\n  typeOnly,\n  typed,\n  ...\n}: ({a, B, f}: [\n(import' [a (typeOnly B)] (str \"x\"))\nblank\n(export ({p, n}: (fn f (typed [(typed p B)] number) [\n(if' (not p) (return 0))\n(const n (chain (call (dot (call (dot p \"items.map\") [({i}: (arrow [i] (infix [i \"*\" 2])))]) \"filter\") [Boolean])))\n(return (ternary (infix [(dot n \"length\") \">\" 0]) (idx n 0) (-1)))\n])))\n])");
}

#[test]
fn ts_operators_are_infix_lists_and_parentheses_are_derived() {
    let source = "const a = 1;\nconst b = 2;\nconst x = (a || b) && a;\nconst y = (a && b) || a;\nconst z = a - (b - a) * 2;\nconst g = async () => (await f()).y;\nconst h = (p) => ({ p });\nfunction f() {}\n";
    assert_eq!(expression("ts", source, &[]).unwrap(), "{\n  arrow,\n  async,\n  await,\n  call,\n  const,\n  dot,\n  fn,\n  infix,\n  obj,\n  spaces,\n  ...\n}: (spaces 2 ({a, b, x, y, z, g, h, f}: [\n(const a 1)\n(const b 2)\n(const x (infix [[a \"||\" b] \"&&\" a]))\n(const y (infix [a \"&&\" b \"||\" a]))\n(const z (infix [a \"-\" [b \"-\" a] \"*\" 2]))\n(const g (async (arrow [] (dot (await (call f [])) \"y\"))))\n(const h ({p}: (arrow [p] (obj [p]))))\n(fn f [] [])\n]))");
}

#[test]
fn ts_broken_lists_operators_and_indentation_are_kept() {
    let source = "const a = 1;\nconst x =\n\ta &&\n\ta;\nf(\n\t1,\n\t2,\n);\nfunction f(n, m) {}\n";
    assert_eq!(expression("ts", source, &[]).unwrap(), "{\n  below,\n  call,\n  const,\n  fn,\n  infix,\n  rows,\n  ...\n}: ({a, x, f}: [\n(const a 1)\n(const x (below (infix (rows [\na\n\"&&\"\na\n]))))\n(call f (rows [\n1\n2\n]))\n({n, m}: (fn f [n m] []))\n])");
    assert_eq!(
        data_file("ts", "if (globalThis) {\n  const call = 1;\n}\n", &[]).unwrap(),
        "{\n  const,\n  globalThis,\n  if',\n  spaces,\n  ...\n}: (spaces 2 [\n(if' globalThis ({call'}: [\n(const call' 1)\n]))\n])\n"
    );
}

#[test]
fn ts_names_nothing_declares_and_layout_the_printer_cannot_reproduce_are_errors() {
    assert!(expression("ts", "f(1);\n", &[])
        .unwrap_err()
        .contains("`f` is declared nowhere"));
    assert!(expression("ts", "const $x = 1;\n", &[])
        .unwrap_err()
        .contains("cannot be spelled"));
    assert!(expression("ts", "const a = 1;\nconst x = (a);\n", &[])
        .unwrap_err()
        .contains("parentheses the printer would not write"));
    assert!(
        expression("ts", "const a = 1;\nconst x = a && a || a;\n", &[])
            .unwrap_err()
            .contains("parentheses")
    );
    assert!(expression("ts", "const a = 1;\na(a,  a);\n", &[])
        .unwrap_err()
        .contains("expected"));
    assert!(expression("ts", "const a = 1;\na();  // note\n", &[]).is_err());
    assert!(expression("ts", "const a = 1;\nconst x = a &&\n\ta;\n", &[]).is_err());
    assert!(expression("ts", "const a = 1;\na(a,\n\ta);\n", &[]).is_err());
}
