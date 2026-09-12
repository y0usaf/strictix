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

## Rules

90 builtin rules (`strictix list`), plus the pipeline-level `syntax-error` diagnostic:

| Family | Rules |
| --- | --- |
| Dead code | unused-let-binding, unused-lambda-param, unused-formal, unused-inherit, unused-rec-binding, redundant-with, unnecessary-rec, unreachable-branch |
| Certain evaluation errors | self-referential-let, circular-let, undefined-variable, non-boolean-condition, ill-typed-binop, ill-typed-unary-op, non-callable-application, literal-division-by-zero, coerced-interpolation, assert-false, duplicate-attribute, duplicate-formal, dangling-path, missing-import, import-cycle, missing-function-argument, unexpected-function-argument, missing-attribute, builtin-argument-type, builtin-arity, invalid-list-access, invalid-list-to-attrs-entry, replace-strings-length-mismatch, invalid-builtin-range |
| Hallucination checks | unknown-option, option-type-mismatch, unknown-builtin, unknown-lib-type |
| Traps | shadowed-binding, shadowed-formal, rebound-constant, bare-import-in-list, accidental-path-division, optional-list-argument, repeated-keys, unquoted-uri, search-path-reference, duplicate-inherit, unnecessary-or, duplicate-list-to-attrs-name, duplicate-function-argument, dynamic-import, suspicious-import-argument, suspicious-recursion, unsafe-with-shadowing, shallow-merge-overwrite (opt-in) |
| Modules | mixed-module-syntax, config-dependent-imports |
| Maintainability | cyclomatic-complexity, cognitive-complexity |
| Simplification | constant-if, constant-if-branches, constant-boolean-not, constant-boolean-binop, boolean-if, tautology, assert-true, negation-simplification, trivial-let, identity-lambda, empty-attrset-merge, empty-list-concat, singleton-list-concat, singleton-optionals, collapsible-let-in, empty-let-in, eta-reduction, empty-pattern, redundant-pattern-bind, empty-inherit, useless-parens, useless-has-attr, redundant-boolean-comparison, redundant-interpolation, duplicate-literal-list-item, manual-inherit, manual-inherit-from, manual-hasattr, manual-getattr, manual-optional, deprecated-is-null, deprecated-to-path |

`cognitive-complexity` warns when a lambda scores above **5**. It counts
control-flow nesting, conditional helpers (`lib.mkIf`, `lib.optional*`),
and boolean operator sequences, measuring nested lambdas separately. Ordinary
attribute-set nesting adds no cost. It has no automatic fix. See [scoring and default calibration](docs/cognitive-complexity.md) for the
Nix-specific algorithm and measurements from `~/finix`.

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
For a one-off finding, put a full-line `# strictix: disable-next-line=CODE`
comment immediately above the code it reports. The directive is intentionally
narrow: malformed or unknown codes are ignored, and it suppresses only that
rule's diagnostic on the next source line. `--schema options.json` enables the
NixOS option checks (unknown option names on both the read and write side,
plus literal/type mismatches); `--format json` selects machine output.

## Development

See [docs/architecture.md](./docs/architecture.md) for locked
decisions, the module map, and the milestone roadmap.
