//! Conservative access to the syntax value of a resolved local binding.
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Binding, Expr, LambdaExpr, LambdaParam, LetExpr, RecAttrsetExpr,
    SyntaxToken,
};

fn may_bind(item: AttrItem<'_>, name: &str, source: &str) -> bool {
    match item {
        AttrItem::Binding(binding) => binding
            .attrpath()
            .and_then(|path| path.elements().next())
            .is_none_or(|attr| match attr {
                AttrName::Ident(token) => token.text(source) == name,
                // The semantic model does not resolve quoted/dynamic names.
                _ => true,
            }),
        AttrItem::Inherit(inherit) => inherit.names().any(|token| token.text(source) == name),
    }
}

/// Whether an enclosing scope can shadow the named binding. A declaration's
/// own scope is excluded when checking references to a known local binding.
fn shadowed(
    token: &SyntaxToken,
    expected: Option<&SyntaxToken>,
    model: &SemanticModel<'_>,
) -> bool {
    let name = token.text(model.source());
    for node in model
        .root()
        .descendants()
        .filter(|node| node.range().contains(token.range().start()))
    {
        if expected.is_some_and(|binding| node.range().contains(binding.range().start())) {
            continue;
        }
        if let Some(expr) = LetExpr::cast(node) {
            if expr.bindings().is_none_or(|bindings| {
                bindings
                    .items()
                    .any(|item| may_bind(item, name, model.source()))
            }) {
                return true;
            }
        }
        if let Some(expr) = RecAttrsetExpr::cast(node) {
            if expr.attrset().is_none_or(|bindings| {
                bindings
                    .items()
                    .any(|item| may_bind(item, name, model.source()))
            }) {
                return true;
            }
        }
        if let Some(lambda) = LambdaExpr::cast(node) {
            let binds = match lambda.param() {
                LambdaParam::Ident(token) => token.text(model.source()) == name,
                LambdaParam::Formals(formals, at_name) => {
                    at_name.is_some_and(|token| token.text(model.source()) == name)
                        || formals
                            .params()
                            .any(|param| param.name.text(model.source()) == name)
                }
            };
            if binds {
                return true;
            }
        }
    }
    false
}

/// Verify resolution without overlooking forward or quoted local declarations.
pub(crate) fn resolves_to(
    token: &SyntaxToken,
    expected: &SyntaxToken,
    model: &SemanticModel<'_>,
) -> bool {
    model
        .resolve(token)
        .is_some_and(|binding| binding.name.range() == expected.range())
        && !shadowed(token, Some(expected), model)
}

pub(crate) fn value<'a>(token: &SyntaxToken, model: &SemanticModel<'a>) -> Option<Expr<'a>> {
    let resolved = model.resolve(token)?;
    if !matches!(
        resolved.kind,
        BindingKind::LetBinding | BindingKind::RecAttr
    ) || !resolves_to(token, resolved.name, model)
    {
        return None;
    }
    model.root().descendants().find_map(|node| {
        let binding = Binding::cast(node)?;
        let mut path = binding.attrpath()?.elements();
        match path.next()? {
            AttrName::Ident(name)
                if name.range() == resolved.name.range() && path.next().is_none() =>
            {
                binding.value()
            }
            _ => None,
        }
    })
}

/// Check a global name without overlooking a forward local declaration.
pub(crate) fn is_unshadowed(token: &SyntaxToken, model: &SemanticModel<'_>) -> bool {
    model.resolve(token).is_none()
        && !model.references().iter().any(|reference| {
            reference.name.range() == token.range() && reference.via_with.is_some()
        })
        && !shadowed(token, None, model)
}
