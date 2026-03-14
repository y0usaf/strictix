# TODO

Purpose: a durable, repo-specific backlog for raising `strictix` from "strong niche tool" to a locally verifiable, high-trust codebase without relying on hosted CI.

## Local Quality Gate

- [ ] Add one canonical local entrypoint for repo checks using `cargo xtask`, `just`, or a Nix app.
- [ ] Define `fast`, `full`, and `release` check tiers with clear scope and runtime expectations.
- [ ] Make `fast` run formatting checks, `clippy`, and the normal test suite.
- [ ] Make `full` run `fast` plus all-features checks, doc tests, and slower integration/corpus checks.
- [ ] Make `release` run `full` plus coverage, fuzz smoke tests, and benchmarks.
- [ ] Document the exact commands required before merge and before release.

## Local Automation

- [ ] Extend local git hooks so `pre-commit` runs the cheap gate.
- [ ] Extend local git hooks so `pre-push` runs the stronger gate.
- [ ] Ensure hook installation is part of the normal dev-shell workflow.
- [ ] Add a way to bypass expensive checks intentionally and visibly when needed.

## Test Depth

- [ ] Add lower-level unit tests for AST matching helpers in `crates/core`.
- [ ] Add lower-level unit tests for replacement construction and rewrite helpers in `crates/core`.
- [ ] Add unit tests for config discovery and resolution in `crates/cli`.
- [ ] Add unit tests for output formatting, especially `json` and `errfmt`.
- [ ] Add explicit tests for malformed or partial Nix input so failures stay graceful.
- [ ] Add differential tests that compare diagnostics and fixes before/after lint behavior changes.

## Rewrite Invariants

- [ ] Add property tests for fix idempotence.
- [ ] Add property tests that assert fixed output remains parseable.
- [ ] Add property tests that assert unrelated source regions stay unchanged after a fix.
- [ ] Add property tests that assert converged output produces no further `fix --dry-run` changes.
- [ ] Add property tests for interactions between multiple fixes applied to the same file.

## Fuzzing

- [ ] Add `cargo-fuzz` targets for parsing and AST traversal.
- [ ] Add `cargo-fuzz` targets for fix application and repeated-fix behavior.
- [ ] Persist discovered crashers as regression fixtures in-repo.
- [ ] Add a local smoke command that runs fuzz targets for a bounded time.

## Real-World Corpus

- [ ] Build a local corpus of real-world Nix files and representative repos.
- [ ] Add a corpus runner for `strictix check`.
- [ ] Add a corpus runner for `strictix fix --dry-run`.
- [ ] Add corpus assertions for fix idempotence and post-fix cleanliness.
- [ ] Track and review false positives from the corpus separately from crashes.

## Coverage

- [ ] Add coverage measurement with `cargo llvm-cov`.
- [ ] Set explicit coverage targets for `crates/core`.
- [ ] Set explicit coverage targets for fix application paths.
- [ ] Make coverage part of the `release` local gate.

## Performance

- [ ] Add benchmarks for parse, lint, and fix phases.
- [ ] Create benchmark inputs for small, medium, and large Nix files.
- [ ] Record local baseline numbers and acceptable regression thresholds.
- [ ] Make major performance regressions fail the `release` gate.

## Robustness

- [ ] Audit panic paths in analysis and fix code.
- [ ] Replace avoidable panics with explicit errors or graceful degradation.
- [ ] Add tests for edge-case syntax, comment-heavy files, and formatting-sensitive rewrites.
- [ ] Confirm fixes preserve comments and layout where intended.

## Lint Quality Standards

- [ ] Write a lint authoring checklist covering intent, exclusions, false positives, and autofix safety.
- [ ] Add per-lint contracts describing what each lint detects and what it intentionally ignores.
- [ ] Add per-lint examples for true positives, false positives to avoid, and fix safety constraints.
- [ ] Revisit severity and configurability so lint metadata is consistent across the workspace.

## Developer Documentation

- [ ] Add or expand `CONTRIBUTING.md` with local workflow, hook setup, and test commands.
- [ ] Document how to add a new lint end-to-end, including snapshots and fix invariants.
- [ ] Document how to update snapshots safely and when snapshot churn is suspicious.
- [ ] Add a local release checklist describing the required validation steps before tagging.

## Architecture Follow-Ups

- [ ] Revisit crate boundaries as lint count grows to keep shared AST/query utilities coherent.
- [ ] Separate detection logic from rewrite logic more consistently where that improves maintainability.
- [ ] Centralize lint metadata if per-lint definitions start to drift.
- [ ] Periodically prune or reorganize snapshot coverage so it stays readable and intentional.

## Suggested Execution Order

1. Add the canonical local check entrypoint and define `fast` / `full` / `release`.
2. Wire local hooks into the cheap and strong gates.
3. Add unit tests for core helpers and CLI config/output paths.
4. Add property tests for rewrite invariants.
5. Add coverage reporting.
6. Add corpus testing.
7. Add fuzzing.
8. Add benchmarks and regression thresholds.
9. Write the contributor and release documentation.
