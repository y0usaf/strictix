{
  lib,
  root,
  ...
}:
{
  perSystem =
    psArgs@{ pkgs, ... }:
    {
      packages = rec {
        strictix = pkgs.rustPlatform.buildRustPackage {
          pname = "strictix";
          inherit ((lib.importTOML (root + "/crates/cli/Cargo.toml")).package) version;

          src = lib.fileset.toSource {
            inherit root;
            fileset = lib.fileset.unions [
              (lib.fileset.fileFilter (
                file:
                lib.any lib.id [
                  (file.name == "Cargo.toml")
                  (file.hasExt "rs")
                  (file.hasExt "snap")
                ]
              ) root)
              (root + "/Cargo.lock")
            ];
          };
          cargoLock.lockFile = root + "/Cargo.lock";
          buildFeatures = [ "json" ];
          RUSTFLAGS = "--deny warnings";
          doCheck = false;

          meta = {
            mainProgram = "strictix";
            description = "Strict lints and suggestions for the Nix programming language";
            homepage = "https://github.com/y0usaf/strictix";
            license = lib.licenses.mit;
          };
        };

        strictix-checked = strictix.overrideAttrs (old: {
          doCheck = true;
          nativeCheckInputs = (old.nativeCheckInputs or [ ]) ++ [ pkgs.clippy ];
          postCheck = ''
            echo "Starting postCheck"
            cargo clippy
            echo "Finished postCheck"
          '';
        });

        default = psArgs.config.packages.strictix;
      };
    };

  partitions.dev.module.perSystem = psArgs: {
    treefmt.settings.global.excludes = [ "crates/cli/tests/data/*.nix" ];
    checks."packages/strictix" = psArgs.config.packages.strictix-checked;
  };
}
