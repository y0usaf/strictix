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

        import-roundtrip =
          pkgs.runCommand "strictix-import-roundtrip"
            {
              nativeBuildInputs = [
                self.packages.${pkgs.system}.default
                iets.packages.${pkgs.system}.default
                pkgs.nix
                pkgs.diffutils
              ];
            }
            ''
              export HOME=$TMPDIR NIX_CONFIG="experimental-features = nix-command"
              mkdir -p nixpkgs work
              cp -r ${nixpkgs}/lib ${nixpkgs}/.version nixpkgs/
              cp ${nix-generators}/lib.nix work/generators.nix
              lib=$PWD/nixpkgs/lib
              generators=$PWD/work/generators.nix
              for source in ${./checks/import}/*; do
                file=$(basename "$source")
                language=''${file%%.*}
                strictix import "$language" "$source" > "work/$file.nix"
                expression="let generators = import $generators { lib = import $lib; }; in generators.$language.render (import $PWD/work/$file.nix generators.$language)"
                nix eval --store dummy:// --impure --raw --expr "$expression" > "work/$file.nix-out"
                iets eval --no-cache --impure --raw -E "$expression" > "work/$file.iets-out"
                diff -u "$source" "work/$file.nix-out"
                diff -u "$source" "work/$file.iets-out"
                echo "$file: nix and iets render the import byte for byte"
              done
              touch $out
            '';
      });
    };
}
