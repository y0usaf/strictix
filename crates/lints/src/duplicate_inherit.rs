//! Detects repeated names in inherit statements within one binding container.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{AstNode, AttrItem, AttrsetExpr, LetExpr, SyntaxKind as K};

/// Flags an inherited attribute more than once in one attrset or let block.
/// `duplicate-attribute` owns collisions between inherit and ordinary
/// bindings; this rule owns inherit-versus-inherit collisions.
pub struct DuplicateInherit;

impl Rule for DuplicateInherit {
    fn code(&self) -> &'static str {
        "duplicate-inherit"
    }
    fn name(&self) -> &'static str {
        "Duplicate inherit"
    }
    fn description(&self) -> &'static str {
        "Flags an attribute inherited more than once in one attrset or let-binding section."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            match node.kind() {
                K::AttrsetExpr => {
                    if let Some(attrset) = AttrsetExpr::cast(node) {
                        check_items(attrset.items(), model.source(), diags);
                    }
                }
                K::LetExpr => {
                    if let Some(let_expr) = LetExpr::cast(node) {
                        if let Some(bindings) = let_expr.bindings() {
                            check_items(bindings.items(), model.source(), diags);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn check_items<'a>(
    items: impl Iterator<Item = AttrItem<'a>>,
    source: &'a str,
    diags: &mut Vec<Diagnostic>,
) {
    let mut seen = Vec::new();
    for item in items {
        let AttrItem::Inherit(inherit) = item else {
            continue;
        };
        for name in inherit.names() {
            let text = name.text(source);
            if seen.contains(&text) {
                diags.push(Diagnostic::new(
                    "duplicate-inherit",
                    Severity::Error,
                    format!("attribute '{text}' inherited more than once"),
                    name.range(),
                ));
            } else {
                seen.push(text);
            }
        }
    }
}
