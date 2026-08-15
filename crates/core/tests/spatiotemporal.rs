//! Spatiotemporal-composability checks: the `Context` undo log (temporal
//! axis — clean unmount, no residue) and the `fixpoint` reactive loop
//! (spatial axis — a committed fix re-runs consumers on changed text).

use strictix_core::config::LintConfig;
use strictix_core::context::Context;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::{Fix, TextEdit};
use strictix_core::rules::{lint, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_syntax::TextRange;

// --- temporal axis: residue -----------------------------------------

#[test]
fn commit_then_rollback_all_leaves_no_residue() {
    let original = "let a = 1; in a";
    let mut ctx = Context::new(original.to_string());
    let snapshot = ctx.source().to_string();

    // Exercise: one commit carrying two edits (a single fix pass).
    let edits = vec![
        TextEdit::new(TextRange::new(0, 3), "X"), // "let" -> "X"
        TextEdit::new(TextRange::new(4, 5), "Y"), // "a" -> "Y"
    ];
    ctx.commit(&edits)
        .expect("edits are valid and non-overlapping");
    assert_ne!(ctx.source(), snapshot, "commit changed the text");

    // Unmount: apply inverses in reverse order.
    let undone = ctx.rollback_all();
    assert_eq!(undone, 1, "one commit undone");
    assert_eq!(ctx.source(), snapshot, "no residue after unmount");
}

#[test]
fn rollback_restores_in_reverse_commit_order() {
    let mut ctx = Context::new("abc");
    ctx.commit(&[TextEdit::new(TextRange::new(0, 1), "X")])
        .unwrap(); // "Xbc"
    ctx.commit(&[TextEdit::new(TextRange::new(2, 3), "Y")])
        .unwrap(); // "XbY"
    assert_eq!(ctx.source(), "XbY");

    assert!(ctx.rollback(), "undo second commit");
    assert_eq!(ctx.source(), "Xbc");
    assert!(ctx.rollback(), "undo first commit");
    assert_eq!(ctx.source(), "abc");
    assert!(!ctx.rollback(), "nothing left to undo");
}

// --- spatial axis: reactivity ---------------------------------------

/// A file rule that replaces the first occurrence of `from` with `to`.
/// Two of these chained make a fix reveal another fix: `a` -> `b` -> `c`.
struct ReplaceRule {
    code: &'static str,
    from: char,
    to: char,
}

impl Rule for ReplaceRule {
    fn code(&self) -> &'static str {
        self.code
    }
    fn name(&self) -> &'static str {
        "replace"
    }
    fn description(&self) -> &'static str {
        "test rule"
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let src = model.source();
        if let Some(pos) = src.find(self.from) {
            let range = TextRange::new(pos as u32, pos as u32 + 1);
            let fix = Fix::new("replace").edit(range, self.to.to_string());
            diags.push(
                Diagnostic::new(self.code, Severity::Warning, "replace", range).with_fix(fix),
            );
        }
    }
}

#[test]
fn fixpoint_reruns_consumers_after_a_commit() {
    // Rule A turns "a" into "b"; rule B turns "b" into "c". Source "a":
    // pass 1 applies A, pass 2 applies B (only visible after A), pass 3
    // is clean. Exactly two passes, final text "c".
    let rules: Vec<Box<dyn Rule>> = vec![
        Box::new(ReplaceRule {
            code: "replace-a",
            from: 'a',
            to: 'b',
        }),
        Box::new(ReplaceRule {
            code: "replace-b",
            from: 'b',
            to: 'c',
        }),
    ];
    let config = LintConfig::default();

    let run = lint(&rules, "a", &config, true);

    assert_eq!(run.passes, 2, "one pass per revealed fix");
    assert_eq!(
        run.fixed.as_deref(),
        Some("c"),
        "final text reaches fixpoint"
    );
    // First pass reported only rule A's finding; B fires only after A.
    assert_eq!(run.diagnostics.len(), 1, "first pass sees one finding");
    assert_eq!(run.diagnostics[0].code, "replace-a");
}

#[test]
fn lint_is_stable_when_no_fixes_exist() {
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(ReplaceRule {
        code: "replace-z",
        from: 'z',
        to: 'q',
    })];
    let config = LintConfig::default();

    let run = lint(&rules, "abc", &config, true);

    assert_eq!(run.passes, 0, "no commit when nothing to fix");
    assert_eq!(run.fixed, None, "unchanged text yields no fixed output");
    assert!(run.diagnostics.is_empty(), "no findings");
}

#[test]
fn check_mode_does_not_apply_fixes() {
    // A rule that would fix "a" -> "b". In check mode (fix=false) the
    // engine must report the finding but commit nothing: no fixed text,
    // no passes.
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(ReplaceRule {
        code: "replace-a",
        from: 'a',
        to: 'b',
    })];
    let config = LintConfig::default();

    let run = lint(&rules, "a", &config, false);

    assert_eq!(run.passes, 0, "check mode commits nothing");
    assert_eq!(run.fixed, None, "check mode leaves text untouched");
    assert_eq!(
        run.diagnostics.len(),
        1,
        "check mode still reports the finding"
    );
    assert_eq!(run.diagnostics[0].code, "replace-a");
}

#[test]
fn overlapping_fixes_fail_atomically_and_preserve_diagnostics() {
    let rules: Vec<Box<dyn Rule>> = vec![
        Box::new(ReplaceRule {
            code: "replace-a1",
            from: 'a',
            to: 'b',
        }),
        Box::new(ReplaceRule {
            code: "replace-a2",
            from: 'a',
            to: 'c',
        }),
    ];
    let config = LintConfig::default();

    let run = lint(&rules, "a", &config, true);

    assert!(run.error.is_some(), "overlapping edits error out");
    assert_eq!(run.fixed, None, "atomic: file left untouched on error");
    assert_eq!(run.passes, 0, "no commit succeeded");
    assert_eq!(run.diagnostics.len(), 2, "both findings still reported");
}
