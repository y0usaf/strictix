{
  description = "strictix — strict lints and suggestions for the Nix language";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    nix-generators = {
      url = "github:y0usaf/nix-generators";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.iets.follows = "iets";
    };

    iets = {
      url = "github:numtide/iets";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      nix-generators,
      iets,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      renamesData = "crates/lints/data/nixpkgs-renames.tsv";
      renames =
        pkgs:
        pkgs.writeText "nixpkgs-renames.tsv" (
          import ./nix/nixpkgs-renames.nix {
            inherit nixpkgs;
            system = builtins.head systems;
          }
        );
    in
    {
      packages = forAll (
        pkgs:
        let
          rustPlatform =
            if pkgs.stdenv.isLinux then
              pkgs.pkgsStatic.rustPlatform
            else
              pkgs.rustPlatform;
        in
        {
          default = rustPlatform.buildRustPackage (
            {
              pname = "strictix";
              version = "0.1.0";
              src = ./.;
              cargoLock = {
                lockFile = ./Cargo.lock;
                outputHashes."tree-sitter-vbscript-0.0.1" = "sha256-P1kpdbUropKYs9Bi1FPt+SQTetzzR5HvskmBkRVqMg8=";
              };
            }
            // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
              stripAllList = [ "bin" ];
            }
          );
        }
      );

      apps = forAll (pkgs: {
        nixpkgs-renames = {
          type = "app";
          meta.description = "Write ${renamesData} from the locked nixpkgs aliases";
          program = toString (
            pkgs.writeShellScript "nixpkgs-renames" ''
              set -eu
              test -f ${renamesData}
              install -m 644 ${renames pkgs} ${renamesData}
            ''
          );
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
          ];
        };
      });

      checks = forAll (
        pkgs:
        let
          pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        in
        {
          build = pkg;

          nixpkgs-renames = pkgs.runCommand "strictix-nixpkgs-renames" { } ''
            if ! diff -u ${./. + "/${renamesData}"} ${renames pkgs}; then
              echo "${renamesData} differs from the locked nixpkgs; run nix run .#nixpkgs-renames"
              exit 1
            fi
            touch $out
          '';

          import-inline =
            pkgs.runCommand "strictix-import-inline"
              {
                nativeBuildInputs = [
                  pkg
                  iets.packages.${pkgs.stdenv.hostPlatform.system}.default
                  pkgs.nix
                ];
              }
              ''
                export HOME=$TMPDIR NIX_CONFIG="experimental-features = nix-command"
                mkdir -p nixpkgs module pristine
                cp -r ${nixpkgs}/lib ${nixpkgs}/.version nixpkgs/
                cp ${nix-generators}/lib.nix work-generators.nix
                lib=$PWD/nixpkgs/lib
                generators="import $PWD/work-generators.nix { lib = import $lib; }"
                samples=${./checks/import}

                for source in "$samples"/*; do
                  name=$(basename "$source")
                  case "$name" in
                    *.input) name="markdown.md" ;;
                  esac
                  cp "$source" "pristine/$name"
                  cp "$source" "module/$name"
                done
                original=$PWD/pristine
                {
                  printf '{generators}: {\n'
                  for source in "$samples"/*; do
                    name=$(basename "$source")
                    case "$name" in
                      *.input) name="markdown.md" ;;
                    esac
                    printf '  "%s" = builtins.readFile ./%s;\n' "$name" "$name"
                  done
                  printf '}\n'
                } > module/mod.nix

                strictix import --generators "($generators)" module | tee report
                if grep -q '^kept' report; then
                  echo "a sample was left as a file; every language has a generator"
                  exit 1
                fi

                echo "module holds $(ls module | wc -l) entries"
                test "$(ls module | wc -l)" -eq 1
                if [ -n "$(find module -name '*.data.nix' -print -quit)" ]; then
                  echo "a data sidecar was written beside the module"
                  exit 1
                fi

                check="
                  let
                    generators = $generators;
                    original = \"$original\";
                    module = import ./module/mod.nix { inherit generators; };
                    mismatched = builtins.filter (name: builtins.getAttr name module != builtins.readFile (original + \"/\" + name)) (builtins.attrNames module);
                  in
                  builtins.length mismatched == 0
                "

                nix eval --store dummy:// --impure --json --expr "$check" | tee nix-out
                iets eval --no-cache --impure --json -E "$check" | tee iets-out
                grep -qx true nix-out
                grep -qx true iets-out

                mkdir -p letpristine letmodule
                cp "$samples/lisp.lisp" letpristine/probe.lisp
                cp "$samples/lisp.lisp" letmodule/probe.lisp
                {
                  printf '{\n  lib,\n  ...\n}: let\n  helper = 1;\nin {\n'
                  printf '  a = builtins.readFile ./probe.lisp;\n'
                  printf '  b = builtins.readFile ./probe.lisp;\n'
                  printf '  inherit helper;\n}\n'
                } > letmodule/mod.nix

                strictix import --generators "($generators)" letmodule
                strictix check letmodule

                letcheck="
                  let
                    generators = $generators;
                    lib = import $lib;
                    module = import ./letmodule/mod.nix { inherit generators lib; };
                    original = builtins.readFile ./letpristine/probe.lisp;
                  in
                  module.a == original && module.b == original
                "

                nix eval --store dummy:// --impure --json --expr "$letcheck" | tee let-nix-out
                grep -qx true let-nix-out

                mkdir -p splitpristine splitmodule
                cp "$samples/lua.lua" splitpristine/shared.lua
                cp "$samples/lua.lua" splitpristine/ok.lua
                cp "$samples/lua.lua" splitmodule/shared.lua
                cp "$samples/lua.lua" splitmodule/ok.lua
                printf '{ ... }: {\n  shared = builtins.readFile ./shared.lua;\n  ok = builtins.readFile ./ok.lua;\n}\n' > splitmodule/a.nix
                printf 'pkgs: {\n  shared = builtins.readFile ./shared.lua;\n}\n' > splitmodule/b.nix
                printf '{ ... }: {\n  ok = builtins.readFile ./ok.lua;\n}\n' > splitmodule/c.nix

                strictix import --split --generators "($generators)" splitmodule | tee split-report
                test -f splitmodule/shared.lua
                test ! -f splitmodule/shared.lua.nix
                test ! -f splitmodule/ok.lua
                test -f splitmodule/ok.lua.nix

                splitcheck="
                  let
                    generators = $generators;
                    a = import ./splitmodule/a.nix { inherit generators; };
                    b = import ./splitmodule/b.nix {};
                    c = import ./splitmodule/c.nix { inherit generators; };
                    origShared = builtins.readFile ./splitpristine/shared.lua;
                    origOk = builtins.readFile ./splitpristine/ok.lua;
                  in
                  a.shared == origShared && b.shared == origShared && a.ok == origOk && c.ok == origOk
                "

                nix eval --store dummy:// --impure --json --expr "$splitcheck" | tee split-nix-out
                grep -qx true split-nix-out
                touch $out
              '';
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          self-contained =
            let
              closure = pkgs.closureInfo { rootPaths = [ pkg ]; };
            in
            pkgs.runCommand "strictix-self-contained"
              {
                nativeBuildInputs = [ pkgs.binutils ];
              }
              ''
                bin=${pkg}/bin/strictix
                test -x "$bin"
                if readelf -lW "$bin" | awk '$1 == "INTERP" { found = 1 } END { exit !found }'; then
                  echo "strictix has a PT_INTERP dynamic loader segment"
                  exit 1
                fi
                ${pkgs.lib.optionalString pkgs.stdenv.hostPlatform.isx86_64 ''
                  if ! readelf -dW "$bin" | awk '$2 == "(RELR)" { found = 1 } END { exit !found }'; then
                    echo "strictix does not pack its relative relocations"
                    exit 1
                  fi
                ''}
                needed=$(readelf -dW "$bin" 2>/dev/null | awk '$2 == "(NEEDED)" { print }')
                if [ -n "$needed" ]; then
                  echo "strictix has dynamic NEEDED libraries:"
                  echo "$needed"
                  exit 1
                fi
                count=$(wc -l < ${closure}/store-paths)
                if [ "$count" -ne 1 ]; then
                  echo "expected runtime closure of 1 store path, got $count:"
                  cat ${closure}/store-paths
                  exit 1
                fi
                if ! grep -Fxq "${pkg}" ${closure}/store-paths; then
                  echo "unexpected store path in runtime closure:"
                  cat ${closure}/store-paths
                  exit 1
                fi
                test -n "$(env -i "$bin" list)"
                touch "$out"
              '';
        }
      );
    };
}
