# Architecture

`strictix` is a one-shot Nix linter in the Ruff/Prettier mold: a single
binary, run, report, exit. No daemon, no LSP server. Anything expensive
(e.g. fetched option schemas) is cached on disk, not kept in a server.

**strictix is a from-scratch codebase.** Every component — lexer,
parser, syntax tree, semantic model, rules engine, CLI — is owned and
written in-house. The goal is full comprehension: every module small
enough to hold in your head.

## Locked decisions

- **Own parser.** Lossless CST with error recovery from day one
  (AI-generated code is often broken mid-generation; the linter must
  still produce diagnostics). Error nodes + synchronization at `;`,
  `}`, `in`.
- **Own green tree.** `NodeOrToken`, text ranges, trivia attached to
  tokens so fixes never destroy formatting. No rowan.
- **Minimal blessed deps:** `serde` + `toml` only. Hand-written arg
  parsing, diagnostic rendering, and file walking.
- **Strict no-peek rule:** no code from the original statix fork is
  read or reused during this rewrite. Builtin lint rules are written
  fresh, from Nix knowledge and first principles.
- **Repo:** fresh git history in this folder; the fork repo stays
  untouched.

## Crates

- `strictix-syntax`: lexer, parser, green tree, typed AST layer.
  Zero deps. The foundation everything sits on.
- `strictix-core`: diagnostics, fix machinery, rule traits, and
  `SemanticModel` (scope graph). Depends only on `strictix-syntax`.
- `strictix-lints` (later): all builtin rules. First customer of
  `strictix-core`; uses only its public API. API is **unstable**:
  bump semver-major freely, no stability promises yet.
- `strictix-cli`: entrypoint. Human + JSON output (`serde`),
  TOML config (`toml`). Std threads for per-file parallelism.

## Pipeline

```
text → lex → parse (lossless CST, error recovery)
     → typed AST
     → SemanticModel (lazy, once per file, shared)
     → rules run (node rules + file rules, one registry)
     → diagnostics out (text-splice fixes)
```

## SemanticModel

One walk of a file's tree produces: static bindings (lambda params,
`let` entries, `rec` attrsets, pattern entries), shadow-aware
references resolved to their binding, `with` sites, and `import` sites.
Built lazily — node rules never pay for it; the first file rule that
asks triggers the build, shared thereafter.

## Rules

- **Node rules**: stateless, dispatched by node kind.
- **File rules**: receive the `SemanticModel` + `LintConfig` (no
  global/atomic state). Unused-*, shadowing, dead-binding,
  const-folding rules live here.

Both declared via one mechanism, one registry.

## Fixes

Text splices (range + replacement), applied in reverse order with an
overlap check. AST-level rewriting deferred until semantic fixes must
compose.

## Seams (designed, not built)

- **`ProjectContext`**: empty handle passed to file rules today;
  cross-file import DAG slots in later without signature changes.
- **Disk cache**: materializes when schema checks need it.
- **Schema checks**: verify `config.*` attrpaths against nixpkgs
  `options.json` + locally declared options. No evaluator, ever.

## Non-goals

- Daemon or LSP server.
- A Nix evaluator, partial or otherwise.
- Third-party plugin stability.
- Incremental reparsing (linter scale doesn't need it).

## Milestones

- M0: skeleton — crates exist, `strictix --help` runs, `nix build` green
- M1: lexer (interpolation mode-stack, indented strings, paths, URIs)
- M2: green tree + parser + error recovery
- M3: typed AST layer
- M4: SemanticModel
- M5: rules engine + diagnostics renderer
- M6: builtin lints written fresh (snapshot tests; growing coverage)
- M7: CLI + config polish
- M8+: AI-slop rules, schema checks
