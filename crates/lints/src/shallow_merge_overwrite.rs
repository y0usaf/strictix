//! Finds literal attrset merges that replace an entire nested attrset.
//!
//! Nix's `//` is shallow: when both operands define `a`, the right hand
//! `a` replaces the left hand value.  This rule only reports cases where
//! both sides are literal attrsets and the key sets prove that a nested
//! left-hand key disappears.  Expressions and dynamic attribute names are
//! deliberately treated as unknown.

use std::collections::{BTreeMap, BTreeSet};

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, AttrsetExpr, BinExpr, Expr, StringPart, SyntaxKind, SyntaxNode,
};

/// Flags shallow merges that discard known nested attributes.
pub struct ShallowMergeOverwrite;

impl Rule for ShallowMergeOverwrite {
    fn code(&self) -> &'static str {
        "shallow-merge-overwrite"
    }

    fn name(&self) -> &'static str {
        "Shallow merge overwrite"
    }

    fn description(&self) -> &'static str {
        "Flags literal attrset merges where the right-hand side replaces an attrset and loses known nested attributes from the left-hand side."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(bin) = BinExpr::cast(node) else {
            return;
        };
        if bin.op() != Some(SyntaxKind::SlashSlash) {
            return;
        }
        let (Some(lhs), Some(rhs)) = (bin.lhs(), bin.rhs()) else {
            return;
        };
        let (Some(left), Some(right)) = (literal_shape(lhs, source), literal_shape(rhs, source))
        else {
            return;
        };

        // A top-level right-hand key replaces the whole left-hand value.
        // Compare immediate child keys only: deeper values may be unknown.
        let lost = left.iter().any(|(root, left_keys)| {
            left_keys.as_ref().is_some_and(|left_keys| {
                right
                    .get(root)
                    .and_then(Option::as_ref)
                    .is_some_and(|right_keys| left_keys.iter().any(|key| !right_keys.contains(key)))
            })
        });
        if lost {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                "shallow merge overwrites nested attributes from the left-hand side",
                node.content_range(),
            ));
        }
    }
}

type Shape = BTreeMap<String, Option<BTreeSet<String>>>;

/// Known immediate children of top-level attributes; None means unknown.
fn literal_shape(expr: Expr<'_>, source: &str) -> Option<Shape> {
    let Expr::Attrset(set) = unwrap_parens(expr) else {
        return None;
    };
    let mut shape = Shape::new();
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            return None;
        };
        let path = static_path(binding.attrpath()?, source)?;
        let first = path.first()?.clone();
        let children = if let Some(child) = path.get(1) {
            Some(BTreeSet::from([child.clone()]))
        } else {
            match unwrap_parens(binding.value()?) {
                Expr::Attrset(nested) => immediate_keys(nested, source),
                _ => None,
            }
        };
        shape
            .entry(first)
            .and_modify(|previous| match (previous.as_mut(), children.as_ref()) {
                (Some(previous), Some(children)) => previous.extend(children.iter().cloned()),
                _ => *previous = None,
            })
            .or_insert(children);
    }
    Some(shape)
}

fn immediate_keys(set: AttrsetExpr<'_>, source: &str) -> Option<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            return None;
        };
        let path = static_path(binding.attrpath()?, source)?;
        keys.insert(path.first()?.clone());
    }
    Some(keys)
}

fn unwrap_parens(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        let Some(inner) = paren.expr() else { break };
        expr = inner;
    }
    expr
}

fn static_path(path: strictix_syntax::Attrpath<'_>, source: &str) -> Option<Vec<String>> {
    path.elements()
        .map(|element| match element {
            AttrName::Ident(token) => Some(token.text(source).to_string()),
            AttrName::Str(string)
                if !string.parts().any(|p| matches!(p, StringPart::Interp(_))) =>
            {
                let range = string.syntax().content_range();
                let text = &source[range.start() as usize..range.end() as usize];
                if text.contains('\\') {
                    return None;
                }
                Some(text.strip_prefix('"')?.strip_suffix('"')?.to_string())
            }
            AttrName::Str(_) | AttrName::Interp(_) => None,
        })
        .collect()
}
