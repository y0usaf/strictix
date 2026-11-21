# Strictix

- No unit tests. Keep only end-to-end tests that run the built `strictix` (or `nix` on what it writes) and check the output: `crates/cli/tests` and the `import-inline` flake check.
- Add an end-to-end test only after a bug has escaped, and show it fails on the old build.
- For unchanged behavior, compare exit codes, output bytes and, for `fix`, the resulting files against the previous binary on real repositories such as ~/finix. Run `fix` on a disposable copy.
- Keep temporary verification under `$XDG_RUNTIME_DIR/agent/<slug>/`.
