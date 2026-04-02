use crate::{Metadata, Report, Rule, Suggestion, make};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Attr, AttrpathValue, Expr},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for bindings of the form `a = someAttr.a` or deeper paths like
/// `a = x.y.z.a`.
///
/// ## Why is this bad?
/// If the aim is to extract or bring attributes of an attrset into
/// scope, prefer an inherit statement.
///
/// ## Example
///
/// ```nix
/// let
///   mtl = pkgs.haskellPackages.mtl;
/// in
///   null
/// ```
///
/// Try `inherit` instead:
///
/// ```nix
/// let
///   inherit (pkgs.haskellPackages) mtl;
/// in
///   null
/// ```
#[lint(
    name = "manual_inherit_from",
    note = "Assignment instead of inherit from",
    code = 4,
    match_with = SyntaxKind::NODE_ATTRPATH_VALUE,
)]
struct ManualInheritFrom;

impl Rule for ManualInheritFrom {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let key_value_stmt = AttrpathValue::cast(node.clone())?;
        let key = key_value_stmt.attrpath()?;
        let mut key_path = key.attrs();
        let key_node = key_path.next()?;

        if key_path.next().is_some() {
            return None;
        }

        let Attr::Ident(key) = key_node else {
            return None;
        };

        let Some(Expr::Select(value)) = key_value_stmt.value() else {
            return None;
        };

        // `x.y or default` is a safe access with fallback — not replaceable with inherit
        if value.or_token().is_some() {
            return None;
        }

        let select_attrpath = value.attrpath()?;
        let attrs: Vec<_> = select_attrpath.attrs().collect();

        if attrs.is_empty() {
            return None;
        }

        // The last attr in the select path must match the binding key
        let last_attr = attrs.last()?;
        let Attr::Ident(index) = last_attr else {
            return None;
        };

        if key.to_string() != index.to_string() {
            return None;
        }

        // All intermediate attrs must be plain idents (no dynamic `${...}`)
        let intermediate = &attrs[..attrs.len() - 1];
        for attr in intermediate {
            if !matches!(attr, Attr::Ident(_)) {
                return None;
            }
        }

        // Build the "from" set path: base_expr.attr1.attr2...attrN-1
        let base = value.expr()?;
        let set_text = if intermediate.is_empty() {
            base.syntax().to_string()
        } else {
            let middle: Vec<_> = intermediate
                .iter()
                .map(|a| a.syntax().to_string())
                .collect();
            format!("{}.{}", base.syntax(), middle.join("."))
        };

        let at = node.text_range();
        let replacement = make::inherit_from_stmt_text(&set_text, &[key])?
            .syntax()
            .clone();

        Some(self.report().suggest(
            at,
            "This assignment is better written with `inherit`",
            Suggestion::with_replacement(at, replacement),
        ))
    }
}
