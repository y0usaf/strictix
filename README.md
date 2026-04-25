# strictix

> Strict lints and suggestions for the Nix programming language.

`strictix` is a fork of [statix](https://github.com/nerdypepper/statix) with additional, stricter lints targeting anti-patterns common in real NixOS configurations.

`strictix check` highlights anti-patterns in Nix code. `strictix fix` can automatically fix several such occurrences.

## Repo Layout

- `crates/cli`: command-line interface and integration tests
- `crates/core`: lint definitions and analysis/fix logic
- `crates/strictix-macros`: proc-macros used to declare lints and tests
- `crates/strictix-vfs`: in-memory filesystem support
- `docs/`: architecture notes and release docs
- `integrations/`: editor integration code such as the Vim plugin
- `nix/parts`: flake-parts modules used by the root flake

## Examples

```shell
$ strictix check tests/c.nix
[W04] Warning: Assignment instead of inherit from
   ╭─[tests/c.nix:2:3]
   │
 2 │   mtl = pkgs.haskellPackages.mtl;
 · ───────────────┬───────────────
 ·                ╰───────────────── This assignment is better written with inherit
───╯

$ strictix fix --dry-run tests/c.nix
--- tests/c.nix
+++ tests/c.nix [fixed]
@@ -1,6 +1,6 @@
 let
-  mtl = pkgs.haskellPackages.mtl;
+  inherit (pkgs.haskellPackages) mtl;
 in
 null
```

## Installation

```shell
# build from source
nix build github:y0usaf/strictix
./result/bin/strictix --help

# run directly
nix run github:y0usaf/strictix -- check /path/to/dir
```

## Usage

```shell
# recursively find nix files and raise lints
strictix check /path/to/dir

# ignore generated files
strictix check /path/to/dir -i Cargo.nix

# ignore more than one file
strictix check /path/to/dir -i a.nix b.nix c.nix

# ignore an entire directory
strictix check /path/to/dir -i .direnv

# strictix respects .gitignore by default
strictix check /path/to/dir

# ignore .gitignore handling completely
strictix check /path/to/dir -u

# enable all lints including opt-in ones
strictix check /path/to/dir --strict

# enable specific opt-in lints in addition to the default set
strictix check /path/to/dir -e with_expression -e single_use_let
```

Apply suggestions:

```shell
strictix fix /path/to/file

# show diff, do not write to file
strictix fix --dry-run /path/to/file

# fix with opt-in lints enabled
strictix fix --strict /path/to/file
```

Output formats:

```shell
strictix check /path/to/dir -o json    # requires --all-features build
strictix check /path/to/dir -o errfmt  # single-line, integrates with vim
```

### Configuration

Create a `strictix.toml` to configure lints:

```toml
# strictix.toml

# Disable specific lints
disabled = [
  "empty_pattern",
]

# Enable opt-in lints (see table below)
enabled = [
  "with_expression",
  "single_use_let",
]

# Add extra gitignore-style ignore patterns during traversal
ignore = [".direnv", "result"]

# Or enable all opt-in lints at once
strict = true

# Optional per-lint settings
[lints.unused_pattern_param]
remove_ellipsis = false # true: { config, lib, ... }: config → { config }: config
```

By default, `strictix` discovers configuration relative to the target you pass to `check`, `fix`, or `single`: it looks for `strictix.toml` starting from that file or directory and then walks upward through parent directories.

Use `--config <path>` to override discovery explicitly. The path may point either to `strictix.toml` itself or to a directory containing it.

Config-driven lint enables are additive: the default lint set stays enabled, `enabled = [...]` adds opt-in lints on top, and `--enable` adds more for a particular invocation. `strict = true` or `--strict` enables all opt-in lints. `disabled = [...]` is applied last and takes precedence over both `enabled` and `strict`.

`strictix` also respects `.gitignore` files by default when walking directories. Pass `-u`/`--unrestricted` to ignore `.gitignore` handling, and use `ignore = [...]` or `-i/--ignore` for extra project-specific gitignore-style exclusions.

### Lints

Inherited from `statix`:

| Code | Name                     | Auto-fix | Description                                         |
| ---- | ------------------------ | -------- | --------------------------------------------------- |
| W01  | `bool_comparison`        | ✓        | `x == true` → `x`                                   |
| W02  | `empty_let_in`           | ✓        | `let in expr` → `expr`                              |
| W03  | `manual_inherit`         | ✓        | `x = y; inherit x;` style repetition → `inherit x`  |
| W04  | `manual_inherit_from`    | ✓        | `a = x.y.z.a` → `inherit (x.y.z) a`                 |
| W05  | `legacy_let_syntax`      | ✓        | `let { body = ...; }` → `let ... in ...`            |
| W06  | `collapsible_let_in`     | ✓        | Merge nested `let in` expressions                   |
| W07  | `eta_reduction`          | ✓        | `x: f x` → `f`                                      |
| W08  | `useless_parens`         | ✓        | Remove unnecessary parentheses                      |
| W09  | `unquoted_splice`        | ✓        | Quote bare `${...}` splices in string contexts      |
| W10  | `empty_pattern`          | ✓        | `{...}: expr` → `_: expr`                           |
| W11  | `redundant_pattern_bind` | ✓        | `{...} @ args: expr` → `args: expr`                 |
| W12  | `unquoted_uri`           | ✓        | Quote bare URIs in string contexts                  |
| W14  | `empty_inherit`          | ✓        | Remove empty `inherit;`                             |
| W17  | `deprecated_to_path`     | ✓        | Warn on deprecated `toPath`/`builtins.toPath` usage |
| W18  | `bool_simplification`    | ✓        | `!(x == y)` → `x != y`, `!(x != y)` → `x == y`      |
| W19  | `useless_has_attr`       | ✓        | `if x ? a then x.a else d` → `x.a or d`             |
| W20  | `repeated_keys`          | ✓\*      | Suggest grouping repeated attrpath prefixes         |
| W23  | `empty_list_concat`      | ✓        | `[] ++ x` → `x`                                     |

Added by `strictix`:

| Code | Name                       | Auto-fix | Opt-in | Description                                                 |
| ---- | -------------------------- | -------- | ------ | ----------------------------------------------------------- |
| W24  | `with_expression`          | ✓\*      | ✓      | Warn on `with`; breaks tooling and causes shadowing         |
| W25  | `collapsible_inherit_from` | ✓        |        | `inherit (x) a; inherit (x) b;` → `inherit (x) a b;`        |
| W26  | `empty_attrset_merge`      | ✓        |        | `{} // x` → `x`                                             |
| W27  | `redundant_if_bool`        | ✓        |        | `if x then true else false` → `x`                           |
| W28  | `if_else_empty_attrset`    | ✓        |        | Suggest `lib.optionalAttrs` over `if c then {...} else {}`  |
| W29  | `unnecessary_rec`          | ✓        |        | Remove `rec` when no binding references a sibling           |
| W30  | `single_use_let`           | ✓\*      | ✓      | Inline or remove `let` bindings used at most once           |
| W31  | `unused_lambda_param`      | ✓        |        | `x: expr` where `x` is unused → `_: expr`                   |
| W32  | `unused_pattern_bind`      | ✓        |        | `args @ { x }: x` where `args` is unused → `{ x }: x`       |
| W33  | `if_else_empty_list`       | ✓        |        | `if c then [...] else []` → `lib.optionals c [...]`         |
| W34  | `repeated_expression`      |          |        | Expression repeated; consider extracting into a let binding |
| W35  | `unsorted_pattern_params`  | ✓        |        | Sort pattern params: `config`, `lib`, `pkgs`, then alphabetical |
| W36  | `list_concat_merge`        | ✓        |        | Merge adjacent unconditional list literals in concatenations     |
| W37  | `unused_inherit`           | ✓        |        | Remove unused names from `let`-level `inherit` statements        |
| W38  | `unused_pattern_param`     | ✓\*      | ✓      | Remove unused named params from variadic function patterns       |

`*` Conditional autofix: some flagged cases are intentionally left as diagnostics when a rewrite would be unsafe or would discard comments.

**Opt-in lints** are disabled by default and must be explicitly enabled via `--strict`, `-e <lint>`, or config file. Use `strictix list` to see which lints are opt-in.
