{ lib, ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages.strictix-vim = pkgs.vimUtils.buildVimPlugin {
        pname = "strictix-vim";
        version = "0.1.0";
        src = lib.fileset.toSource {
          root = ./.;
          fileset = lib.fileset.union ./plugin/strictix.vim ./ftplugin/nix.vim;
        };
      };
    };

  partitions.dev.module.perSystem = psArgs: {
    treefmt.settings.global.excludes = [ "*.vim" ];
    checks."packages/strictix-vim" = psArgs.config.packages.strictix-vim;
  };
}
