use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, AttrsetExpr, Binding, Expr, NodeOrToken, StringPart,
    SyntaxKind as K, SyntaxNode, TextRange,
};

pub struct SingletonAttrset;

fn text(source: &str, range: TextRange) -> &str {
    &source[range.start() as usize..range.end() as usize]
}

fn link<'a>(binding: Binding<'a>, source: &str) -> Option<Binding<'a>> {
    let Some(Expr::Attrset(set)) = binding.value() else {
        return None;
    };
    if !text(source, set.syntax().content_range()).contains('\n') {
        return None;
    }
    let mut items = set.items();
    let (Some(AttrItem::Binding(inner)), None) = (items.next(), items.next()) else {
        return None;
    };
    inner.attrpath()?;
    inner.value()?;
    Some(inner)
}

fn key(name: AttrName<'_>, source: &str) -> String {
    match name {
        AttrName::Ident(token) => token.text(source).to_owned(),
        AttrName::Str(string) => {
            let mut out = String::new();
            for part in string.parts() {
                match part {
                    StringPart::Content(token) => out.push_str(token.text(source)),
                    StringPart::Interp(_) => {
                        return text(source, string.syntax().content_range()).to_owned()
                    }
                }
            }
            out
        }
        AttrName::Interp(interp) => text(source, interp.syntax().content_range()).to_owned(),
    }
}

fn first_key(binding: Binding<'_>, source: &str) -> Option<String> {
    binding
        .attrpath()?
        .elements()
        .next()
        .map(|name| key(name, source))
}

fn placed(binding: Binding<'_>, ancestors: &[&SyntaxNode], source: &str) -> bool {
    let Some((parent, rest)) = ancestors.split_last() else {
        return false;
    };
    let Some(set) = AttrsetExpr::cast(parent) else {
        return false;
    };
    if rest
        .last()
        .is_some_and(|grand| grand.kind() == K::RecAttrsetExpr)
    {
        return false;
    }
    let Some(own) = first_key(binding, source) else {
        return false;
    };
    set.items().all(|item| match item {
        AttrItem::Binding(other) => {
            other.range() == binding.range() || first_key(other, source) != Some(own.clone())
        }
        AttrItem::Inherit(_) => true,
    })
}

fn absorbed(binding: Binding<'_>, ancestors: &[&SyntaxNode], source: &str) -> bool {
    let [.., outer, _] = ancestors else {
        return false;
    };
    let Some(outer) = Binding::cast(outer) else {
        return false;
    };
    link(outer, source).is_some_and(|inner| inner.range() == binding.range())
        && placed(outer, &ancestors[..ancestors.len() - 2], source)
}

fn has_comment_outside(node: &SyntaxNode, keep: TextRange) -> bool {
    node.children().iter().any(|child| match child {
        NodeOrToken::Token(token) => {
            token.kind() == K::Comment
                && (token.range().start() < keep.start() || token.range().end() > keep.end())
        }
        NodeOrToken::Node(inner) => has_comment_outside(inner, keep),
    })
}

fn has_multiline_string(node: &SyntaxNode, source: &str) -> bool {
    node.children().iter().any(|child| match child {
        NodeOrToken::Token(_) => false,
        NodeOrToken::Node(inner) => {
            (inner.kind() == K::StringExpr && text(source, inner.content_range()).contains('\n'))
                || has_multiline_string(inner, source)
        }
    })
}

fn dedent(value: &str, by: usize) -> Option<String> {
    let mut lines = value.split('\n');
    let mut out = lines.next()?.to_owned();
    for line in lines {
        out.push('\n');
        if line.trim().is_empty() {
            out.push_str(line.get(by.min(line.len())..)?);
        } else if line.len() >= by && line.as_bytes()[..by].iter().all(|b| *b == b' ') {
            out.push_str(&line[by..]);
        } else {
            return None;
        }
    }
    Some(out)
}

fn exempt(model: &SemanticModel<'_>, config: &LintConfig, code: &str) -> bool {
    let Some(path) = model.path() else {
        return false;
    };
    let path = path.to_string_lossy();
    config
        .setting_list(code, "exempt")
        .any(|suffix| path.ends_with(suffix))
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

impl Rule for SingletonAttrset {
    fn code(&self) -> &'static str {
        "singleton-attrset"
    }

    fn name(&self) -> &'static str {
        "Singleton attribute set"
    }

    fn description(&self) -> &'static str {
        "Flags a binding of an attribute set whose value is a set of exactly one binding written over several lines, `x = {\n  y = v;\n};`, and rewrites the chain of such sets into one attribute path, `x.y = v;`, which Nix evaluates to the same value. A `let` binding, a binding of a `rec` set, a binding whose set has another binding starting with the same name (so `repeated-keys` keeps its grouping), a set on one line, a set holding an `inherit` and a comment the rewrite would drop are left alone. A finding inside another's rewrite has no fix until that one is applied. The `exempt` setting lists path suffixes it skips. Opt-in."
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
        let mut ancestors = Vec::new();
        let mut fixed: Vec<TextRange> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            let Some(binding) = Binding::cast(node) else {
                return;
            };
            let Some(first) = link(binding, source) else {
                return;
            };
            if !placed(binding, ancestors, source) || absorbed(binding, ancestors, source) {
                return;
            }
            let Some(outer_path) = binding.attrpath() else {
                return;
            };
            let mut paths = vec![text(source, outer_path.syntax().content_range())];
            let mut last = first;
            let mut levels = 1;
            while let Some(next) = link(last, source) {
                paths.push(text(
                    source,
                    last.attrpath().expect("linked").syntax().content_range(),
                ));
                last = next;
                levels += 1;
            }
            paths.push(text(
                source,
                last.attrpath().expect("linked").syntax().content_range(),
            ));
            let value = last.value().expect("linked");
            let kept = value.content_range();
            if has_comment_outside(binding.syntax(), kept) {
                return;
            }
            let raw = text(source, kept);
            let body = if has_multiline_string(last.syntax(), source) {
                raw.to_owned()
            } else {
                dedent(raw, 2 * levels).unwrap_or_else(|| raw.to_owned())
            };
            let joined = paths.join(".");
            let range = binding.syntax().content_range();
            let diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!("this set holds one binding; write `{joined} = ...;`"),
                outer_path.syntax().content_range(),
            );
            if fixed
                .iter()
                .any(|outer| outer.start() <= range.start() && range.end() <= outer.end())
            {
                diags.push(diag);
                return;
            }
            fixed.push(range);
            diags.push(
                diag.with_fix(
                    Fix::new("write the bindings as one attribute path")
                        .edit(range, format!("{joined} = {body};")),
                ),
            );
        });
    }
}
