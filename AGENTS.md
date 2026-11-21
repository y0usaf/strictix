# Strictix

- Do not commit unit tests, integration tests, test fixtures, or snapshots.
- Keep the build, linting, rename-table freshness and packaged-binary checks. Verify behavior against real repositories using the built binary. For unchanged behavior, compare exit codes, output bytes and, for `fix`, the resulting files against the previous binary.
- Run `fix` on a disposable copy, never on the reference repo. Keep temporary verification under `$XDG_RUNTIME_DIR/agent/<slug>/`.
