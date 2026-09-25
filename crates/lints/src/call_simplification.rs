//! Call-form simplification lints: builtins with operator equivalents,
//! deprecated predicates, and `lib.optional*` opportunities.
//!
//! All five rules are FILE rules even though each matches a single
//! node: they must prove that the callee really is the global builtin
//! (or nixpkgs `lib`) it looks like, and only the [SemanticModel] can
//! rule out a local shadow or a `with`-provided lookalike. When the
//! model cannot prove the callee's identity, the rule stays silent.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::{BindingKind, SemanticModel},
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, AttrName, Expr, IfExpr, LambdaParam, StringExpr, StringPart,
    SyntaxKind, SyntaxNode, TextRange,
};

use SyntaxKind as K;

/// Nix keywords. Real Nix rejects these as *bare* attrpath elements
/// (`x ? if` is a parse error upstream even though our tolerant parser
/// accepts keyword attr names), so a key colliding with one stays
/// quoted in generated fixes.
const KEYWORDS: &[&str] = &[
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];

/// Whether `text` can stand bare as an attrpath element: a valid Nix
/// identifier (`[a-zA-Z_][a-zA-Z0-9_'-]*`) that is not a keyword.
/// Anything else — empty, escaped (escapes contain `\`), spaced,
/// keyword — keeps its original quotes.
fn is_bare_key(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-')) {
        return false;
    }
    !KEYWORDS.contains(&text)
}

/// Depth-first walk with an ancestor stack, so a rule can inspect the
/// slot its match sits in. Local copy of the style_rules walker —
/// small-helper duplication across rule files is the house convention.
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

/// The trivia-trimmed range of an expression. The parser flushes
/// leading trivia into node ranges, so splicing `expr.range()` text
/// would drag whitespace or comments along; token atoms carry no
/// trivia and node wrappers expose `content_range`.
fn trimmed_range(expr: Expr<'_>) -> TextRange {
    match expr {
        Expr::Ident(t)
        | Expr::Int(t)
        | Expr::Float(t)
        | Expr::Path(t)
        | Expr::SearchPath(t)
        | Expr::Uri(t) => t.range(),
        Expr::Let(e) => e.syntax().content_range(),
        Expr::With(e) => e.syntax().content_range(),
        Expr::Assert(e) => e.syntax().content_range(),
        Expr::If(e) => e.syntax().content_range(),
        Expr::Attrset(e) => e.syntax().content_range(),
        Expr::RecAttrset(e) => e.syntax().content_range(),
        Expr::List(e) => e.syntax().content_range(),
        Expr::Lambda(e) => e.syntax().content_range(),
        Expr::Apply(e) => e.syntax().content_range(),
        Expr::Unary(e) => e.syntax().content_range(),
        Expr::Bin(e) => e.syntax().content_range(),
        Expr::Select(e) => e.syntax().content_range(),
        Expr::HasAttr(e) => e.syntax().content_range(),
        Expr::String(e) => e.syntax().content_range(),
        Expr::IndString(e) => e.syntax().content_range(),
        Expr::Paren(e) => e.syntax().content_range(),
    }
}

/// The trivia-trimmed source text of an expression.
fn trimmed_text<'s>(expr: Expr<'_>, source: &'s str) -> &'s str {
    let r = trimmed_range(expr);
    &source[r.start() as usize..r.end() as usize]
}

/// Whether an expression's text is self-delimiting: it can be spliced
/// next to other tokens without parens changing how it groups. A
/// select with an `or` default is excluded — the default clause can
/// capture a following token.
fn is_self_delimiting(expr: Expr<'_>) -> bool {
    match expr {
        Expr::Ident(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_)
        | Expr::String(_)
        | Expr::IndString(_)
        | Expr::List(_)
        | Expr::Attrset(_)
        | Expr::RecAttrset(_)
        | Expr::Paren(_) => true,
        Expr::Select(s) => s.default().is_none(),
        _ => false,
    }
}

/// `expr` rendered for an operand slot of `?` or `==`: both bind
/// looser than application, so an apply may stand bare; every other
/// compound expression is parenthesized.
fn operand_text(expr: Expr<'_>, source: &str) -> String {
    let text = trimmed_text(expr, source);
    if is_self_delimiting(expr) || matches!(expr, Expr::Apply(_)) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

/// `expr` rendered for an application-argument slot: application binds
/// tighter than everything, so only self-delimiting expressions stand
/// bare — notably another apply must be parenthesized.
fn argument_text(expr: Expr<'_>, source: &str) -> String {
    let text = trimmed_text(expr, source);
    if is_self_delimiting(expr) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

/// Whether a replacement expression must be parenthesized because the
/// replaced node sits in a slot binding tighter than the replacement's
/// own operator (`?`, `==`, or application). Extra parens are
/// semantically inert, so this errs on wrapping under any binary
/// operator rather than modelling exact precedence.
fn parent_requires_parens(ancestors: &[&SyntaxNode]) -> bool {
    ancestors.last().is_some_and(|p| {
        matches!(
            p.kind(),
            K::ApplyExpr | K::SelectExpr | K::HasAttrExpr | K::UnaryExpr | K::BinExpr
        )
    })
}

/// Whether `callee` provably denotes the global builtin `name`.
///
/// Bare form: the ident resolves to nothing lexically AND no enclosing
/// `with` could supply it — statically we cannot know what a with's
/// attrset provides, so `via_with` counts as a shadow. Select form: a
/// single-hop `builtins.<name>` on the unshadowed `builtins` constant
/// with no `or` default (same shape as UnknownBuiltin's check).
fn is_global_callee(model: &SemanticModel<'_>, callee: Expr<'_>, name: &str) -> bool {
    let source = model.source();
    match callee {
        Expr::Ident(tok) => {
            if tok.text(source) != name {
                return false;
            }
            model
                .references()
                .iter()
                .find(|r| r.name.range() == tok.range())
                .is_some_and(|r| r.resolved.is_none() && r.via_with.is_none())
        }
        Expr::Select(sel) => {
            let Some(Expr::Ident(base)) = sel.base() else {
                return false;
            };
            if base.text(source) != "builtins" || model.resolve(base).is_some() {
                return false;
            }
            if sel.default().is_some() {
                return false;
            }
            let Some(attrpath) = sel.attrpath() else {
                return false;
            };
            let mut elements = attrpath.elements();
            let Some(AttrName::Ident(attr)) = elements.next() else {
                return false;
            };
            if elements.next().is_some() {
                return false;
            }
            attr.text(source) == name
        }
        _ => false,
    }
}

/// Matches `node` as exactly the two-argument call `name first second`
/// of the global builtin `name` (bare or `builtins.`-qualified).
/// Partial applications never match — they have no nested apply — and
/// parser-recovery applies missing an operand fail the range guards.
fn two_arg_call<'a>(
    model: &SemanticModel<'a>,
    node: &'a SyntaxNode,
    name: &str,
) -> Option<(Expr<'a>, Expr<'a>)> {
    let outer = ApplyExpr::cast(node)?;
    let Expr::Apply(inner) = outer.func()? else {
        return None;
    };
    let callee = inner.func()?;
    if !is_global_callee(model, callee, name) {
        return None;
    }
    let first = inner.arg()?;
    if first.range() == callee.range() {
        return None;
    }
    let second = outer.arg()?;
    if second.range() == inner.range() {
        return None;
    }
    Some((first, second))
}

/// Matches `node` as exactly the one-argument call `name arg` of the
/// global builtin `name` (bare or `builtins.`-qualified).
fn one_arg_call<'a>(
    model: &SemanticModel<'a>,
    node: &'a SyntaxNode,
    name: &str,
) -> Option<Expr<'a>> {
    let apply = ApplyExpr::cast(node)?;
    let callee = apply.func()?;
    if !is_global_callee(model, callee, name) {
        return None;
    }
    let arg = apply.arg()?;
    if arg.range() == callee.range() {
        return None;
    }
    Some(arg)
}

/// The raw inner text of a plain (non-interpolated) string literal —
/// `None` when any part interpolates, because the key is then not
/// statically known. Escapes are left as written: an escape sequence
/// contains `\`, which fails [is_bare_key], so escaped keys always
/// stay quoted verbatim.
fn plain_string_inner<'s>(string: StringExpr<'_>, source: &'s str) -> Option<&'s str> {
    if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
        return None;
    }
    let range = string.syntax().content_range();
    let text = &source[range.start() as usize..range.end() as usize];
    text.strip_prefix('"')?.strip_suffix('"')
}

/// Strip one paren layer from an expression. The manual-optional
/// shapes are often written with parenthesized branches; one layer is
/// enough to prove the shape without chasing arbitrary nesting.
fn unwrap_paren(expr: Expr<'_>) -> Expr<'_> {
    match expr {
        Expr::Paren(p) => p.expr().unwrap_or(expr),
        _ => expr,
    }
}

/// Flags `builtins.hasAttr "a" x` — the `?` operator form is simpler.
pub struct ManualHasattr;

impl Rule for ManualHasattr {
    fn code(&self) -> &'static str {
        "manual-hasattr"
    }

    fn name(&self) -> &'static str {
        "Manual hasAttr"
    }

    fn description(&self) -> &'static str {
        "Flags `builtins.hasAttr \"name\" x` with a static name; the operator form `x ? name` says the same thing shorter."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            let Some((name_arg, set_arg)) = two_arg_call(model, node, "hasAttr") else {
                return;
            };
            let Expr::String(name_string) = name_arg else {
                return;
            };
            let Some(inner) = plain_string_inner(name_string, source) else {
                return;
            };
            let key = if is_bare_key(inner) {
                inner
            } else {
                trimmed_text(name_arg, source)
            };
            let mut replacement = format!("{} ? {key}", operand_text(set_arg, source));
            if parent_requires_parens(ancestors) {
                replacement = format!("({replacement})");
            }
            let range = node.content_range();
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "hasAttr with the static name {} can be the `?` operator",
                        trimmed_text(name_arg, source)
                    ),
                    range,
                )
                .with_help(format!("write `{replacement}`"))
                .with_fix(
                    Fix::new("replace hasAttr call with the ? operator").edit(range, replacement),
                ),
            );
        });
    }
}

/// Flags `builtins.getAttr "a" x` — the select form is simpler.
pub struct ManualGetattr;

impl Rule for ManualGetattr {
    fn code(&self) -> &'static str {
        "manual-getattr"
    }

    fn name(&self) -> &'static str {
        "Manual getAttr"
    }

    fn description(&self) -> &'static str {
        "Flags `builtins.getAttr \"name\" x` with a static name; the select form `x.name` says the same thing shorter."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, _ancestors| {
            let Some((name_arg, set_arg)) = two_arg_call(model, node, "getAttr") else {
                return;
            };
            let Expr::String(name_string) = name_arg else {
                return;
            };
            let Some(inner) = plain_string_inner(name_string, source) else {
                return;
            };
            let key = if is_bare_key(inner) {
                inner
            } else {
                trimmed_text(name_arg, source)
            };
            // A select binds tighter than every slot an apply can occupy,
            // so the replacement never needs an outer paren layer.
            let replacement = format!("{}.{key}", operand_text(set_arg, source));
            let range = node.content_range();
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "getAttr with the static name {} can be a select",
                        trimmed_text(name_arg, source)
                    ),
                    range,
                )
                .with_help(format!("write `{replacement}`"))
                .with_fix(Fix::new("replace getAttr call with a select").edit(range, replacement)),
            );
        });
    }
}

/// Flags the deprecated `isNull x` — `x == null` is the modern form.
pub struct DeprecatedIsNull;

impl Rule for DeprecatedIsNull {
    fn code(&self) -> &'static str {
        "deprecated-is-null"
    }

    fn name(&self) -> &'static str {
        "Deprecated isNull"
    }

    fn description(&self) -> &'static str {
        "Flags `isNull x` and `builtins.isNull x`; the builtin is deprecated in favour of the plain comparison `x == null`."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            let Some(arg) = one_arg_call(model, node, "isNull") else {
                return;
            };
            let mut replacement = format!("{} == null", operand_text(arg, source));
            if parent_requires_parens(ancestors) {
                replacement = format!("({replacement})");
            }
            let range = node.content_range();
            diags.push(
                Diagnostic::new(self.code(), self.severity(), "isNull is deprecated", range)
                    .with_help(format!(
                        "isNull has been deprecated since Nix 2.0; write `{replacement}` instead"
                    ))
                    .with_fix(Fix::new("replace isNull with == null").edit(range, replacement)),
            );
        });
    }
}

/// Flags `if c then [ x ] else [ ]`-shaped conditionals where a
/// `lib.optional*` helper is in scope.
pub struct ManualOptional;

impl Rule for ManualOptional {
    fn code(&self) -> &'static str {
        "manual-optional"
    }

    fn name(&self) -> &'static str {
        "Manual optional"
    }

    fn description(&self) -> &'static str {
        "Flags `if c then [ x ] else [ ]`, `if c then s else \"\"`, and `if c then a else { }` when `lib` is in scope; `lib.optional`, `lib.optionalString`, and `lib.optionalAttrs` say the same thing shorter."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            if node.kind() != K::IfExpr {
                return;
            }
            let Some(ifx) = IfExpr::cast(node) else {
                return;
            };
            let (Some(cond), Some(then_e), Some(else_e)) =
                (ifx.cond(), ifx.then_branch(), ifx.else_branch())
            else {
                return;
            };
            // Suggesting `lib.optional` only makes sense when `lib` is
            // actually visible here; otherwise the rewrite would not
            // even evaluate, so the rule stays entirely silent.
            if !model.is_bound("lib", node.content_range().start()) {
                return;
            }
            // A literal `true`/`false` condition is constant-if's
            // finding (and its fix), never ours — overlapping fixes
            // would make the engine reject both.
            if let Expr::Ident(t) = unwrap_paren(cond) {
                if matches!(t.text(source), "true" | "false") {
                    return;
                }
            }
            let then_u = unwrap_paren(then_e);
            let else_u = unwrap_paren(else_e);
            let (helper, arg) = match (then_u, else_u) {
                // `if c then [ x ] else [ ]` -> lib.optional c x
                // `if c then [ x y ] else [ ]` -> lib.optionals c [ x y ]
                (Expr::List(then_list), Expr::List(else_list))
                    if else_list.items().next().is_none() =>
                {
                    let mut items = then_list.items();
                    let Some(first) = items.next() else {
                        return;
                    };
                    if items.next().is_none() {
                        ("optional", argument_text(first, source))
                    } else {
                        ("optionals", argument_text(then_u, source))
                    }
                }
                // `if c then s else ""` -> lib.optionalString c s
                (Expr::String(_) | Expr::IndString(_), Expr::String(else_string))
                    if else_string.parts().next().is_none() =>
                {
                    ("optionalString", argument_text(then_u, source))
                }
                // `if c then { ... } else { }` -> lib.optionalAttrs c { ... }
                (Expr::Attrset(_), Expr::Attrset(else_set))
                    if else_set.items().next().is_none() =>
                {
                    ("optionalAttrs", argument_text(then_u, source))
                }
                _ => return,
            };
            let mut replacement = format!("lib.{helper} {} {arg}", argument_text(cond, source));
            if parent_requires_parens(ancestors) {
                replacement = format!("({replacement})");
            }
            let range = node.content_range();
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("this conditional is lib.{helper}"),
                    range,
                )
                .with_help(format!("write `{replacement}`"))
                .with_fix(Fix::new(format!("rewrite with lib.{helper}")).edit(range, replacement)),
            );
        });
    }
}

/// The `optional` token range when `callee` provably denotes nixpkgs
/// `lib.optional`.
///
/// Select form: single-hop `lib.optional` whose `lib` base resolves to
/// a formal/lambda parameter (the module-argument idiom) or to nothing
/// lexically. Bare form: an `optional` ident that resolves to nothing
/// lexically but is covered by a `with` — plausibly `with lib;`. A
/// lexically bound bare `optional` is the user's own function, and an
/// unbound one with no `with` in play is undefined-variable's finding.
fn optional_callee_token(model: &SemanticModel<'_>, callee: Expr<'_>) -> Option<TextRange> {
    let source = model.source();
    match callee {
        Expr::Ident(tok) => {
            if tok.text(source) != "optional" {
                return None;
            }
            let reference = model
                .references()
                .iter()
                .find(|r| r.name.range() == tok.range())?;
            (reference.resolved.is_none() && reference.via_with.is_some()).then(|| tok.range())
        }
        Expr::Select(sel) => {
            let Some(Expr::Ident(base)) = sel.base() else {
                return None;
            };
            if base.text(source) != "lib" {
                return None;
            }
            if let Some(binding) = model.resolve(base) {
                if binding.kind != BindingKind::LambdaParam {
                    return None;
                }
            }
            if sel.default().is_some() {
                return None;
            }
            let attrpath = sel.attrpath()?;
            let mut elements = attrpath.elements();
            let Some(AttrName::Ident(attr)) = elements.next() else {
                return None;
            };
            if elements.next().is_some() {
                return None;
            }
            (attr.text(source) == "optional").then(|| attr.range())
        }
        _ => None,
    }
}

/// Flags `lib.optional cond [ ... ]` — a literal list second argument
/// yields a nested list.
pub struct OptionalListArgument;

impl Rule for OptionalListArgument {
    fn code(&self) -> &'static str {
        "optional-list-argument"
    }

    fn name(&self) -> &'static str {
        "List argument to optional"
    }

    fn description(&self) -> &'static str {
        "Flags `lib.optional cond [ ... ]`: optional wraps its element in a list, so a literal list argument produces a nested list. `lib.optionals` takes the list directly."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for node in model.root().descendants() {
            if node.kind() != K::ApplyExpr {
                continue;
            }
            let Some(outer) = ApplyExpr::cast(node) else {
                continue;
            };
            let Some(Expr::Apply(inner)) = outer.func() else {
                continue;
            };
            let Some(callee) = inner.func() else {
                continue;
            };
            let Some(callee_token) = optional_callee_token(model, callee) else {
                continue;
            };
            let Some(first) = inner.arg() else {
                continue;
            };
            if first.range() == callee.range() {
                continue;
            }
            let Some(second) = outer.arg() else {
                continue;
            };
            if second.range() == inner.range() {
                continue;
            }
            // Only a LITERAL list argument is provably wrong; a name
            // that happens to hold a list is not this rule's business.
            let Expr::List(_) = second else {
                continue;
            };
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "lib.optional wraps its element in a list; this yields a nested list",
                    node.content_range(),
                )
                .with_help("lib.optionals takes the list directly")
                // Token-range edit only: renaming `optional` to
                // `optionals` states the intent without touching the
                // arguments, so it can never collide with formatting.
                .with_fix(Fix::new("call lib.optionals instead").edit(callee_token, "optionals")),
            );
        }
    }
}

/// The generated value of a `listToAttrs` entry lambda `x: { name = x;
/// value = e; }` or `x: nameValuePair x e`: `(param, e)` when the entry
/// name is exactly the lambda parameter, else `None`.
fn gen_attrs_entry<'a>(model: &SemanticModel<'a>, func: Expr<'a>) -> Option<(&'a str, Expr<'a>)> {
    let source = model.source();
    let Expr::Lambda(lambda) = unwrap_paren(func) else {
        return None;
    };
    let LambdaParam::Ident(param) = lambda.param() else {
        return None;
    };
    let param = param.text(source);
    let is_param = |e: Expr<'_>| matches!(e, Expr::Ident(t) if t.text(source) == param);
    match unwrap_paren(lambda.body()?) {
        Expr::Attrset(set) => {
            let mut name = None;
            let mut value = None;
            for item in set.items() {
                let AttrItem::Binding(binding) = item else {
                    return None;
                };
                let mut path = binding.attrpath()?.elements();
                let (Some(AttrName::Ident(key)), None) = (path.next(), path.next()) else {
                    return None;
                };
                let slot = match key.text(source) {
                    "name" => &mut name,
                    "value" => &mut value,
                    _ => return None,
                };
                if slot.replace(binding.value()?).is_some() {
                    return None;
                }
            }
            is_param(name?).then_some((param, value?))
        }
        Expr::Apply(outer) => {
            let Expr::Apply(inner) = outer.func()? else {
                return None;
            };
            let callee = match inner.func()? {
                Expr::Ident(t) => t.text(source),
                Expr::Select(s) => match s.attrpath()?.elements().last()? {
                    AttrName::Ident(t) => t.text(source),
                    _ => return None,
                },
                _ => return None,
            };
            (callee == "nameValuePair" && is_param(inner.arg()?)).then_some((param, outer.arg()?))
        }
        _ => None,
    }
}

/// Flags `listToAttrs (map (x: { name = x; value = e; }) xs)` — and the
/// `nameValuePair x e` spelling — which is `lib.genAttrs xs (x: e)`.
pub struct ManualGenAttrs;

impl Rule for ManualGenAttrs {
    fn code(&self) -> &'static str {
        "manual-gen-attrs"
    }

    fn name(&self) -> &'static str {
        "Manual genAttrs"
    }

    fn description(&self) -> &'static str {
        "Flags `listToAttrs (map (x: { name = x; value = e; }) xs)` when `lib` is in scope; when every name is the list element itself, `lib.genAttrs xs (x: e)` says the same thing."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let mut ancestors: Vec<&SyntaxNode> = Vec::new();
        walk(model.root(), &mut ancestors, &mut |node, ancestors| {
            let Some(arg) = one_arg_call(model, node, "listToAttrs") else {
                return;
            };
            let Expr::Apply(mapped) = unwrap_paren(arg) else {
                return;
            };
            let Some((func, list)) = two_arg_call(model, mapped.syntax(), "map") else {
                return;
            };
            let Some((param, value)) = gen_attrs_entry(model, func) else {
                return;
            };
            if !model.is_bound("lib", node.content_range().start()) {
                return;
            }
            let mut replacement = format!(
                "lib.genAttrs {} ({param}: {})",
                argument_text(list, source),
                trimmed_text(value, source)
            );
            if parent_requires_parens(ancestors) {
                replacement = format!("({replacement})");
            }
            let range = node.content_range();
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "this listToAttrs is lib.genAttrs",
                    range,
                )
                .with_fix(Fix::new("rewrite with lib.genAttrs").edit(range, replacement)),
            );
        });
    }
}
