use strictix_core::{
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, BinExpr, Expr, IfExpr, LambdaExpr, LambdaParam, LetExpr,
    SyntaxKind, SyntaxNode, TextRange, UnaryExpr,
};

use SyntaxKind as K;

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
        let (Some(cond), Some(then_b), Some(else_b)) =
            (if_expr.cond(), if_expr.then_branch(), if_expr.else_branch())
        else {
            return;
        };
        if bool_literal(cond, source).is_some() {
            return;
        }
        let then_lit = bool_literal(then_b, source);
        let else_lit = bool_literal(else_b, source);
        let rewrite = match (then_lit, else_lit) {
            (Some(true), Some(false)) => {
                Some((operand_text(cond, source), "replace with the condition"))
            }
            (Some(false), Some(true)) => Some((
                format!("!{}", operand_text(cond, source)),
                "replace with the negated condition",
            )),
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
        if !node.child_tokens().any(|t| t.kind() == K::Bang) {
            return;
        }
        let Some(operand) = unary.operand() else {
            return;
        };
        let inner = match operand {
            Expr::Paren(paren) => match paren.expr() {
                Some(inner) => inner,
                None => return,
            },
            other => other,
        };
        match inner {
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
                    .with_fix(
                        Fix::new("remove double negation")
                            .edit(node.content_range(), trimmed_text(inner_operand, source)),
                    ),
                );
            }
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

pub struct ConstantBooleanNot;

impl Rule for ConstantBooleanNot {
    fn code(&self) -> &'static str {
        "constant-boolean-not"
    }

    fn name(&self) -> &'static str {
        "Constant boolean negation"
    }

    fn description(&self) -> &'static str {
        "Flags boolean negations with a constant operand: `!true` is `false` and `!false` is `true`."
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
        if !node.child_tokens().any(|t| t.kind() == K::Bang) {
            return;
        }
        let Some(operand) = unary.operand() else {
            return;
        };
        let Some(value) = bool_literal(operand, source) else {
            return;
        };
        let replacement = if value { "false" } else { "true" };
        diags.push(
            Diagnostic::new(
                self.code(),
                self.severity(),
                "boolean negation has a constant result",
                node.content_range(),
            )
            .with_fix(
                Fix::new("replace with the opposite boolean")
                    .edit(node.content_range(), replacement),
            ),
        );
    }
}

pub struct ConstantBooleanBinop;

impl Rule for ConstantBooleanBinop {
    fn code(&self) -> &'static str {
        "constant-boolean-binop"
    }

    fn name(&self) -> &'static str {
        "Constant boolean operation"
    }

    fn description(&self) -> &'static str {
        "Flags `&&` and `||` expressions whose two operands are boolean constants."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binary) = BinExpr::cast(node) else {
            return;
        };
        let Some(op) = binary.op() else { return };
        if !matches!(op, K::AndAnd | K::OrOr) {
            return;
        }
        let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs()) else {
            return;
        };
        let (Some(left), Some(right)) = (bool_literal(lhs, source), bool_literal(rhs, source))
        else {
            return;
        };
        let result = match op {
            K::AndAnd => left && right,
            K::OrOr => left || right,
            _ => unreachable!("operator checked above"),
        };
        let replacement = if result { "true" } else { "false" };
        diags.push(
            Diagnostic::new(
                self.code(),
                self.severity(),
                "boolean operation has a constant result",
                node.content_range(),
            )
            .with_fix(
                Fix::new("replace with the boolean result").edit(node.content_range(), replacement),
            ),
        );
    }
}

pub struct IdentityLambda;

impl Rule for IdentityLambda {
    fn code(&self) -> &'static str {
        "identity-lambda"
    }

    fn name(&self) -> &'static str {
        "Identity lambda"
    }

    fn description(&self) -> &'static str {
        "Flags a lambda that returns its only argument unchanged: `x: x`."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::LambdaExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(lambda) = LambdaExpr::cast(node) else {
            return;
        };
        let LambdaParam::Ident(param) = lambda.param() else {
            return;
        };
        let Some(Expr::Ident(body)) = lambda.body() else {
            return;
        };
        if param.text(source) != body.text(source) {
            return;
        }
        diags.push(Diagnostic::new(
            self.code(),
            self.severity(),
            "lambda returns its argument unchanged",
            node.content_range(),
        ));
    }
}

pub struct EmptyAttrsetMerge;

impl Rule for EmptyAttrsetMerge {
    fn code(&self) -> &'static str {
        "empty-attrset-merge"
    }

    fn name(&self) -> &'static str {
        "Empty attribute-set merge"
    }

    fn description(&self) -> &'static str {
        "Flags merges where one operand is an empty attribute set: `{ } // attrs` and `attrs // { }` return the other operand."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(SyntaxKind::BinExpr)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binary) = BinExpr::cast(node) else {
            return;
        };
        if binary.op() != Some(K::SlashSlash) {
            return;
        }
        let (Some(lhs), Some(rhs)) = (binary.lhs(), binary.rhs()) else {
            return;
        };
        let lhs_empty = is_empty_attrset(lhs);
        let rhs_empty = is_empty_attrset(rhs);
        if lhs_empty == rhs_empty {
            return;
        }
        let kept = if lhs_empty { rhs } else { lhs };
        let replacement = splice_operand(kept, source);
        let has_comments = node.child_tokens().any(|t| t.kind() == K::Comment);
        if has_comments {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                "merge with an empty attribute set",
                node.content_range(),
            ));
        } else {
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "merge with an empty attribute set",
                    node.content_range(),
                )
                .with_fix(
                    Fix::new("remove the empty attribute set")
                        .edit(node.content_range(), replacement),
                ),
            );
        }
    }
}

fn is_empty_attrset(expr: Expr<'_>) -> bool {
    match unwrap_parens(expr) {
        Expr::Attrset(set) => set.items().next().is_none(),
        _ => false,
    }
}

fn splice_operand(expr: Expr<'_>, source: &str) -> String {
    let text = trimmed_text(expr, source);
    match expr {
        Expr::Bin(_)
        | Expr::If(_)
        | Expr::Let(_)
        | Expr::Lambda(_)
        | Expr::With(_)
        | Expr::Assert(_) => format!("({text})"),
        _ => text.to_owned(),
    }
}

fn unwrap_parens(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        let Some(inner) = paren.expr() else { break };
        expr = inner;
    }
    expr
}

fn bool_literal(expr: Expr<'_>, source: &str) -> Option<bool> {
    match unwrap_parens(expr) {
        Expr::Ident(token) if token.text(source) == "true" => Some(true),
        Expr::Ident(token) if token.text(source) == "false" => Some(false),
        _ => None,
    }
}

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

fn atom_text<'a>(expr: Expr<'a>, source: &'a str) -> Option<&'a str> {
    match expr {
        Expr::Ident(t) | Expr::Int(t) => Some(t.text(source)),
        _ => None,
    }
}

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

fn trimmed_text<'s>(expr: Expr<'_>, source: &'s str) -> &'s str {
    let r = trimmed_range(expr);
    &source[r.start() as usize..r.end() as usize]
}

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
        Expr::Int(_) | Expr::Float(_) | Expr::Path(_) | Expr::SearchPath(_) | Expr::Uri(_) => false,
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
