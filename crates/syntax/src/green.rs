//! Lossless green tree: nodes, tokens, and text ranges.
//!
//! The parser produces an immutable, lossless concrete syntax tree
//! (CST). Every token from the lexer — including whitespace and comments
//! (trivia) — is preserved so that reconstructing all token texts in
//! order reproduces the source byte-for-byte. This is what lets
//! future fixes edit text without destroying formatting.

use crate::kind::SyntaxKind;

/// A half-open byte range `[start, end)` into the source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    start: u32,
    end: u32,
}

impl TextRange {
    /// Create a range `[start, end)`. Panics (in debug) if `start > end`.
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "invalid range {start}..{end}");
        Self { start, end }
    }

    /// The inclusive start offset.
    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    /// The exclusive end offset.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.end
    }

    /// Whether `offset` falls inside `[start, end)`.
    #[must_use]
    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }
}

/// A single token in the CST. Trivia (whitespace, comments) are ordinary
/// tokens preserved in the tree for lossless round-trips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyntaxToken {
    pub kind: SyntaxKind,
    pub range: TextRange,
}

impl SyntaxToken {
    /// The kind of this token.
    #[must_use]
    pub const fn kind(self) -> SyntaxKind {
        self.kind
    }

    /// The byte range this token covers.
    #[must_use]
    pub const fn range(self) -> TextRange {
        self.range
    }

    /// The source text this token covers.
    #[must_use]
    pub fn text(self, source: &str) -> &str {
        let start = self.range.start as usize;
        &source[start..start + self.range.len() as usize]
    }
}

impl TextRange {
    const fn len(self) -> u32 {
        self.end - self.start
    }
}

/// Either a child node or a token in a node's children list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeOrToken {
    Node(SyntaxNode),
    Token(SyntaxToken),
}

impl NodeOrToken {
    /// The kind of this child.
    #[must_use]
    pub fn kind(&self) -> SyntaxKind {
        match self {
            NodeOrToken::Node(n) => n.kind,
            NodeOrToken::Token(t) => t.kind,
        }
    }

    /// The byte range covered by this child.
    #[must_use]
    pub fn range(&self) -> TextRange {
        match self {
            NodeOrToken::Node(n) => n.range,
            NodeOrToken::Token(t) => t.range,
        }
    }
}

/// A node in the lossless concrete syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxNode {
    kind: SyntaxKind,
    children: Box<[NodeOrToken]>,
    range: TextRange,
}

impl SyntaxNode {
    /// This node's kind.
    #[must_use]
    pub const fn kind(&self) -> SyntaxKind {
        self.kind
    }

    /// The byte range covered by this node and all its descendants.
    #[must_use]
    pub const fn range(&self) -> TextRange {
        self.range
    }

    /// The byte range spanning this node's first and last non-trivia
    /// token, excluding leading and trailing whitespace/comments.
    /// Diagnostics and replacement fixes use this so they point at code,
    /// not trivia. Falls back to the full range when the subtree has no
    /// non-trivia token.
    #[must_use]
    pub fn content_range(&self) -> TextRange {
        let first = first_significant(self).map(|t| t.range.start());
        let last = last_significant(self).map(|t| t.range.end());
        match (first, last) {
            (Some(s), Some(e)) => TextRange::new(s, e),
            _ => self.range,
        }
    }

    /// The direct children of this node, in source order.
    #[must_use]
    pub fn children(&self) -> &[NodeOrToken] {
        &self.children
    }
    /// The child nodes (excluding tokens) of this node, in source order.
    pub fn child_nodes(&self) -> impl Iterator<Item = &SyntaxNode> {
        self.children.iter().filter_map(|c| match c {
            NodeOrToken::Node(n) => Some(n),
            NodeOrToken::Token(_) => None,
        })
    }

    /// The child tokens (excluding nodes) of this node, in source order.
    pub fn child_tokens(&self) -> impl Iterator<Item = &SyntaxToken> {
        self.children.iter().filter_map(|c| match c {
            NodeOrToken::Token(t) => Some(t),
            NodeOrToken::Node(_) => None,
        })
    }

    /// The first child node of the given kind, if any.
    #[must_use]
    pub fn child_node(&self, kind: SyntaxKind) -> Option<&SyntaxNode> {
        self.child_nodes().find(|n| n.kind() == kind)
    }

    pub fn descendants(&self) -> impl Iterator<Item = &SyntaxNode> {
        let mut stack = Vec::with_capacity(32);
        stack.push(self);
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            for child in node.children.iter().rev() {
                if let NodeOrToken::Node(n) = child {
                    stack.push(n);
                }
            }
            Some(node)
        })
    }

    #[must_use]
    pub fn child_at_offset(&self, offset: u32) -> Option<&SyntaxNode> {
        let idx = self.children.partition_point(|c| c.range().end() <= offset);
        match self.children.get(idx) {
            Some(NodeOrToken::Node(n)) if n.range.contains(offset) => Some(n),
            _ => None,
        }
    }

    #[must_use]
    pub fn find_innermost_at(&self, kind: SyntaxKind, offset: u32) -> Option<&SyntaxNode> {
        if !self.range.contains(offset) {
            return None;
        }
        let mut found = (self.kind == kind).then_some(self);
        let mut cur = self;
        while let Some(next) = cur.child_at_offset(offset) {
            if next.kind == kind {
                found = Some(next);
            }
            cur = next;
        }
        found
    }

    pub fn walk_containing<'a>(&'a self, offset: u32, mut f: impl FnMut(&'a SyntaxNode) -> bool) -> bool {
        if !self.range.contains(offset) {
            return false;
        }
        let mut cur = self;
        loop {
            if f(cur) {
                return true;
            }
            match cur.child_at_offset(offset) {
                Some(next) => cur = next,
                None => return false,
            }
        }
    }
    /// All recovered malformed regions, in source order.
    pub fn error_nodes(&self) -> impl Iterator<Item = &SyntaxNode> {
        self.descendants()
            .filter(|node| node.kind() == SyntaxKind::ErrorNode)
    }
    /// The source text this node covers.
    #[must_use]
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        let start = self.range.start as usize;
        &source[start..self.range().len() as usize + start]
    }

    /// Reconstruct all token texts in this subtree, in source order.
    ///
    /// For a well-formed tree this reproduces the source exactly.
    #[must_use]
    pub fn reassemble(&self, source: &str) -> String {
        let mut out = String::new();
        self.reassemble_into(source, &mut out);
        out
    }

    fn reassemble_into(&self, source: &str, out: &mut String) {
        for child in &self.children {
            match child {
                NodeOrToken::Node(node) => node.reassemble_into(source, out),
                NodeOrToken::Token(token) => out.push_str(token.text(source)),
            }
        }
    }
}

/// Incremental builder that assembles a [`SyntaxNode`] tree from a
/// stream of `start` / `token` / `finish` events. Node ranges are
/// computed from their children when the node is closed.
#[derive(Default)]
pub struct TreeBuilder {
    stack: Vec<(SyntaxKind, usize)>,
    children_buf: Vec<NodeOrToken>,
    root: Option<SyntaxNode>,
    last_end: u32,
}

impl TreeBuilder {
    pub fn with_capacity(events_hint: usize) -> Self {
        Self {
            stack: Vec::with_capacity(32),
            children_buf: Vec::with_capacity(events_hint.min(256)),
            root: None,
            last_end: 0,
        }
    }

    pub fn start(&mut self, kind: SyntaxKind) {
        self.stack.push((kind, self.children_buf.len()));
    }

    pub fn token(&mut self, kind: SyntaxKind, range: TextRange) {
        self.last_end = range.end();
        let token = SyntaxToken { kind, range };
        if !self.stack.is_empty() {
            self.children_buf.push(NodeOrToken::Token(token));
        }
    }

    pub fn finish(&mut self) {
        let (kind, start_idx) = self.stack.pop().expect("finish without open node");
        let slice = &self.children_buf[start_idx..];
        let (children, range) = if slice.is_empty() {
            (Box::default(), TextRange::new(self.last_end, self.last_end))
        } else {
            let range = TextRange::new(slice[0].range().start(), slice[slice.len() - 1].range().end());
            let boxed: Box<[NodeOrToken]> = self.children_buf.drain(start_idx..).collect();
            (boxed, range)
        };
        let node = SyntaxNode {
            kind,
            children,
            range,
        };
        if !self.stack.is_empty() {
            self.children_buf.push(NodeOrToken::Node(node));
        } else {
            self.root = Some(node);
        }
    }

    /// Close any remaining open nodes and return the root.
    pub fn finish_all(mut self) -> SyntaxNode {
        while !self.stack.is_empty() {
            self.finish();
        }
        self.root.expect("no root node was produced")
    }
}

/// The first non-trivia token in a node's subtree, depth-first.
fn first_significant(node: &SyntaxNode) -> Option<SyntaxToken> {
    for child in node.children() {
        match child {
            NodeOrToken::Token(t) if !t.kind().is_trivia() => return Some(*t),
            NodeOrToken::Token(_) => {}
            NodeOrToken::Node(n) => {
                if let Some(t) = first_significant(n) {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// The last non-trivia token in a node's subtree, depth-first.
fn last_significant(node: &SyntaxNode) -> Option<SyntaxToken> {
    for child in node.children().iter().rev() {
        match child {
            NodeOrToken::Token(t) if !t.kind().is_trivia() => return Some(*t),
            NodeOrToken::Token(_) => {}
            NodeOrToken::Node(n) => {
                if let Some(t) = last_significant(n) {
                    return Some(t);
                }
            }
        }
    }
    None
}
