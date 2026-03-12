{
  lib,
  root,
  ...
}:
{
  partitions.dev.module.perSystem =
    psArgs@{ pkgs, ... }:
    let
      src = lib.fileset.toSource {
        inherit root;
        fileset = lib.fileset.fileFilter (file: file.hasExt "nix") root;
      };
    in
    {
      checks.dogfood =
        pkgs.runCommand "dogfood" { nativeBuildInputs = [ psArgs.config.packages.default ]; }
          ''
            cd ${src}
            strictix check --ignore /crates/cli/tests/data
            touch $out
          '';
    };
}
