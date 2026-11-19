use std::collections::HashMap;
use std::sync::OnceLock;

use strictix_syntax::{
    AstNode, AttrItem, AttrName, Expr, LambdaParam, Root, StringPart, SyntaxNode, SyntaxToken,
    TextRange,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingKind {
        LambdaParam,
        AtName,
        LetBinding,
        RecAttr,
        InheritName,
}

pub struct Binding<'a> {
        pub name: &'a SyntaxToken,
        pub scope: ScopeId,
        pub references: Vec<TextRange>,
        pub kind: BindingKind,
}

pub struct Reference<'a> {
        pub name: &'a SyntaxToken,
        pub resolved: Option<usize>,
        pub via_with: Option<usize>,
}

pub struct WithSite {
        pub scope_range: TextRange,
        pub body_range: TextRange,
}

pub struct ImportSite {
        pub call_range: TextRange,
        pub path_range: TextRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeKind {
    Root,
    Let,
    Lambda,
    Rec,
    With,
    Attrset,
}

struct ScopeIndex<'a> {
    heads: HashMap<&'a str, u32>,
    next: Vec<u32>,
}

struct Scope<'a> {
    kind: ScopeKind,
    parent: Option<ScopeId>,
    range: TextRange,
    bindings: Vec<usize>,
    visible_from: Vec<u32>,
    index: Option<ScopeIndex<'a>>,
}

pub struct Built<'a> {
    source: &'a str,
    scopes: Vec<Scope<'a>>,
    bindings: Vec<Binding<'a>>,
    references: Vec<Reference<'a>>,
    ref_by_start: Vec<(u32, u32)>,
    binding_by_start: Vec<(u32, u32)>,
    with_sites: Vec<WithSite>,
    import_sites: Vec<ImportSite>,
}

pub struct SemanticModel<'a> {
    source: &'a str,
    root: &'a SyntaxNode,
    path: Option<&'a std::path::Path>,
    built: OnceLock<Built<'a>>,
}

impl<'a> SemanticModel<'a> {
        pub fn new(source: &'a str, root: &'a SyntaxNode) -> Self {
        Self {
            source,
            root,
            path: None,
            built: OnceLock::new(),
        }
    }

        #[must_use]
    pub fn with_path(mut self, path: Option<&'a std::path::Path>) -> Self {
        self.path = path;
        self
    }

        #[must_use]
    pub fn path(&self) -> Option<&'a std::path::Path> {
        self.path
    }

        #[must_use]
    pub fn source(&self) -> &'a str {
        self.source
    }

        #[must_use]
    pub fn root(&self) -> &'a SyntaxNode {
        self.root
    }

        fn ensure_built(&self) -> &Built<'a> {
        self.built
            .get_or_init(|| Built::build(self.source, self.root))
    }

        #[must_use]
    pub fn bindings(&self) -> &[Binding<'a>] {
        &self.ensure_built().bindings
    }

        #[must_use]
    pub fn references(&self) -> &[Reference<'a>] {
        &self.ensure_built().references
    }

        #[must_use]
    pub fn with_sites(&self) -> &[WithSite] {
        &self.ensure_built().with_sites
    }

        #[must_use]
    pub fn import_sites(&self) -> &[ImportSite] {
        &self.ensure_built().import_sites
    }

        #[must_use]
    pub fn reference_at(&self, range: TextRange) -> Option<&Reference<'a>> {
        let built = self.ensure_built();
        let pos = built
            .ref_by_start
            .partition_point(|&(start, _)| start < range.start());
        for &(start, idx) in &built.ref_by_start[pos..] {
            if start != range.start() {
                break;
            }
            let r = &built.references[idx as usize];
            if r.name.range() == range {
                return Some(r);
            }
        }
        None
    }

    pub fn references_in_range_indexed(
        &self,
        range: TextRange,
    ) -> impl Iterator<Item = (usize, &Reference<'a>)> {
        let built = self.ensure_built();
        let lo = built
            .ref_by_start
            .partition_point(|&(start, _)| start < range.start());
        let hi = built.ref_by_start[lo..].partition_point(|&(start, _)| start < range.end()) + lo;
        built.ref_by_start[lo..hi]
            .iter()
            .map(|&(_, idx)| (idx as usize, &built.references[idx as usize]))
    }

    pub fn references_in_range(&self, range: TextRange) -> impl Iterator<Item = &Reference<'a>> {
        let built = self.ensure_built();
        let lo = built
            .ref_by_start
            .partition_point(|&(start, _)| start < range.start());
        let hi = built.ref_by_start[lo..].partition_point(|&(start, _)| start < range.end()) + lo;
        built.ref_by_start[lo..hi]
            .iter()
            .map(|&(_, idx)| &built.references[idx as usize])
    }

    pub fn bindings_in_range(&self, range: TextRange) -> impl Iterator<Item = &Binding<'a>> {
        let built = self.ensure_built();
        let lo = built
            .binding_by_start
            .partition_point(|&(start, _)| start < range.start());
        let hi =
            built.binding_by_start[lo..].partition_point(|&(start, _)| start < range.end()) + lo;
        built.binding_by_start[lo..hi]
            .iter()
            .map(|&(_, idx)| &built.bindings[idx as usize])
    }

    #[must_use]
    pub fn binding_at(&self, range: TextRange) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        let pos = built
            .binding_by_start
            .partition_point(|&(start, _)| start < range.start());
        for &(start, idx) in &built.binding_by_start[pos..] {
            if start != range.start() {
                break;
            }
            let b = &built.bindings[idx as usize];
            if b.name.range() == range {
                return Some(b);
            }
        }
        None
    }

    #[must_use]
    pub fn resolve(&self, token: &SyntaxToken) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        let idx = self.reference_at(token.range()).and_then(|r| r.resolved)?;
        Some(&built.bindings[idx])
    }

        #[must_use]
    pub fn resolve_lexical(&self, name: &str, offset: u32) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        built
            .resolve_lexical_index(name, offset)
            .map(|i| &built.bindings[i])
    }

        #[must_use]
    pub fn outer_shadow(&self, binding: &Binding<'a>) -> Option<&Binding<'a>> {
        let built = self.ensure_built();
        let offset = binding.name.range().start();
        let name = binding.name.text(built.source);
        let start_scope = built.scopes[binding.scope.0 as usize].parent?;
        built
            .lookup_from_scope(start_scope, name, offset)
            .map(|bi| &built.bindings[bi])
    }

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
    fn build(source: &'a str, root: &'a SyntaxNode) -> Self {
        let mut builder = Builder {
            source,
            scopes: Vec::new(),
            bindings: Vec::new(),
            with_sites: Vec::new(),
            import_sites: Vec::new(),
            idents: Vec::new(),
            stack: Vec::new(),
            with_stack: Vec::new(),
        };
        builder.new_scope(ScopeKind::Root, root.range());
        if let Some(expr) = Root::cast(root).and_then(|r| r.expr()) {
            builder.walk_expr(expr);
        }
        for scope in &mut builder.scopes {
            if scope.kind != ScopeKind::Attrset && scope.bindings.len() > 8 {
                let len = scope.bindings.len();
                let mut heads = HashMap::with_capacity(len);
                let mut next = vec![u32::MAX; len];
                for pos in (0..len).rev() {
                    let bi = scope.bindings[pos];
                    let name = builder.bindings[bi].name.text(source);
                    if let Some(prev) = heads.insert(name, pos as u32) {
                        next[pos] = prev;
                    }
                }
                scope.index = Some(ScopeIndex { heads, next });
            }
        }
        let mut binding_by_start: Vec<(u32, u32)> = builder
            .bindings
            .iter()
            .enumerate()
            .map(|(i, b)| (b.name.range().start(), i as u32))
            .collect();
        binding_by_start.sort_unstable_by_key(|&(start, idx)| (start, idx));
        let mut built = Self {
            source,
            scopes: builder.scopes,
            bindings: builder.bindings,
            references: Vec::with_capacity(builder.idents.len()),
            ref_by_start: Vec::with_capacity(builder.idents.len()),
            binding_by_start,
            with_sites: builder.with_sites,
            import_sites: builder.import_sites,
        };
        for (token, scope_id, via_with_cand) in builder.idents {
            built.record_reference(token, scope_id, via_with_cand);
        }
        built
            .ref_by_start
            .sort_unstable_by_key(|&(start, idx)| (start, idx));
        built
    }

    fn record_reference(
        &mut self,
        token: &'a SyntaxToken,
        scope_id: ScopeId,
        via_with_cand: Option<usize>,
    ) {
        let offset = token.range().start();
        let name = token.text(self.source);
        let resolved = self.lookup_from_scope(scope_id, name, offset);
        let via_with = if resolved.is_none() {
            via_with_cand
        } else {
            None
        };
        if let Some(bi) = resolved {
            self.bindings[bi].references.push(token.range());
        }
        let ref_idx = self.references.len() as u32;
        self.references.push(Reference {
            name: token,
            resolved,
            via_with,
        });
        self.ref_by_start.push((offset, ref_idx));
    }

        fn innermost_scope(&self, offset: u32) -> Option<ScopeId> {
        self.scopes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| s.range.contains(offset))
            .map(|(i, _)| ScopeId(i as u32))
    }

    fn resolve_lexical_index(&self, name: &str, offset: u32) -> Option<usize> {
        let scope_id = self.innermost_scope(offset)?;
        self.lookup_from_scope(scope_id, name, offset)
    }

    fn lookup_from_scope(&self, mut scope_id: ScopeId, name: &str, offset: u32) -> Option<usize> {
        loop {
            let scope = &self.scopes[scope_id.0 as usize];
            if scope.kind != ScopeKind::Attrset {
                if let Some(index) = &scope.index {
                    if let Some(&first_pos) = index.heads.get(name) {
                        let mut pos = first_pos;
                        while pos != u32::MAX {
                            let p = pos as usize;
                            if self.is_visible(scope_id, p, offset) {
                                return Some(scope.bindings[p]);
                            }
                            pos = index.next[p];
                        }
                    }
                } else {
                    for (pos, &bi) in scope.bindings.iter().enumerate() {
                        let binding = &self.bindings[bi];
                        if binding.name.text(self.source) == name
                            && self.is_visible(scope_id, pos, offset)
                        {
                            return Some(bi);
                        }
                    }
                }
            }
            scope_id = scope.parent?;
        }
    }

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

struct Builder<'a> {
    source: &'a str,
    scopes: Vec<Scope<'a>>,
    bindings: Vec<Binding<'a>>,
    with_sites: Vec<WithSite>,
    import_sites: Vec<ImportSite>,
    idents: Vec<(&'a SyntaxToken, ScopeId, Option<usize>)>,
    stack: Vec<ScopeId>,
    with_stack: Vec<usize>,
}

impl<'a> Builder<'a> {
    fn push_ident(&mut self, token: &'a SyntaxToken) {
        let scope_id = *self.stack.last().expect("root scope always active");
        self.push_ident_in(token, scope_id);
    }

    fn push_ident_in(&mut self, token: &'a SyntaxToken, scope_id: ScopeId) {
        let via_with = self.with_stack.last().copied();
        self.idents.push((token, scope_id, via_with));
    }

    fn new_scope(&mut self, kind: ScopeKind, range: TextRange) -> ScopeId {
        let parent = self.stack.last().copied();
        let id = ScopeId(self.scopes.len() as u32);
        self.scopes.push(Scope {
            kind,
            parent,
            range,
            bindings: Vec::new(),
            visible_from: Vec::new(),
            index: None,
        });
        self.stack.push(id);
        id
    }

        fn pop_scope(&mut self) {
        self.stack.pop();
    }

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

        fn binding_name(binding: &strictix_syntax::Binding<'a>) -> Option<&'a SyntaxToken> {
        match binding.attrpath()?.elements().next()? {
            AttrName::Ident(token) => Some(token),
            AttrName::Str(_) | AttrName::Interp(_) => None,
        }
    }

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

        fn walk_expr(&mut self, expr: Expr<'a>) {
        match expr {
            Expr::Ident(token) => self.push_ident(token),
            Expr::Let(let_expr) => {
                let parent_scope = *self.stack.last().expect("root scope always active");
                let scope = self.new_scope(ScopeKind::Let, let_expr.range());
                if let Some(bindings) = let_expr.bindings() {
                    for item in bindings.items() {
                        match item {
                            AttrItem::Binding(binding) => {
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
                                if let Some(ap) = binding.attrpath() {
                                    self.walk_attrpath_interps(Some(ap));
                                }
                            }
                            AttrItem::Inherit(inherit) => {
                                if let Some(source) = inherit.source() {
                                    self.walk_expr(source);
                                }
                                for name in inherit.names() {
                                    if inherit.source().is_none() {
                                        self.push_ident_in(name, parent_scope);
                                    }
                                    let visible_from = let_expr.range().start();
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
                        self.register(token, BindingKind::LambdaParam, scope, token.range().end());
                    }
                    LambdaParam::Formals(formals, at_name) => {
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
                if let Some(scope_expr) = with_expr.scope() {
                    self.walk_expr(scope_expr);
                }
                if let Some(body) = with_expr.body() {
                    let site_idx = self.with_sites.len();
                    self.with_sites.push(WithSite {
                        scope_range: with_expr.range(),
                        body_range: body.range(),
                    });
                    self.with_stack.push(site_idx);
                    self.new_scope(ScopeKind::With, body.range());
                    self.walk_expr(body);
                    self.pop_scope();
                    self.with_stack.pop();
                }
            }
            Expr::Attrset(attrset) => {
                let scope = self.new_scope(ScopeKind::Attrset, attrset.range());
                for item in attrset.items() {
                    match item {
                        AttrItem::Binding(binding) => {
                            if let Some(value) = binding.value() {
                                self.walk_expr(value);
                            }
                            if let Some(ap) = binding.attrpath() {
                                self.walk_attrpath_interps(Some(ap));
                            }
                        }
                        AttrItem::Inherit(inherit) => {
                            if let Some(source) = inherit.source() {
                                self.walk_expr(source);
                            }
                            for name in inherit.names() {
                                if inherit.source().is_none() {
                                    self.push_ident(name);
                                }
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
                    let parent_scope = *self.stack.last().expect("root scope always active");
                    let scope = self.new_scope(ScopeKind::Rec, attrset.range());
                    for item in attrset.items() {
                        match item {
                            AttrItem::Binding(binding) => {
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
                                if let Some(ap) = binding.attrpath() {
                                    self.walk_attrpath_interps(Some(ap));
                                }
                            }
                            AttrItem::Inherit(inherit) => {
                                if let Some(source) = inherit.source() {
                                    self.walk_expr(source);
                                }
                                for name in inherit.names() {
                                    if inherit.source().is_none() {
                                        self.push_ident_in(name, parent_scope);
                                    }
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
                self.walk_attrpath_interps(select.attrpath());
                if let Some(default) = select.default() {
                    self.walk_expr(default);
                }
            }
            Expr::HasAttr(has_attr) => {
                if let Some(base) = has_attr.base() {
                    self.walk_expr(base);
                }
                self.walk_attrpath_interps(has_attr.attrpath());
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

        fn walk_string_parts(&mut self, parts: impl IntoIterator<Item = StringPart<'a>>) {
        for part in parts {
            if let StringPart::Interp(interp) = part {
                if let Some(inner) = interp.expr() {
                    self.walk_expr(inner);
                }
            }
        }
    }

        fn walk_attrpath_interps(&mut self, path: Option<strictix_syntax::Attrpath<'a>>) {
        if let Some(p) = path {
            for element in p.elements() {
                match element {
                    AttrName::Interp(interp) => {
                        if let Some(inner) = interp.expr() {
                            self.walk_expr(inner);
                        }
                    }
                    AttrName::Str(string_expr) => {
                        self.walk_string_parts(string_expr.parts());
                    }
                    AttrName::Ident(_) => {}
                }
            }
        }
    }
}
