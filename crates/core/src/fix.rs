//! Text-splice fixes: edit a source string without reparsing.
//!
//! Fixes are computed from the lossless tree's byte ranges, then applied
//! to the raw source text. Because the tree keeps trivia (whitespace,
//! comments) as ordinary tokens, a fix targeting a node's range replaces
//! exactly that node — formatting around it survives untouched. That is
//! the "lossless-safe" guarantee: a fix never destroys formatting.
//!
//! `apply_fixes` is the single write path. It validates every edit
//! (bounds, overlap) before splicing anything, so a buggy rule can never
//! corrupt a file silently — it gets an error it can report instead.

use strictix_syntax::TextRange;

/// One replacement: the byte range to cut out and the text to put there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
}

impl TextEdit {
    /// Create an edit replacing `range` with `replacement`.
    ///
    /// An empty replacement deletes the range; an empty range inserts
    /// text at a point. Both are ordinary cases, not special ones.
    #[must_use]
    pub fn new(range: TextRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

/// A labeled set of edits, the payload of `Diagnostic::fix`.
///
/// The label is what a user sees in the CLI when a fix is offered, so it
/// should say what the change does ("remove unused binding"), not how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<TextEdit>,
}

impl Fix {
    /// Start a fix with a human label and no edits yet.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            edits: Vec::new(),
        }
    }

    /// Append one edit. Builder-style so multi-edit fixes read in one
    /// expression; edits may be added in any order — `apply_fixes`
    /// sorts them itself.
    #[must_use]
    pub fn edit(mut self, range: TextRange, replacement: impl Into<String>) -> Self {
        self.edits.push(TextEdit::new(range, replacement));
        self
    }
}

/// Why a set of edits could not be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixError {
    /// Two edits' ranges intersect; applying both would corrupt the text.
    /// Touching (adjacent) ranges are fine — no overlap by definition.
    Overlap(TextRange, TextRange),
    /// An edit's range extends past the end of the source.
    InvalidRange(TextRange),
}

/// Apply `edits` to `source`, producing the edited text.
///
/// Edits may arrive in any order; they are sorted by start descending
/// (so later edits are validated first, and an overlap surfaces as soon
/// as an earlier edit's end crosses the later edit's start). Validation
/// happens before any splicing, so on error the source is untouched.
///
/// # Errors
///
/// - `InvalidRange` if an edit ends beyond `source`'s length.
/// - `Overlap` if two edits' ranges intersect.
pub fn apply_fixes(source: &str, edits: &[TextEdit]) -> Result<String, FixError> {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    // Descending by start: the edit closest to the end of the file
    // first. That ordering makes the overlap rule a single comparison
    // between neighbours (see below).
    sorted.sort_by_key(|b| std::cmp::Reverse(b.range.start()));

    let src_len = source.len() as u32;
    for edit in &sorted {
        if edit.range.end() > src_len {
            return Err(FixError::InvalidRange(edit.range));
        }
    }

    // Non-overlap in descending order means each later edit's start sits
    // at or after the previous (earlier) edit's end. Equal starts that
    // are non-empty fail here, which is right: they intersect.
    for pair in sorted.windows(2) {
        let later = pair[0];
        let earlier = pair[1];
        if later.range.start() < earlier.range.end() {
            return Err(FixError::Overlap(later.range, earlier.range));
        }
    }

    // Splicing walks the source once, left to right (reverse of the
    // validation order), pushing the untouched gap, the replacement, and
    // advancing past each edit's end. Validation guarantees every slice
    // index stays in bounds and no edit is skipped.
    let mut out = String::with_capacity(source.len());
    let mut pos: u32 = 0;
    for edit in sorted.iter().rev() {
        out.push_str(&source[pos as usize..edit.range.start() as usize]);
        out.push_str(&edit.replacement);
        pos = edit.range.end();
    }
    out.push_str(&source[pos as usize..]);
    Ok(out)
}
