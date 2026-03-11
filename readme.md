# strictix

> Strict lints and suggestions for the Nix programming language.

`strictix` is a fork of [statix](https://github.com/nerdypepper/statix) with additional, stricter lints targeting anti-patterns common in real NixOS configurations.

`strictix check` highlights anti-patterns in Nix code. `strictix fix` can automatically fix several such occurrences.

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

# strictix respects .gitignore; use -u to disable
strictix check /path/to/dir -u
```

Apply suggestions:

```shell
strictix fix /path/to/file

# show diff, do not write to file
strictix fix --dry-run /path/to/file
```

Output formats:

```shell
strictix check /path/to/dir -o json    # requires --all-features build
strictix check /path/to/dir -o errfmt  # single-line, integrates with vim
```

### Configuration

Create a `strictix.toml` at your project root to disable specific lints:

```toml
# strictix.toml
disabled = [
  "with_expression",
  "empty_pattern",
]
```

`strictix` discovers config by traversing parent directories. Pass an explicit path with `--config`.

### Lints

All lints inherited from statix, plus strictix additions:

| Code | Name                       | Auto-fix | Description                                                |
| ---- | -------------------------- | -------- | ---------------------------------------------------------- |
| W01  | `bool_comparison`          | ✓        | `x == true` → `x`                                          |
| W04  | `manual_inherit_from`      | ✓        | `a = x.y.z.a` → `inherit (x.y.z) a`                        |
| W06  | `collapsible_let_in`       | ✓        | Merge nested `let in` expressions                          |
| W07  | `eta_reduction`            | ✓        | `x: f x` → `f`                                             |
| W08  | `useless_parens`           | ✓        | Remove unnecessary parentheses                             |
| W14  | `empty_inherit`            | ✓        | Remove empty `inherit;`                                    |
| W18  | `bool_simplification`      | ✓        | `!(x == y)` → `x != y`, `!(x != y)` → `x == y`             |
| W19  | `useless_has_attr`         | ✓        | `if x ? a then x.a else d` → `x.a or d`                    |
| W23  | `empty_list_concat`        | ✓        | `[] ++ x` → `x`                                            |
| W24  | `with_expression`          | —        | Warn on `with`; breaks tooling and causes shadowing        |
| W25  | `collapsible_inherit_from` | ✓        | `inherit (x) a; inherit (x) b;` → `inherit (x) a b;`       |
| W26  | `empty_attrset_merge`      | ✓        | `{} // x` → `x`                                            |
| W27  | `redundant_if_bool`        | ✓        | `if x then true else false` → `x`                          |
| W28  | `if_else_empty_attrset`    | —        | Suggest `lib.optionalAttrs` over `if c then {...} else {}` |
| W29  | `unnecessary_rec`          | ✓        | Remove `rec` when no binding references a sibling          |

Run `strictix list` for the full up-to-date list.
