# strictix

> Strict lints and suggestions for the Nix programming language.

strictix is a from-scratch Nix linter in the Ruff/Prettier mold: a
single binary, run, report, exit. Every component — lexer, parser,
syntax tree, semantic model, rules engine, CLI — is owned and written
in-house. Design center: catch the semantic mistakes that AI-generated
and hand-written Nix actually make: hallucinated option names, dead
bindings, shadowing, with-smuggling.

## Usage

```shell
nix build
./result/bin/strictix check ~/nixos-config        # lint a repo
./result/bin/strictix fix ~/nixos-config          # apply safe fixes
./result/bin/strictix check --schema options.json # check config.* paths
./result/bin/strictix list                        # all rules
./result/bin/strictix explain unused-let-binding  # one rule
```

Config (strictix.toml) toggles rules; .strictixignore prunes paths;
--format json for machine output.

## Development

```shell
cargo test --workspace   # unit + integration tests
nix flake check          # build + tests via Nix
```

See DESIGN.md for locked decisions and the roadmap (M0–M8, all
complete), and docs/architecture.md for the module map.
