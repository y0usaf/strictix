//! Conservative recognition of conventional Nix library helpers and aliases.
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{AstNode, AttrName, Expr, InheritStmt, WithExpr};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryValue {
    Library,
    Function(&'static str),
}

fn member(base: LibraryValue, name: &str) -> Option<LibraryValue> {
    if base != LibraryValue::Library {
        return None;
    }
    [
        "mkIf",
        "optional",
        "optionals",
        "optionalAttrs",
        "optionalString",
    ]
    .into_iter()
    .find(|candidate| *candidate == name)
    .map(LibraryValue::Function)
}

pub(crate) fn library_member(model: &SemanticModel<'_>, expr: Expr<'_>) -> Option<&'static str> {
    match library_value(model, expr, 32)? {
        LibraryValue::Function(name) => Some(name),
        LibraryValue::Library => None,
    }
}

pub(crate) fn unparen(mut expr: Expr<'_>) -> Option<Expr<'_>> {
    while let Expr::Paren(paren) = expr {
        expr = paren.expr()?;
    }
    Some(expr)
}

fn library_value(model: &SemanticModel<'_>, expr: Expr<'_>, fuel: usize) -> Option<LibraryValue> {
    let fuel = fuel.checked_sub(1)?;
    match unparen(expr)? {
        Expr::Select(select) if select.default().is_none() => {
            let base = library_value(model, select.base()?, fuel)?;
            let mut path = select.attrpath()?.elements();
            let AttrName::Ident(name) = path.next()? else {
                return None;
            };
            if path.next().is_some() {
                return None;
            }
            member(base, name.text(model.source()))
        }
        Expr::Ident(token) => {
            let name = token.text(model.source());
            if let Some(binding) = model.resolve(token) {
                if !crate::static_binding::resolves_to(token, binding.name, model) {
                    return None;
                }
                if binding.kind == BindingKind::LambdaParam && name == "lib" {
                    return Some(LibraryValue::Library);
                }
                if let Some(value) = crate::static_binding::value(token, model) {
                    return library_value(model, value, fuel);
                }
                let mut inherit = None;
                model
                    .root()
                    .walk_containing(binding.name.range().start(), |node| {
                        if let Some(stmt) =
                            InheritStmt::cast(node).filter(|i| i.names().any(|n| n.range() == binding.name.range()))
                        {
                            inherit = Some(stmt);
                            true
                        } else {
                            false
                        }
                    });
                let inherit = inherit?;
                return member(library_value(model, inherit.source()?, fuel)?, name);
            }
            let reference = model.reference_at(token.range())?;
            if let Some(site) = reference.via_with {
                let range = model.with_sites()[site].scope_range;
                let mut with_expr = None;
                model.root().walk_containing(range.start(), |node| {
                    if let Some(expr) = WithExpr::cast(node).filter(|expr| expr.range() == range) {
                        with_expr = Some(expr);
                        true
                    } else {
                        false
                    }
                });
                let scope = with_expr?.scope()?;
                return member(library_value(model, scope, fuel)?, name);
            }
            (name == "lib" && crate::static_binding::is_unshadowed(token, model))
                .then_some(LibraryValue::Library)
        }
        _ => None,
    }
}
