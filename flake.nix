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
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "strictix";
          version = "0.1.0";
          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes."tree-sitter-vbscript-0.0.1" = "sha256-P1kpdbUropKYs9Bi1FPt+SQTetzzR5HvskmBkRVqMg8=";
          };
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

      checks = forAll (pkgs: {
        build = self.packages.${pkgs.system}.default;

        import-inline =
          pkgs.runCommand "strictix-import-inline"
            {
              nativeBuildInputs = [
                self.packages.${pkgs.system}.default
                iets.packages.${pkgs.system}.default
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
            '';      });
    };
}
