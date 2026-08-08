# strictix — design & roadmap

strictix is a Nix linter whose design center is catching the *semantic*
mistakes that AI-generated and hand-written Nix actually make:
hallucinated option names, dead bindings, shadowing, `with`-smuggling.
It exists because syntax-level linters can't see these, and the tools
that can (nixd) are evaluator projects, not linters.

## Vision

What "done" feels like:

- **As fast and boring to run as Ruff** — one binary, one command,
  sub-second on a whole config repo, disk cache for anything expensive.
- **As trusted as clippy** — low false-positive rate; every diagnostic
  has an explanation and, where safe, an automatic fix.
- **Sees what no other linter sees** — NixOS option schema checking
  against `options.json`: catches hallucinated option names, the most
  common and most costly AI-Nix failure.

## Doctrine conformance

| Doctrine | Status | Notes |
|---|---|---|
| 01 extension-first core | follows | `strictix-core` holds the public rule API; builtin lints live in `strictix-lints` and use only that API. No third-party stability promised yet. |
| 02 snapshot in, actions out | follows | Rules read an immutable parse/`SemanticModel` snapshot and emit diagnostics/fixes; no rule mutates host state. No watchdog needed: rules are same-process functions, not hosted extensions. |
| 03 daemon + thin client | diverges | Deliberately no daemon. One-shot CLI with disk cache (Ruff model). Rationale: CI/pre-commit/AI-hook usage is one-shot; incremental state buys nothing at linter scale. Relitigate if editor integration ever demands it. |
| 04 declarative front, idempotent executor | n/a | Not a system-provisioning tool. `strictix fix` is idempotent by construction (fixes applied in reverse range order, overlap-checked). |
| 05 one declaration mechanism | follows | Every rule — node or file kind — declared via the same macro and one registry. No hand-wired special cases. |
| 06 bare core must boot | follows | `strictix-syntax` + `strictix-core` compile and pass tests with zero rules and zero config. CI: `nix flake check` builds and tests each crate. |
| 07 nix source of truth | follows | Build and verify via `nix build` / `nix flake check`. `cargo` allowed locally for iteration (sanctioned exception: fmt/clippy/tests). |

## Locked decisions

| Decision | Choice | Rationale |
|---|---|---|
| Parser | hand-written, lossless CST, error recovery from day one | AI-generated Nix is often broken mid-generation; the linter must still diagnose. Owning the parser means owning error quality. |
| Syntax tree | own green tree (`NodeOrToken`, ranges, trivia) | rowan's cleverness is incremental reparse, which a one-shot linter never uses. Owning ~400 lines demystifies everything above it. |
| Dependencies | blessed: `serde`, `toml` only | Comprehension-per-dep ratio. Arg parsing, rendering, walking are small enough to own. |
| Old fork code | strict no-peek, no specs | Rewrite must not inherit decisions by osmosis. Lints written fresh from Nix knowledge. |
| Runtime | one-shot CLI, disk cache later | Matches CI/pre-commit/AI-hook reality; daemon is months of plumbing for zero lint-quality gain. |
| Plugin API | public but unstable | Dogfood via builtin lints; bump semver-major freely until the semantic model settles. |

## Architecture

```
crates/
  syntax/   lexer → parser → green tree → typed AST      (zero deps; core)
  core/     diagnostics, fixes, rule traits, SemanticModel (core)
  lints/    builtin rules, first customer of core API      (policy)
  cli/      arg parsing, rendering, file walking, config   (policy)
```

See `docs/architecture.md` for the full description.

## Deferred (and why)

- **Daemon / LSP server** — one-shot model covers CI + AI hooks; revisit
  only if editor integration demands it.
- **Nix evaluator (even partial)** — schema checks consume
  `options.json` instead; nixd spent years on partial eval.
- **Incremental reparsing** — whole-file parse is milliseconds at
  linter scale.
- **AST-level (green-node) rewriting fixes** — text splices suffice
  until semantic fixes need to compose; overlap check guards meanwhile.
- **Cross-file import DAG** — `ProjectContext` seam exists; built when
  intra-file semantics are done.
- **Third-party plugin stability** — API churns while the semantic
  model settles.

## Roadmap

- [x] M0 — skeleton: workspace, CLI `--help`, flake builds.
  *Accept: `nix build && ./result/bin/strictix --help` prints usage; `nix flake check` green.*
- [x] M1 — lexer: full Nix token set, interpolation mode-stack,
  indented strings, paths, URIs, comments.
  *Accept: lexes every `.nix` file in a real config repo without panic; round-trip test (tokens + trivia = source).*
- [x] M2 — green tree + parser + error recovery.
  *Accept: parses a real config repo with zero panics; broken files produce error nodes + synced recovery, not aborts; lossless round-trip test.*
- [x] M3 — typed AST layer over the CST.
- [x] M4 — SemanticModel: bindings, shadow-aware refs, `with`/`import`
  sites, built lazily once per file.
  *Accept: scope resolution tests incl. shadowing, `rec`, patterns; model builds on real corpus.*
- [x] M5 — rules engine: node rules + file rules, one registry,
  diagnostics renderer, text-splice fixes.
  *Accept: one trivial rule end-to-end via `strictix check` and `strictix fix` on a fixture.*
- [x] M6 — builtin lints, written fresh, with snapshot tests.
  *Accept: growing rule set covering unused-*, shadowing, dead bindings, AI-slop patterns; zero false positives on the author's own config.*
- [x] M7 — CLI + config polish: directory walking, ignore file,
  TOML config, JSON output.
  *Accept: runs clean over a full NixOS config repo; config toggles rules.*
- [x] M8+ — schema checks against `options.json`; AI-slop rule suite.
  *Accept: flags a hallucinated option name in a fixture config.*
