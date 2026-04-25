mod _utils;

use macros::generate_tests;

#[rustfmt::skip]
generate_tests! {
    rule: unused_inherit,
    expressions: [
        // match: remove unused names from a multi-attr inherit
        "let inherit (lib) mkIf mkOption types; in mkIf true {}",

        // match: remove a single unused inherit while keeping the let
        "let inherit (lib) mkIf; keep = 1; in lib.mkIf (keep == keep) {}",

        // match: remove unused bare inherited names
        "let inherit foo bar; in foo",

        // don't match: inherited name is used in the body
        "let inherit (lib) mkIf; in mkIf true {}",

        // don't match: bare inherit in the result attrset uses the local binding
        "let inherit (lib) mkIf; in { inherit mkIf; }",

        // don't match: attrset inherit exports an attribute, not a local import
        "{ inherit (lib) mkIf; }",
    ],
}
