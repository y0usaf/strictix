{
  lib,
  root,
  ...
}:
{
  partitions.dev.module.perSystem =
    psArgs@{ pkgs, ... }:
    {
      checks.dogfood =
        pkgs.runCommand "dogfood" { nativeBuildInputs = [ psArgs.config.packages.default ]; }
          ''
            cd ${
              lib.fileset.toSource {
                inherit root;
                fileset = lib.fileset.fileFilter (file: file.hasExt "nix") root;
              }
            }
            strictix check --ignore /crates/cli/tests/data
            touch $out
          '';
    };
}
