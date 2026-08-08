# strictix

> Strict lints and suggestions for the Nix programming language.

strictix is a from-scratch Nix linter in the Ruff/Prettier mold: a
single binary, run, report, exit. Every component — lexer, parser,
syntax tree, semantic model, rules engine, CLI — is owned and written
in-house. Design center: catch the semantic mistakes that AI-generated
and hand-written Nix actually make: hallucinated option names, dead
bindings, shadowing, with-smuggling.

## Usage

./crates/cli/tests/fixtures/dirty.nix:1:5: warning[unused-let-binding]: binding 'x' is never used
let x = 1; in 2
    ^
./crates/cli/tests/fixtures/ignore/subdir/dirty.nix:1:5: warning[unused-let-binding]: binding 'x' is never used
let x = 1; in 2
    ^
./crates/lints/tests/fixtures/module_bad.nix:1:18: error[unknown-option]: could not load options schema: No such file or directory (os error 2)
{ config, ... }: config.services.exmaple.enable = true;
                 ^^^^^^
./crates/lints/tests/fixtures/module_ok.nix:1:171: error[unknown-option]: could not load options schema: No such file or directory (os error 2)
{ config, lib, pkgs, ... }: { config.services.example.enable = true; options.services.example.port = lib.mkOption { type = lib.types.int; }; environment.systemPackages = config.environment.systemPackages; }
                                                                                                                                                                          ^^^^^^
./crates/syntax/tests/corpus/module.nix:16:17: error[unknown-option]: could not load options schema: No such file or directory (os error 2)
  config = mkIf config.services.example.enable {
                ^^^^^^
./flake.nix:15:49: warning[unused-lambda-param]: parameter 'system' is never used
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
                                                ^^^^^^

./flake.nix:38:24: warning[unused-lambda-param]: parameter 'pkgs' is never used
      checks = forAll (pkgs: {
                       ^^^^
11 file(s) linted, 7 diagnostic(s) found
constant-if              Constant condition           warning  node
assert-true              Assert always true           warning  node
tautology                Tautological comparison      warning  node
unused-let-binding       Unused let binding           warning  file
unused-lambda-param      Unused lambda parameter      warning  file
unused-formal            Unused formal parameter      warning  file
shadowed-binding         Shadowed binding             warning  file
redundant-with           Redundant with               warning  file
self-referential-let     Self-referential let binding error    file
unknown-option           Unknown option               error    file
code: unused-let-binding
name: Unused let binding
severity: warning
kind: file
description: Flags let bindings that are never referenced. Dead bindings are misleading and can be removed.

Config (strictix.toml) toggles rules; .strictixignore prunes paths;
--format json for machine output.

## Development



See  for locked decisions and the roadmap (M0–M8, all
complete), and  for the module map.
