{ config, lib, pkgs, ... }:

let
  inherit (lib) mkIf mkOption types;
  port = 8080;
in
{
  options.services.example = {
    enable = mkOption {
      type = types.bool;
      default = false;
      description = "Enable the example service.";
    };
  };

  config = mkIf config.services.example.enable {
    environment.etc."example/config.json".text = builtins.toJSON {
      inherit port;
      url = "http://localhost:${toString port}/api";
    };

    systemd.services.example = {
      script = ''
        #!${pkgs.runtimeShell}
        exec ${pkgs.example}/bin/example --port ${toString port}
      '';
    };
  };
}
