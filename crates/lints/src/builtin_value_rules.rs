//! Literal value contracts that can be checked without evaluating Nix.

use std::collections::HashSet;

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, AttrName, Expr, StringExpr, StringPart, SyntaxKind, TextRange,
};

pub struct InvalidListToAttrsEntry;
pub struct ReplaceStringsLengthMismatch;
pub struct InvalidBuiltinRange;

impl Rule for InvalidListToAttrsEntry {
    fn code(&self) -> &'static str {
        "invalid-list-to-attrs-entry"
    }
    fn name(&self) -> &'static str {
        "Invalid listToAttrs entry"
    }
    fn description(&self) -> &'static str {
        "Flags literal listToAttrs entries missing required fields or having invalid types."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let Some(("listToAttrs", args)) =
                ApplyExpr::cast(node).and_then(|apply| builtin_call(apply, model))
            else {
                continue;
            };
            let [arg] = args.as_slice() else { continue };
            let Some(Expr::List(list)) = unparen(*arg) else {
                continue;
            };
            let mut names = HashSet::new();
            let mut unknown_prior = false;
            for item in list.items() {
                let Some(entry) = static_entry(item, model.source()) else {
                    if non_set(item, model) {
                        diags.push(Diagnostic::new(
                            self.code(),
                            self.severity(),
                            "builtin 'listToAttrs' expects each entry to be an attribute set",
                            item.content_range(),
                        ));
                    }
                    unknown_prior = true;
                    continue;
                };
                if entry.name.is_none() && entry.dotted_name.is_none() {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "builtin 'listToAttrs' entry is missing the 'name' attribute",
                        item.content_range(),
                    ));
                    unknown_prior = true;
                    continue;
                }
                let invalid_name = entry.dotted_name.or_else(|| {
                    entry
                        .name
                        .filter(|name| non_string(*name, model))
                        .map(|name| name.content_range())
                });
                if let Some(range) = invalid_name {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "builtin 'listToAttrs' expects the 'name' attribute to be a string",
                        range,
                    ));
                    unknown_prior = true;
                    continue;
                }
                let name = entry.name.and_then(unparen).and_then(|expr| match expr {
                    Expr::String(string) => literal_string(string, model.source()),
                    _ => None,
                });
                // Nix checks `value` only for the first entry with a given
                // name. An earlier unknown name may make this entry a duplicate.
                let definitely_first = !unknown_prior
                    && match &name {
                        Some(name) => !names.contains(name),
                        None => names.is_empty(),
                    };
                if !entry.has_value && definitely_first {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "builtin 'listToAttrs' entry is missing the 'value' attribute",
                        item.content_range(),
                    ));
                }
                if let Some(name) = name {
                    names.insert(name);
                } else {
                    unknown_prior = true;
                }
            }
        }
    }
}

impl Rule for ReplaceStringsLengthMismatch {
    fn code(&self) -> &'static str {
        "replace-strings-length-mismatch"
    }
    fn name(&self) -> &'static str {
        "replaceStrings length mismatch"
    }
    fn description(&self) -> &'static str {
        "Flags fully applied replaceStrings calls whose literal lists have different lengths."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let Some(("replaceStrings", args)) =
                ApplyExpr::cast(node).and_then(|apply| builtin_call(apply, model))
            else {
                continue;
            };
            let [from, to, _] = args.as_slice() else {
                continue;
            };
            let (Some(Expr::List(from)), Some(Expr::List(to))) = (unparen(*from), unparen(*to))
            else {
                continue;
            };
            let (from_len, to_len) = (from.items().count(), to.items().count());
            if from_len != to_len {
                diags.push(Diagnostic::new(
                    self.code(), self.severity(),
                    format!("builtin 'replaceStrings' search list has length {from_len} but replacement list has length {to_len}"),
                    to.syntax().content_range(),
                ));
            }
        }
    }
}

impl Rule for InvalidBuiltinRange {
    fn code(&self) -> &'static str {
        "invalid-builtin-range"
    }
    fn name(&self) -> &'static str {
        "Invalid builtin range"
    }
    fn description(&self) -> &'static str {
        "Flags negative literal genList lengths and substring start positions."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            let Some((name, args)) =
                ApplyExpr::cast(node).and_then(|apply| builtin_call(apply, model))
            else {
                continue;
            };
            let (arg, label) = match (name, args.as_slice()) {
                ("genList", [_, length]) => (*length, "length"),
                ("substring", [start, _, _]) => (*start, "start position"),
                _ => continue,
            };
            if literal_integer(arg, model.source()).is_some_and(|n| n < 0) {
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("builtin '{name}' {label} must not be negative"),
                    arg.content_range(),
                ));
            }
        }
    }
}

fn unparen(mut expr: Expr<'_>) -> Option<Expr<'_>> {
    while let Expr::Paren(paren) = expr {
        expr = paren.expr()?;
    }
    Some(expr)
}

fn builtin_call<'a>(
    apply: ApplyExpr<'a>,
    model: &SemanticModel<'a>,
) -> Option<(&'a str, Vec<Expr<'a>>)> {
    let mut args = Vec::new();
    let mut callee = Expr::Apply(apply);
    while let Expr::Apply(apply) = unparen(callee)? {
        args.push(apply.arg()?);
        callee = apply.func()?;
    }
    let Expr::Select(select) = unparen(callee)? else {
        return None;
    };
    let Expr::Ident(base) = unparen(select.base()?)? else {
        return None;
    };
    if base.text(model.source()) != "builtins"
        || !crate::static_binding::is_unshadowed(base, model)
        || select.default().is_some()
    {
        return None;
    }
    let mut path = select.attrpath()?.elements();
    let AttrName::Ident(name) = path.next()? else {
        return None;
    };
    if path.next().is_some() {
        return None;
    }
    args.reverse();
    Some((name.text(model.source()), args))
}

fn literal_integer(expr: Expr<'_>, source: &str) -> Option<i128> {
    match unparen(expr)? {
        Expr::Int(token) => token.text(source).parse().ok(),
        Expr::Unary(unary)
            if unary
                .syntax()
                .child_tokens()
                .any(|token| token.kind() == SyntaxKind::Minus) =>
        {
            let Expr::Int(token) = unparen(unary.operand()?)? else {
                return None;
            };
            token.text(source).parse::<i128>().ok()?.checked_neg()
        }
        _ => None,
    }
}

fn non_string(expr: Expr<'_>, model: &SemanticModel<'_>) -> bool {
    match unparen(expr) {
        Some(
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Path(_)
            | Expr::SearchPath(_)
            | Expr::List(_)
            | Expr::Attrset(_)
            | Expr::RecAttrset(_)
            | Expr::Lambda(_),
        ) => true,
        Some(Expr::Ident(token)) => {
            matches!(token.text(model.source()), "true" | "false" | "null")
                && crate::static_binding::is_unshadowed(token, model)
        }
        Some(Expr::Unary(unary)) => {
            unary
                .syntax()
                .child_tokens()
                .any(|token| token.kind() == SyntaxKind::Minus)
                && matches!(
                    unary.operand().and_then(unparen),
                    Some(Expr::Int(_) | Expr::Float(_))
                )
        }
        _ => false,
    }
}

fn non_set(expr: Expr<'_>, model: &SemanticModel<'_>) -> bool {
    match unparen(expr) {
        Some(Expr::Attrset(_) | Expr::RecAttrset(_)) | None => false,
        Some(Expr::String(_) | Expr::IndString(_) | Expr::Uri(_)) => true,
        Some(expr) => non_string(expr, model),
    }
}

struct Entry<'a> {
    name: Option<Expr<'a>>,
    dotted_name: Option<TextRange>,
    has_value: bool,
}

fn static_entry<'a>(expr: Expr<'a>, source: &str) -> Option<Entry<'a>> {
    let attrset = match unparen(expr)? {
        Expr::Attrset(attrset) => attrset,
        Expr::RecAttrset(rec) => rec.attrset()?,
        _ => return None,
    };
    let mut entry = Entry {
        name: None,
        dotted_name: None,
        has_value: false,
    };
    for item in attrset.items() {
        // Inherited values and dynamic names are outside this rule's proof.
        let AttrItem::Binding(binding) = item else {
            return None;
        };
        let path = binding.attrpath()?;
        let names: Option<Vec<_>> = path
            .elements()
            .map(|attr| match attr {
                AttrName::Ident(token) => Some(token.text(source).to_owned()),
                AttrName::Str(string) => literal_string(string, source),
                AttrName::Interp(_) => None,
            })
            .collect();
        let names = names?;
        match names.first()?.as_str() {
            "name" if names.len() == 1 => entry.name = Some(binding.value()?),
            "name" => entry.dotted_name = Some(path.syntax().content_range()),
            "value" => entry.has_value = true,
            _ => {}
        }
    }
    Some(entry)
}

fn literal_string(string: StringExpr<'_>, source: &str) -> Option<String> {
    let mut out = String::new();
    for part in string.parts() {
        let StringPart::Content(token) = part else {
            return None;
        };
        let mut chars = token.text(source).chars();
        while let Some(ch) = chars.next() {
            out.push(if ch == '\\' {
                match chars.next()? {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                }
            } else {
                ch
            });
        }
    }
    Some(out)
}
