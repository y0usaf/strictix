use strictix_syntax::{AttrName, Expr, TextRange};

use crate::semantic::SemanticModel;

#[must_use]
pub fn tree_ranges(model: &SemanticModel<'_>) -> Vec<TextRange> {
    let source = model.source();
    if !source.contains("generators") {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut parts = Vec::with_capacity(4);
    for node in model.root().descendants() {
        let Some(Expr::Apply(apply)) = Expr::cast(node) else {
            continue;
        };
        parts.clear();
        let Some(func) = apply.func() else {
            continue;
        };
        if !collect_dotted(func, source, &mut parts) {
            continue;
        }
        if parts.first() == Some(&"generators")
            && (parts.len() >= 3 || (parts.len() == 2 && parts[1].ends_with("Source")))
        {
            if let Some(arg) = apply.arg() {
                ranges.push(arg.content_range());
            }
        }
    }
    ranges
}

#[must_use]
pub fn inside(ranges: &[TextRange], range: TextRange) -> bool {
    ranges
        .iter()
        .any(|outer| outer.start() <= range.start() && range.end() <= outer.end())
}

fn collect_dotted<'a>(expr: Expr<'a>, source: &'a str, out: &mut Vec<&'a str>) -> bool {
    match expr {
        Expr::Ident(token) => {
            out.push(token.text(source));
            true
        }
        Expr::Select(select) => {
            let Some(base) = select.base() else {
                return false;
            };
            if !collect_dotted(base, source, out) {
                return false;
            }
            let Some(ap) = select.attrpath() else {
                return false;
            };
            for element in ap.elements() {
                let AttrName::Ident(token) = element else {
                    return false;
                };
                out.push(token.text(source));
            }
            true
        }
        Expr::Paren(paren) => paren
            .expr()
            .is_some_and(|inner| collect_dotted(inner, source, out)),
        _ => false,
    }
}
