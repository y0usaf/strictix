#![allow(clippy::needless_raw_string_hashes)]
mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: deprecated,
    expressions: [
        "builtins.toPath x",
        "toPath x",
        r#"toPath "/abc/def""#,
        r#"builtins.toPath "/some/path""#,
        r#"toPath __strictix_to_path_arg"#,
        r#"toPath (if cond then "/abs" else "rel")"#,
        "lib.nixpkgsVersion",
        "lib.isInOldestRelease 2411",
        "lib.evalOptionValue",
        "lib.lists.fold f z xs",
        "lib.cli.toGNUCommandLine opts attrs",
        "lib.cli.toGNUCommandLineShell opts attrs",
        "pkgs.buildPlatform",
        "pkgs.hostPlatform",
        "pkgs.system",
        "pkgs.targetPlatform",
        "pkgs.dontRecurseIntoAttrs",
        "pkgs.stringsWithDeps",
        "pkgs.forceSystem",
        "mkAliasOptionModuleMD old new",
        "mkAliasIfDef option",
        "lib.mkFixStrictness x",
        "lib.mkFixStrictness (if cond then a else b)",
    ],
}

#[test]
fn deprecated_negative_lookalikes_are_clean() {
    for expression in [
        "toPathX x",
        "foo.toPath x",
        "lib.cli.toGNUCommandLineShellExtra opts attrs",
        "pkgs.systems",
        "lib.version",
        "pkgs.stdenv.hostPlatform.system",
        "mkIf option.isDefined",
    ] {
        _utils::assert_check_clean(expression).expect(expression);
    }
}
