//! Style lints: mechanical anti-pattern rewrites ported from the statix
//! lint set, written fresh against strictix's own tree.
//!
//! Twelve are node rules (fire on one node kind). Three need parent or
//! token context — `useless-parens`, `repeated-keys`, `unquoted-uri` —
//! and are file rules that walk the tree themselves.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Binding, BinExpr, Expr, IfExpr, InheritStmt, LambdaExpr,
    LambdaParam, LetExpr, NodeOrToken, ParenExpr, SyntaxKind as K, SyntaxNode, SyntaxToken,
    TextRange,
};

// --- empty-let-in ---------------------------------------------------

pub struct EmptyLetIn;

impl Rule for EmptyLetIn {
    fn code(&self) -> &'static str { "empty-let-in" }
    fn name(&self) -> &'static str { "Empty let-in" }
    fn description(&self) -> &'static str {
        "Flags let-in expressions that create no bindings. They are useless remnants of debugging or editing; the body alone suffices."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::LetExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(let_expr) = LetExpr::cast(node) else { return };
        let Some(bindings) = let_expr.bindings() else { return };
        if bindings.items().count() != 0 { return; }
        let Some(body) = let_expr.body() else { return };
        let mut diag = Diagnostic::new(
            "empty-let-in", Severity::Warning, "useless let-in expression", node.content_range(),
        );
        let has_comments = node.child_tokens().any(|t| t.kind() == K::Comment);
        if !has_comments {
            diag = diag.with_fix(Fix::new("remove empty let").edit(node.content_range(), body.text(source)));
        }
        diags.push(diag);
    }
}

// --- manual-inherit -------------------------------------------------

pub struct ManualInherit;

impl Rule for ManualInherit {
    fn code(&self) -> &'static str { "manual-inherit" }
    fn name(&self) -> &'static str { "Assignment instead of inherit" }
    fn description(&self) -> &'static str {
        "Flags bindings of the form `a = a;`. Prefer `inherit a;` to bring a name from the outer scope."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::Binding) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binding) = Binding::cast(node) else { return };
        let Some(attrpath) = binding.attrpath() else { return };
        let mut elems = attrpath.elements();
        let Some(AttrName::Ident(key)) = elems.next() else { return };
        if elems.next().is_some() { return; }
        let Some(Expr::Ident(value)) = binding.value() else { return };
        if key.text(source) != value.text(source) { return; }
        let fix = Fix::new("use inherit").edit(node.content_range(), format!("inherit {};", key.text(source)));
        diags.push(Diagnostic::new(
            "manual-inherit", Severity::Warning, "assignment instead of inherit", node.content_range(),
        ).with_fix(fix));
    }
}

// --- manual-inherit-from --------------------------------------------

pub struct ManualInheritFrom;

impl Rule for ManualInheritFrom {
    fn code(&self) -> &'static str { "manual-inherit-from" }
    fn name(&self) -> &'static str { "Assignment instead of inherit from" }
    fn description(&self) -> &'static str {
        "Flags bindings of the form `a = some.a;`. Prefer `inherit (some) a;`."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::Binding) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binding) = Binding::cast(node) else { return };
        let Some(attrpath) = binding.attrpath() else { return };
        let mut elems = attrpath.elements();
        let Some(AttrName::Ident(key)) = elems.next() else { return };
        if elems.next().is_some() { return; }
        let Some(Expr::Select(select)) = binding.value() else { return };
        // A select with an `or` default (`x = a.b or default`) cannot be
        // rewritten via inherit without dropping the fallback, so bail out.
        if select.default().is_some() { return; }
        let Some(select_attrpath) = select.attrpath() else { return };
        let Some(base) = select.base() else { return };

        // Walk the selected attribute path. It must be composed entirely
        // of static idents (no `${...}` interpolation, no quoted segment)
        // so that `dev = cfg.devices.dev` -> `inherit (cfg.devices) dev;`
        // is a sound rewrite. A dynamic select like `cfg.devices.${name}`
        // is NOT a static attribute access and must not be flagged.
        let mut elements = select_attrpath.elements();
        let mut index: Option<&SyntaxToken> = None;
        let mut from_suffix = String::new();
        while let Some(element) = elements.next() {
            match element {
                AttrName::Ident(token) => {
                    if let Some(prev) = index {
                        if !from_suffix.is_empty() {
                            from_suffix.push('.');
                        }
                        from_suffix.push_str(prev.text(source));
                    }
                    index = Some(token);
                }
                // Dynamic or quoted segment: not a static `some.path`
                // access, so this binding cannot be rewritten via inherit.
                AttrName::Str(_) | AttrName::Interp(_) => return,
            }
        }
        let Some(index) = index else { return };
        if key.text(source) != index.text(source) { return; }

        // The `from` expression is the base + every path element except
        // the final one: `cfg.devices.dev` -> `inherit (cfg.devices) dev;`
        // and `some.a` -> `inherit (some) a;`.
        let from = if from_suffix.is_empty() {
            base.text(source).to_string()
        } else {
            format!("{}.{}", base.text(source), from_suffix)
        };
        let fix = Fix::new("use inherit").edit(
            node.content_range(),
            format!("inherit ({}) {};", from, key.text(source)),
        );
        diags.push(Diagnostic::new(
            "manual-inherit-from", Severity::Warning, "assignment instead of inherit from", node.content_range(),
        ).with_fix(fix));
    }
}

// --- collapsible-let-in ---------------------------------------------

pub struct CollapsibleLetIn;

impl Rule for CollapsibleLetIn {
    fn code(&self) -> &'static str { "collapsible-let-in" }
    fn name(&self) -> &'static str { "Collapsible let-in" }
    fn description(&self) -> &'static str {
        "Flags a let-in whose body is another let-in. The two binding sections can be merged by deleting the `in ... let` between them."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::LetExpr) }
    fn check_node(&self, node: &SyntaxNode, _source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(let_expr) = LetExpr::cast(node) else { return };
        let Some(Expr::Let(inner)) = let_expr.body() else { return };
        let Some(in_token) = node.child_tokens().find(|t| t.kind() == K::KwIn) else { return };
        let Some(let_token) = inner.syntax().child_tokens().find(|t| t.kind() == K::KwLet) else { return };
        let range = TextRange::new(in_token.range().start(), let_token.range().end());
        let fix = Fix::new("collapse let-in").edit(range, "");
        diags.push(Diagnostic::new(
            "collapsible-let-in", Severity::Warning, "these let-in expressions are collapsible", node.content_range(),
        ).with_fix(fix));
    }
}

// --- eta-reduction --------------------------------------------------

pub struct EtaReduction;

impl Rule for EtaReduction {
    fn code(&self) -> &'static str { "eta-reduction" }
    fn name(&self) -> &'static str { "Eta-reducible function" }
    fn description(&self) -> &'static str {
        "Flags lambdas of the form `x: f x` where `f` is a bare name not mentioning `x`. The lambda can be replaced with `f`."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::LambdaExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(lambda) = LambdaExpr::cast(node) else { return };
        let LambdaParam::Ident(param) = lambda.param() else { return };
        let Some(Expr::Apply(apply)) = lambda.body() else { return };
        let Some(Expr::Ident(arg)) = apply.arg() else { return };
        if arg.text(source) != param.text(source) { return; }
        let Some(Expr::Ident(func)) = apply.func() else { return };
        if func.text(source) == param.text(source) { return; }
        let fix = Fix::new("eta-reduce").edit(node.content_range(), func.text(source));
        diags.push(Diagnostic::new(
            "eta-reduction", Severity::Warning, "this function is eta-reducible", node.content_range(),
        ).with_fix(fix));
    }
}

// --- empty-pattern --------------------------------------------------

pub struct EmptyPattern;

impl Rule for EmptyPattern {
    fn code(&self) -> &'static str { "empty-pattern" }
    fn name(&self) -> &'static str { "Empty pattern" }
    fn description(&self) -> &'static str {
        "Flags `{ ... }: body` — a variadic pattern that binds nothing. Prefer `_` to signal the argument is ignored."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::LambdaExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(lambda) = LambdaExpr::cast(node) else { return };
        let LambdaParam::Formals(formals, at_name) = lambda.param() else { return };
        if formals.params().count() != 0 { return; }
        if !formals.has_ellipsis() { return; }
        if at_name.is_some() { return; }
        if is_module(lambda.body(), source) { return; }
        let fix = Fix::new("use `_`").edit(formals.syntax().content_range(), "_");
        diags.push(Diagnostic::new(
            "empty-pattern", Severity::Warning, "empty pattern in function argument", node.content_range(),
        ).with_fix(fix));
    }
}

fn is_module(body: Option<Expr<'_>>, source: &str) -> bool {
    let Some(Expr::Attrset(attrset)) = body else { return false };
    attrset.items().any(|item| {
        let AttrItem::Binding(binding) = item else { return false };
        let Some(ap) = binding.attrpath() else { return false };
        matches!(ap.elements().next(), Some(AttrName::Ident(t)) if t.text(source) == "imports")
    })
}

// --- redundant-pattern-bind -----------------------------------------

pub struct RedundantPatternBind;

impl Rule for RedundantPatternBind {
    fn code(&self) -> &'static str { "redundant-pattern-bind" }
    fn name(&self) -> &'static str { "Redundant pattern bind" }
    fn description(&self) -> &'static str {
        "Flags `args @ { ... }: body` — the variadic pattern captures nothing, so the bind can be just `args`."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::LambdaExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(lambda) = LambdaExpr::cast(node) else { return };
        let LambdaParam::Formals(formals, at_name) = lambda.param() else { return };
        if formals.params().count() != 0 { return; }
        if !formals.has_ellipsis() { return; }
        let Some(at) = at_name else { return };
        let start = at.range().start().min(formals.syntax().content_range().start());
        let end = at.range().end().max(formals.syntax().content_range().end());
        let fix = Fix::new("remove redundant pattern").edit(TextRange::new(start, end), at.text(source));
        diags.push(Diagnostic::new(
            "redundant-pattern-bind", Severity::Warning, "redundant pattern bind in function argument", node.content_range(),
        ).with_fix(fix));
    }
}

// --- empty-inherit --------------------------------------------------

pub struct EmptyInherit;

impl Rule for EmptyInherit {
    fn code(&self) -> &'static str { "empty-inherit" }
    fn name(&self) -> &'static str { "Empty inherit" }
    fn description(&self) -> &'static str {
        "Flags `inherit;` statements that bring nothing into scope. Useless code, probably a refactor remnant."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::InheritStmt) }
    fn check_node(&self, node: &SyntaxNode, _source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(inherit) = InheritStmt::cast(node) else { return };
        if inherit.source().is_some() { return; }
        if inherit.names().count() != 0 { return; }
        let fix = Fix::new("remove empty inherit").edit(node.range(), "");
        diags.push(Diagnostic::new(
            "empty-inherit", Severity::Warning, "empty inherit statement", node.content_range(),
        ).with_fix(fix));
    }
}

// --- deprecated-to-path ---------------------------------------------

pub struct DeprecatedToPath;

impl Rule for DeprecatedToPath {
    fn code(&self) -> &'static str { "deprecated-to-path" }
    fn name(&self) -> &'static str { "Deprecated toPath" }
    fn description(&self) -> &'static str {
        "Flags use of the deprecated `toPath` builtin. Prefer `/. + str` for absolute paths or `./. + str` for relative ones."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::ApplyExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(apply) = strictix_syntax::ApplyExpr::cast(node) else { return };
        let Some(func) = apply.func() else { return };
        let func_text = func.text(source);
        if func_text != "toPath" && func_text != "builtins.toPath" { return; }
        diags.push(Diagnostic::new(
            "deprecated-to-path", Severity::Warning,
            format!("`{func_text}` is deprecated; use `/. +` or `./. +`"),
            node.content_range(),
        ));
    }
}

// --- useless-has-attr -----------------------------------------------

pub struct UselessHasAttr;

impl Rule for UselessHasAttr {
    fn code(&self) -> &'static str { "useless-has-attr" }
    fn name(&self) -> &'static str { "Useless has-attr" }
    fn description(&self) -> &'static str {
        "Flags `if x ? a then x.a else d`. The `or` operator is more readable: `x.a or d`."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::IfExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(if_expr) = IfExpr::cast(node) else { return };
        let Some(Expr::HasAttr(has_attr)) = if_expr.cond() else { return };
        let Some(set) = has_attr.base() else { return };
        let Some(attrpath) = has_attr.attrpath() else { return };
        let Some(then_branch) = if_expr.then_branch() else { return };
        if !matches!(then_branch, Expr::Select(_)) { return; }
        let expected = format!("{}.{}", set.text(source), attrpath.text(source));
        // The select node's range is prefixed with the trivia (space)
        // separating `then` from its branch, so compare trimmed.
        if then_branch.text(source).trim() != expected { return; }
        let Some(default_expr) = if_expr.else_branch() else { return };
        let default_text = default_expr.text(source);
        let default = if is_primary(default_expr) {
            default_text.to_string()
        } else {
            format!("({default_text})")
        };
        let replacement = format!("{expected} or {default}");
        let fix = Fix::new("use `or`").edit(node.content_range(), replacement);
        diags.push(Diagnostic::new(
            "useless-has-attr", Severity::Warning, "this if-expression can be simplified with `or`", node.content_range(),
        ).with_fix(fix));
    }
}

fn is_primary(expr: Expr<'_>) -> bool {
    matches!(
        expr,
        Expr::Ident(_) | Expr::Int(_) | Expr::Float(_) | Expr::Path(_)
            | Expr::SearchPath(_) | Expr::Uri(_) | Expr::String(_) | Expr::IndString(_)
            | Expr::List(_) | Expr::Attrset(_) | Expr::RecAttrset(_) | Expr::Paren(_)
            | Expr::Select(_)
    )
}

// --- empty-list-concat ----------------------------------------------

pub struct EmptyListConcat;

impl Rule for EmptyListConcat {
    fn code(&self) -> &'static str { "empty-list-concat" }
    fn name(&self) -> &'static str { "Unnecessary empty-list concat" }
    fn description(&self) -> &'static str {
        "Flags concatenation with the empty list, a no-op: `[] ++ x` is just `x`."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn node_kind(&self) -> Option<K> { Some(K::BinExpr) }
    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(bin) = BinExpr::cast(node) else { return };
        if bin.op() != Some(K::PlusPlus) { return; }
        let (Some(lhs), Some(rhs)) = (bin.lhs(), bin.rhs()) else { return };
        let survivor = if is_empty_list(lhs) {
            rhs
        } else if is_empty_list(rhs) {
            lhs
        } else {
            return;
        };
        let fix = Fix::new("remove empty-list concat").edit(node.content_range(), survivor.text(source));
        diags.push(Diagnostic::new(
            "empty-list-concat", Severity::Warning, "concatenation with the empty list is a no-op", node.content_range(),
        ).with_fix(fix));
    }
}

fn is_empty_list(expr: Expr<'_>) -> bool {
    matches!(expr, Expr::List(l) if l.items().count() == 0)
}

// --- useless-parens (file rule) -------------------------------------

pub struct UselessParens;

impl Rule for UselessParens {
    fn code(&self) -> &'static str { "useless-parens" }
    fn name(&self) -> &'static str { "Useless parentheses" }
    fn description(&self) -> &'static str {
        "Flags parentheses that can be omitted: around a binding value, a let body, or a primitive expression."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let root = model.root();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(root, &mut ancestors, &mut |node, ancestors| {
            match node.kind() {
                K::Binding => {
                    let Some(binding) = Binding::cast(node) else { return };
                    if let Some(Expr::Paren(paren)) = binding.value() {
                        if let Some(inner) = paren.expr() {
                            push_paren_diag(diags, paren_trimmed_range(paren), inner.text(source),
                                "useless parentheses around value in binding");
                        }
                    }
                }
                K::LetExpr => {
                    let Some(let_expr) = LetExpr::cast(node) else { return };
                    if let Some(Expr::Paren(paren)) = let_expr.body() {
                        if let Some(inner) = paren.expr() {
                            push_paren_diag(diags, paren_trimmed_range(paren), inner.text(source),
                                "useless parentheses around body of let");
                        }
                    }
                }
                K::ParenExpr => {
                    if let Some(parent) = ancestors.last() {
                        if parent.kind() == K::Binding || parent.kind() == K::LetExpr { return; }
                    }
                    let Some(paren) = ParenExpr::cast(node) else { return };
                    let Some(inner) = paren.expr() else { return };
                    if !is_primitive(inner) { return; }
                    push_paren_diag(diags, paren_trimmed_range(paren), inner.text(source),
                        "useless parentheses around primitive expression");
                }
                _ => {}
            }
        });
    }
}

fn push_paren_diag(diags: &mut Vec<Diagnostic>, range: TextRange, inner: &str, message: &str) {
    diags.push(Diagnostic::new("useless-parens", Severity::Warning, message, range)
        .with_fix(Fix::new("remove parentheses").edit(range, inner)));
}

/// The `(`...`)` span of a paren, excluding any leading trivia. The
/// parser flushes leading whitespace into the node, so `paren.range()`
/// can include a space that must be preserved (removing it would merge
/// `in` + `2` into `in2`).
fn paren_trimmed_range(paren: ParenExpr) -> TextRange {
    let node = paren.syntax();
    let start = node
        .child_tokens()
        .find(|t| t.kind() == K::LParen)
        .map(|t| t.range().start())
        .unwrap_or_else(|| paren.range().start());
    let end = node
        .child_tokens()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .find(|t| t.kind() == K::RParen)
        .map(|t| t.range().end())
        .unwrap_or_else(|| paren.range().end());
    TextRange::new(start, end)
}

fn is_primitive(expr: Expr<'_>) -> bool {
    match expr {
        Expr::List(_) | Expr::Paren(_) | Expr::String(_) | Expr::Attrset(_) | Expr::Ident(_) => true,
        Expr::Select(select) => select.default().is_none(),
        _ => false,
    }
}

// --- repeated-keys (file rule) --------------------------------------

pub struct RepeatedKeys;

impl Rule for RepeatedKeys {
    fn code(&self) -> &'static str { "repeated-keys" }
    fn name(&self) -> &'static str { "Repeated keys" }
    fn description(&self) -> &'static str {
        "Flags attrsets where the same first key component is repeated across three or more bindings; nesting is clearer."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let root = model.root();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(root, &mut ancestors, &mut |node, ancestors| {
            if node.kind() != K::Binding { return; }
            let Some(binding) = Binding::cast(node) else { return };
            let Some(attrpath) = binding.attrpath() else { return };
            let mut elems = attrpath.elements();
            let Some(AttrName::Ident(first)) = elems.next() else { return };
            if elems.next().is_none() { return; }
            let Some(parent) = ancestors.last() else { return };
            if parent.kind() != K::AttrsetExpr { return; }
            if ancestors.len() >= 2 && ancestors[ancestors.len() - 2].kind() == K::RecAttrsetExpr { return; }
            let name = first.text(source);
            let attrset = strictix_syntax::AttrsetExpr::cast(parent).expect("kind checked");
            let mut count = 0usize;
            let mut first_range: Option<TextRange> = None;
            for item in attrset.items() {
                if let AttrItem::Binding(b) = item {
                    if let Some(ap) = b.attrpath() {
                        if let Some(AttrName::Ident(id)) = ap.elements().next() {
                            if id.text(source) == name {
                                count += 1;
                                if first_range.is_none() { first_range = Some(b.range()); }
                            }
                        }
                    }
                }
            }
            if count < 3 { return; }
            if first_range != Some(node.range()) { return; }
            diags.push(Diagnostic::new(
                "repeated-keys", Severity::Warning,
                format!("key `{name}` is repeated {count} times; consider nesting"),
                node.content_range(),
            ));
        });
    }
}

// --- unquoted-uri (file rule) ---------------------------------------

pub struct UnquotedUri;

impl Rule for UnquotedUri {
    fn code(&self) -> &'static str { "unquoted-uri" }
    fn name(&self) -> &'static str { "Unquoted URI" }
    fn description(&self) -> &'static str {
        "Flags URI expressions that are not quoted. Nix URLs have no special properties; always quote them as strings (RFC 0045)."
    }
    fn severity(&self) -> Severity { Severity::Warning }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let root = model.root();
        walk_tokens(root, &mut |token| {
            if token.kind() != K::Uri { return; }
            let text = token.text(source);
            let fix = Fix::new("quote URI").edit(token.range(), format!("\"{text}\""));
            diags.push(Diagnostic::new(
                "unquoted-uri", Severity::Warning, "unquoted URI expression", token.range(),
            ).with_fix(fix));
        });
    }
}

// --- walkers ---------------------------------------------------------

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

fn walk_tokens<'a>(node: &'a SyntaxNode, f: &mut impl FnMut(&'a SyntaxToken)) {
    for child in node.children() {
        match child {
            NodeOrToken::Token(t) => f(t),
            NodeOrToken::Node(n) => walk_tokens(n, f),
        }
    }
}
