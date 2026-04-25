# Release Checklist

## Pre-release gates

- Ensure the working tree is clean: `git status --short` should be empty.
- Confirm the release branch/commit is the intended one and CI is green.
- Review unreleased changes and decide whether any docs, examples, or integration notes need updating.
- If snapshots changed, review and accept them intentionally:
  - run `cargo test`
  - inspect `.snap.new` files, if any
  - accept expected updates with `cargo insta accept`
  - rerun `cargo test` to confirm the accepted snapshots pass
- Ensure no pending snapshot artifacts remain: `find crates -type f -name '*.snap.new'` should print nothing.

## Versioning and metadata

- Bump the version in `crates/cli/Cargo.toml`.
- Verify Cargo metadata is consistent (`description`, `repository`, `rust-version`, license/workspace inheritance where applicable).
- Regenerate `Cargo.lock` if dependency resolution changed: `cargo build --release`.

## Local verification gates

Run the same core checks expected in CI:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`

If the release depends on optional JSON output support, also verify that feature set explicitly:

- `cargo test --workspace --all-features`
- `find crates -type f -name '*.snap.new'`

## Packaging and distribution

- Build release artifacts: `cargo build --release`.
- Build the Nix package(s) and verify they succeed, e.g. `nix build`.
- Update Cachix or other binary caches if you publish prebuilt artifacts.
- Smoke-test the release binary, for example:
  - `./target/release/strictix --help`
  - `./target/release/strictix check .`

## Tagging and publishing

- Commit the release changes.
- Tag the commit with the release version, e.g. `v0.5.8`.
- Push the commit and tag to the canonical remote.
- Publish release notes/changelog entry if applicable.

## Post-release

- Verify the GitHub/tagged release references the expected commit.
- Confirm downstream installation paths still work (`cargo install`, Nix/flake usage, etc.).
- If follow-up version bumps or next-dev-cycle notes are needed, open them immediately after release.
