# Architecture

`strictix` has the following components:

- `crates/cli`: the CLI entrypoint
- `crates/core`: library of lints and utilities to define them
- `crates/strictix-vfs`: virtual filesystem
- `crates/strictix-macros`: procedural macros to help define a lint
- `nix/parts`: flake-parts modules for packaging, dev tooling, and checks
- `integrations/`: editor and tool integrations


## crates/cli

This is the main point of interaction between `strictix` and
the end user. Its output is human-readable and should also
support JSON/errorfmt outputs for external tools to use.


## crates/core

A library of AST-based lints and utilities to help write
those lints. It should be easy for newcomers to write lints
without being familiar with the rest of the codebase.


## crates/strictix-vfs

VFS is an in-memory filesystem. It provides cheap-to-copy
handles (`FileId`s) to access paths and file contents.


## crates/strictix-macros

This crate intends to be a helper layer to declare lints and
their metadata.


## nix/parts

These modules keep the root flake small and isolate packaging,
formatting, CI-like checks, and development shell concerns.
