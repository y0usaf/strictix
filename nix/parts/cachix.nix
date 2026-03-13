{ lib, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      apps.cachix-push = {
        type = "app";
        program = lib.getExe (
          pkgs.writeShellApplication {
            name = "cachix-push";
            runtimeInputs = [
              pkgs.nix
              pkgs.jq
              pkgs.cachix
            ];
            text = ''
              nix build --json \
              | jq -r '.[].outputs | to_entries[].value' \
              | cachix push strictix
            '';
          }
        );
      };
    };
}
