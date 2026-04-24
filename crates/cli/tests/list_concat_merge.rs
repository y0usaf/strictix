mod _utils;

use macros::generate_tests;

generate_tests! {
    rule: list_concat_merge,
    expressions: [
        // match - list ++ lib.optionals ++ list (should merge unconditional lists)
        "[ a b ] ++ lib.optionals cfg.enable [ x ] ++ [ d e ]",
        
        // match - list ++ lib.optional ++ list  
        "[ x y ] ++ lib.optional cond [ z ] ++ [ n m ]",
        
        // match - multiple items in each list
        "[ piAgentsPkg piCodexFastPkg ] ++ lib.optionals cfg.rtk.enable [ piRtkPkg ] ++ [ piCompactToolsPkg piToolManagementPkg ]",
        
        // match - single items
        "[ a ] ++ lib.optionals cfg.enable [ b ] ++ [ c ]",
        
        // match - multiline format preserved in single-line form
        "[ a b c ] ++ lib.optionals cfg.someCond [ d ] ++ [ e f g ]",
        
        // match - lib.optionals with complex condition
        "[ base ] ++ lib.optionals (config.features.enable && config.features.advanced) [ optional1 ] ++ [ extra1 extra2 ]",
        
        // match - with long package names
        "[ veryLongPackageName1 veryLongPackageName2 ] ++ lib.optionals cfg.enable [ pkg ] ++ [ veryLongPackageName3 veryLongPackageName4 ]",
    ],
}