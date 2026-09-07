//! Conservative access to the syntax value of a resolved local binding.
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Binding, Expr, LambdaExpr, LambdaParam, LetExpr, SyntaxToken,
};

pub(crate) fn value<'a>(token: &SyntaxToken, model: &SemanticModel<'a>) -> Option<Expr<'a>> {
    let resolved = model.resolve(token)?;
    if !matches!(
        resolved.kind,
        BindingKind::LetBinding | BindingKind::RecAttr
    ) {
        return None;
    }
    // The semantic model currently has sequential visibility for let bindings
    // and formal defaults. Do not follow an outer binding through a forward
    // declaration that actually shadows it under Nix's recursive semantics.
    for node in model
        .root()
        .descendants()
        .filter(|node| node.range().contains(token.range().start()))
    {
        if node.range().contains(resolved.name.range().start()) {
            continue;
        }
        if let Some(expr) = LetExpr::cast(node) {
            for item in expr.bindings()?.items() {
                let names: Vec<_> = match item {
                    AttrItem::Binding(b) => b
                        .attrpath()?
                        .elements()
                        .take(1)
                        .filter_map(|n| match n {
                            AttrName::Ident(t) => Some(t),
                            _ => None,
                        })
                        .collect(),
                    AttrItem::Inherit(i) => i.names().collect(),
                };
                if names.iter().any(|name| {
                    name.text(model.source()) == token.text(model.source())
                        && name.range() != resolved.name.range()
                }) {
                    return None;
                }
            }
        }
        if let Some(lambda) = LambdaExpr::cast(node) {
            if let LambdaParam::Formals(formals, _) = lambda.param() {
                if formals
                    .params()
                    .any(|param| param.name.text(model.source()) == token.text(model.source()))
                {
                    return None;
                }
            }
        }
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
    if model.resolve(token).is_some()
        || model.references().iter().any(|reference| {
            reference.name.range() == token.range() && reference.via_with.is_some()
        })
    {
        return false;
    }
    let name = token.text(model.source());
    for node in model
        .root()
        .descendants()
        .filter(|node| node.range().contains(token.range().start()))
    {
        if let Some(expr) = LetExpr::cast(node) {
            let Some(bindings) = expr.bindings() else {
                return false;
            };
            for item in bindings.items() {
                match item {
                    AttrItem::Binding(binding) => {
                        if binding
                            .attrpath()
                            .and_then(|path| path.elements().next())
                            .is_some_and(|attr| match attr {
                                AttrName::Ident(t) => t.text(model.source()) == name,
                                _ => true,
                            })
                        {
                            return false;
                        }
                    }
                    AttrItem::Inherit(inherit) => {
                        if inherit.names().any(|t| t.text(model.source()) == name) {
                            return false;
                        }
                    }
                }
            }
        }
        if let Some(lambda) = LambdaExpr::cast(node) {
            if let LambdaParam::Formals(formals, _) = lambda.param() {
                if formals
                    .params()
                    .any(|param| param.name.text(model.source()) == name)
                {
                    return false;
                }
            }
        }
    }
    true
}
