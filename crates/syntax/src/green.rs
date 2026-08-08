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

    /// Whether the range covers no text.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
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
    children: Vec<NodeOrToken>,
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

    /// All child nodes of the given kind, in source order.
    pub fn child_nodes_of_kind(&self, kind: SyntaxKind) -> impl Iterator<Item = &SyntaxNode> {
        self.child_nodes().filter(move |n| n.kind() == kind)
    }

    /// All descendant nodes, depth-first pre-order, self first.
    pub fn descendants(&self) -> impl Iterator<Item = &SyntaxNode> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            let children: Vec<_> = node.child_nodes().collect();
            stack.extend(children.into_iter().rev());
            Some(node)
        })
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
    stack: Vec<OpenNode>,
    root: Option<SyntaxNode>,
    /// End offset of the last token emitted; used to give empty nodes a
    /// sensible (empty) range at the current position.
    last_end: u32,
}

struct OpenNode {
    kind: SyntaxKind,
    children: Vec<NodeOrToken>,
}

impl TreeBuilder {
    /// Start a new node with the given kind.
    pub fn start(&mut self, kind: SyntaxKind) {
        self.stack.push(OpenNode {
            kind,
            children: Vec::new(),
        });
    }

    /// Emit a token as a child of the innermost open node.
    pub fn token(&mut self, kind: SyntaxKind, range: TextRange) {
        self.last_end = range.end();
        let token = SyntaxToken { kind, range };
        match self.stack.last_mut() {
            Some(node) => node.children.push(NodeOrToken::Token(token)),
            None => {
                // A token outside any node should not happen; the parser
                // always wraps everything in a Root. Defensive no-op.
            }
        }
    }

    /// Close the innermost open node, computing its range from children.
    pub fn finish(&mut self) {
        let node = self.stack.pop().expect("finish without open node");
        let range = match (node.children.first(), node.children.last()) {
            (Some(first), Some(last)) => TextRange::new(first.range().start(), last.range().end()),
            _ => TextRange::new(self.last_end, self.last_end),
        };
        let node = SyntaxNode {
            kind: node.kind,
            children: node.children,
            range,
        };
        match self.stack.last_mut() {
            Some(parent) => parent.children.push(NodeOrToken::Node(node)),
            None => self.root = Some(node),
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
