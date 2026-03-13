# The `default.nix` in flake-compat reads `flake.nix` and `flake.lock` from `src` and
# returns an attribute set of the shape `{ defaultNix, shellNix }`
(import
  (fetchTarball {
    url = "https://github.com/edolstra/flake-compat/archive/35bb57c0c8d8b62bbfd284272c928ceb64ddbde9.tar.gz";
    sha256 = "1prd9b1xx8c65rpg3an9f63jvwnb8i49fg7d18czlhxnf4yydsr0";
  })
  {
    src = ./.;
  }
).defaultNix
