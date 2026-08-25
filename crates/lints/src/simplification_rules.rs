//! Expression-simplification lints: boolean `if` forms, double/comparison
//! negation, and trivial `let`s that reduce to their own binding.
//!
//! Every fix here rewrites a whole expression node with text spliced from
//! its own children, so the rules only fire when the rewrite is provably
//! parse-safe at any slot the original node could occupy. When operand
//! precedence cannot be proven from the node alone, the replacement keeps
//! explicit parentheses rather than guessing at the parent context.

use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Expr, IfExpr, LetExpr, SyntaxKind, SyntaxNode, TextRange,
    UnaryExpr,
};

use SyntaxKind as K;

/// Flags `if` expressions reducible to a boolean expression.
pub struct BooleanIf;

impl Rule for BooleanIf {
    fn code(&self) -> &'static str {
        "boolean-if"
    }

    fn name(&self) -> &'static str {
        "Boolean if"
    }

    fn description(&self) -> &'static str {
        "Flags if-expressions that reduce to a boolean expression: `if c then true else false` is `c`, `if c then true else e` is `c || e` (when e is definitely boolean), and identical branches make the if pointless."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::IfExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(if_expr) = IfExpr::cast(node) else {
            return;
        };
        let (Some(cond), Some(then_b), Some(else_b)) = (
            if_expr.cond(),
            if_expr.then_branch(),
            if_expr.else_branch(),
        ) else {
            return;
        };
        // Ownership exclusion: a literal true/false condition belongs to
        // constant-if. Firing here too would attach a second fix to the
        // same node and the two fixes would overlap.
        if bool_literal(cond, source).is_some() {
            return;
        }
        let then_lit = bool_literal(then_b, source);
        let else_lit = bool_literal(else_b, source);
        // `if` slots are full-expression slots (Nix parses `if` at the
        // lowest precedence level), so a `c || e` / `c && e` replacement
        // is parse-safe bare; only the *operands* need the guard below.
        let rewrite = match (then_lit, else_lit) {
            // if c then true else false  ->  c
            (Some(true), Some(false)) => Some((
                operand_text(cond, source),
                "replace with the condition",
            )),
            // if c then false else true  ->  !c
            (Some(false), Some(true)) => Some((
                format!("!{}", operand_text(cond, source)),
                "replace with the negated condition",
            )),
            // One literal branch, one definitely-boolean branch. The
            // boolean proof matters: Nix `||`/`&&` force both sides to
            // booleans, while an if-branch of any other type is legal,
            // so rewriting a non-boolean branch would add a type error.
            (Some(true), None) if is_definitely_boolean(else_b, source) => Some((
                format!(
                    "{} || {}",
                    operand_text(cond, source),
                    operand_text(else_b, source)
                ),
                "replace with `||`",
            )),
            (Some(false), None) if is_definitely_boolean(else_b, source) => Some((
                format!(
                    "!{} && {}",
                    operand_text(cond, source),
                    operand_text(else_b, source)
                ),
                "replace with `&&`",
            )),
            (None, Some(false)) if is_definitely_boolean(then_b, source) => Some((
                format!(
                    "{} && {}",
                    operand_text(cond, source),
                    operand_text(then_b, source)
                ),
                "replace with `&&`",
            )),
            (None, Some(true)) if is_definitely_boolean(then_b, source) => Some((
                format!(
                    "!{} || {}",
                    operand_text(cond, source),
                    operand_text(then_b, source)
                ),
                "replace with `||`",
            )),
            _ => None,
        };
        if let Some((replacement, label)) = rewrite {
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "this if-expression reduces to a boolean expression",
                    node.content_range(),
                )
                .with_fix(Fix::new(label).edit(node.content_range(), replacement)),
            );
            return;
        }
        // Identical branches: only same-kind atoms with identical text
        // are compared (the Tautology approach), so `1` vs `1.0` or two
        // structurally equal attrsets never match. No fix: inlining a
        // branch skips forcing the condition, which changes error and
        // divergence behavior.
        let same_kind = matches!(
            (then_b, else_b),
            (Expr::Ident(_), Expr::Ident(_)) | (Expr::Int(_), Expr::Int(_))
        );
        let same_text = atom_text(then_b, source)
            .zip(atom_text(else_b, source))
            .is_some_and(|(l, r)| l == r);
        if same_kind && same_text {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                "both branches are identical",
                node.content_range(),
            ));
        }
    }
}

/// Flags negations that flip to a simpler operator.
pub struct NegationSimplification;

impl Rule for NegationSimplification {
    fn code(&self) -> &'static str {
        "negation-simplification"
    }

    fn name(&self) -> &'static str {
        "Simplifiable negation"
    }

    fn description(&self) -> &'static str {
        "Flags `!(!x)` (double negation) and negated comparisons — `!(a == b)` is `a != b`, `!(a < b)` is `a >= b` — which flip to a simpler operator with identical semantics."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::UnaryExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(unary) = UnaryExpr::cast(node) else {
            return;
        };
        // Only boolean `!`. The parser currently builds UnaryExpr for
        // Bang alone, but check the token so a future arithmetic `-`
        // unary never lands here by accident.
        if !node.child_tokens().any(|t| t.kind() == K::Bang) {
            return;
        }
        let Some(operand) = unary.operand() else {
            return;
        };
        // Unwrap exactly one paren layer: `!` binds tighter than every
        // comparison, so a negated comparison can only appear as
        // `!(a == b)`, and Nix rejects `!!x` without parens — the single
        // paren layer is the only shape these forms take.
        let inner = match operand {
            Expr::Paren(paren) => match paren.expr() {
                Some(inner) => inner,
                None => return,
            },
            other => other,
        };
        match inner {
            // !(!x) -> x. The inner operand sat in a unary slot, so it
            // is atom/paren/select/apply-shaped and binds tighter than
            // the outer UnaryExpr's own slot — safe to splice bare.
            Expr::Unary(inner_unary) => {
                let Some(inner_operand) = inner_unary.operand() else {
                    return;
                };
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "double negation cancels out",
                        node.content_range(),
                    )
                    .with_fix(Fix::new("remove double negation").edit(
                        node.content_range(),
                        trimmed_text(inner_operand, source),
                    )),
                );
            }
            // !(a op b) -> (a flipped-op b). Nix comparisons are total
            // on comparable values (both sides throw identically on
            // incomparable ones), so flipping the operator is an exact
            // equivalence.
            Expr::Bin(binary) => {
                let Some(op) = binary.op() else { return };
                let flipped = match op {
                    K::EqEq => "!=",
                    K::Neq => "==",
                    K::Lt => ">=",
                    K::Gt => "<=",
                    K::LtEq => ">",
                    K::GtEq => "<",
                    _ => return,
                };
                let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs()) else {
                    return;
                };
                // Ownership exclusion: identical same-kind atoms are
                // tautology's finding (`x == x`); a second fix on the
                // same text would overlap with its rewrite.
                let same_kind = matches!(
                    (lhs, rhs),
                    (Expr::Ident(_), Expr::Ident(_)) | (Expr::Int(_), Expr::Int(_))
                );
                let same_text = atom_text(lhs, source)
                    .zip(atom_text(rhs, source))
                    .is_some_and(|(l, r)| l == r);
                if same_kind && same_text {
                    return;
                }
                // The replacement keeps the operand's parentheses:
                // without a parent pointer we cannot prove the outer
                // UnaryExpr is not itself a binop operand (`!(a == b) //
                // c` or a non-associative comparison chain), and a bare
                // comparison there would re-associate or fail to parse.
                // `(a != b)` is precedence-inert at every slot `!(…)`
                // could occupy.
                let replacement = format!(
                    "({} {} {})",
                    trimmed_text(lhs, source),
                    flipped,
                    trimmed_text(rhs, source)
                );
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "negated comparison flips to a simpler operator",
                        node.content_range(),
                    )
                    .with_fix(
                        Fix::new("flip the comparison").edit(node.content_range(), replacement),
                    ),
                );
            }
            _ => {}
        }
    }
}

/// Flags `let x = e; in x` — a let that reduces to its own value.
pub struct TrivialLet;

impl Rule for TrivialLet {
    fn code(&self) -> &'static str {
        "trivial-let"
    }

    fn name(&self) -> &'static str {
        "Trivial let"
    }

    fn description(&self) -> &'static str {
        "Flags a let with exactly one binding whose body is exactly that binding's name: `let x = e; in x` is just `e`."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::LetExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(let_expr) = LetExpr::cast(node) else {
            return;
        };
        let Some(bindings) = let_expr.bindings() else {
            return;
        };
        // Exactly one plain binding: any inherit or second entry means
        // the let introduces more than the returned name.
        let mut items = bindings.items();
        let Some(AttrItem::Binding(binding)) = items.next() else {
            return;
        };
        if items.next().is_some() {
            return;
        }
        let Some(attrpath) = binding.attrpath() else {
            return;
        };
        // A single bare-ident path only: `x.y = …` builds a nested
        // attrset, and quoted or dynamic segments are not the plain
        // name the body could repeat.
        let mut elements = attrpath.elements();
        let Some(AttrName::Ident(name_token)) = elements.next() else {
            return;
        };
        if elements.next().is_some() {
            return;
        }
        let name = name_token.text(source);
        let Some(Expr::Ident(body_token)) = let_expr.body() else {
            return;
        };
        if body_token.text(source) != name {
            return;
        }
        let Some(value) = binding.value() else {
            return;
        };
        // Let bindings are recursive, so a value mentioning its own name
        // (`let x = f x; in x`) resolves that name to the binding, not
        // the enclosing scope — splicing the value text out would
        // rebind it. The token scan over-approximates (a select attr
        // named `x` also matches), which only makes the rule quieter.
        if mentions_name(value, source, name) {
            return;
        }
        diags.push(
            Diagnostic::new(
                self.code(),
                self.severity(),
                format!("let binds '{name}' only to return it"),
                node.content_range(),
            )
            .with_fix(
                Fix::new("replace let with the bound value")
                    .edit(node.content_range(), trimmed_text(value, source)),
            ),
        );
    }
}

/// Unwrap every paren layer around an expression.
///
/// Duplicated per house convention (suggested_rules has its own): rule
/// files stay self-contained rather than sharing a helpers module.
fn unwrap_parens(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        let Some(inner) = paren.expr() else { break };
        expr = inner;
    }
    expr
}

/// The boolean literal an expression spells, seen through parens.
///
/// In Nix, true/false lex as Ident tokens; in expression position these
/// texts are always the literals.
fn bool_literal(expr: Expr<'_>, source: &str) -> Option<bool> {
    match unwrap_parens(expr) {
        Expr::Ident(token) if token.text(source) == "true" => Some(true),
        Expr::Ident(token) if token.text(source) == "false" => Some(false),
        _ => None,
    }
}

/// True when the expression is syntactically guaranteed to produce a
/// boolean: comparisons, boolean binops, `!`, `?`, and bool literals.
///
/// This is what licenses the `c || e` rewrite — `||`/`&&` force both
/// sides to booleans, so an if-branch of any other type must not move
/// under them.
fn is_definitely_boolean(expr: Expr<'_>, source: &str) -> bool {
    match unwrap_parens(expr) {
        Expr::Unary(_) | Expr::HasAttr(_) => true,
        Expr::Bin(binary) => matches!(
            binary.op(),
            Some(
                K::EqEq
                    | K::Neq
                    | K::Lt
                    | K::LtEq
                    | K::Gt
                    | K::GtEq
                    | K::AndAnd
                    | K::OrOr
                    | K::Arrow
            )
        ),
        other => bool_literal(other, source).is_some(),
    }
}

/// The source text of an ident or int atom; `None` for anything else.
///
/// Same-kind atoms with identical text are the only branch/operand pair
/// this module treats as "the same expression" — matching Tautology.
fn atom_text<'a>(expr: Expr<'a>, source: &'a str) -> Option<&'a str> {
    match expr {
        Expr::Ident(t) | Expr::Int(t) => Some(t.text(source)),
        _ => None,
    }
}

/// The range of an expression with edge trivia trimmed.
///
/// Node ranges flush leading trivia into the node, so a node's raw text
/// can drag whitespace or comments along when spliced elsewhere; token
/// exprs are exact already.
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

/// The expression's text, parenthesized unless it can stand alone as an
/// operand of `!`, `&&`, or `||`.
///
/// Atoms, parens, selects, and applications all bind tighter than every
/// boolean operator, so they splice bare; anything else (a comparison,
/// an arrow, a lambda) gets parens to survive any surrounding operator.
fn operand_text(expr: Expr<'_>, source: &str) -> String {
    let text = trimmed_text(expr, source);
    match expr {
        Expr::Ident(_)
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::Path(_)
        | Expr::SearchPath(_)
        | Expr::Uri(_)
        | Expr::Paren(_)
        | Expr::Select(_)
        | Expr::Apply(_) => text.to_owned(),
        _ => format!("({text})"),
    }
}

/// True when any ident token anywhere under `expr` spells `name`.
///
/// Deliberately over-approximate: `a.x` matches a search for `x` even
/// though the select attr is not a reference. TrivialLet only uses this
/// to stay silent, so a false positive just skips a finding.
fn mentions_name(expr: Expr<'_>, source: &str, name: &str) -> bool {
    fn node_mentions(node: &SyntaxNode, source: &str, name: &str) -> bool {
        node.children().iter().any(|child| match child {
            strictix_syntax::NodeOrToken::Token(t) => {
                t.kind() == K::Ident && t.text(source) == name
            }
            strictix_syntax::NodeOrToken::Node(n) => node_mentions(n, source, name),
        })
    }
    match expr {
        Expr::Ident(t) => t.text(source) == name,
        Expr::Int(_) | Expr::Float(_) | Expr::Path(_) | Expr::SearchPath(_) | Expr::Uri(_) => {
            false
        }
        Expr::Let(e) => node_mentions(e.syntax(), source, name),
        Expr::With(e) => node_mentions(e.syntax(), source, name),
        Expr::Assert(e) => node_mentions(e.syntax(), source, name),
        Expr::If(e) => node_mentions(e.syntax(), source, name),
        Expr::Attrset(e) => node_mentions(e.syntax(), source, name),
        Expr::RecAttrset(e) => node_mentions(e.syntax(), source, name),
        Expr::List(e) => node_mentions(e.syntax(), source, name),
        Expr::Lambda(e) => node_mentions(e.syntax(), source, name),
        Expr::Apply(e) => node_mentions(e.syntax(), source, name),
        Expr::Unary(e) => node_mentions(e.syntax(), source, name),
        Expr::Bin(e) => node_mentions(e.syntax(), source, name),
        Expr::Select(e) => node_mentions(e.syntax(), source, name),
        Expr::HasAttr(e) => node_mentions(e.syntax(), source, name),
        Expr::String(e) => node_mentions(e.syntax(), source, name),
        Expr::IndString(e) => node_mentions(e.syntax(), source, name),
        Expr::Paren(e) => node_mentions(e.syntax(), source, name),
    }
}
