use std::collections::HashMap;

use lib::{LINTS, Lint};
use rnix::SyntaxKind;

pub fn lint_map_of(lints: &[&'static dyn Lint]) -> HashMap<SyntaxKind, Vec<&'static dyn Lint>> {
    let mut map = HashMap::new();
    for lint in lints {
        for &m in lint.match_kind() {
            map.entry(m)
                .and_modify(|v: &mut Vec<_>| v.push(*lint))
                .or_insert_with(|| vec![*lint]);
        }
    }
    map
}

pub fn lint_map() -> HashMap<SyntaxKind, Vec<&'static dyn Lint>> {
    lint_map_of(&LINTS)
}
