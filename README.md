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

75 builtin rules (`strictix list`), plus the pipeline-level `syntax-error` diagnostic:

| Family | Rules |
| --- | --- |
| Dead code | unused-let-binding, unused-lambda-param, unused-formal, unused-inherit, unused-rec-binding, redundant-with, unnecessary-rec |
| Certain evaluation errors | self-referential-let, circular-let, undefined-variable, non-boolean-condition, ill-typed-binop, ill-typed-unary-op, non-callable-application, literal-division-by-zero, coerced-interpolation, assert-false, duplicate-attribute, duplicate-formal, dangling-path, missing-import, import-cycle |
| Hallucination checks | unknown-option, option-type-mismatch, unknown-builtin, unknown-lib-type |
| Traps | shadowed-binding, shadowed-formal, rebound-constant, bare-import-in-list, accidental-path-division, optional-list-argument, repeated-keys, unquoted-uri, search-path-reference, duplicate-inherit, unnecessary-or |
| Simplification | constant-if, constant-if-branches, constant-boolean-not, constant-boolean-binop, boolean-if, tautology, assert-true, negation-simplification, trivial-let, identity-lambda, empty-attrset-merge, empty-list-concat, singleton-list-concat, collapsible-let-in, empty-let-in, eta-reduction, empty-pattern, redundant-pattern-bind, empty-inherit, useless-parens, useless-has-attr, redundant-boolean-comparison, redundant-interpolation, duplicate-literal-list-item, manual-inherit, manual-inherit-from, manual-hasattr, manual-getattr, manual-optional, deprecated-is-null, deprecated-to-path |

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
