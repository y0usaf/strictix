//! Conservative recognition of conventional Nix library helpers and aliases.
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{AstNode, AttrName, Expr, InheritStmt, WithExpr};

// Resolve only the conventional module `lib` and direct aliases. This is a
// maintainability heuristic, not evaluation of arbitrary imported libraries.
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
                if binding.kind == BindingKind::LambdaParam && name == "lib" {
                    return Some(LibraryValue::Library);
                }
                if let Some(value) = crate::static_binding::value(token, model) {
                    return library_value(model, value, fuel);
                }
                // Let inherits are registered as LetBinding; rec inherits use
                // RecAttr. Locate the defining syntax rather than relying on kind.
                let inherit = model
                    .root()
                    .descendants()
                    .filter_map(InheritStmt::cast)
                    .find(|inherit| inherit.names().any(|n| n.range() == binding.name.range()))?;
                return member(library_value(model, inherit.source()?, fuel)?, name);
            }
            let reference = model
                .references()
                .iter()
                .find(|reference| reference.name.range() == token.range())?;
            if let Some(site) = reference.via_with {
                let range = model.with_sites()[site].scope_range;
                let scope = model
                    .root()
                    .descendants()
                    .filter_map(WithExpr::cast)
                    .find(|expr| expr.range() == range)?
                    .scope()?;
                return member(library_value(model, scope, fuel)?, name);
            }
            (name == "lib" && crate::static_binding::is_unshadowed(token, model))
                .then_some(LibraryValue::Library)
        }
        _ => None,
    }
}
