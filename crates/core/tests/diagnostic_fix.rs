//! End-to-end tests for the M5 diagnostics and text-splice fix
//! machinery: edit validation, ordering, splicing, and the
//! Diagnostic builder chain.

use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::{apply_fixes, Fix, FixError, TextEdit};
use strictix_syntax::TextRange;

#[test]
fn single_edit_mid_string() {
    // Replace a word in the middle; text before and after survives.
    let source = "hello brave world";
    let edits = [TextEdit::new(TextRange::new(6, 11), "cruel")];
    assert_eq!(
        apply_fixes(source, &edits),
        Ok("hello cruel world".to_string())
    );
}

#[test]
fn multiple_edits_reverse_order() {
    // Edits arrive later-first (descending by position); result must be
    // independent of input order.
    let source = "abcdef";
    let edits = [
        TextEdit::new(TextRange::new(4, 6), "XY"), // later
        TextEdit::new(TextRange::new(0, 2), "AB"), // earlier
    ];
    assert_eq!(apply_fixes(source, &edits), Ok("ABcdXY".to_string()));
}

#[test]
fn multiple_edits_forward_order() {
    // Same two edits given in source order — sorting inside apply_fixes
    // must make both orders equivalent.
    let source = "abcdef";
    let edits = [
        TextEdit::new(TextRange::new(0, 2), "AB"),
        TextEdit::new(TextRange::new(4, 6), "XY"),
    ];
    assert_eq!(apply_fixes(source, &edits), Ok("ABcdXY".to_string()));
}

#[test]
fn overlapping_edits_error() {
    // Ranges 1..4 and 2..5 intersect: neither edit can be applied
    // without corrupting the other's span.
    let source = "abcdef";
    let edits = [
        TextEdit::new(TextRange::new(1, 4), "X"),
        TextEdit::new(TextRange::new(2, 5), "Y"),
    ];
    assert_eq!(
        apply_fixes(source, &edits),
        Err(FixError::Overlap(
            TextRange::new(2, 5),
            TextRange::new(1, 4)
        )),
    );
}

#[test]
fn edit_beyond_source_error() {
    // Source is 3 bytes; an edit ending at 5 points past the end.
    let source = "abc";
    let edits = [TextEdit::new(TextRange::new(1, 5), "X")];
    assert_eq!(
        apply_fixes(source, &edits),
        Err(FixError::InvalidRange(TextRange::new(1, 5))),
    );
}

#[test]
fn empty_replacement_deletes_range() {
    let source = "aXb";
    let edits = [TextEdit::new(TextRange::new(1, 2), "")];
    assert_eq!(apply_fixes(source, &edits), Ok("ab".to_string()));
}

#[test]
fn replacement_with_newlines() {
    // Replacement text is arbitrary; newlines must pass through whole.
    let source = "a b";
    let edits = [TextEdit::new(TextRange::new(1, 2), "\n\n")];
    assert_eq!(apply_fixes(source, &edits), Ok("a\n\nb".to_string()));
}

#[test]
fn adjacent_edits_allowed() {
    // Touching ranges (0..2 | 2..4 | 4..6) do not overlap by the
    // intersection rule and must all apply.
    let source = "abcdef";
    let edits = [
        TextEdit::new(TextRange::new(2, 4), "CD"),
        TextEdit::new(TextRange::new(4, 6), "EF"),
        TextEdit::new(TextRange::new(0, 2), "AB"),
    ];
    assert_eq!(apply_fixes(source, &edits), Ok("ABCDEF".to_string()));
}

#[test]
fn no_edits_returns_source_unchanged() {
    let source = "hello";
    assert_eq!(apply_fixes(source, &[]), Ok("hello".to_string()));
}

#[test]
fn diagnostic_builder_chain() {
    let d = Diagnostic::new("my-rule", Severity::Warning, "msg", TextRange::new(1, 2))
        .with_help("do this")
        .with_fix(Fix::new("fix it").edit(TextRange::new(1, 2), "x"));
    assert_eq!(d.code, "my-rule");
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(d.message, "msg");
    assert_eq!(d.range, TextRange::new(1, 2));
    assert_eq!(d.help.as_deref(), Some("do this"));
    let fix = d.fix.as_ref().expect("fix attached");
    assert_eq!(fix.label, "fix it");
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].range, TextRange::new(1, 2));
    assert_eq!(fix.edits[0].replacement, "x");
    assert_eq!(d.severity_str(), "warning");

    let err = Diagnostic::new("r2", Severity::Error, "m", TextRange::new(0, 0));
    assert_eq!(err.severity_str(), "error");
    assert_eq!(err.help, None);
    assert_eq!(err.fix, None);
}

#[test]
fn fix_two_edits_swap_pieces() {
    // One fix carrying two non-overlapping edits: swap 'one' and 'three'
    // by replacing each word in place. Proves the fix's edit list flows
    // straight into apply_fixes end-to-end.
    let source = "one two three";
    let fix = Fix::new("swap")
        .edit(TextRange::new(0, 3), "three")
        .edit(TextRange::new(8, 13), "one");
    assert_eq!(fix.label, "swap");
    assert_eq!(fix.edits.len(), 2);
    assert_eq!(
        apply_fixes(source, &fix.edits),
        Ok("three two one".to_string()),
    );
}
