use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{AstNode, ListExpr, NodeOrToken, SyntaxKind as K, SyntaxNode};

use crate::singleton_attrset::exempt;

pub struct SingletonList;

fn has_comment(node: &SyntaxNode) -> bool {
    node.children().iter().any(|child| match child {
        NodeOrToken::Token(token) => token.kind() == K::Comment,
        NodeOrToken::Node(inner) => has_comment(inner),
    })
}

impl Rule for SingletonList {
    fn code(&self) -> &'static str {
        "singleton-list"
    }

    fn name(&self) -> &'static str {
        "Singleton list"
    }

    fn description(&self) -> &'static str {
        "Flags a list of exactly one item written over several lines whose item fits on one line, `[\n  x\n]`, and rewrites it as `[x]`. A list holding a comment is left alone. The `exempt` setting lists path suffixes it skips. Opt-in."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        if exempt(model, config, self.code()) {
            return;
        }
        let source = model.source();
        for node in model.root().descendants() {
            let Some(list) = ListExpr::cast(node) else {
                continue;
            };
            let mut items = list.items();
            let (Some(item), None) = (items.next(), items.next()) else {
                continue;
            };
            let range = node.content_range();
            let whole = &source[range.start() as usize..range.end() as usize];
            let text = item.content_text(source);
            if !whole.contains('\n') || text.contains('\n') || has_comment(node) {
                continue;
            }
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("this list holds one item; write `[{text}]`"),
                    range,
                )
                .with_fix(Fix::new("write the list on one line").edit(range, format!("[{text}]"))),
            );
        }
    }
}
