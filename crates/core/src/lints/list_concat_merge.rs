use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{BinOp, BinOpKind, Expr},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for multiple consecutive list concatenations that can be merged.
///
/// ## Why is this bad?
/// Multiple `++` operations with simple lists make the code harder to read.
/// Adjacent unconditional lists should be merged together, not mixed into
/// conditional lists.
///
/// ## Example
/// ```nix
/// [ a b ] ++ lib.optional cfg.enable [ c ] ++ [ d e ]
/// ```
///
/// Merge the unconditional lists:
///
/// ```nix
/// [ a b d e ] ++ lib.optional cfg.enable [ c ]
/// ```
#[lint(
    name = "list_concat_merge",
    note = "Multiple list concatenations that can be merged",
    code = 35,
    match_with = SyntaxKind::NODE_BIN_OP
)]
struct ListConcatMerge;

impl Rule for ListConcatMerge {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        let bin_expr = BinOp::cast(node.clone())?;
        let Some(BinOpKind::Concat) = bin_expr.operator() else {
            return None;
        };

        // Look for patterns like: [ ... ] ++ LIB_OPTIONALS ++ [ ... ]
        let replacement = find_mergeable_concat(bin_expr.clone())?;

        Some(self.report().suggest(
            node.text_range(),
            "Multiple list concatenations that can be merged",
            Suggestion::with_replacement(node.text_range(), replacement),
        ))
    }
}

/// Find mergeable concatenation patterns and return the replacement text
fn find_mergeable_concat(bin_expr: BinOp) -> Option<rnix::SyntaxNode> {
    let lhs = bin_expr.lhs()?;
    let rhs = bin_expr.rhs()?;
    
    // Check if RHS is a concatenation like: lib.optionals ... ++ [ ... ]
    if let Expr::BinOp(rhs_bin) = &rhs {
        if rhs_bin.operator() == Some(BinOpKind::Concat) {
            let rhs_lhs = rhs_bin.lhs()?;
            let rhs_rhs = rhs_bin.rhs()?;
            
            // Check if rhs_lhs contains "optionals" or "optional" (lib.optionals, lib.optional, etc.)
            let rhs_lhs_text = rhs_lhs.to_string();
            if rhs_lhs_text.contains("optionals") || rhs_lhs_text.contains("optional") {
                // Pattern: [ ... ] ++ lib.optionals cond [...] ++ [ unconditional_items ]
                // Should merge the unconditional lists, NOT merge into the conditional list
                
                // Check if both lhs is a list and rhs_rhs is a list (both unconditional)
                if let (Expr::List(lhs_list), Expr::List(rhs_list)) = (&lhs, &rhs_rhs) {
                    // Get items from both unconditional lists
                    let lhs_items: Vec<String> = lhs_list
                        .items()
                        .map(|item| item.to_string())
                        .collect();
                    let rhs_items: Vec<String> = rhs_list
                        .items()
                        .map(|item| item.to_string())
                        .collect();
                    
                    if lhs_items.is_empty() || rhs_items.is_empty() {
                        return None; // Empty list, no merging needed
                    }
                    
                    // Merge the two unconditional lists
                    let merged_unconditional_text = format!(
                        "[ {all_unconditional_items} ] ++ {optionals}",
                        all_unconditional_items = [lhs_items, rhs_items].concat().join(" "),
                        optionals = rhs_lhs_text
                    );
                    
                    // Parse the merged text to create replacement
                    let parse = rnix::Root::parse(&merged_unconditional_text).ok().ok()?;
                    let replacement = Expr::cast(parse.syntax().clone())?;
                    
                    return Some(replacement.syntax().clone());
                }
            }
        }
    }
    
    None
}