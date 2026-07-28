# strictix

> Strict lints and suggestions for the Nix programming language.

strictix is a from-scratch Nix linter in the Ruff/Prettier mold: a
single binary, run, report, exit. Every component — lexer, parser,
syntax tree, semantic model, rules engine, CLI — is owned and written
in-house. Design center: catch the semantic mistakes that AI-generated
and hand-written Nix actually make.

## Status

Early rewrite. See `docs/architecture.md` for milestones (M0–M8) and
`DESIGN.md` for locked decisions.

## Usage

```shell
nix build
./result/bin/strictix --help
```
