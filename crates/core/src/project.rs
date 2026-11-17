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
    source_index: HashMap<PathBuf, usize>,
    present: BTreeSet<PathBuf>,
    memo: Mutex<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

fn extract_imports(path: &Path, source: &str) -> Vec<(PathBuf, u32)> {
    if !source.contains("import") || (!source.contains("./") && !source.contains(".\\")) {
        return Vec::new();
    }
    let tree = parse(source);
    let mut edges = Vec::new();
    if tree.error_nodes().next().is_none() {
        let model = SemanticModel::new(source, &tree).with_path(Some(path));
        for site in model.import_sites() {
            if let Some(raw) = literal_import(source, &tree, site.path_range) {
                let target = normalize(&path.parent().unwrap_or(Path::new(".")).join(raw));
                edges.push((target, site.call_range.start()));
            }
        }
    }
    edges
}

impl ProjectContext {
    pub fn new(files: impl IntoIterator<Item = ProjectFile>) -> Self {
        let sources: Vec<ProjectFile> = files.into_iter().collect();
        let norm_paths: Vec<PathBuf> = sources.iter().map(|f| normalize(&f.path)).collect();
        let workers = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
            .min(8);
        let edge_lists: Vec<Vec<(PathBuf, u32)>> = if sources.len() >= 16 && workers > 1 {
            const LARGE_BYTES: usize = 512 * 1024;
            let mut large = Vec::new();
            let mut small = Vec::new();
            for (idx, file) in sources.iter().enumerate() {
                if file.source.len() >= LARGE_BYTES {
                    large.push(idx);
                } else {
                    small.push(idx);
                }
            }
            large.sort_unstable_by(|&a, &b| sources[b].source.len().cmp(&sources[a].source.len()));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let slots: Vec<Mutex<Vec<(PathBuf, u32)>>> =
                (0..sources.len()).map(|_| Mutex::new(Vec::new())).collect();
            let run_small = || loop {
                let pos = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if pos >= small.len() {
                    break;
                }
                let idx = small[pos];
                let edges = extract_imports(&norm_paths[idx], &sources[idx].source);
                if !edges.is_empty() {
                    *slots[idx].lock().unwrap() = edges;
                }
            };
            std::thread::scope(|scope| {
                for _ in 0..(workers - 1).min(small.len()) {
                    scope.spawn(&run_small);
                }
                for &idx in &large {
                    let edges = extract_imports(&norm_paths[idx], &sources[idx].source);
                    if !edges.is_empty() {
                        *slots[idx].lock().unwrap() = edges;
                    }
                }
                run_small();
            });
            slots
                .into_iter()
                .map(|slot| slot.into_inner().unwrap())
                .collect()
        } else {
            sources
                .iter()
                .zip(&norm_paths)
                .map(|(file, path)| extract_imports(path, &file.source))
                .collect()
        };
        let mut all = BTreeSet::new();
        let mut imports = BTreeMap::new();
        let mut source_index = HashMap::with_capacity(sources.len());
        for (idx, (path, edges)) in norm_paths.into_iter().zip(edge_lists).enumerate() {
            all.insert(path.clone());
            source_index.entry(path.clone()).or_insert(idx);
            imports.insert(path, edges);
        }
        Self {
            files: all,
            imports,
            sources,
            source_index,
            present: BTreeSet::new(),
            memo: Mutex::default(),
        }
    }

    /// Builder: files that exist for [Self::contains] and
    /// [Self::contains_dir] without being part of the linted sources.
    #[must_use]
    pub fn with_present(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.present = paths.into_iter().map(|path| normalize(&path)).collect();
        self
    }

    pub fn sources(&self) -> &[ProjectFile] {
        &self.sources
    }

    #[must_use]
    pub fn source_index(&self, path: &Path) -> Option<usize> {
        self.source_index.get(&normalize(path)).copied()
    }

    #[must_use]
    pub fn source_by_norm_path(&self, normalized: &Path) -> Option<(usize, &ProjectFile)> {
        let idx = *self.source_index.get(normalized)?;
        Some((idx, &self.sources[idx]))
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
        let path = normalize(path);
        self.files.contains(&path) || self.present.contains(&path)
    }

    /// Whether some supplied file lies strictly under `path`, so the
    /// directory exists for the project even when no disk entry does
    /// (the files of a merged file live only in memory).
    pub fn contains_dir(&self, path: &Path) -> bool {
        let dir = normalize(path);
        [&self.files, &self.present].into_iter().any(|set| {
            set.range(dir.clone()..)
                .find(|file| **file != dir)
                .is_some_and(|file| file.starts_with(&dir))
        })
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

/// Lexically resolve `.` and `..`, as Nix does for path literals: a merged
/// file's split modules live only in memory, so the filesystem cannot walk
/// their `..`. A leading `..` that climbs above a relative path is kept.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(
                    out.components().next_back(),
                    None | Some(std::path::Component::ParentDir)
                ) {
                    out.push("..");
                } else {
                    out.pop();
                }
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
    let mut cur = tree;
    let expr = loop {
        if cur.range() == range {
            if let Some(expr) = Expr::cast(cur) {
                break Some(expr);
            }
        }
        let Some(next) = cur.child_at_offset(range.start()) else {
            break None;
        };
        cur = next;
    }?;
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
