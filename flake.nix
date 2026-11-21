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
      ...
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
            if pkgs.stdenv.hostPlatform.isLinux then
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
              doCheck = false;
              cargoLock = {
                lockFile = ./Cargo.lock;
                outputHashes."tree-sitter-vbscript-0.0.1" = "sha256-P1kpdbUropKYs9Bi1FPt+SQTetzzR5HvskmBkRVqMg8=";
              };
            }
            // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
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
        }
        // pkgs.lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux {
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
