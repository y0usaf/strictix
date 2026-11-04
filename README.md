# strictix

> Strict lints and suggestions for the Nix programming language.

strictix is a from-scratch Nix linter in the Ruff/Prettier mold: a
single binary, run, report, exit. Every component — lexer, parser,
syntax tree, semantic model, rules engine, CLI — is owned and written
in-house. Design center: catch the semantic mistakes that AI-generated
and hand-written Nix actually make: hallucinated option names and file
paths, dead bindings, shadowing, with-smuggling, uncoercible
interpolations — plus the simplifications that shrink honest code.

## Usage

```
$ strictix check .
./module.nix:3:3: error[unknown-option]: option 'services.ngnix.virtualHosts' is not declared in options.json
  services.ngnix.virtualHosts = { };
  ^^^^^^^^^^^^^^^^^^^^^^^^^^^

./overlay.nix:4:30: error[dangling-path]: path './pkgs/example' does not exist (resolved to ./pkgs/example)
  example = prev.callPackage ./pkgs/example {
                             ^^^^^^^^^^^^^^
  help: create the file or correct the path
```

`strictix fix` applies every safe rewrite and re-runs to a fixpoint, so
one fix revealing another finding is caught in the same run:

```
$ strictix fix slop.nix
-  flag = if 1 == 2 then true else false;
+  flag = 1 == 2;
-  has = builtins.hasAttr "k" s;
+  has = s ? k;
-  ty = lib.types.string;
+  ty = lib.types.str;
```

`strictix merge` collapses a tree into one file. Starting from `--entry`,
every `.nix` file reached through a relative path literal given to
`import`/`callPackage`, listed in a module list, bound as a module value
(`nixosModules.x = ./x.nix;`, or a `let` name used only in module lists), or
named by a string (`"modules/x.nix"`, `"${self}/modules/x.nix"`) becomes one
`let` binding named after its path; other relative paths are rebased to the
output's directory, and strings are kept as written: the manifest path names
the binding. `--expand NAME` evaluates, with `nix eval`, calls of a
discovery function `NAME` (e.g. a `listFilesRecursive` helper) bound in the
top-level `let` of any reached file shaped `[ARG:] let ... in BODY`, in that
`let`'s scope with paths relative to that file and `ARG` given the flake
inputs, replaces each call with the explicit module list and drops the
binding once nothing uses it. A module listed more than once is wrapped in a
keyer that adds `key = "path"` without nesting it, so the module system keeps
deduplicating it at the same depth. A file still named by a path outside
those positions (`${./x.nix}` in a string), or imported by a file that is not
merged, stays a file and merge says why; every other merged file is deleted,
so each file exists exactly once.

`--out` may equal `--entry`, which is then replaced. A flake entry (an
attribute set with `outputs`) stays an attribute set, as Nix requires: its
`description` and `inputs` are kept and the bindings go into a `let` inside
`outputs`, under its lambda (`outputs = inputs: let ... in builtins.seq
merge_manifest (...)`, since Nix rejects an `outputs` that is not
syntactically a function). Every other entry becomes `let ... in builtins.seq
merge_manifest ENTRY`. `merge_manifest` is plain data recording each binding's
path, the expanded calls and removed bindings per file, module values, any
reference spelled differently from the default, and the original text of any
file the flake's formatter would change. The output is formatted with the
flake's formatter when the root has one:

```
$ strictix merge . --entry flake.nix --out flake.nix --expand nixFiles
```

`strictix split` is the inverse: it writes every binding of `--merged` back to
its path in the manifest, restores `import ./x.nix`, `./x.nix` in module lists
and module values, `./dir` for `dir/default.nix`, the flake shell, the
expanded calls and removed bindings, points the entry's referrers back at it,
replaces the merged file (or deletes it when it is not the entry), and
formats what it wrote; a file recorded verbatim comes back byte for byte
unless it was edited. Edits made in the merged file land in the original
files; a new binding needs its path added to `merge_manifest.files`:

```
$ strictix split . --merged flake.nix
```

`strictix import LANGUAGE FILE` parses a source file with the language's
tree-sitter grammar (`glsl`: tree-sitter-glsl, `javascript`:
tree-sitter-javascript, `lisp`: tree-sitter-commonlisp, `lua`:
tree-sitter-lua, `luau`: tree-sitter-luau, `markdown`: tree-sitter-md,
`patch`: tree-sitter-diff, `python`: tree-sitter-python, `shell`:
tree-sitter-bash, `ts`: tree-sitter-typescript, `vbs`: jcolozzi's
tree-sitter-VB6-VBA-VBS, whose `source_file` cannot start with a comment or
blank line, so the leading run of comment and blank lines is read without it,
and which parses `Set x = y` as a call of `Set`, read back as the assignment
it is) and prints it as the Nix syntax tree finix's generators render: a data
file `{rows, str, ...}: [ ... ]`, a function of the constructors of
`generators.LANGUAGE` it uses. A `.nix` FILE is read as a line list (one
string per source line); when it is not a list itself but holds one
`generators.<name>Source [ ... ]` call, the file is printed back with that
call replaced by
`generators.LANGUAGE.render (let inherit (generators.LANGUAGE) ...; in [ ... ])`
(`generators.LANGUAGE.render ((DATA FILE) generators.LANGUAGE)` for GLSL and
shell, whose data file binds names),
and the list's interpolations come back as interpolations inside the tree's
strings. Layout the printer derives (Lisp indentation; shell, Luau and Python
nesting; Python's spacing; a hunk's line counts and new start; a Markdown
paragraph filled to 78 columns, kept as `wrap`; TypeScript blocks, parentheses and member chains' rows) is left out; layout it cannot
derive is kept as `rows`, `inline`, `at`, `below` (TypeScript's break after
`=`, `=>`, `:` or `(`) and `blank` nodes (and, for Python, `aligned` and
`hang` brackets), and anything else it would not reproduce byte for byte
(double spaces, trailing comments, unsupported syntax; in a patch, text before
the first file, renames, `\ No newline at end of file`) is an error naming the
line. The Luau and JavaScript importers print every tree back as the printer
would and refuse the file at the first line that differs from the source.
Lisp names are Nix bindings too: `import lisp` prints the whole data file,
`{defun, getf, kw, ...}: ({f, g}: [ ... ])`, its formals the constructors and
the `generators.lisp` globals the file uses (Common Lisp symbols, `loop`
keywords, tomoe's and ekko's API, mirrored in the importer's `CL`, `LOOP`,
`TOMOE` and `EKKO`). A form that binds names (the file's definitions, lambda
lists, `let`, `let*`, `flet`, `labels`, `macrolet`, `destructuring-bind`,
`multiple-value-bind`, `dolist`, `dotimes`, the stream macros, `loop`
variables) is a lambda of them; a list's head resolves in the function
namespace, other symbols as variables first. A name that is a Nix keyword, a
constructor or a global gets a trailing `'`, as does a rebinding its own
initializer reads; `+x+` and `*x*` are `constant x` and `special x`, the
operators Nix cannot spell strings, a keyword `kw`, `#:x` `uninterned`, a
character `char`, a ratio, `Nd0` and `#x` number `ratio`, `double` and `hex`,
and a `loop` clause its words. Symbols nothing binds that are no global
(listed together in one error), a name Nix cannot spell, a package-qualified
symbol, other reader syntax, a name declared twice in one form and a name a
nearer Nix binding shadows are errors.
GLSL is read for meaning, not layout: `import glsl` prints a shader as a
function of the generator's constructors and of the shader itself, `{fn,
vec2, mix, ...}: self: let inherit (self) PAL hash; in {defines, consts,
fns}`, attribute sets keyed by name. A parameter or local is a Nix binding: a
function body or a block that declares names is a lambda whose formals are its
parameters and locals (`({c, v}: [(local v uvec2 ...) ...])`), a global is
inherited from `self`, and a builtin, type or prelude uniform is a formal, so
every name is a Nix variable. Numbers and booleans are native Nix values,
operators infix lists nested only where precedence needs parentheses, and a
swizzle in any of GLSL's three letter sets is an attribute of the value it
selects from (`(vec3 p 1.0).bgr`, `(index M 0).ts`). A constant is its bare
value when the importer can infer the value's type by the generator's rules
(literals, constructors, other constants and non-macro defines, swizzles,
indexing, builtins and every arithmetic, matrix, comparison and logical
operator) and `typed type value` only when it cannot (a macro's result);
layout, blank lines, redundant parentheses and literal spelling are dropped,
and a global that is not one `const` name, a const whose value has another
type than its declaration, a name declared twice, a name the generator or Nix
already uses, a name nothing declares, a field that is not a swizzle, a
swizzle of an infix expression, a literal or an increment, a suffixed float or
a bare nested block is an error.
TypeScript keeps its layout and reads names like GLSL: `import ts` prints the
whole data file, `{call, dot, string, ...}: ({toolRenderers, failed, ...}: [
... ])`. Every identifier and type name is a Nix variable: the file, a block,
a function (its parameters and its body's declarations), a `for` (its binding
and body), a `catch` (its parameter and body) and a `switch` that declare names
are lambdas of them (`({cell}: arrow cell ...)`), a global the generator
lists (`JSON`, `Promise`, `string`, `undefined`, `this`, ...) is a formal, a
name Nix or the generator already uses is spelled with a trailing `'`
(`call'`, `null'`), and a name nothing declares, a name with `$` and an import
renamed with `as` are errors. Member access is `dot x "a?.b"`, operators an
infix list parsed by precedence and nested only where precedence needs
parentheses, unary operators constructors (`not`, `await`, `typeof`,
`spread`, ...), and parentheses that differ from those the printer writes
(prettier's: mixed `&&` and `||`, `??` in a ternary branch, `as` in an
operand, an inline ternary or object arrow body, ...) are an error.
`import luau` prints a whole data file too, a function of the constructors
and globals it uses returning the file's block, `{dot, local, tern, WindowCx,
...}: ({last, note}: [ ... ])`. A local, local function or type alias is a Nix
binding: a block that declares names (the file, a function body, a branch, a
loop body) is a lambda whose formals are exactly those names, a function's
parameters and a loop's variables are listed in order as one-attribute sets
(`[{cx = WindowCx;} {id = null;}]`, `null` when unannotated) and bound by the
body's lambda, and the Luau and Tern globals and builtin types the files use
(`tern`, `string`, `table'`, `pcall`, `number`, `WindowCx`, ...) are formals
taken from `generators.luau`. A field path is `dot base "session:focused"`
on a bound name, a literal `str`, `long`, a Nix number (`hex 57344` for
`0xE000`), boolean or `nil`; operators are infix lists parsed by Luau
precedence, nested only where the grouping needs parentheses, and `paren`
keeps only parentheses that truncate a call to one value or that StyLua keeps
around a binary expression precedence already groups. A name that is a Nix
keyword or an attribute of `generators.luau` (`type`, `table`) is spelled
with a trailing `'`; a name nothing declares that is not one of those globals,
a use of a name before the local Nix would bind it to, a global function,
parentheses StyLua removes and an `if` or cast operand without parentheses are
errors.
JavaScript keeps its layout but is printed like GLSL as a function of the
constructors and Node globals it uses, `{call, dot, process, ...}: ({buf,
handle}: [ ... ])`: the program, a function or arrow, a braced body and a
`catch` that declare names are lambdas of them (a function's returning
`{params = [...]; body = ...;}`), a name that is a Nix keyword or an attribute
of `generators.javascript` is spelled with a trailing `'`, a member path is
`dot` or `optional` with a property string, a call `call f [args]`, and
operators infix lists nested only where precedence needs parentheses (the
printer adds prettier's own); a name nothing declares that is not one of the
generator's globals, a name Nix cannot spell (`$x`), parentheses the printer
would not write and an unbraced body that is an operation are errors.
Shell keeps its layout and binds its names the same way: `import shell`
prints a script as `{assign, dq, var, HOME, ...}: ({state, usage, ...}: [
... ])`. Bash scopes a name to the script or to a function, so the script's
lambda binds its function names and every variable it sets (`x=`, `for x`,
`read x`) outside a function or inside one without `local`, and a function
body is a lambda of the names it declares `local` (`fn covers ({base}: inline
[ ... ])`). A word naming a variable or one of the script's functions is that
binding, `$x` is `var x` and `${x:-w}`, `${x:?w}`, `${x#w}`, `${x##w}`,
`${x%w}`, `${x%%w}` and `${x[@]}` are `default`, `required`, `dropPrefix`,
`dropLongestPrefix`, `dropSuffix`, `dropLongestSuffix` and `elements` of it; a
positional parameter is a Nix integer (`var 1`), `$@`, `$*`, `$#` and `$?` are
`args`, `argsJoined`, `argCount` and `exitStatus`, and the environment
variables `generators.shell` lists (`HOME`, `IFS`, `PATH`, `XDG_STATE_HOME`,
...) are formals of the data file. A double-quoted string's text is unescaped,
a whole bare word that is a decimal integer is a Nix integer, and `[ ... ]` is
`test` of a unary test (`nonEmpty`, `isEmpty`, `exists`, `isDir`, `isFile`)
or a `[left "=" right]` comparison. A name that is a Nix keyword or an
attribute of `generators.shell`, or a local named like a script-level name
(which Nix would shadow), gets a trailing `'`; a name nothing declares
and the generator does not list, `local` outside a function, `export` and
other declarations, a function inside a function, a name set inside `$(...)`,
braces or backslashes the printer would not write, a heredoc whose delimiter is
not single-quoted, `[[`, `$((...))` and backquotes are errors.
tree-sitter-glsl reads `(float(x) + y)` as a broken cast, so the importer
parses a constructor call of a builtin scalar type (`float(`, `int(`, `uint(`,
`bool(`, `double(`) as a call of an identifier, keeping its source bytes.
Lua keeps its layout and binds its names: `import lua` prints the whole data
file, a function of the constructors and globals of `generators.lua` it uses
(`{local, pcall, pg, table', ...}: ({INTERVAL, log, ...}: [ ... ])`). A block
that declares names is a lambda whose formals are its locals, local functions,
parameters or loop variables; a function with parameters is `({self, r}:
{params = [self vararg]; body = ...;})`, a loop `{names; values; body}` or
`{name; range; body}`, the order Lua needs being a list of bound variables. A
global is one of the generator's fixed list (Lua's standard globals, LuaJIT's
`bit` and `jit`, Aniimo's `CS`, `pg` and `Vector3`, Hyprland's `hl`), mirrored
in the importer; a name that is a Nix keyword, a constructor or a global is
primed (`table'`, `assert'`, a local `field'`). A call is a list `[f x]`, a
field path `dot pg "me.space"`, a method call `[(method x "m") y]`, an
operator run an infix list `[a "and" b "==" c]` nested exactly where the source
writes parentheses; `paren` is kept only around a call or `...`, whose first
value it keeps, and the parentheses before `.`, `:`, `[` or `(` that a
non-prefix expression needs are left to the printer. A name nothing declares,
a local named like a primed global, a name used before a local of its own
block declares it (the Nix binding would capture it), a global function
declaration, any other parentheses, a number Nix does not spell as the source
does (`0x10`, `1e3`), a long bracket string, `repeat` and `goto` are errors.
Python's `@name@` substitution placeholders are names to the importer.
Python is read as code Nix binds: `import python` prints the whole data file,
`{call, def, print, str', ...}: ({os, Path, main}: [ ... ])`, a function of
the constructors it uses and the Python builtins it names (a builtin spelled
like a constructor takes a `'`), returning the module's scope. Each Python
scope (the module, a `def`, a `class` body, a `lambda`, a comprehension) is a
lambda whose formals are the names Python binds in it (imports, assignment,
`for`, `with ... as`, `except ... as` and `:=` targets, `def` and `class`
names, parameters), `(def f ({a, x}: {params = [a]; body = [ ... ];}))`, so
every name is a Nix variable resolved as Python resolves it; a name that is a
Nix keyword, a constructor or a builtin takes a `'`. Statements and
expressions are typed constructors (`assign`, `call`, `dot x "environ.get"`,
`ternary`, `listComp`, ...), an operator chain an infix list nested only where
Python's precedence needs parentheses, `True` and `False` Nix booleans and
`@name@` substitution placeholders `placeholder "name"`. A name nothing binds
that is not a builtin the generator knows, a name Nix would bind to a
different scope than Python (a parameter default or a comprehension's first
iterable naming what its own scope binds, a class attribute a method names, a
`global` an enclosing function also binds), a `:=` outside an `if` or `while`
test, a name Nix cannot spell and a parenthesis neither the grouping nor a
line break needs are errors.
Markdown is read as typed blocks and inlines: `heading`, a tight `- ` list
(`bullets`), `table header rows`, an HTML `comment` and paragraphs of text,
`code`, `strong`, `em` and `link text url`. Text is only literal: brackets
that are no link stay text around their typed inlines, and text that would
read as markup the printer does not write (a stray `*`, `` ` ``, `\`, `<`
or `&`, `~~`, `](`, `][`, `![`, an `_` at the edge of a word) is an error, as
is every other inline node (escapes, entities, HTML, autolinks, reference
links, strikethrough).
VBScript is read for its names too: `import vbs` prints a script as a
function of the constructors and of the VBScript and WScript globals it uses,
`{CreateObject, WScript, dim, ...}: ({sh, wav}: [ ... ])`. The script and each
`Sub` or `Function` body is a lambda of the names it declares (its `Dim`s,
wherever they stand, its procedures, a procedure's parameters), so every name
is a Nix variable and one the generator or Nix already uses is spelled with a
trailing `'`. A member is `dot base "A.B"`, a call `call f [args]` (written
`f args` as a statement), integers and booleans are Nix values, and operators
are infix lists nested only where VBScript's precedence needs parentheses; the
importer re-reads each operator run by that precedence, since the grammar binds
`Not` looser than VBScript does. A name nothing declares that is not a known
global, a name declared twice, a number other than a plain integer, `ElseIf`,
`Call`, a statement call with parenthesized arguments and any layout the
printer would not give back are errors.

`strictix check` and `strictix fix` lint a merged file binding by binding, as
the files its manifest names: each binding is restored in memory and linted
at its manifest path, so `[schemas]` prefixes, `lint.generated`,
`.strictixignore`, path existence (`dangling-path`, `missing-import`) and the
project-wide rules see the same tree as before the merge. Diagnostics point
into the merged file; a fix is applied there when every edit falls inside text
the binding copies verbatim, and is withheld otherwise.

## Rules

103 builtin rules (`strictix list`), plus the pipeline-level `syntax-error` diagnostic:

| Family | Rules |
| --- | --- |
| Dead code | unused-let-binding, unused-lambda-param, unused-formal, unused-inherit, unused-rec-binding, redundant-with, unnecessary-rec, unreachable-branch |
| Certain evaluation errors | self-referential-let, circular-let, undefined-variable, non-boolean-condition, ill-typed-binop, ill-typed-unary-op, non-callable-application, literal-division-by-zero, coerced-interpolation, assert-false, duplicate-attribute, duplicate-formal, dangling-path, missing-import, import-cycle, missing-function-argument, unexpected-function-argument, missing-attribute, builtin-argument-type, builtin-arity, invalid-list-access, invalid-list-to-attrs-entry, replace-strings-length-mismatch, invalid-builtin-range |
| Hallucination checks | unknown-option, option-type-mismatch, unknown-builtin, unknown-lib-type |
| Traps | shadowed-binding, shadowed-formal, rebound-constant, bare-import-in-list, accidental-path-division, optional-list-argument, repeated-keys, unquoted-uri, search-path-reference, duplicate-inherit, unnecessary-or, duplicate-list-to-attrs-name, duplicate-function-argument, dynamic-import, suspicious-import-argument, suspicious-recursion, unsafe-with-shadowing, shallow-merge-overwrite (opt-in), discouraged-lib-type, masked-generator |
| Modules | mixed-module-syntax, config-dependent-imports, unused-option (opt-in), never-enabled-option (opt-in), constant-option (opt-in) |
| Maintainability | cyclomatic-complexity, cognitive-complexity |
| Simplification | constant-if, constant-if-branches, constant-boolean-not, constant-boolean-binop, boolean-if, tautology, assert-true, negation-simplification, trivial-let, identity-lambda, empty-attrset-merge, empty-list-concat, singleton-list-concat, singleton-optionals, singleton-attrset (opt-in), singleton-list (opt-in), collapsible-let-in, empty-let-in, eta-reduction, empty-pattern, redundant-pattern-bind, empty-inherit, useless-parens, useless-has-attr, redundant-boolean-comparison, redundant-interpolation, redundant-quoted-attr, duplicate-literal-list-item, manual-inherit, manual-inherit-from, manual-hasattr, manual-getattr, manual-optional, manual-gen-attrs, manual-file-contents, deprecated-is-null, deprecated-to-path, hand-written-config, hand-maintained-data-file, hand-written-lua-data, codec-candidate, duplicate-definitions (opt-in) |

`cognitive-complexity` warns when a lambda scores above **5**. It counts
control-flow nesting, conditional helpers (`lib.mkIf`, `lib.optional*`),
and boolean operator sequences, measuring nested lambdas separately. Ordinary
attribute-set nesting adds no cost. It has no automatic fix.

| Construct | Cost |
| --- | --- |
| `if` | 1 plus enclosing conditional depth |
| Final `else` | 1 |
| Direct `else if` | 1; stays at the preceding `if`'s depth |
| `lib.mkIf`, `lib.optional`, `lib.optionals`, `lib.optionalAttrs`, `lib.optionalString` (fully applied) | 1 plus enclosing conditional depth |
| Run of the same `&&`, `||` or `->` | 1 |
| Nested lambda | scored on its own, from depth zero |

Conditions and branches raise the depth for their children, so
`x: if a then (if b then 1 else 2) else 3` scores 5 and passes, one more
nested `if` scores 9, and a flat three-condition `else if` chain scores 4.
Helpers are recognised through the module's `lib`, its local aliases,
`inherit (lib)` and `with lib;`; the linter does not evaluate imports. The
maximum of 5 was calibrated on `~/finix`, which scores at most 5 everywhere.

`duplicate-literal-list-item` skips a list of only strings that is the whole
of a `*.data.nix` file (a text kept one line per string, where blank and
closing lines repeat by nature), and every list of a `*.data.nix` file that is
a function returning a list or, for GLSL's keyed shaders, a function of `...`
constructors returning an attribute set (a syntax tree built from a generator's constructors, where a symbol repeats as
often as the code uses it), including such a binding of a merged file; the
schema rules do not read that file as a module. `duplicate-function-argument` skips that
syntax-tree file too, where a constructor takes the same name twice as often
as the code does (Luau's `f(x, x)` is `[f x x]`). `shadowed-formal` skips it as well: its nested
block lambdas bind the names each block of the code declares, so an inner
block's local shadows an outer one exactly where the code's own scoping does.

`singleton-optionals` simplifies `lib.optionals condition [value]` to
`lib.optional condition value`. Qualified calls have an automatic fix that
preserves comments and expression grouping. Inherited or aliased calls are
reported without a fix because `optional` may not be in scope. Empty lists,
multi-item lists, and explicit nested-list elements are left alone.

`strictix explain <code>` prints a rule's full description:

```
$ strictix explain trivial-let
code: trivial-let
name: Trivial let
severity: warning
kind: node
description: Flags a let with exactly one binding whose body is exactly
that binding's name: `let x = e; in x` is just `e`.
```

The semantic checks conservatively inspect literal values and known local
bindings; dynamic values remain unknown. The new argument, attribute, builtin-type, list-access, duplicate-name, and
merge checks report diagnostics without automatic fixes.

`invalid-list-to-attrs-entry` checks literal entries for required `name` and
`value` fields and string names, respecting entries ignored because an earlier
name wins. `replace-strings-length-mismatch` compares literal replacement-list
lengths. `invalid-builtin-range` rejects negative `genList` lengths and
`substring` start positions; negative substring lengths remain valid.
These checks report errors without automatic fixes and leave dynamic values
unknown. `builtin-arity` accepts partial applications and calls through
builtins that can return functions, such as `head`, `elemAt`, and `getAttr`.

The module checks require a recognizable module body and no options schema.
`mixed-module-syntax` reports ordinary definitions beside an explicit `config`
or `options` section; module metadata is allowed. `config-dependent-imports`
warns when choosing imports requires reading the module's `config`. Import
modules unconditionally and put the condition on definitions with `lib.mkIf`.
Deferred configuration inside imported module bodies is left alone. Neither
rule has an automatic fix.

`unused-option` is opt-in and project-wide: it reports `options.*`
declarations that nothing in the linted files reads. Reads are counted
conservatively: `config.a.b` or `options.a.b` off a module argument, `cfg = config.a;`
aliases (so `cfg.b` reads `a.b`), `inherit (config.a) b;`,
`lib.attrByPath [ "a" "b" ] d config`, and `system.config.a.b` on an
evaluated system. Anything that might read a subtree counts as reading all
of it. Lint the whole project in one run, or options read only by
unlinted files are reported. A library that declares options for outside
consumers should leave it off.

`never-enabled-option` is opt-in and project-wide: it reports enable
options (`mkEnableOption`, or `mkOption` of type `bool` defaulting to
`false`) that no linted file ever defines, so everything gated on them is
dead. Any definition of the option or of a parent attrset counts, with or
without a `config.` prefix and through `mkIf`/`mkMerge`/`mkDefault`
wrappers, including inline modules in `imports`/`modules` lists. A
dynamic segment (`${...}`), an `inherit`, or a non-literal value at a
prefix that could reach the option counts as setting it.

`constant-option` is opt-in and project-wide: it reports `mkOption`
declarations with a literal default that no linted file ever defines,
matching definitions exactly as `never-enabled-option` does, so every
configuration uses the default. Replace such an option with a `let` value
in the module that reads it, or delete it if nothing reads it. Modules
from flake inputs are not linted, so an option they define is still
reported unless the `defined` list below names it.
The `readers` setting (`[lint.constant-option] readers = 3`) keeps such an
option when at least that many linted files read it, counting reads as
`unused-option` finds them (the declaring module counts), for a project
whose convention is that a value several modules share stays an option.
The `defined` list (`defined = ["a.b"]`) names options that something outside
the linted files defines, such as a module from a flake input that a linted
module imports, so an evaluation of the configurations can supply them.

`singleton-attrset` is opt-in: it reports an attribute-set binding whose
value is a set of exactly one binding written over several lines,
`x = {\n  y = v;\n};`, and fixes the whole chain of such sets into one
attribute path, `x.y = v;`, moving the value's lines out by the two columns
each dissolved set indented them (unless a quoted string spans lines or a
line sits further out). Nix builds the same value, and the two lines of
braces each set cost are gone. It leaves alone a `let` binding, a binding
of a `rec` set, a binding with a sibling that starts with the same name
(`repeated-keys` keeps that grouping), a set written on one line, a set
holding an `inherit` and any comment the rewrite would drop. A finding
inside another's rewrite gets its fix on the next `strictix fix` pass. The
`exempt` list skips files whose path ends with one of its entries:

```toml
[lint.singleton-attrset]
exempt = ["flake.nix", ".data.nix"]
```

`singleton-list` is opt-in: it reports a list of exactly one item written
over several lines whose item fits on one line, `[\n  x\n]`, and fixes it
to `[x]`. A list holding a comment is left alone, and its `exempt` list
works as `singleton-attrset`'s does.

`discouraged-lib-type` flags `types.attrs`, which merges definitions
shallowly and does not discharge `mkIf`/`mkDefault`, and fixes it to
`types.attrsOf types.anything`. It also flags `types.unspecified`, without
a fix. `manual-gen-attrs` rewrites `listToAttrs (map (x: { name = x;
value = e; }) xs)` to `lib.genAttrs xs (x: e)`.

`shallow-merge-overwrite` is opt-in because replacing a nested attribute set
with `//` can be intentional. Enable it with
`strictix check . --enable shallow-merge-overwrite`, or in `strictix.toml`:

```toml
[lint]
enabled = ["shallow-merge-overwrite"]
```

`--enable` is repeatable. `disabled` / `--disable` takes precedence over explicit
enabling. `strictix list` marks opt-in rules; `strictix explain` shows whether a
rule runs by default.

Config (strictix.toml) toggles rules; .strictixignore prunes paths.
`[lint] generated = ["path/prefix", ...]` marks generated data files (matched like
`[schemas]` prefixes) so `hand-maintained-data-file` skips them.
A `[lint.<rule-code>]` section holds that rule's settings, such as
`[lint.constant-option] readers = 3`; a rule documents the settings it reads.
`[codecs.<id>] extensions = ["json"]` sections enable `codec-candidate`, which flags literal data texts and data files of those extensions and adds a `codec` object to `--format json` output for a converter to rewrite them as Nix values. A codec section with `generator = "lispSource"` also flags `generators.lispSource (import ./x.data.nix)`, a line list, as a `file` candidate with the `lines` loader, for the converter to replace with the syntax tree `strictix import` prints.
For a one-off finding, put a full-line `# strictix: disable-next-line=CODE`
comment immediately above the code it reports. The directive is intentionally
narrow: malformed or unknown codes are ignored, and it suppresses only that
rule's diagnostic on the next source line. `--schema options.json` enables the
NixOS option checks (unknown option names on both the read and write side,
plus literal/type mismatches); `--format json` selects machine output.

A repository that evaluates more than one module system (hosts, a
flake-level `evalModules`, nix-on-droid, ...) needs one schema per system.
Map path prefixes to schemas in `strictix.toml`; the longest prefix wins
over `lint.schema`, and `""` turns the schema checks off for that prefix:

```toml
[lint]
schema = "options/hosts.json"

[schemas]
"modules/hosts/android-phone" = "options/droid.json"
"modules/flake" = ""
```

Generate an options.json from any module evaluation `e`:

```
nix eval --impure --json --expr 'let lib = (builtins.getFlake (toString ./.)).inputs.nixpkgs.lib; e = ...; in { options = builtins.listToAttrs (map (o: { name = o.name; value = { type = o.type or ""; }; }) (lib.optionAttrSetToDocList e.options)); }'
```

## Development

See [docs/architecture.md](./docs/architecture.md) for locked
decisions, the module map, and the milestone roadmap.

## License

AGPL-3.0-only. See [LICENSE](./LICENSE).

Copyright (C) 2026 y0usaf
