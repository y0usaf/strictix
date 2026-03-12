{
  nixConfig = {
    abort-on-warn = true;
    allow-import-from-derivation = false;
  };

  inputs = {
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };

    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";

    systems = {
      url = "github:nix-systems/default";
      flake = false;
    };
  };
  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } (
      { lib, ... }:
      {
        _module.args.root = ./.;
        systems = import inputs.systems;

        imports = [
          inputs.flake-parts.flakeModules.partitions
          ./nix/parts/docs.nix
          ./nix/parts/cachix.nix
          ./nix/parts/dev-shell.nix
          ./nix/parts/dogfood.nix
          ./nix/parts/files.nix
          ./nix/parts/fmt.nix
          ./nix/parts/git-hooks.nix
          ./nix/parts/git-ignore.nix
          ./nix/parts/license.nix
          ./nix/parts/overlay.nix
          ./nix/parts/rust.nix
          ./nix/parts/strictix.nix
          ./integrations/vim/flake-part.nix
        ];

        partitionedAttrs = lib.genAttrs [
          "checks"
          "apps"
        ] (_: "dev");

        partitions.dev.extraInputsFlake = ./nix/dev;
      }
    );
}
