use strictix_syntax::TextRange;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub range: TextRange,
    pub replacement: String,
}

impl TextEdit {
        #[must_use]
    pub fn new(range: TextRange, replacement: impl Into<String>) -> Self {
        Self {
            range,
            replacement: replacement.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<TextEdit>,
}

impl Fix {
        #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            edits: Vec::new(),
        }
    }

        #[must_use]
    pub fn edit(mut self, range: TextRange, replacement: impl Into<String>) -> Self {
        self.edits.push(TextEdit::new(range, replacement));
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixError {
        Overlap(TextRange, TextRange),
        InvalidRange(TextRange),
        InvalidBoundary(TextRange),
}

pub fn apply_fixes(source: &str, edits: &[TextEdit]) -> Result<String, FixError> {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|b| (std::cmp::Reverse(b.range.start()), std::cmp::Reverse(b.range.end())));

    let src_len = source.len();
    for edit in &sorted {
        let start = edit.range.start() as usize;
        let end = edit.range.end() as usize;
        if start > end || end > src_len {
            return Err(FixError::InvalidRange(edit.range));
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(FixError::InvalidBoundary(edit.range));
        }
    }

    for pair in sorted.windows(2) {
        let later = pair[0];
        let earlier = pair[1];
        if later.range.start() < earlier.range.end() {
            return Err(FixError::Overlap(later.range, earlier.range));
        }
    }

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
