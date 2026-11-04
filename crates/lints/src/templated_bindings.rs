use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, AttrsetExpr, Binding, Expr, LetBindings, NodeOrToken,
    SyntaxKind as K, SyntaxNode, TextRange,
};

use crate::singleton_attrset::exempt;

pub struct TemplatedBindings;

const MIN: usize = 3;

struct Member<'a> {
    name: &'a str,
    binding: Binding<'a>,
    holes: Vec<TextRange>,
}

fn holes(node: &SyntaxNode, source: &str, name: &str, out: &mut Vec<TextRange>) {
    let in_path = node.kind() == K::Attrpath;
    for child in node.children() {
        match child {
            NodeOrToken::Token(token) => {
                let hole = match token.kind() {
                    K::StringContent => true,
                    kind => in_path && kind.is_attr_name(),
                };
                if hole && token.text(source) == name {
                    out.push(token.range());
                }
            }
            NodeOrToken::Node(inner) => holes(inner, source, name, out),
        }
    }
}

fn shape(node: &SyntaxNode, source: &str, holes: &[TextRange], out: &mut Vec<String>) {
    for child in node.children() {
        match child {
            NodeOrToken::Token(token) => {
                if token.kind().is_trivia() {
                    continue;
                }
                if holes.contains(&token.range()) {
                    out.push("\u{0}".to_owned());
                } else {
                    out.push(token.text(source).to_owned());
                }
            }
            NodeOrToken::Node(inner) => shape(inner, source, holes, out),
        }
    }
}

fn value_node(value: Expr<'_>) -> Option<&SyntaxNode> {
    Some(match value {
        Expr::Let(e) => e.syntax(),
        Expr::With(e) => e.syntax(),
        Expr::Assert(e) => e.syntax(),
        Expr::If(e) => e.syntax(),
        Expr::Attrset(e) => e.syntax(),
        Expr::RecAttrset(e) => e.syntax(),
        Expr::List(e) => e.syntax(),
        Expr::Lambda(e) => e.syntax(),
        Expr::Apply(e) => e.syntax(),
        Expr::Unary(e) => e.syntax(),
        Expr::Bin(e) => e.syntax(),
        Expr::Select(e) => e.syntax(),
        Expr::HasAttr(e) => e.syntax(),
        Expr::String(e) => e.syntax(),
        Expr::IndString(e) => e.syntax(),
        Expr::Paren(e) => e.syntax(),
        _ => return None,
    })
}

fn groups<'a>(items: impl Iterator<Item = AttrItem<'a>>, source: &'a str) -> Vec<Vec<Member<'a>>> {
    let mut groups: Vec<(Vec<String>, Vec<Member<'a>>)> = Vec::new();
    for item in items {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
            continue;
        };
        let mut elements = path.elements();
        let (Some(AttrName::Ident(name)), None) = (elements.next(), elements.next()) else {
            continue;
        };
        let name = name.text(source);
        let Some(node) = value_node(value) else {
            continue;
        };
        let mut found = Vec::new();
        holes(node, source, name, &mut found);
        if found.is_empty() {
            continue;
        }
        let mut tokens = Vec::new();
        shape(node, source, &found, &mut tokens);
        let member = Member {
            name,
            binding,
            holes: found,
        };
        match groups.iter_mut().find(|(known, _)| *known == tokens) {
            Some((_, members)) => members.push(member),
            None => groups.push((tokens, vec![member])),
        }
    }
    groups.into_iter().map(|(_, members)| members).collect()
}

fn text(source: &str, range: TextRange) -> &str {
    &source[range.start() as usize..range.end() as usize]
}

fn binds(node: &SyntaxNode, source: &str, param: &str) -> bool {
    let named = |item: AttrItem<'_>| match item {
        AttrItem::Binding(binding) => binding
            .attrpath()
            .and_then(|path| path.elements().next())
            .is_some_and(|first| matches!(first, AttrName::Ident(t) if t.text(source) == param)),
        AttrItem::Inherit(inherit) => inherit.names().any(|t| t.text(source) == param),
    };
    if let Some(lets) = node.child_node(K::LetBindings).and_then(LetBindings::cast) {
        return lets.items().any(named);
    }
    if let Some(lambda) = strictix_syntax::LambdaExpr::cast(node) {
        return match lambda.param() {
            strictix_syntax::LambdaParam::Ident(token) => token.text(source) == param,
            strictix_syntax::LambdaParam::Formals(formals, at) => {
                at.is_some_and(|t| t.text(source) == param)
                    || formals.params().any(|p| p.name.text(source) == param)
            }
        };
    }
    false
}

fn generator(
    model: &SemanticModel<'_>,
    ancestors: &[&SyntaxNode],
    member: &Member<'_>,
) -> Option<String> {
    let source = model.source();
    let value = member.binding.value()?;
    let range = value.content_range();
    let body = text(source, range);
    if body.contains('\n') {
        return None;
    }
    let param = ["name", "key", "tag", "label"].into_iter().find(|param| {
        !model.is_bound(param, range.start())
            && !ancestors.iter().any(|node| binds(node, source, param))
            && !body
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '\'' || c == '-'))
                .any(|word| word == *param)
    })?;
    let mut out = String::new();
    let mut at = range.start();
    for hole in &member.holes {
        let quoted = hole.start() > at
            && source.as_bytes()[hole.start() as usize - 1] == b'"'
            && source.as_bytes().get(hole.end() as usize) == Some(&b'"');
        if quoted {
            out.push_str(text(source, TextRange::new(at, hole.start() - 1)));
            out.push_str(param);
            at = hole.end() + 1;
        } else {
            out.push_str(text(source, TextRange::new(at, hole.start())));
            out.push_str(&format!("${{{param}}}"));
            at = hole.end();
        }
    }
    out.push_str(text(source, TextRange::new(at, range.end())));
    if out == param {
        return Some("lib.id".to_owned());
    }
    Some(format!("({param}: {out})"))
}

fn line_span(source: &str, range: TextRange) -> TextRange {
    let bytes = source.as_bytes();
    let mut start = range.start() as usize;
    while start > 0 && matches!(bytes[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    let mut end = range.end() as usize;
    if bytes.get(end) == Some(&b'\n') {
        end += 1;
        let rest = &source[end..];
        if let Some(blank) = rest.find('\n') {
            if rest[..blank].trim().is_empty() {
                end += blank + 1;
            }
        }
    }
    TextRange::new(start as u32, end as u32)
}

fn has_comment(node: &SyntaxNode) -> bool {
    node.children().iter().any(|child| match child {
        NodeOrToken::Token(token) => token.kind() == K::Comment,
        NodeOrToken::Node(inner) => has_comment(inner),
    })
}

fn walk<'a>(
    node: &'a SyntaxNode,
    ancestors: &mut Vec<&'a SyntaxNode>,
    f: &mut impl FnMut(&'a SyntaxNode, &[&'a SyntaxNode]),
) {
    f(node, ancestors);
    ancestors.push(node);
    for child in node.child_nodes() {
        walk(child, ancestors, f);
    }
    ancestors.pop();
}

impl Rule for TemplatedBindings {
    fn code(&self) -> &'static str {
        "templated-bindings"
    }

    fn name(&self) -> &'static str {
        "Templated bindings"
    }

    fn description(&self) -> &'static str {
        "Flags bindings of one attribute set or `let` whose values are the same expression but for the binding's own name written as an attribute name or string text, `str = s: {str = s;}; sq = s: {sq = s;};`, which one `lib.genAttrs [\"str\" \"sq\"] (name: s: {${name} = s;})` defines together. When `lib` is in scope, a value fits on one line and nothing in the block holds a comment, the fix writes `inherit (lib.genAttrs [\"a\" \"b\"] (name: ...)) a b;` in the first binding's place and deletes the others. The `min` setting is how many such bindings make a group (3 by default), and `exempt` lists path suffixes it skips. Opt-in."
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
        let min = config.setting_usize(self.code(), "min").unwrap_or(MIN);
        let source = model.source();
        let mut ancestors = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            let found = if let Some(set) = AttrsetExpr::cast(node) {
                groups(set.items(), source)
            } else if let Some(lets) = LetBindings::cast(node) {
                groups(lets.items(), source)
            } else {
                return;
            };
            let found: Vec<_> = found.into_iter().filter(|g| g.len() >= min).collect();
            if found.is_empty() {
                return;
            }
            let end = node.content_range().end();
            let fixable = model.is_bound("lib", end.saturating_sub(1)) && !has_comment(node);
            for members in &found {
                let names: Vec<&str> = members.iter().map(|m| m.name).collect();
                let quoted: Vec<String> = names.iter().map(|name| format!("\"{name}\"")).collect();
                let diag = Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "{} bindings differ only by their own name; define them with `lib.genAttrs [{}] (name: ...)`",
                        members.len(),
                        quoted.join(" ")
                    ),
                    members[0].binding.syntax().content_range(),
                );
                let generator = members
                    .iter()
                    .find_map(|member| generator(model, ancestors, member))
                    .filter(|_| fixable);
                let Some(generator) = generator else {
                    diags.push(diag);
                    continue;
                };
                let mut fix = Fix::new("define the bindings with lib.genAttrs");
                for (index, member) in members.iter().enumerate() {
                    let range = member.binding.syntax().content_range();
                    fix = if index == 0 {
                        fix.edit(
                            range,
                            format!(
                                "inherit (lib.genAttrs [{}] {generator}) {};",
                                quoted.join(" "),
                                names.join(" ")
                            ),
                        )
                    } else {
                        fix.edit(line_span(source, range), "")
                    };
                }
                diags.push(diag.with_fix(fix));
            }
        });
    }
}
