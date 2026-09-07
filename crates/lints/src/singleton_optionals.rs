//! Use `optional` when a conditional list has exactly one element.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{ApplyExpr, AstNode, AttrName, Expr, SyntaxKind};

use crate::lib_helpers::{library_member, unparen};

pub struct SingletonOptionals;

impl Rule for SingletonOptionals {
    fn code(&self) -> &'static str {
        "singleton-optionals"
    }
    fn name(&self) -> &'static str {
        "Singleton optionals"
    }
    fn description(&self) -> &'static str {
        "Use optional condition value instead of optionals condition [value]. Recognizes conventional lib helpers, direct aliases, inherit (lib), and with lib. Fixes qualified calls while preserving comments and grouping. Bare or aliased callees are reported without a fix because optional may not be in scope. Explicit nested-list elements are excluded to avoid conflicting with optional-list-argument."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for call in model.root().descendants().filter_map(ApplyExpr::cast) {
            let Some(Expr::Apply(condition)) = call.func().and_then(unparen) else {
                continue;
            };
            let Some(callee) = condition.func().and_then(unparen) else {
                continue;
            };
            if library_member(model, callee) != Some("optionals") {
                continue;
            }
            if condition
                .arg()
                .is_none_or(|arg| arg.range() == callee.range())
            {
                continue;
            }
            let Some(Expr::List(list)) = call.arg().and_then(unparen) else {
                continue;
            };
            if list
                .syntax()
                .descendants()
                .any(|node| node.kind() == SyntaxKind::ErrorNode)
            {
                continue;
            }
            let mut items = list.items();
            let Some(item) = items.next() else {
                continue;
            };
            if items.next().is_some() || matches!(unparen(item), Some(Expr::List(_))) {
                continue;
            }
            let mut diag = Diagnostic::new(self.code(), self.severity(),
                "optionals builds a one-element list; use optional with the element directly",
                call.syntax().content_range())
                .with_help("Use optional condition value; for an inherited helper, import optional from the same library.");
            // A qualified replacement keeps precisely the same library base.
            // Changing an inherited/aliased name could select a different binding.
            if let Expr::Select(select) = callee {
                if let Some(AttrName::Ident(name)) =
                    select.attrpath().and_then(|path| path.elements().next())
                {
                    let open = list
                        .syntax()
                        .child_tokens()
                        .find(|token| token.kind() == SyntaxKind::LBracket);
                    let close = list
                        .syntax()
                        .child_tokens()
                        .find(|token| token.kind() == SyntaxKind::RBracket);
                    if let (Some(open), Some(close)) = (open, close) {
                        let source = model.source();
                        let range = item.content_range();
                        let before = &source[open.range().end() as usize..range.start() as usize];
                        let after = &source[range.end() as usize..close.range().start() as usize];
                        let mut fix = Fix::new("use optional for a single element")
                            .edit(name.range(), "optional");
                        if before.trim().is_empty() && after.trim().is_empty() {
                            let text = &source[range.start() as usize..range.end() as usize];
                            // Parenthesize selections with defaults and control flow
                            // so a surrounding application cannot capture their body.
                            let grouped = match item {
                                Expr::Ident(_)
                                | Expr::Int(_)
                                | Expr::Float(_)
                                | Expr::Path(_)
                                | Expr::SearchPath(_)
                                | Expr::Uri(_)
                                | Expr::String(_)
                                | Expr::IndString(_)
                                | Expr::Paren(_)
                                | Expr::Attrset(_)
                                | Expr::RecAttrset(_) => text.to_owned(),
                                Expr::Select(select) if select.default().is_none() => {
                                    text.to_owned()
                                }
                                _ => format!("({text})"),
                            };
                            fix = fix.edit(list.syntax().content_range(), grouped);
                        } else {
                            // Retain comments surrounding the item verbatim.
                            fix = fix.edit(open.range(), "(").edit(close.range(), ")");
                        }
                        diag = diag.with_fix(fix);
                    }
                }
            }
            diags.push(diag);
        }
    }
}
