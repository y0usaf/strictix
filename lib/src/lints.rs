use crate::lints;

lints! {
    bool_comparison,
    empty_let_in,
    manual_inherit,
    manual_inherit_from,
    legacy_let_syntax,
    collapsible_let_in,
    eta_reduction,
    useless_parens,
    // unquoted_splice,
    empty_pattern,
    redundant_pattern_bind,
    unquoted_uri,
    empty_inherit,
    deprecated_to_path,
    bool_simplification,
    useless_has_attr,
    repeated_keys,
    empty_list_concat,
    with_expression,
    collapsible_inherit_from,
    empty_attrset_merge,
    redundant_if_bool,
    if_else_empty_attrset
}
