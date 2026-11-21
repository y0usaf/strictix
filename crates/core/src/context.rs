use crate::fix::{apply_fixes, FixError, TextEdit};

#[derive(Clone, Debug)]
pub struct Context {
    source: String,
        undo: Vec<String>,
}

impl Context {
        #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            undo: Vec::new(),
        }
    }

        #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

        pub fn commit(&mut self, edits: &[TextEdit]) -> Result<(), FixError> {
        let result = apply_fixes(&self.source, edits)?;
        let old = std::mem::replace(&mut self.source, result);
        self.undo.push(old);
        Ok(())
    }

        pub fn rollback(&mut self) -> bool {
        match self.undo.pop() {
            Some(snapshot) => {
                self.source = snapshot;
                true
            }
            None => false,
        }
    }

        pub fn rollback_all(&mut self) -> usize {
        let mut n = 0;
        while self.rollback() {
            n += 1;
        }
        n
    }
}
