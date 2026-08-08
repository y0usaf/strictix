final: prev:

{
  example = prev.callPackage ./pkgs/example {
    inherit (prev) lib stdenv;
  };

  # keep this comment
  patched = prev.hello.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ [ ../patches/hello.patch ];
  });

  uri = "https://example.com/${final.example.version}/release";
  floats = [ 1.5 2.0e3 ];
}
