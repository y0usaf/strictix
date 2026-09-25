//! Static project context for cross-file import diagnostics.
use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::semantic::SemanticModel;
use strictix_syntax::{parse, Expr, StringPart, SyntaxNode};

#[derive(Clone, Debug)]
pub struct ProjectFile {
    pub path: PathBuf,
    pub source: String,
}

/// Deterministic graph of supplied files. Only literal relative imports are represented.
pub struct ProjectContext {
    files: BTreeSet<PathBuf>,
    imports: BTreeMap<PathBuf, Vec<(PathBuf, u32)>>,
    sources: Vec<ProjectFile>,
    /// Per-type project indexes built on first use by [Self::memo].
    memo: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl ProjectContext {
    pub fn new(files: impl IntoIterator<Item = ProjectFile>) -> Self {
        let mut all = BTreeSet::new();
        let mut imports = BTreeMap::new();
        let sources: Vec<ProjectFile> = files.into_iter().collect();
        for file in &sources {
            let path = normalize(&file.path);
            all.insert(path.clone());
            let tree = parse(&file.source);
            let mut edges = Vec::new();
            if tree.error_nodes().next().is_none() {
                let model = SemanticModel::new(&file.source, &tree).with_path(Some(&path));
                for site in model.import_sites() {
                    if let Some(raw) = literal_import(&file.source, &tree, site.path_range) {
                        let target = normalize(&path.parent().unwrap_or(Path::new(".")).join(raw));
                        edges.push((target, site.call_range.start()));
                    }
                }
            }
            imports.insert(path, edges);
        }
        Self {
            files: all,
            imports,
            sources,
            memo: Mutex::default(),
        }
    }

    /// Every supplied file with its source, in input order.
    pub fn sources(&self) -> &[ProjectFile] {
        &self.sources
    }

    /// Build a project-wide index of type `T` once and share it across
    /// every file (and worker thread) of the run. `build` must not call
    /// `memo` itself: the lock is held while it runs.
    pub fn memo<T: Any + Send + Sync>(&self, build: impl FnOnce(&Self) -> T) -> Arc<T> {
        let mut memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        let entry = memo
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Arc::new(build(self)));
        Arc::clone(entry)
            .downcast::<T>()
            .expect("memo entry keyed by its own TypeId")
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.files.contains(&normalize(path))
    }

    pub fn imports(&self, path: &Path) -> &[(PathBuf, u32)] {
        self.imports
            .get(&normalize(path))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn has_cycle_from(&self, start: &Path, target: &Path) -> bool {
        let start = normalize(start);
        let mut stack = vec![normalize(target)];
        let mut seen = BTreeSet::new();
        while let Some(path) = stack.pop() {
            if path == start {
                return true;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            for (next, _) in self.imports(&path) {
                stack.push(next.clone());
            }
        }
        false
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn literal_import(
    source: &str,
    tree: &SyntaxNode,
    range: strictix_syntax::TextRange,
) -> Option<PathBuf> {
    let expr = tree
        .descendants()
        .find(|n| n.range() == range)
        .and_then(Expr::cast)?;
    match expr {
        Expr::Path(path) => {
            let text = path.text(source);
            (text.starts_with("./") || text.starts_with("../")).then(|| PathBuf::from(text))
        }
        Expr::String(string) => {
            let mut text = String::new();
            for part in string.parts() {
                match part {
                    StringPart::Content(token) => text.push_str(token.text(source)),
                    StringPart::Interp(_) => return None,
                }
            }
            (text.starts_with("./") || text.starts_with("../")).then(|| PathBuf::from(text))
        }
        _ => None,
    }
}
