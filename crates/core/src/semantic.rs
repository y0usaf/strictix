//! The semantic model: scopes, bindings, and references for one file.
//!
//! The syntax crate gives us a shape-aware tree but no meaning: the `a`
//! in `let a = 1; in a` and the `a` in `{ a = 1; }` are the same kind
//! of token to the parser, yet one is a definition and the other a use.
//! This module is the layer that answers "what does this ident refer
//! to?" — it records every scope, every binding, and every ident used
//! in expression position, then resolves each use to the binding its
//! name and position select under Nix scoping rules.
//!
//! The model is LAZY: [`SemanticModel::new`] only stores the source and
//! the tree. The first accessor call runs the one-time build and every
//! later accessor reuses it. Rules therefore pay for the walk only when
//! a file rule actually needs the model, and pay it exactly once per
//! file. The build lives in a `OnceLock` rather than a
//! `RefCell<Option<Built>>` because the accessors hand out `&[Binding]`
//! slices: a slice borrowed through a `RefCell` guard cannot outlive
//! the temporary guard, so the guard would have to be leaked on every
//! access. `OnceLock` gives the same build-once semantics with plain
//! borrows.
//!
//! Nix scoping, as implemented here:
//!
//! - `let` is sequential: binding *i* is visible in bindings *i+1..n*
//!   and in the body, never in its own value — the classic
//!   `let x = x;` infinite-recursion gotcha. The same rule governs
//!   formals defaults: parameter *i* is visible in the defaults of
//!   *j > i* and in the body.
//! - `rec { ... }` is the opposite: every attribute name is visible in
//!   every value, forwards and backwards.
//! - A bare (non-rec) attrset binds nothing for resolution: its names
//!   are not in scope anywhere, not even inside the attrset itself.
//!   `inherit (x) a b;` inside one still records `a` and `b` as
//!   [`BindingKind::InheritName`] entries, because rules may want to
//!   see them.
//! - `with e; body` introduces a fallback, never a shadow: the body's
//!   idents resolve lexically first, and only when the whole lexical
//!   path fails does the innermost `with` whose body covers the ident
//!   become a candidate ([`Reference::via_with`]).
//!
//! Scopes are stored as byte ranges so that "which scope governs this
//! offset" is a containment question, not a traversal one. Because the
//! walker creates scopes depth-first in source order and every new
//! scope's range nests inside its parent's, the innermost scope
//! containing an offset is simply the last-created scope whose range
//! contains it.
//!
//! The build runs in two passes. Pass 1 walks the tree once, creating
//! scopes, bindings, with sites, import sites, and collecting every
//! expression-position ident token. Pass 2 resolves each collected
//! ident against the finished scope table and fills
//! [`Binding::references`]. Two passes are required because `rec`
//! makes names visible *before* their defining value: a binding must
//! exist before any value can be resolved against it, so resolution
//! cannot happen during the walk.

use std::sync::OnceLock;

use strictix_syntax::{
    AstNode, AttrItem, AttrName, Expr, LambdaParam, Root, StringPart, SyntaxNode, SyntaxToken,
    TextRange,
};

/// The stable identity of one scope.
///
/// A u32 index into the model's internal scope table. Opaque to
/// consumers: comparing two ids tells you whether they are the same
/// scope, and nothing more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

/// What kind of construct created a binding. Rules use this to decide
/// whether a binding is the kind they care about — an unused let
/// binding is only a finding for [`BindingKind::LetBinding`], not for a
/// formal parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
    /// `x: ...` or a formal parameter in `{ a, b ? 1, ... }`.
    LambdaParam,
    /// The `args` in `args@{ ... }: ...` or `{ ... } @ args: ...`.
    AtName,
    /// `let a = ...;` — sequential visibility.
    LetBinding,
    /// An attribute of `rec { ... }` — mutually visible.
    RecAttr,
    /// `inherit (x) a b;` — bound in the containing block per its rules.
    InheritName,
}

/// One definition: the ident token that introduces a name, the scope it
/// lives in, and the byte ranges of the uses that resolve to it.
///
/// `references` is filled by the second pass of the build; before any
/// accessor runs it is empty.
pub struct Binding<'a> {
    /// The defining ident token (the first element of an attrpath).
    pub name: &'a SyntaxToken,
    /// The scope this binding belongs to.
    pub scope: ScopeId,
    /// Byte ranges of resolved uses, in source order (second pass).
    pub references: Vec<TextRange>,
    /// What kind of construct created this binding.
    pub kind: BindingKind,
}

/// One use of a name in expression position.
///
/// Every ident token that appears where a value is expected is a
/// reference. Ident tokens in binding position (attrpath elements,
/// inherit names, formals) and select fields (`a.b`'s `b`) are not
/// references.
pub struct Reference<'a> {
    /// The ident token used in expression position.
    pub name: &'a SyntaxToken,
    /// Index into [`SemanticModel::bindings`] when a lexical binding
    /// was found; `None` when the name is unbound lexically.
    pub resolved: Option<usize>,
    /// Index into [`SemanticModel::with_sites`] when no lexical binding
    /// was found but an enclosing `with` body covers this position.
    /// The reference still resolves to nothing (`resolved` stays
    /// `None`): statically we cannot know whether the with's attrset
    /// actually provides the name, so `via_with` only records the
    /// innermost candidate.
    pub via_with: Option<usize>,
}

/// One `with e; body` construct.
pub struct WithSite {
    /// The whole `with` expression (from `with` through the body).
    pub scope_range: TextRange,
    /// The body after the semicolon — the region the fallback covers.
    pub body_range: TextRange,
}

/// One call to `import`, plain or via `builtins.import`.
pub struct ImportSite {
    /// The whole application expression.
    pub call_range: TextRange,
    /// The argument expression (the path or string being imported).
    pub path_range: TextRange,
}

/// The internal classification of a scope. Only [`ScopeKind::Attrset`]
/// scopes are excluded from resolution: they host `inherit` names but
/// bind nothing for name lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Let,
    Lambda,
    Rec,
    With,
    Attrset,
}

/// One scope: the byte range it governs, its parent, and the slice of
/// the model's binding table it owns.
struct Scope {
    kind: ScopeKind,
    parent: Option<ScopeId>,
    range: TextRange,
    /// Indices into [`Built::bindings`], in declaration order.
    bindings: Vec<usize>,
    /// One offset per binding in this scope: the point from which the
    /// binding becomes visible. `let` bindings and formals use this to
    /// implement sequential visibility; rec scopes ignore it
    /// (everything is visible everywhere).
    visible_from: Vec<u32>,
}

/// The result of the one-time build: everything the model knows about
/// one file. Built lazily on the first accessor call and shared
/// thereafter; fields are internal, reached through
/// [`SemanticModel`]'s accessors.
pub struct Built<'a> {
    source: &'a str,
    scopes: Vec<Scope>,
    bindings: Vec<Binding<'a>>,
    references: Vec<Reference<'a>>,
    with_sites: Vec<WithSite>,
    import_sites: Vec<ImportSite>,
}

/// Lazy semantic model over one parsed file.
pub struct SemanticModel<'a> {
    source: &'a str,
    root: &'a SyntaxNode,
    built: OnceLock<Built<'a>>,
}

impl<'a> SemanticModel<'a> {
    /// Wrap a parsed tree. Does no work: the build runs on the first
    /// accessor call and is cached.
    pub fn new(source: &'a str, root: &'a SyntaxNode) -> Self {
        Self {
            source,
            root,
            built: OnceLock::new(),
        }
    }

    /// The source text this model was built over.
    #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// The parse tree this model was built over.
    #[must_use]
    pub fn root(&self) -> &'a SyntaxNode {
        self.root
    }

    /// Build once, then return the shared result. The build is
    /// idempotent: only the first call walks the tree.
    fn ensure_built(&self) -> &Built<'a> {
        self.built
            .get_or_init(|| Built::build(self.source, self.root))
    }

    /// All bindings, in declaration order.
    #[must_use]
    pub fn bindings(&self) -> &[Binding<'a>] {
        &self.ensure_built().bindings
    }

    /// All references in source order.
    #[must_use]
    pub fn references(&self) -> &[Reference<'a>] {
        &self.ensure_built().references
    }

    /// All `with` constructs, in source order.
    #[must_use]
    pub fn with_sites(&self) -> &[WithSite] {
        &self.ensure_built().with_sites
    }

    /// All `import` calls, in source order.
    #[must_use]
    pub fn import_sites(&self) -> &[ImportSite] {
        &self.ensure_built().import_sites
    }

    /// Resolve an ident token belonging to this tree: find the
    /// reference with the same byte range and return the binding it
    /// resolved to. Tokens that are not expression-position references
    /// (attrpath elements, inherit names, formals) yield `None`.
    ///
    /// Tokens are `Copy` values, so identity is compared by byte range
    /// — two tokens can never share a range within one tree.
    #[must_use]
    pub fn resolve(&self, token: &SyntaxToken) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        let idx = built
            .references
            .iter()
            .find(|r| r.name.range() == token.range())
            .and_then(|r| r.resolved)?;
        Some(&built.bindings[idx])
    }

    /// The nearest lexical binding for `name` visible at `offset`, or
    /// `None`. Scopes are searched innermost-out and the first binding
    /// whose scope rules make it visible at `offset` wins. No `with`
    /// fallback — use [`Self::is_bound`] for the with-aware answer.
    #[must_use]
    pub fn resolve_lexical(&self, name: &str, offset: u32) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        built
            .resolve_lexical_index(name, offset)
            .map(|i| &built.bindings[i])
    }

    /// The nearest outer binding shadowed by `binding`, if any.
    ///
    /// Nix `let` is recursive, so at its own name position a binding
    /// resolves to itself; shadowing must therefore be checked from the
    /// enclosing scope outward.
    #[must_use]
    pub fn outer_shadow(&self, binding: &Binding<'a>) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        let offset = binding.name.range().start();
        let name = binding.name.text(built.source);
        let mut scope_id = built.scopes[binding.scope.0 as usize].parent?;
        loop {
            let scope = &built.scopes[scope_id.0 as usize];
            if scope.kind != ScopeKind::Attrset {
                for (pos, &bi) in scope.bindings.iter().enumerate() {
                    let candidate = &built.bindings[bi];
                    if candidate.name.text(built.source) == name
                        && built.is_visible(scope_id, pos, offset)
                    {
                        return Some(candidate);
                    }
                }
            }
            scope_id = scope.parent?;
        }
    }

    /// Whether any binding — lexical, or any `with` whose body covers
    /// `offset` — could provide `name` at `offset`. The `with` side is
    /// deliberately conservative: statically we cannot know whether a
    /// with's attrset provides the name, so every covering with counts.
    #[must_use]
    pub fn is_bound(&self, name: &str, offset: u32) -> bool {
        let built = self.ensure_built();
        built.resolve_lexical_index(name, offset).is_some()
            || built
                .with_sites
                .iter()
                .any(|w| w.body_range.contains(offset))
    }
}

impl<'a> Built<'a> {
    /// The two-pass build described in the module docs.
    fn build(source: &'a str, root: &'a SyntaxNode) -> Self {
        let mut builder = Builder {
            source,
            scopes: Vec::new(),
            bindings: Vec::new(),
            with_sites: Vec::new(),
            import_sites: Vec::new(),
            idents: Vec::new(),
            stack: Vec::new(),
        };
        builder.new_scope(ScopeKind::Root, root.range());
        if let Some(expr) = Root::cast(root).and_then(|r| r.expr()) {
            builder.walk_expr(expr);
        }
        let mut built = Self {
            source,
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: Vec::new(),
            with_sites: builder.with_sites,
            import_sites: builder.import_sites,
        };
        for token in builder.idents {
            built.record_reference(token);
        }
        built
    }

    /// Record one reference and resolve it against the finished scope
    /// table, filling both the reference's fields and the resolved
    /// binding's `references`.
    fn record_reference(&mut self, token: &'a SyntaxToken) {
        let offset = token.range().start();
        let name = token.text(self.source);
        let resolved = self.resolve_lexical_index(name, offset);
        let via_with = if resolved.is_none() {
            self.with_fallback(offset)
        } else {
            None
        };
        if let Some(bi) = resolved {
            self.bindings[bi].references.push(token.range());
        }
        self.references.push(Reference {
            name: token,
            resolved,
            via_with,
        });
    }

    /// The innermost scope whose range contains `offset`. Scopes nest
    /// strictly (each new scope's range lies inside its parent's) and
    /// are created depth-first in source order, so the last scope
    /// containing the offset is the innermost.
    fn innermost_scope(&self, offset: u32) -> Option<ScopeId> {
        self.scopes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| s.range.contains(offset))
            .map(|(i, _)| ScopeId(i as u32))
    }

    /// The innermost `with` whose body contains `offset`, if any. Sites
    /// are recorded in source order and nested bodies are subranges, so
    /// a reverse scan finds the innermost one first.
    fn with_fallback(&self, offset: u32) -> Option<usize> {
        self.with_sites
            .iter()
            .enumerate()
            .rev()
            .find(|(_, w)| w.body_range.contains(offset))
            .map(|(i, _)| i)
    }

    /// Lexical resolution: walk scopes outward from the innermost one
    /// containing `offset`; the first binding whose name matches and
    /// whose scope rules make it visible at `offset` wins.
    fn resolve_lexical_index(&self, name: &str, offset: u32) -> Option<usize> {
        let mut scope_id = self.innermost_scope(offset)?;
        loop {
            let scope = &self.scopes[scope_id.0 as usize];
            // Attrset scopes host inherit names but bind nothing for
            // resolution; Root, With, and empty slices fall through the
            // loop naturally.
            if scope.kind != ScopeKind::Attrset {
                for (pos, &bi) in scope.bindings.iter().enumerate() {
                    let binding = &self.bindings[bi];
                    if binding.name.text(self.source) == name
                        && self.is_visible(scope_id, pos, offset)
                    {
                        return Some(bi);
                    }
                }
            }
            scope_id = scope.parent?;
        }
    }

    /// Whether the binding at `pos` (within `scope_id`) is visible at
    /// `offset`. `let` and lambda scopes gate on their per-binding
    /// visibility start; rec scopes are visible everywhere in their
    /// range, which contains `offset` by construction.
    fn is_visible(&self, scope_id: ScopeId, pos: usize, offset: u32) -> bool {
        let scope = &self.scopes[scope_id.0 as usize];
        match scope.kind {
            ScopeKind::Let | ScopeKind::Lambda => {
                offset >= scope.visible_from[pos] && offset < scope.range.end()
            }
            ScopeKind::Rec => offset < scope.range.end(),
            _ => false,
        }
    }
}

/// Pass-1 scratch state: the walker's output plus the scope stack that
/// tracks which scope each newly created construct belongs to.
struct Builder<'a> {
    source: &'a str,
    scopes: Vec<Scope>,
    bindings: Vec<Binding<'a>>,
    with_sites: Vec<WithSite>,
    import_sites: Vec<ImportSite>,
    /// Expression-position ident tokens, in source order, resolved in
    /// pass 2 once every scope and binding exists.
    idents: Vec<&'a SyntaxToken>,
    stack: Vec<ScopeId>,
}

impl<'a> Builder<'a> {
    /// Open a scope as a child of the current one and make it current.
    fn new_scope(&mut self, kind: ScopeKind, range: TextRange) -> ScopeId {
        let parent = self.stack.last().copied();
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            kind,
            parent,
            range,
            bindings: Vec::new(),
            visible_from: Vec::new(),
        });
        self.stack.push(id);
        id
    }

    /// Close the current scope, restoring its parent.
    fn pop_scope(&mut self) {
        self.stack.pop();
    }

    /// Record one binding in `scope`, visible from `visible_from`.
    fn register(
        &mut self,
        name: &'a SyntaxToken,
        kind: BindingKind,
        scope: ScopeId,
        visible_from: u32,
    ) {
        let idx = self.bindings.len();
        self.bindings.push(Binding {
            name,
            kind,
            scope,
            references: Vec::new(),
        });
        let slot = &mut self.scopes[scope.0 as usize];
        slot.bindings.push(idx);
        slot.visible_from.push(visible_from);
    }

    /// The first plain-name element of a binding's attrpath: the single
    /// name that binding introduces. String-led paths (`"a".b = ...`)
    /// bind a name we cannot represent as one token, so they are
    /// skipped.
    fn binding_name(binding: &strictix_syntax::Binding<'a>) -> Option<&'a SyntaxToken> {
        match binding.attrpath()?.elements().next()? {
            AttrName::Ident(token) => Some(token),
            AttrName::Str(_) | AttrName::Interp(_) => None,
        }
    }

    /// Whether an application is an `import` call: `import arg` or
    /// `builtins.import arg`.
    fn is_import(source: &str, apply: &strictix_syntax::ApplyExpr<'a>) -> bool {
        match apply.func() {
            Some(Expr::Ident(token)) => token.text(source) == "import",
            Some(Expr::Select(select)) => {
                if !matches!(
                    select.base(),
                    Some(Expr::Ident(t)) if t.text(source) == "builtins"
                ) {
                    return false;
                }
                let mut elements = select
                    .attrpath()
                    .map(|ap| ap.elements())
                    .into_iter()
                    .flatten();
                let first_is_import = match elements.next() {
                    Some(AttrName::Ident(t)) => t.text(source) == "import",
                    _ => false,
                };
                first_is_import && elements.next().is_none()
            }
            _ => false,
        }
    }

    /// Walk one expression subtree: open scopes, register bindings,
    /// collect import sites, and push every expression-position ident
    /// token for pass 2. Malformed regions never reach this function —
    /// `Expr::cast` rejects ErrorNode subtrees — so a broken source
    /// yields an empty or partial model, never a panic.
    fn walk_expr(&mut self, expr: Expr<'a>) {
        match expr {
            Expr::Ident(token) => self.idents.push(token),
            Expr::Let(let_expr) => {
                let scope = self.new_scope(ScopeKind::Let, let_expr.range());
                if let Some(bindings) = let_expr.bindings() {
                    for item in bindings.items() {
                        match item {
                            AttrItem::Binding(binding) => {
                                // Nix `let` is recursive: bindings are
                                // visible inside their own value (forward
                                // references and self-recursion both work;
                                // evaluation is lazy). Visible from the
                                // start of the scope.
                                if let Some(name) = Self::binding_name(&binding) {
                                    self.register(
                                        name,
                                        BindingKind::LetBinding,
                                        scope,
                                        let_expr.range().start(),
                                    );
                                }
                                if let Some(value) = binding.value() {
                                    self.walk_expr(value);
                                }
                            }
                            AttrItem::Inherit(inherit) => {
                                if let Some(source) = inherit.source() {
                                    self.walk_expr(source);
                                }
                                for name in inherit.names() {
                                // Record the inherited name as a
                                // reference to the outer binding so the
                                // linter treats it as "used".
                                self.idents.push(name);
                                // The new binding enters visibility at its
                                // own token's end, NOT at the let's start,
                                // so the reference above resolves to the
                                // outer binding, not this one.
                                let visible_from = name.range().end();
                                self.register(
                                    name,
                                    BindingKind::LetBinding,
                                    scope,
                                    visible_from,
                                );
                            }
                            }
                        }
                    }
                }
                if let Some(body) = let_expr.body() {
                    self.walk_expr(body);
                }
                self.pop_scope();
            }
            Expr::Lambda(lambda) => {
                let scope = self.new_scope(ScopeKind::Lambda, lambda.range());
                match lambda.param() {
                    LambdaParam::Ident(token) => {
                        // `x: body` — x is visible in the body, never
                        // in itself.
                        self.register(token, BindingKind::LambdaParam, scope, token.range().end());
                    }
                    LambdaParam::Formals(formals, at_name) => {
                        // Nix formals defaults are recursive: a param is
                        // visible in its own and later defaults, so
                        // register before walking.
                        for param in formals.params() {
                            self.register(
                                param.name,
                                BindingKind::LambdaParam,
                                scope,
                                lambda.range().start(),
                            );
                            if let Some(default) = param.default {
                                self.walk_expr(default);
                            }
                        }
                        if let Some(at) = at_name {
                            // The at-name is visible only in the body,
                            // whichever side of the formals it sits on.
                            let visible_from = formals.range().end().max(at.range().end());
                            self.register(at, BindingKind::AtName, scope, visible_from);
                        }
                    }
                }
                if let Some(body) = lambda.body() {
                    self.walk_expr(body);
                }
                self.pop_scope();
            }
            Expr::With(with_expr) => {
                // The scope expression is evaluated in the surrounding
                // scope, not the with's own.
                if let Some(scope_expr) = with_expr.scope() {
                    self.walk_expr(scope_expr);
                }
                if let Some(body) = with_expr.body() {
                    self.with_sites.push(WithSite {
                        scope_range: with_expr.range(),
                        body_range: body.range(),
                    });
                    // The with-scope governs only the body and never
                    // shadows: its binding set is empty, so resolution
                    // walks through it and the with shows up only as a
                    // via_with fallback.
                    self.new_scope(ScopeKind::With, body.range());
                    self.walk_expr(body);
                    self.pop_scope();
                }
            }
            Expr::Attrset(attrset) => {
                let scope = self.new_scope(ScopeKind::Attrset, attrset.range());
                for item in attrset.items() {
                    match item {
                        AttrItem::Binding(binding) => {
                            // A non-rec attrset binds nothing for
                            // resolution; only its values are walked.
                            if let Some(value) = binding.value() {
                                self.walk_expr(value);
                            }
                        }
                        AttrItem::Inherit(inherit) => {
                            if let Some(source) = inherit.source() {
                                self.walk_expr(source);
                            }
                            for name in inherit.names() {
                                // Record as a reference to the outer
                                // binding (the name came from there).
                                self.idents.push(name);
                                // The new name binds forward in the
                                // attrset — other attrs can inherit it.
                                let visible_from = name.range().end();
                                self.register(name, BindingKind::InheritName, scope, visible_from);
                            }
                        }
                    }
                }
                self.pop_scope();
            }
            Expr::RecAttrset(rec) => {
                if let Some(attrset) = rec.attrset() {
                    let scope = self.new_scope(ScopeKind::Rec, attrset.range());
                    for item in attrset.items() {
                        match item {
                            AttrItem::Binding(binding) => {
                                // Register before walking the value:
                                // rec makes every name visible in every
                                // value, including earlier ones
                                // (`rec { b = a; a = 1; }` is valid).
                                if let Some(name) = Self::binding_name(&binding) {
                                    self.register(
                                        name,
                                        BindingKind::RecAttr,
                                        scope,
                                        attrset.range().start(),
                                    );
                                }
                                if let Some(value) = binding.value() {
                                    self.walk_expr(value);
                                }
                            }
                            AttrItem::Inherit(inherit) => {
                                if let Some(source) = inherit.source() {
                                    self.walk_expr(source);
                                }
                                for name in inherit.names() {
                                    // Record as a reference to the outer
                                    // binding.
                                    self.idents.push(name);
                                    self.register(
                                        name,
                                        BindingKind::RecAttr,
                                        scope,
                                        name.range().end(),
                                    );
                                }
                            }
                        }
                    }
                    self.pop_scope();
                }
            }
            Expr::Assert(assert_expr) => {
                if let Some(cond) = assert_expr.cond() {
                    self.walk_expr(cond);
                }
                if let Some(body) = assert_expr.body() {
                    self.walk_expr(body);
                }
            }
            Expr::If(if_expr) => {
                if let Some(cond) = if_expr.cond() {
                    self.walk_expr(cond);
                }
                if let Some(then_branch) = if_expr.then_branch() {
                    self.walk_expr(then_branch);
                }
                if let Some(else_branch) = if_expr.else_branch() {
                    self.walk_expr(else_branch);
                }
            }
            Expr::List(list) => {
                for item in list.items() {
                    self.walk_expr(item);
                }
            }
            Expr::Apply(apply) => {
                if Self::is_import(self.source, &apply) {
                    if let Some(arg) = apply.arg() {
                        self.import_sites.push(ImportSite {
                            call_range: apply.range(),
                            path_range: arg.range(),
                        });
                    }
                }
                if let Some(func) = apply.func() {
                    self.walk_expr(func);
                }
                if let Some(arg) = apply.arg() {
                    self.walk_expr(arg);
                }
            }
            Expr::Unary(unary) => {
                if let Some(operand) = unary.operand() {
                    self.walk_expr(operand);
                }
            }
            Expr::Bin(bin) => {
                if let Some(lhs) = bin.lhs() {
                    self.walk_expr(lhs);
                }
                if let Some(rhs) = bin.rhs() {
                    self.walk_expr(rhs);
                }
            }
            Expr::Select(select) => {
                if let Some(base) = select.base() {
                    self.walk_expr(base);
                }
                // Field names are not references, but interpolated
                // segments (a.${b}.c) ARE expression positions.
                self.walk_attrpath_interps(select.attrpath());
                if let Some(default) = select.default() {
                    self.walk_expr(default);
                }
            }
            Expr::HasAttr(has_attr) => {
                // `x ? a.b` — the right side is an attribute path, not
                // a reference, even though the parser represents it as
                // an expression.
                if let Some(base) = has_attr.base() {
                    self.walk_expr(base);
                }
            }
            Expr::String(string) => self.walk_string_parts(string.parts()),
            Expr::IndString(string) => self.walk_string_parts(string.parts()),
            Expr::Paren(paren) => {
                if let Some(inner) = paren.expr() {
                    self.walk_expr(inner);
                }
            }
            Expr::Int(_) | Expr::Float(_) | Expr::Path(_) | Expr::SearchPath(_) | Expr::Uri(_) => {}
        }
    }

    /// Walk the interpolated subexpressions of a string. Interpolations
    /// ARE expression positions; literal content is not.
    fn walk_string_parts(&mut self, parts: impl IntoIterator<Item = StringPart<'a>>) {
        for part in parts {
            if let StringPart::Interp(interp) = part {
                if let Some(inner) = interp.expr() {
                    self.walk_expr(inner);
                }
            }
        }
    }

    /// Interpolations inside (quoted or unquoted) attrpath segments are
    /// evaluated, so they are expression positions.
    fn walk_attrpath_interps(&mut self, path: Option<strictix_syntax::Attrpath<'a>>) {
        if let Some(p) = path {
            for element in p.elements() {
                if let AttrName::Interp(interp) = element {
                    if let Some(inner) = interp.expr() {
                        self.walk_expr(inner);
                    }
                }
            }
        }
    }
}
