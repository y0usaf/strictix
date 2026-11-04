//! Flags option defaults that only resolve on the machine that wrote them.
//!
//! A `default` holding an absolute home path bakes the author's layout into
//! the option: it evaluates everywhere, then fails at runtime on any other
//! host, and it survives review because it looks like configuration. The
//! fix is to derive the path from the module's own options (`config.user.homeDirectory`,
//! an option the host sets) or to leave the option without a default.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, ApplyExpr, AttrItem, AttrName, Attrpath, Expr, StringPart, SyntaxKind, TextRange,
};

use crate::schema::unwrap_parens;

/// Flags a machine-local absolute path used as an option default.
pub struct MachineLocalDefault;

impl Rule for MachineLocalDefault {
    fn code(&self) -> &'static str {
        "machine-local-default"
    }

    fn name(&self) -> &'static str {
        "Machine-local option default"
    }

    fn description(&self) -> &'static str {
        "Flags an option whose `default` is an absolute path under /home, /Users or /root: it evaluates anywhere and only works on the machine that wrote it."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let allowed: Vec<&str> = config.setting_list(self.code(), "allow").collect();
        for node in model.root().descendants() {
            if node.kind() != SyntaxKind::ApplyExpr {
                continue;
            }
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let Some(func) = apply.func() else {
                continue;
            };
            let name = func.content_text(source);
            if name != "mkOption" && !name.ends_with(".mkOption") {
                continue;
            }
            let Some(Expr::Attrset(options)) = apply.arg().map(unwrap_parens) else {
                continue;
            };
            for item in options.items() {
                let AttrItem::Binding(binding) = item else {
                    continue;
                };
                let Some(path) = static_path(binding.attrpath(), source) else {
                    continue;
                };
                if path.as_slice() != ["default"] {
                    continue;
                }
                let Some(value) = binding.value() else {
                    continue;
                };
                for (text, range) in literals(value, source) {
                    if allowed.iter().any(|rule| text.starts_with(rule)) {
                        continue;
                    }
                    if !machine_local(&text) {
                        continue;
                    }
                    diags.push(
                        Diagnostic::new(
                            self.code(),
                            self.severity(),
                            format!("option default '{text}' is an absolute path on one machine"),
                            range,
                        )
                        .with_help(
                            "derive it from the module's options (config.user.homeDirectory, a host-set option) instead of a literal path",
                        ),
                    );
                }
            }
        }
    }
}

fn static_path(path: Option<Attrpath<'_>>, source: &str) -> Option<Vec<String>> {
    path?
        .elements()
        .map(|element| match element {
            AttrName::Ident(token) => Some(token.text(source).to_string()),
            AttrName::Str(string) if !string.parts().any(|p| matches!(p, StringPart::Interp(_))) => {
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

/// Literal strings inside a default: the value itself or a one-level list.
fn literals<'a>(value: Expr<'a>, source: &'a str) -> Vec<(String, TextRange)> {
    match value {
        Expr::String(string) => string_text(string, source).into_iter().collect(),
        Expr::List(list) => list
            .items()
            .filter_map(|item| match item {
                Expr::String(string) => string_text(string, source),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn string_text<'a>(string: strictix_syntax::StringExpr<'a>, source: &'a str) -> Option<(String, TextRange)> {
    if string.parts().any(|part| matches!(part, StringPart::Interp(_))) {
        return None;
    }
    let range = string.syntax().content_range();
    let text = &source[range.start() as usize..range.end() as usize];
    Some((
        text.strip_prefix('"')?.strip_suffix('"')?.to_string(),
        range,
    ))
}

/// Whether the literal is a home-relative absolute path and not, say, a store path.
fn machine_local(text: &str) -> bool {
    let mut parts = text.split('/');
    matches!(parts.next(), Some("")) && matches!(parts.next(), Some("home" | "Users" | "root"))
}
