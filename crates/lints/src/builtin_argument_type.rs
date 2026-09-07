//! Checks literal arguments passed to builtins with simple, fixed contracts.
//!
//! This is deliberately syntax directed: names and compound expressions are
//! unknown and are left alone.  In particular, this rule does not evaluate
//! Nix expressions.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{ApplyExpr, AstNode, AttrItem, AttrName, Expr};

pub struct BuiltinArgumentType;

impl Rule for BuiltinArgumentType {
    fn code(&self) -> &'static str {
        "builtin-argument-type"
    }
    fn name(&self) -> &'static str {
        "Builtin argument type"
    }
    fn description(&self) -> &'static str {
        "Flags literal arguments whose type cannot satisfy a builtin's argument contract."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let mut callee = apply.func();
            let mut index = 0;
            while let Some(Expr::Apply(inner)) = callee {
                index += 1;
                callee = inner.func();
            }
            let Some((name, _)) = builtin_name(callee, source, model) else {
                continue;
            };
            let Some(expected) = contracts(name).get(index).copied() else {
                continue;
            };
            let Some(arg) = apply.arg() else { continue };
            if definitely_wrong(arg, expected, source) {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("builtin '{name}' expects a {} argument", expected.label()),
                    arg.range(),
                ));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Ty {
    Attrset,
    List,
    Int,
    String,
    Function,
}
impl Ty {
    fn label(self) -> &'static str {
        match self {
            Ty::Attrset => "attribute set",
            Ty::List => "list",
            Ty::Int => "integer",
            Ty::String => "string",
            Ty::Function => "function",
        }
    }
}

// The order is Nix's argument order.  Only contracts for which a literal
// mismatch is useful are listed; polymorphic arguments remain unconstrained.
fn contracts(name: &str) -> &'static [Ty] {
    use Ty::*;
    match name {
        "attrNames" | "attrValues" => &[Attrset],
        "functionArgs" => &[Function],
        "listToAttrs" => &[List],
        "map" | "filter" | "all" | "any" => &[Function, List],
        "removeAttrs" => &[Attrset, List],
        "intersectAttrs" => &[Attrset, Attrset],
        "getAttr" | "hasAttr" => &[String, Attrset],
        "elemAt" => &[List, Int],
        "genList" => &[Function, Int],
        "concatLists" | "head" | "tail" => &[List],
        "concatStringsSep" => &[String, List],
        "replaceStrings" => &[List, List, String],
        "substring" => &[Int, Int, String],
        "stringLength" | "fromJSON" | "fromTOML" | "toPath" => &[String],
        "match" | "split" => &[String, String],
        "sort" => &[Function, List],
        _ => &[],
    }
}

fn builtin_name<'a>(
    expr: Option<Expr<'a>>,
    source: &'a str,
    model: &SemanticModel<'a>,
) -> Option<(&'a str, usize)> {
    match expr? {
        Expr::Select(select) => {
            let Some(Expr::Ident(base)) = select.base() else {
                return None;
            };
            if base.text(source) != "builtins"
                || model.resolve(base).is_some()
                || model
                    .references()
                    .iter()
                    .any(|r| r.name.range() == base.range() && r.via_with.is_some())
            {
                return None;
            }
            let path = select.attrpath()?;
            let mut elements = path.elements();
            let AttrName::Ident(name) = elements.next()? else {
                return None;
            };
            if elements.next().is_some() {
                return None;
            }
            Some((name.text(source), 0))
        }
        Expr::Ident(token) => {
            // Bare builtin functions are globals, but a lexical binding or a
            // `with` reference wins over that global.
            if !crate::static_binding::is_unshadowed(token, model)
                || model
                    .references()
                    .iter()
                    .any(|r| r.name.range() == token.range() && r.via_with.is_some())
            {
                return None;
            }
            if !matches!(token.text(source), "map" | "removeAttrs" | "fromTOML") {
                return None;
            }
            Some((token.text(source), 0))
        }
        _ => None,
    }
}

fn definitely_wrong(expr: Expr<'_>, expected: Ty, source: &str) -> bool {
    let expr = match expr {
        Expr::Paren(p) => p.expr().unwrap_or(expr),
        _ => expr,
    };
    match expected {
        Ty::List => matches!(
            expr,
            Expr::Int(_)
                | Expr::Float(_)
                | Expr::String(_)
                | Expr::IndString(_)
                | Expr::Attrset(_)
                | Expr::RecAttrset(_)
        ),
        Ty::Attrset => matches!(
            expr,
            Expr::Int(_) | Expr::Float(_) | Expr::String(_) | Expr::IndString(_) | Expr::List(_)
        ),
        Ty::Int => matches!(
            expr,
            Expr::Float(_) | Expr::String(_) | Expr::IndString(_) | Expr::List(_)
        ),
        // Paths and attribute sets may be coerced to strings; leave them unknown.
        Ty::String => matches!(expr, Expr::Float(_) | Expr::Int(_) | Expr::List(_)),
        Ty::Function => definitely_non_function(expr, source),
    }
}

fn definitely_non_function(expr: Expr<'_>, source: &str) -> bool {
    match expr {
        Expr::Int(_) | Expr::Float(_) | Expr::String(_) | Expr::IndString(_) | Expr::List(_) => {
            true
        }
        Expr::Ident(_) => false,
        Expr::Attrset(a) => {
            let f = functor_state(a, source);
            !f.callable && !f.allows_unknown
        }
        Expr::RecAttrset(r) => r.attrset().is_some_and(|a| {
            let f = functor_state(a, source);
            !f.callable && !f.allows_unknown
        }),
        _ => false,
    }
}

struct FunctorState {
    callable: bool,
    allows_unknown: bool,
}
fn functor_state(a: strictix_syntax::AttrsetExpr<'_>, source: &str) -> FunctorState {
    let mut state = FunctorState {
        callable: false,
        allows_unknown: false,
    };
    for item in a.items() {
        match item {
            AttrItem::Inherit(_) => state.allows_unknown = true,
            AttrItem::Binding(b) => {
                let Some(path) = b.attrpath() else {
                    state.allows_unknown = true;
                    continue;
                };
                let mut it = path.elements();
                match it.next() {
                    Some(AttrName::Ident(t)) if t.text(source) == "__functor" => {
                        state.callable = true
                    }
                    Some(AttrName::Ident(_)) if it.next().is_some() => {}
                    Some(AttrName::Ident(_)) => {}
                    _ => state.allows_unknown = true,
                }
            }
        }
    }
    state
}
