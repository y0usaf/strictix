//! Detect statically provable missing attributes on local literal attrsets.
//!
//! This deliberately only follows lexical bindings whose value is a literal
//! attrset.  Imported values, `with` scopes, inherited names, and dynamic
//! attrpaths are unknown and therefore do not produce findings.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{AstNode, AttrItem, AttrName, Attrpath, Expr, SelectExpr, StringPart};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Presence {
    Present,
    Missing,
    Unknown,
}

fn name<'a>(n: AttrName<'a>, source: &'a str) -> Option<String> {
    match n {
        AttrName::Ident(t) => Some(t.text(source).to_owned()),
        AttrName::Str(s) => {
            if s.parts().any(|p| matches!(p, StringPart::Interp(_))) {
                return None;
            }
            let text = Expr::String(s).content_text(source);
            if text.contains('\\') {
                return None;
            }
            Some(
                text.strip_prefix('"')
                    .and_then(|x| x.strip_suffix('"'))
                    .unwrap_or(text)
                    .to_owned(),
            )
        }
        AttrName::Interp(_) => None,
    }
}

fn path<'a>(p: Attrpath<'a>, source: &'a str) -> Option<Vec<String>> {
    p.elements().map(|e| name(e, source)).collect()
}

/// Search one literal attrset for a path. `Unknown` means a dynamic or
/// inherited entry could provide the requested name.
fn lookup<'a>(
    set: strictix_syntax::AttrsetExpr<'a>,
    wanted: &[String],
    source: &'a str,
) -> Presence {
    let mut unknown = false;
    for item in set.items() {
        match item {
            AttrItem::Inherit(_) => {
                unknown = true;
            }
            AttrItem::Binding(b) => {
                let Some(p) = b.attrpath().and_then(|p| path(p, source)) else {
                    unknown = true;
                    continue;
                };
                if p.is_empty() || wanted.is_empty() || !p.iter().zip(wanted).all(|(a, b)| a == b) {
                    continue;
                }
                if p.len() == wanted.len() {
                    return Presence::Present;
                }
                // A dotted binding creates its leading namespaces too.
                if wanted.len() < p.len() {
                    return Presence::Present;
                }
                if p.len() < wanted.len() {
                    match b.value() {
                        Some(Expr::Attrset(child)) => {
                            let r = lookup(child, &wanted[p.len()..], source);
                            if r != Presence::Missing {
                                return r;
                            }
                        }
                        Some(Expr::RecAttrset(rec)) => {
                            let Some(child) = rec.attrset() else {
                                unknown = true;
                                continue;
                            };
                            let r = lookup(child, &wanted[p.len()..], source);
                            if r != Presence::Missing {
                                return r;
                            }
                        }
                        _ => unknown = true,
                    }
                }
            }
        }
    }
    if unknown {
        Presence::Unknown
    } else {
        Presence::Missing
    }
}

fn root_set<'a>(
    expr: Expr<'a>,
    model: &SemanticModel<'a>,
) -> Option<strictix_syntax::AttrsetExpr<'a>> {
    match expr {
        Expr::Attrset(s) => Some(s),
        Expr::RecAttrset(rec) => rec.attrset(),
        Expr::Ident(t) => match crate::static_binding::value(t, model)? {
            Expr::Attrset(s) => Some(s),
            Expr::RecAttrset(rec) => rec.attrset(),
            _ => None,
        },
        Expr::Paren(p) => root_set(p.expr()?, model),
        _ => None,
    }
}

pub struct MissingAttribute;

impl Rule for MissingAttribute {
    fn code(&self) -> &'static str {
        "missing-attribute"
    }
    fn name(&self) -> &'static str {
        "Missing attribute"
    }
    fn description(&self) -> &'static str {
        "Flags selections of attributes that are provably absent from a local literal attrset."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(select) = SelectExpr::cast(node) else {
                continue;
            };
            if select.default().is_some() {
                continue;
            }
            let Some(base) = select.base() else {
                continue;
            };
            let Some(set) = root_set(base, model) else {
                continue;
            };
            let Some(p) = select.attrpath().and_then(|p| path(p, source)) else {
                continue;
            };
            if lookup(set, &p, source) == Presence::Missing {
                let missing = p.last().map(String::as_str).unwrap_or("");
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("attribute '{missing}' is missing"),
                    node.content_range(),
                ));
            }
        }
    }
}
