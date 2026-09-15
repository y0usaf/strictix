//! Static project context for cross-file import diagnostics.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
}

impl ProjectContext {
    pub fn new(files: impl IntoIterator<Item = ProjectFile>) -> Self {
        let mut all = BTreeSet::new();
        let mut imports = BTreeMap::new();
        for file in files {
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
        }
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
