//! Duplicate attribute path detection for one binding container.
//!
//! Nix rejects defining the same attribute path twice inside one
//! binding container (an attrset, a rec attrset, or the binding section
//! of a let) with `attribute '...' already defined`. The recovering
//! parser here accepts that silently, so we surface it.
//!
//! Confirmed against `nix eval --expr '{ a = 1; a = 2; }'` (errors
//! `attribute 'a' already defined`), and the prefix cases:
//!
//! ```text
//! nix eval --expr '{ a = 1; a.b = 2; }'     -> error: attribute 'a' already defined
//! nix eval --expr '{ a = { }; a.c = 1; }'   -> { a = { c = 1; }; }   (merges)
//! ```
//!
//! So an identical static path (>1 bound) is always an error, but an
//! attrset literal leaf that a child descends under just *merges* in
//! Nix and is legal. The prefix rule therefore only fires when the leaf
//! (shortest) binding's value is NOT a bare attrset literal.
//!
//! Scope is a single container: `let a = 1; in { a = 2; }` is fine
//! because the two `a`'s live in different containers. No auto-fix:
//! deleting the wrong duplicate changes intent. A quoted `"a"` and an
//! ident `a` normalize to the same key (Nix treats them as the same
//! attribute), and `${...}` / interpolated path elements make the whole
//! binding dynamic, which is excluded because a collision cannot be
//! proven.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Attrpath, AttrsetExpr, Expr, LetExpr, StringPart, SyntaxKind as K,
    TextRange,
};

/// The one path a binding container assigns in a known way.
struct StaticAttr {
    /// The bound dotted path, normalized to keys (idents and plain
    /// quoted strings both become their text).
    path: Vec<String>,
    /// Whether the binding is a bare attrset (or rec attrset) literal
    /// value. Only such leaves legally merge with dotted children in
    /// Nix, so they are exempt from prefix collisions.
    is_attrset_literal: bool,
    /// Duplicate inherit names belong to `duplicate-inherit`.
    is_inherit: bool,
}

/// Flags an attribute path defined twice inside one binding container.
pub struct DuplicateAttribute;

impl Rule for DuplicateAttribute {
    fn code(&self) -> &'static str {
        "duplicate-attribute"
    }
    fn name(&self) -> &'static str {
        "Duplicate attribute"
    }
    fn description(&self) -> &'static str {
        "Flags an attribute path defined more than once inside one attrset, rec attrset, or let-bindings section — Nix rejects it with `attribute '...' already defined`, and the recovering parser accepts it silently."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            match node.kind() {
                // AttrsetExpr also covers rec attrsets: the parser nests
                // an AttrsetExpr node under a RecAttrsetExpr, so walking
                // this kind visits both the plain and the rec body.
                K::AttrsetExpr => {
                    let Some(attrset) = AttrsetExpr::cast(node) else {
                        continue;
                    };
                    check_container(attrset.items(), source, diags);
                }
                K::LetExpr => {
                    let Some(let_expr) = LetExpr::cast(node) else {
                        continue;
                    };
                    if let Some(bindings) = let_expr.bindings() {
                        check_container(bindings.items(), source, diags);
                    }
                }
                _ => {}
            }
        }
    }
}

/// Check every static path bound by one container for duplicates.
fn check_container<'a>(
    items: impl Iterator<Item = AttrItem<'a>>,
    source: &'a str,
    diags: &mut Vec<Diagnostic>,
) {
    let mut seen: Vec<StaticAttr> = Vec::new();
    for item in items {
        match item {
            AttrItem::Binding(binding) => {
                let Some(attrpath) = binding.attrpath() else {
                    continue;
                };
                let Some((path, range)) = static_attrpath(attrpath, source) else {
                    continue; // dynamic path: cannot prove a collision
                };
                let is_attrset_literal = matches!(
                    binding.value(),
                    Some(Expr::Attrset(_)) | Some(Expr::RecAttrset(_))
                );
                record(path, range, is_attrset_literal, false, &mut seen, diags);
            }
            AttrItem::Inherit(inherit) => {
                // `inherit a;` binds the one-element path `a`; `inherit
                // (x) a b;` binds `a` and `b` (the source `x` is not a
                // binding). The inherited value is never a bare attrset
                // literal here, so it can be a leaf in a prefix clash.
                for name in inherit.names() {
                    record(
                        vec![name.text(source).to_string()],
                        name.range(),
                        false,
                        true,
                        &mut seen,
                        diags,
                    );
                }
            }
        }
    }
}

/// Decode a binding's attrpath into a static dotted path, or None when
/// any element is dynamic (`${...}` or a string containing an
/// interpolation), in which case the collision cannot be proven.
fn static_attrpath<'a>(
    attrpath: Attrpath<'a>,
    source: &'a str,
) -> Option<(Vec<String>, TextRange)> {
    let mut path = Vec::new();
    let mut first: Option<TextRange> = None;
    for element in attrpath.elements() {
        match element {
            AttrName::Ident(token) => {
                if first.is_none() {
                    first = Some(token.range());
                }
                path.push(token.text(source).to_string());
            }
            AttrName::Str(string) => {
                // Interpolated string as a path element is dynamic.
                if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
                    return None;
                }
                if first.is_none() {
                    // A StringExpr node flushes leading trivia into its
                    // range; the trimmed content range spans exactly the
                    // opening quote through the closing quote.
                    first = Some(string.syntax().content_range());
                }
                path.push(decode_string(string, source));
            }
            AttrName::Interp(_) => return None,
        }
    }
    Some((path, first?))
}

/// Record one static path, reporting a duplicate when it collides with a
/// previously seen binding in this container.
fn record(
    path: Vec<String>,
    range: TextRange,
    is_attrset_literal: bool,
    is_inherit: bool,
    seen: &mut Vec<StaticAttr>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut message: Option<String> = None;
    // (a) an identical full static path bound more than once.
    for prior in seen.iter() {
        if prior.path == path && !(prior.is_inherit && is_inherit) {
            message = Some(path.join("."));
            break;
        }
    }
    // (b) a static path that is a strict prefix of another, when the
    // leaf binding defines a value that is NOT a bare attrset literal.
    if message.is_none() {
        for prior in seen.iter() {
            if prior.path.len() < path.len() && path.starts_with(&prior.path) {
                // The already-bound leaf (prefix) is earlier; only a
                // non-attrset-literal leaf is a real collision in Nix.
                if !prior.is_attrset_literal {
                    message = Some(prior.path.join("."));
                    break;
                }
            } else if path.len() < prior.path.len() && prior.path.starts_with(&path) {
                // This path is the leaf; a child was bound earlier. Only
                // fire when this leaf is a value, not a mergeable attrset.
                if !is_attrset_literal {
                    message = Some(path.join("."));
                    break;
                }
            }
        }
    }
    // Remember the binding regardless, so a third occurrence reports too.
    if let Some(name) = message {
        diags.push(Diagnostic::new(
            "duplicate-attribute",
            Severity::Error,
            format!("attribute '{name}' defined more than once"),
            range,
        ));
    }
    seen.push(StaticAttr {
        path,
        is_attrset_literal,
        is_inherit,
    });
}

/// Unescape a plain `"..."` string into its attribute-name value. Only
/// called on strings known to hold no interpolation; escapes use Nix's
/// single-quote string rules.
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
