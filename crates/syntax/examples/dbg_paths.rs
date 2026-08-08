fn main() {
    for src in [
        "./foo/bar.nix",
        "../up/dir",
        "/abs/path",
        "~/home/dir",
        "foo/bar",
        "<nixpkgs>",
        "<nixpkgs/nixos>",
    ] {
        let tokens: Vec<_> = strictix_syntax::lex(src);
        let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
        println!("{src:22} -> {kinds:?}");
    }
}
