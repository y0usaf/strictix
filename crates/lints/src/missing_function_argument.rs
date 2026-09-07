//! Flags calls to known formal-set functions whose literal argument omits a
//! required attribute.
//!
//! This is intentionally a small, syntax-directed check.  Dynamic attribute
//! names, inherits, and unknown callees are left alone because they may supply
//! an attribute that is not visible statically.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{ApplyExpr, AstNode, AttrItem, AttrName, AttrsetExpr, Expr, LambdaParam};

pub struct MissingFunctionArgument;

impl Rule for MissingFunctionArgument {
    fn code(&self) -> &'static str {
        "missing-function-argument"
    }
    fn name(&self) -> &'static str {
        "Missing function argument"
    }
    fn description(&self) -> &'static str {
        "Flags calls to a known formal-set function when a required attribute is missing from a literal argument."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let (Some(func), Some(arg)) = (apply.func(), apply.arg()) else {
                continue;
            };
            let Some(formals) = resolve_formals(func, model, 0) else {
                continue;
            };
            let Some((keys, complete)) = literal_keys(arg, model.source()) else {
                continue;
            };
            if !complete {
                continue;
            }
            for formal in formals {
                if formal.default.is_some()
                    || keys
                        .iter()
                        .any(|key| key == formal.name.text(model.source()))
                {
                    continue;
                }
                let name = formal.name.text(model.source());
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("function argument is missing required attribute '{name}'"),
                    arg.range(),
                ));
            }
        }
    }
}

/// Resolve only lambdas and lexical aliases to lambdas.  The binding check is
/// what prevents an unrelated `with` or a shadowed name from being treated as
/// the alias we found syntactically.
fn resolve_formals<'a>(
    expr: Expr<'a>,
    model: &SemanticModel<'a>,
    depth: usize,
) -> Option<Vec<strictix_syntax::FormalParam<'a>>> {
    if depth > 16 {
        return None;
    }
    let expr = match expr {
        Expr::Paren(p) => p.expr()?,
        other => other,
    };
    match expr {
        Expr::Lambda(lambda) => match lambda.param() {
            LambdaParam::Formals(formals, _) => Some(formals.params().collect()),
            LambdaParam::Ident(_) => None,
        },
        Expr::Ident(token) => resolve_formals(
            crate::static_binding::value(token, model)?,
            model,
            depth + 1,
        ),
        _ => None,
    }
}

/// Return known top-level keys and whether the set is closed.  Any inherit or
/// non-static attribute path makes the result uncertain, so callers skip it.
fn literal_keys<'a>(expr: Expr<'a>, source: &str) -> Option<(Vec<String>, bool)> {
    let expr = match expr {
        Expr::Paren(p) => p.expr()?,
        other => other,
    };
    let attrset = match expr {
        Expr::Attrset(a) => a,
        Expr::RecAttrset(r) => r.attrset()?,
        _ => return None,
    };
    Some(attrset_keys(attrset, source))
}

fn attrset_keys(attrset: AttrsetExpr<'_>, source: &str) -> (Vec<String>, bool) {
    let mut keys = Vec::new();
    for item in attrset.items() {
        match item {
            AttrItem::Inherit(_) => return (keys, false),
            AttrItem::Binding(binding) => {
                let Some(path) = binding.attrpath() else {
                    return (keys, false);
                };
                let mut elements = path.elements();
                let Some(AttrName::Ident(name)) = elements.next() else {
                    return (keys, false);
                };
                if elements.next().is_some() {
                    return (keys, false);
                }
                keys.push(name.text(source).to_owned());
            }
        }
    }
    (keys, true)
}
