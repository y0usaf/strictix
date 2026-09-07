//! Finds statically known attributes passed to a closed pattern function.

use std::collections::HashSet;

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, AttrName, Expr, FormalParam, LambdaExpr, LambdaParam, StringPart,
    SyntaxKind,
};

pub struct UnexpectedFunctionArgument;

impl Rule for UnexpectedFunctionArgument {
    fn code(&self) -> &'static str {
        "unexpected-function-argument"
    }
    fn name(&self) -> &'static str {
        "Unexpected function argument"
    }
    fn description(&self) -> &'static str {
        "Flags statically known attributes passed to a closed pattern function when the pattern does not declare them."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let root = model.root();
        for node in root
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::ApplyExpr)
        {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let Some(mut arg) = apply.arg() else { continue };
            while let Expr::Paren(p) = arg {
                let Some(inner) = p.expr() else { break };
                arg = inner;
            }
            let Some(attrset) = (match arg {
                Expr::Attrset(a) => Some(a),
                Expr::RecAttrset(a) => a.attrset(),
                _ => None,
            }) else {
                continue;
            };
            let Some(lambda) = resolve_lambda(apply.func(), model, &mut HashSet::new()) else {
                continue;
            };
            let LambdaParam::Formals(formals, _) = lambda.param() else {
                continue;
            };
            if formals.has_ellipsis() {
                continue;
            }
            let declared: HashSet<String> = formals
                .params()
                .map(|p: FormalParam<'_>| p.name.text(model.source()).to_owned())
                .collect();
            for item in attrset.items() {
                let AttrItem::Binding(binding) = item else {
                    continue;
                };
                let Some(path) = binding.attrpath() else {
                    continue;
                };
                let Some(first) = path.elements().next() else {
                    continue;
                };
                let (name, range) = match first {
                    AttrName::Ident(t) => (t.text(model.source()).to_owned(), t.range()),
                    AttrName::Str(s) => {
                        if s.parts().any(|p| matches!(p, StringPart::Interp(_))) {
                            continue;
                        }
                        (decode_string(s, model.source()), s.syntax().content_range())
                    }
                    AttrName::Interp(_) => continue,
                };
                if !declared.contains(&name) {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("function does not accept argument '{name}'"),
                        range,
                    ));
                }
            }
        }
    }
}

fn resolve_lambda<'a>(
    expr: Option<Expr<'a>>,
    model: &SemanticModel<'a>,
    seen: &mut HashSet<u32>,
) -> Option<LambdaExpr<'a>> {
    match expr? {
        Expr::Lambda(l) => Some(l),
        Expr::Paren(p) => resolve_lambda(p.expr(), model, seen),
        Expr::Ident(token) => {
            if !seen.insert(token.range().start()) {
                return None;
            }
            resolve_lambda(
                Some(crate::static_binding::value(token, model)?),
                model,
                seen,
            )
        }
        _ => None,
    }
}

fn decode_string(string: strictix_syntax::StringExpr<'_>, source: &str) -> String {
    let mut out = String::new();
    for part in string.parts() {
        let StringPart::Content(token) = part else {
            continue;
        };
        let mut chars = token.text(source).chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(c) => out.push(c),
                None => {}
            }
        }
    }
    out
}
