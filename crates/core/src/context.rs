//! Host-owned lint context: the source text plus an undo log.
//!
//! This is the spatiotemporal-composability core. The source text is
//! the one mutable piece of host state in the whole pipeline; the tree
//! and `SemanticModel` are derived reads of it, rebuilt after each
//! commit. Rules are components that read the context and commit
//! effects (fixes = text edits). Every commit records its inverse — the
//! full pre-commit text snapshot — so a rollback restores the context
//! exactly. Unmounting (`rollback_all`) leaves no residue: the context
//! returns to the snapshot it was mounted with.
//!
//! Inverses are full-text snapshots, not per-edit range inverses. A
//! batch of edits shifts ranges among themselves, so per-edit inverses
//! would need range math to stay valid; a snapshot reproduces inverse
//! replay's observable state without any of that.

use crate::fix::{apply_fixes, FixError, TextEdit};

/// Host-owned lint state: source text + an undo log of snapshots.
#[derive(Clone, Debug)]
pub struct Context {
    source: String,
    /// Pre-commit snapshots, one per commit, oldest first.
    undo: Vec<String>,
}

impl Context {
    /// Mount the context with the given source text.
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            undo: Vec::new(),
        }
    }

    /// The current source text.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Commit one batch of edits (a single fix pass) as one mutation.
    /// On success the text changes and the pre-commit snapshot is
    /// pushed as the inverse. Fails without mutating on invalid or
    /// overlapping edits.
    pub fn commit(&mut self, edits: &[TextEdit]) -> Result<(), FixError> {
        let result = apply_fixes(&self.source, edits)?;
        let old = std::mem::replace(&mut self.source, result);
        self.undo.push(old);
        Ok(())
    }

    /// Roll back the most recent commit, restoring its snapshot.
    /// Returns `false` when there is nothing to undo.
    pub fn rollback(&mut self) -> bool {
        match self.undo.pop() {
            Some(snapshot) => {
                self.source = snapshot;
                true
            }
            None => false,
        }
    }

    /// Roll back every commit (full unmount). Returns commits undone.
    pub fn rollback_all(&mut self) -> usize {
        let mut n = 0;
        while self.rollback() {
            n += 1;
        }
        n
    }
}
