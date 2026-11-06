# strictix

strictix lints Nix code and fixes what it safely can.

It is one Rust binary with its own Nix parser. It reports code that will fail or is likely wrong (option names and paths that don't exist, type errors in literals, dead bindings, shadowing) and suggests simpler forms of code that works. It has 115 rules; 14 are opt-in.

## Build and run

```
nix build                  # result/bin/strictix
nix run . -- check .
nix flake check            # build, plus the import round-trip check
nix develop                # cargo, rustc, rustfmt, clippy
```

`check` exits 1 when it finds anything:

```
$ strictix check slop.nix
slop.nix:6:7: error[dangling-path]: path './missing' does not exist (resolved to missing)
  p = ./missing;
      ^^^^^^^^^
  help: create the file or correct the path
```

## Commands

| Command | Does |
| --- | --- |
| `check [PATH...]` | Lint files or directories (default `.`). |
| `fix [PATH...]` | Apply every safe fix, repeating until nothing changes. `--dry-run` shows the diff. |
| `list` | List rules, marking opt-in ones. |
| `explain CODE` | Print one rule's full description and settings. |
| `merge --entry FILE --out FILE` | Inline every `.nix` file reachable from the entry into one file. |
| `split --merged FILE` | Write a merged file back out to the original files. |
| `import LANGUAGE FILE` | Print a source file as the Nix syntax tree that finix's generators render. |

Common flags: `--format human|json`, `--enable CODE`, `--disable CODE` (wins over enable), `--schema options.json`, `--config FILE`, `--ignore-file FILE`.

**merge / split.** Each merged file becomes a `let` binding, and a `merge_manifest` value records where every binding came from, so `split` can restore the tree. Edits made in the merged file land in the original files. `--expand NAME` evaluates calls of a file-discovery helper into explicit module lists. `check` and `fix` lint a merged file as the files it came from.

**import.** Languages: glsl, javascript, lisp, lua, luau, markdown, patch, python, shell, ts, vbs. It refuses anything the generator could not print back byte for byte, and names the line. `checks/import` holds one sample per language; `nix flake check` renders each with nix and iets and requires an exact match.

## Dependencies

Build: Rust and the tree-sitter grammars in `Cargo.lock`. Run time: nothing for `check` and `fix`. `merge` and `split` call `nix` to evaluate `--expand` helpers and to run the flake's `formatter` when it has one. The `unformatted-file` rule runs whatever formatter you configure.

## Config

`strictix.toml` in the current directory, or `--config FILE`:

| Key | Meaning |
| --- | --- |
| `[lint] enabled = [...]` | Turn on opt-in rules. |
| `[lint] disabled = [...]` | Turn rules off. |
| `[lint] schema = "options.json"` | Option schema for the option checks. |
| `[lint] generated = [...]` | Path prefixes of generated data files, skipped by rules about hand-maintained data. |
| `[schemas] "prefix" = "file.json"` | Per-path schema; longest prefix wins, `""` turns option checks off there. |
| `[lint.RULE] ...` | That rule's settings; `strictix explain RULE` lists them. |
| `[codecs.ID] extensions = [...]` | Enables `codec-candidate` for data files with those extensions. |

`.strictixignore` lists paths to skip. A full-line `# strictix: disable-next-line=CODE` comment silences one rule on the next line.

To make a schema from any module evaluation `e`:

```
nix eval --impure --json --expr 'let lib = (builtins.getFlake (toString ./.)).inputs.nixpkgs.lib; e = ...; in { options = builtins.listToAttrs (map (o: { name = o.name; value = { type = o.type or ""; }; }) (lib.optionAttrSetToDocList e.options)); }'
```

## License

AGPL-3.0-only. See [LICENSE](./LICENSE).
