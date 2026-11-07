//! Where a file's data is machine-shaped rather than written by hand.
//!
//! finix's generators take a syntax tree: the argument of a
//! `generators.<language>.render` call, or the value a `generators.<name>`
//! printer takes. That tree is spelled from constructors, so the names,
//! words and one-item lists it repeats are the generated text's shape and
//! not a reader's code. No rule reports inside one.

use strictix_syntax::{AttrName, Expr, TextRange};

use crate::semantic::SemanticModel;

/// The ranges of a file's generator trees.
#[must_use]
pub fn tree_ranges(model: &SemanticModel<'_>) -> Vec<TextRange> {
    let source = model.source();
    let mut ranges = Vec::new();
    for node in model.root().descendants() {
        let Some(Expr::Apply(apply)) = Expr::cast(node) else {
            continue;
        };
        let Some(callee) = apply.func().and_then(|func| dotted(func, source)) else {
            continue;
        };
        if callee
            .strip_prefix("generators.")
            .is_some_and(|rest| rest.contains('.') || rest.ends_with("Source"))
        {
            if let Some(arg) = apply.arg() {
                ranges.push(arg.content_range());
            }
        }
    }
    ranges
}

/// Whether `range` sits inside one of `ranges`.
#[must_use]
pub fn inside(ranges: &[TextRange], range: TextRange) -> bool {
    ranges
        .iter()
        .any(|outer| outer.start() <= range.start() && range.end() <= outer.end())
}

fn dotted(expr: Expr<'_>, source: &str) -> Option<String> {
    match expr {
        Expr::Ident(token) => Some(token.text(source).to_owned()),
        Expr::Select(select) => {
            let mut name = dotted(select.base()?, source)?;
            for element in select.attrpath()?.elements() {
                let AttrName::Ident(token) = element else {
                    return None;
                };
                name.push('.');
                name.push_str(token.text(source));
            }
            Some(name)
        }
        Expr::Paren(paren) => paren.expr().and_then(|inner| dotted(inner, source)),
        _ => None,
    }
}
