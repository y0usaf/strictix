//! Finds repeated literal `name` fields in `listToAttrs` input lists.
//!
//! `listToAttrs` keeps the first entry for a name when its input contains
//! duplicates. This rule is deliberately syntax directed: it only inspects
//! a literal list, literal attrset entries, and plain literal strings.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, AttrName, Expr, ListExpr, StringPart, TextRange,
};

use crate::static_binding;

/// Flags duplicate literal names in a `listToAttrs` list.
pub struct DuplicateListToAttrsName;

impl Rule for DuplicateListToAttrsName {
    fn code(&self) -> &'static str {
        "duplicate-list-to-attrs-name"
    }
    fn name(&self) -> &'static str {
        "Duplicate listToAttrs name"
    }
    fn description(&self) -> &'static str {
        "Flags duplicate literal name fields in listToAttrs lists; listToAttrs keeps the first occurrence."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            // Only the explicitly qualified builtin is unambiguous here.
            // A bare `listToAttrs` may be a user supplied global.
            if !is_qualified_list_to_attrs(apply.func(), model) {
                continue;
            }
            let Some(Expr::List(list)) = apply.arg() else {
                continue;
            };
            check_list(list, source, self, diags);
        }
    }
}

fn is_qualified_list_to_attrs(expr: Option<Expr<'_>>, model: &SemanticModel<'_>) -> bool {
    let Some(Expr::Select(select)) = expr else {
        return false;
    };
    let Some(Expr::Ident(base)) = select.base() else {
        return false;
    };
    if base.text(model.source()) != "builtins"
        || !static_binding::is_unshadowed(base, model)
        || select.default().is_some()
    {
        return false;
    }
    let Some(path) = select.attrpath() else {
        return false;
    };
    let mut elements = path.elements();
    matches!(elements.next(), Some(AttrName::Ident(name)) if name.text(model.source()) == "listToAttrs")
        && elements.next().is_none()
}

fn check_list(
    list: ListExpr<'_>,
    source: &str,
    rule: &DuplicateListToAttrsName,
    diags: &mut Vec<Diagnostic>,
) {
    let mut seen: Vec<String> = Vec::new();
    for item in list.items() {
        let Some((name, range)) = literal_name(item, source) else {
            continue;
        };
        if seen.iter().any(|prior| prior == &name) {
            diags.push(Diagnostic::new(
                rule.code(),
                rule.severity(),
                format!("duplicate listToAttrs name '{name}'; first occurrence wins"),
                range,
            ));
        } else {
            seen.push(name);
        }
    }
}

fn literal_name(expr: Expr<'_>, source: &str) -> Option<(String, TextRange)> {
    let attrset = match expr {
        Expr::Attrset(a) => a,
        Expr::RecAttrset(r) => r.attrset()?,
        _ => return None,
    };
    for item in attrset.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let mut elements = path.elements();
        let Some(first) = elements.next() else {
            continue;
        };
        if elements.next().is_some() {
            continue;
        }
        let is_name = match first {
            AttrName::Ident(token) => token.text(source) == "name",
            AttrName::Str(string) => {
                !string.parts().any(|p| matches!(p, StringPart::Interp(_)))
                    && decode_string(string, source) == "name"
            }
            AttrName::Interp(_) => false,
        };
        if !is_name {
            continue;
        }
        let Expr::String(string) = binding.value()? else {
            return None;
        };
        if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
            return None;
        }
        return Some((
            decode_string(string, source),
            string.syntax().content_range(),
        ));
    }
    None
}

fn decode_string(string: strictix_syntax::StringExpr<'_>, source: &str) -> String {
    let mut out = String::new();
    for part in string.parts() {
        let StringPart::Content(token) = part else {
            continue;
        };
        let mut chars = token.text(source).chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                Some('$') => out.push('$'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        }
    }
    out
}
