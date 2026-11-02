//! String-interpolation lints: interpolants that can never coerce, and
//! interpolations that add nothing.
//!
//! Both rules walk only the parts of `"..."` and `''...''` string
//! literals. Attrname interpolations (`a.${x}`) and path interpolations
//! are different constructs with different coercion rules and are never
//! string parts, so restricting the walk to string nodes excludes them
//! structurally.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Attrpath, AttrsetExpr, Expr, InterpExpr, StringPart, SyntaxKind,
    TextRange,
};

/// Visit every interpolation part of every string literal under
/// `root`, with a flag for whether the enclosing string is indented
/// (`''...''`). Both rules need the same traversal; going through the
/// typed `parts()` accessor keeps attrpath/path interpolations out by
/// construction, and the callback shape avoids collecting per node.
fn for_each_string_interp<'a>(
    root: &'a strictix_syntax::SyntaxNode,
    mut f: impl FnMut(InterpExpr<'a>, bool),
) {
    for node in root.descendants() {
        match Expr::cast(node) {
            Some(Expr::String(s)) => {
                for part in s.parts() {
                    if let StringPart::Interp(interp) = part {
                        f(interp, false);
                    }
                }
            }
            Some(Expr::IndString(s)) => {
                for part in s.parts() {
                    if let StringPart::Interp(interp) = part {
                        f(interp, true);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The exact `${`...`}` span of an interpolation, delimiters included,
/// computed from the delimiter tokens themselves. The parser can flush
/// leading trivia into a node's range, so a fix must never splice over
/// `interp.syntax().range()` directly.
fn interp_span(interp: InterpExpr<'_>) -> Option<TextRange> {
    let children = interp.syntax().children();
    let start = children
        .iter()
        .find(|c| c.kind() == SyntaxKind::InterpStart)?;
    let end = children
        .iter()
        .rev()
        .find(|c| c.kind() == SyntaxKind::InterpEnd)?;
    Some(TextRange::new(start.range().start(), end.range().end()))
}

/// Strip paren layers: `${(5)}` is exactly as doomed as `${5}`, and a
/// paren wrapper proves nothing about coercibility either way.
fn unwrap_parens(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        match paren.expr() {
            Some(inner) => expr = inner,
            None => break,
        }
    }
    expr
}

/// Whether an attrset literal *may* coerce to a string: it visibly
/// binds `outPath` or `__toString` (interpolation forces `outPath` and
/// falls back to calling `__toString`), or it carries a dynamic attr
/// name that could produce either at eval time. Conservative: any doubt
/// counts as coercible, so the rule stays silent.
fn attrset_may_coerce(set: AttrsetExpr<'_>, source: &str) -> bool {
    if set
        .syntax()
        .child_nodes()
        .any(|n| n.kind() == SyntaxKind::ErrorNode)
    {
        return true;
    }
    for item in set.items() {
        match item {
            AttrItem::Binding(binding) => {
                let Some(path) = binding.attrpath() else {
                    return true;
                };
                let Some(head) = path.elements().next() else {
                    return true;
                };
                match head {
                    AttrName::Ident(token) => {
                        if matches!(token.text(source), "outPath" | "__toString") {
                            return true;
                        }
                    }
                    AttrName::Str(string) => {
                        if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
                            return true;
                        }
                        if matches!(
                            decode_string(string, source).as_str(),
                            "outPath" | "__toString"
                        ) {
                            return true;
                        }
                    }
                    AttrName::Interp(_) => return true,
                }
            }
            AttrItem::Inherit(inherit) => {
                if inherit
                    .names()
                    .any(|n| matches!(n.text(source), "outPath" | "__toString"))
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Unescape a plain `"..."` string into its value. Only called on
/// strings known to hold no interpolation.
fn decode_string(string: strictix_syntax::StringExpr<'_>, source: &str) -> String {
    let mut out = String::new();
    for part in string.parts() {
        if let StringPart::Content(token) = part {
            out.push_str(&decode_escapes(token.text(source)));
        }
    }
    out
}

/// Decode the standard `"..."` escapes from raw content text.
fn decode_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
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
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Flags `"${...}"` whose interpolant is a literal that can never
/// coerce to a string.
pub struct CoercedInterpolation;

impl Rule for CoercedInterpolation {
    fn code(&self) -> &'static str {
        "coerced-interpolation"
    }

    fn name(&self) -> &'static str {
        "Uncoercible interpolation"
    }

    fn description(&self) -> &'static str {
        "Flags string interpolation of a literal int, float, boolean, null, list, or attrset — Nix interpolation only coerces strings and paths, so these always fail with 'cannot coerce ... to a string', and lazily, so the error hides until the string is forced."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for_each_string_interp(model.root(), |interp, _indented| {
            let Some(expr) = interp.expr() else {
                return;
            };
            let Some(span) = interp_span(interp) else {
                return;
            };
            let (what, help): (&str, &str) = match unwrap_parens(expr) {
                Expr::Int(token) => {
                    let digits = token.text(source);
                    let mut diag = Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "interpolating an integer literal always fails — Nix cannot coerce an integer to a string",
                        span,
                    )
                    .with_help("splice the digits directly or wrap the value in toString");
                    if digits.len() == 1 || !digits.starts_with('0') {
                        diag = diag.with_fix(
                            Fix::new("splice the integer's digits into the string")
                                .edit(span, digits),
                        );
                    }
                    diags.push(diag);
                    return;
                }
                Expr::Float(_) => (
                    "a float literal",
                    "wrap the value in toString (note: toString renders floats with six decimals, e.g. \"1.500000\")",
                ),
                Expr::List(_) => (
                    "a list literal",
                    "interpolation cannot render lists; use toString (space-joins coercible elements) or concatStringsSep",
                ),
                Expr::Attrset(set) => {
                    if attrset_may_coerce(set, source) {
                        return;
                    }
                    (
                        "an attrset without outPath or __toString",
                        "only attrsets with an outPath or __toString attribute coerce; use builtins.toJSON or restructure",
                    )
                }
                Expr::RecAttrset(rec) => {
                    let coercible = match rec.attrset() {
                        Some(set) => attrset_may_coerce(set, source),
                        None => true,
                    };
                    if coercible {
                        return;
                    }
                    (
                        "an attrset without outPath or __toString",
                        "only attrsets with an outPath or __toString attribute coerce; use builtins.toJSON or restructure",
                    )
                }
                Expr::Ident(token) => {
                    let name = token.text(source);
                    if !matches!(name, "true" | "false" | "null") {
                        return;
                    }
                    let unbound = model
                        .references()
                        .iter()
                        .find(|r| r.name.range() == token.range())
                        .is_some_and(|r| r.resolved.is_none() && r.via_with.is_none());
                    if !unbound {
                        return;
                    }
                    if name == "null" {
                        (
                            "null",
                            "interpolation cannot render null; use toString (which yields \"\") or an explicit default",
                        )
                    } else {
                        (
                            "a boolean",
                            "interpolation cannot render booleans; use toString (true -> \"1\", false -> \"\") or an if/then string",
                        )
                    }
                }
                _ => return,
            };
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("interpolating {what} always fails — Nix cannot coerce it to a string"),
                    span,
                )
                .with_help(help),
            );
        });
    }
}

/// Flags interpolation of a plain string literal — `"${"foo"}"` is `"foo"`.
pub struct RedundantInterpolation;

impl Rule for RedundantInterpolation {
    fn code(&self) -> &'static str {
        "redundant-interpolation"
    }

    fn name(&self) -> &'static str {
        "Redundant interpolation"
    }

    fn description(&self) -> &'static str {
        "Flags `${\"...\"}` — interpolating a plain string literal wraps a string in itself; splice the content directly."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for_each_string_interp(model.root(), |interp, indented| {
            let Some(Expr::String(inner)) = interp.expr() else {
                return;
            };
            let mut parts = inner.parts();
            let Some(StringPart::Content(content)) = parts.next() else {
                return;
            };
            if parts.next().is_some() {
                return;
            }
            let Some(span) = interp_span(interp) else {
                return;
            };
            let raw = content.text(source);
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                "redundant interpolation of a string literal — the wrapper adds nothing",
                span,
            )
            .with_help("splice the string's content directly into the outer string");
            let safe = !raw.contains('\\')
                && !raw.contains("''")
                && !raw.contains("${")
                && (!indented || !raw.contains('\''));
            if safe {
                diag =
                    diag.with_fix(Fix::new("splice the string content directly").edit(span, raw));
            }
            diags.push(diag);
        });
    }
}

pub struct RedundantQuotedAttr;

impl Rule for RedundantQuotedAttr {
    fn code(&self) -> &'static str {
        "redundant-quoted-attr"
    }

    fn name(&self) -> &'static str {
        "Redundant quoted attribute name"
    }

    fn description(&self) -> &'static str {
        "Flags an attribute name written as `\"${...}\"`: when the interpolated value is a string, `${...}` names the same attribute without the quotes. No automatic fix, because the quoted form also coerces sets with `__toString` or `outPath`, which `${...}` rejects."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            if node.kind() != SyntaxKind::Attrpath {
                continue;
            }
            let Some(path) = Attrpath::cast(node) else {
                continue;
            };
            for element in path.elements() {
                let AttrName::Str(string) = element else {
                    continue;
                };
                let mut parts = string.parts();
                let Some(StringPart::Interp(_)) = parts.next() else {
                    continue;
                };
                if parts.next().is_some() {
                    continue;
                }
                let children = string.syntax().children();
                let Some(open) = children
                    .iter()
                    .find(|c| c.kind() == SyntaxKind::StringStart)
                else {
                    continue;
                };
                let Some(close) = children
                    .iter()
                    .rev()
                    .find(|c| c.kind() == SyntaxKind::StringEnd)
                else {
                    continue;
                };
                let span = TextRange::new(open.range().start(), close.range().end());
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "`\"${...}\"` quotes a lone interpolation; for a string value `${...}` is the same attribute name",
                        span,
                    )
                    .with_help("drop the quotes if the value is always a string"),
                );
            }
        }
    }
}
