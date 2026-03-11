{
  lib,
  root,
  ...
}:
{
  perSystem =
    psArgs@{ pkgs, ... }:
    {
      packages = {
        strictix = pkgs.rustPlatform.buildRustPackage {
          pname = "strictix";
          inherit ((lib.importTOML (root + "/bin/Cargo.toml")).package) version;

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
          nativeCheckInputs = [ pkgs.clippy ];

          postCheck = ''
            echo "Starting postCheck"
            cargo clippy
            echo "Finished postCheck"
          '';

          meta = {
            mainProgram = "strictix";
            description = "Strict lints and suggestions for the Nix programming language";
            homepage = "https://github.com/y0usaf/strictix";
            license = lib.licenses.mit;
          };
        };

        default = psArgs.config.packages.strictix;
      };
    };

  partitions.dev.module.perSystem = psArgs: {
    treefmt.settings.global.excludes = [ "bin/tests/data/*.nix" ];
    checks."packages/strictix" = psArgs.config.packages.strictix;
  };
}
