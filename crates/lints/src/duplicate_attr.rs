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

use std::borrow::Cow;
use std::collections::HashMap;
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

struct LinearAttr<'a> {
    path: Vec<Cow<'a, str>>,
    is_attrset_literal: bool,
    is_inherit: bool,
}

struct TrieNode<'a> {
    children: HashMap<Cow<'a, str>, u32>,
    has_non_inherit: bool,
    has_inherit: bool,
    first_non_attrset_idx: u32,
    first_descendant_idx: u32,
}

impl<'a> TrieNode<'a> {
    fn new() -> Self {
        Self {
            children: HashMap::new(),
            has_non_inherit: false,
            has_inherit: false,
            first_non_attrset_idx: u32::MAX,
            first_descendant_idx: u32::MAX,
        }
    }
}

struct AttrTrie<'a> {
    nodes: Vec<TrieNode<'a>>,
    count: u32,
}

impl<'a> AttrTrie<'a> {
    fn new() -> Self {
        Self {
            nodes: vec![TrieNode::new()],
            count: 0,
        }
    }

    fn insert_and_check(
        &mut self,
        path: &[Cow<'a, str>],
        is_attrset_literal: bool,
        is_inherit: bool,
    ) -> Option<String> {
        let my_idx = self.count;
        self.count += 1;
        let mut cur = 0usize;
        let mut best_prefix_idx = u32::MAX;
        let mut best_prefix_depth = 0usize;
        let len = path.len();
        for (depth, seg) in path.iter().enumerate() {
            let next = if let Some(&child) = self.nodes[cur].children.get(seg.as_ref()) {
                child as usize
            } else {
                let child = self.nodes.len() as u32;
                self.nodes.push(TrieNode::new());
                self.nodes[cur].children.insert(seg.clone(), child);
                child as usize
            };
            if depth + 1 < len {
                let p_idx = self.nodes[next].first_non_attrset_idx;
                if p_idx < best_prefix_idx {
                    best_prefix_idx = p_idx;
                    best_prefix_depth = depth + 1;
                }
                if self.nodes[next].first_descendant_idx == u32::MAX {
                    self.nodes[next].first_descendant_idx = my_idx;
                }
            }
            cur = next;
        }
        let node = &mut self.nodes[cur];
        let message = if node.has_non_inherit || (!is_inherit && node.has_inherit) {
            Some(join_cow_path(path))
        } else {
            let desc_idx = if !is_attrset_literal {
                node.first_descendant_idx
            } else {
                u32::MAX
            };
            if best_prefix_idx < desc_idx {
                Some(join_cow_path(&path[..best_prefix_depth]))
            } else if desc_idx < u32::MAX {
                Some(join_cow_path(path))
            } else {
                None
            }
        };
        if is_inherit {
            node.has_inherit = true;
        } else {
            node.has_non_inherit = true;
        }
        if !is_attrset_literal && node.first_non_attrset_idx == u32::MAX {
            node.first_non_attrset_idx = my_idx;
        }
        message
    }
}

enum ContainerSeen<'a> {
    Linear(Vec<LinearAttr<'a>>),
    Trie(AttrTrie<'a>),
}

fn join_cow_path(path: &[Cow<'_, str>]) -> String {
    let mut out = String::new();
    for (i, seg) in path.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.push_str(seg);
    }
    out
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
    let mut seen = ContainerSeen::Linear(Vec::new());
    let mut path_buf: Vec<Cow<'a, str>> = Vec::with_capacity(4);
    for item in items {
        match item {
            AttrItem::Binding(binding) => {
                let Some(attrpath) = binding.attrpath() else {
                    continue;
                };
                path_buf.clear();
                let Some(range) = static_attrpath_into(attrpath, source, &mut path_buf) else {
                    continue;
                };
                let is_attrset_literal = matches!(
                    binding.value(),
                    Some(Expr::Attrset(_)) | Some(Expr::RecAttrset(_))
                );
                record(&path_buf, range, is_attrset_literal, false, &mut seen, diags);
            }
            AttrItem::Inherit(inherit) => {
                for name in inherit.names() {
                    path_buf.clear();
                    path_buf.push(Cow::Borrowed(name.text(source)));
                    record(&path_buf, name.range(), false, true, &mut seen, diags);
                }
            }
        }
    }
}

fn static_attrpath_into<'a>(
    attrpath: Attrpath<'a>,
    source: &'a str,
    path: &mut Vec<Cow<'a, str>>,
) -> Option<TextRange> {
    let mut first: Option<TextRange> = None;
    for element in attrpath.elements() {
        match element {
            AttrName::Ident(token) => {
                if first.is_none() {
                    first = Some(token.range());
                }
                path.push(Cow::Borrowed(token.text(source)));
            }
            AttrName::Str(string) => {
                if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
                    return None;
                }
                if first.is_none() {
                    first = Some(string.syntax().content_range());
                }
                path.push(Cow::Owned(decode_string(string, source)));
            }
            AttrName::Interp(_) => return None,
        }
    }
    first
}

fn record<'a>(
    path: &[Cow<'a, str>],
    range: TextRange,
    is_attrset_literal: bool,
    is_inherit: bool,
    seen: &mut ContainerSeen<'a>,
    diags: &mut Vec<Diagnostic>,
) {
    if let ContainerSeen::Linear(list) = seen {
        if list.len() >= 16 {
            let mut trie = AttrTrie::new();
            for entry in list.drain(..) {
                trie.insert_and_check(&entry.path, entry.is_attrset_literal, entry.is_inherit);
            }
            *seen = ContainerSeen::Trie(trie);
        }
    }
    let message = match seen {
        ContainerSeen::Linear(list) => {
            let mut message: Option<String> = None;
            for prior in list.iter() {
                if prior.path.as_slice() == path && !(prior.is_inherit && is_inherit) {
                    message = Some(join_cow_path(path));
                    break;
                }
            }
            if message.is_none() {
                for prior in list.iter() {
                    if prior.path.len() < path.len() && path.starts_with(&prior.path) {
                        if !prior.is_attrset_literal {
                            message = Some(join_cow_path(&prior.path));
                            break;
                        }
                    } else if path.len() < prior.path.len()
                        && prior.path.starts_with(path)
                        && !is_attrset_literal
                    {
                        message = Some(join_cow_path(path));
                        break;
                    }
                }
            }
            list.push(LinearAttr {
                path: path.to_vec(),
                is_attrset_literal,
                is_inherit,
            });
            message
        }
        ContainerSeen::Trie(trie) => trie.insert_and_check(path, is_attrset_literal, is_inherit),
    };
    if let Some(name) = message {
        diags.push(Diagnostic::new(
            "duplicate-attribute",
            Severity::Error,
            format!("attribute '{name}' defined more than once"),
            range,
        ));
    }
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
