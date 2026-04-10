mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: unsorted_pattern_params,
    expressions: [
        // match: pkgs before lib
        "({ config, pkgs, lib, ... }: config)",

        // match: lib before config
        "({ lib, config, ... }: config)",

        // match: reversed with extra args
        "({ pkgs, lib, config, flakeInputs, ... }: config)",

        // match: pkgs, config, lib (all priority wrong)
        "({ pkgs, config, lib, ... }: config)",

        // match: extra args not alphabetical
        "({ config, lib, pkgs, zebra, alpha, ... }: config)",

        // don't match: already canonical
        "({ config, lib, pkgs, ... }: config)",

        // don't match: canonical with extra args alphabetical
        "({ config, lib, pkgs, alpha, beta, ... }: config)",

        // don't match: single param
        "({ config, ... }: config)",

        // don't match: empty pattern
        "({ ... }: 42)",

        // match: with @ bind prefix (args used)
        "(args @ { pkgs, config, lib, ... }: args.foo)",

        // match: with @ bind suffix (args used)
        "({ pkgs, config, lib, ... } @ args: args.foo)",

        // match: with defaults
        "({ config, pkgs ? null, lib, ... }: config)",
    ],
}
