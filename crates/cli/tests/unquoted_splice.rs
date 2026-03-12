mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: unquoted_splice,
    expressions: [
        "nixpkgs.legacyPackages.${system}",
    ],
}
