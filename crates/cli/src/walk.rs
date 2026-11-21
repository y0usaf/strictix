use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn is_default_skipped(name: &str) -> bool {
    name == ".git" || name == "target" || name == "result" || name.starts_with("result-")
}

struct Pattern {
        components: Vec<String>,
        dir_only: bool,
        anchored: bool,
}

impl Pattern {
        fn matches(&self, rel: &[String], is_dir: bool) -> bool {
        if self.dir_only && !is_dir {
            return false;
        }
        if self.anchored {
            rel.len() == self.components.len()
                && rel
                    .iter()
                    .zip(&self.components)
                    .all(|(part, pat)| glob_match(pat, part))
        } else {
            rel.iter().any(|part| glob_match(&self.components[0], part))
        }
    }
}

struct IgnoreSet {
    patterns: Vec<Pattern>,
}

impl IgnoreSet {
    fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

        fn parse(text: &str) -> Self {
        let mut patterns = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (line, dir_only) = match line.strip_suffix('/') {
                Some(stripped) => (stripped.trim_end(), true),
                None => (line, false),
            };
            if line.is_empty() {
                continue;
            }
            let anchored = line.contains('/');
            let components: Vec<String> = line
                .split('/')
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect();
            if components.is_empty() {
                continue;
            }
            patterns.push(Pattern {
                components,
                dir_only,
                anchored,
            });
        }
        Self { patterns }
    }

        fn matches(&self, rel: &[String], is_dir: bool) -> bool {
        self.patterns.iter().any(|p| p.matches(rel, is_dir))
    }
}

pub struct Filter {
    ignore: IgnoreSet,
    roots: Vec<PathBuf>,
}

impl Filter {
        pub fn skips(&self, file: &Path) -> bool {
        let Some(rel) = self
            .roots
            .iter()
            .find_map(|root| file.strip_prefix(root).ok())
        else {
            return false;
        };
        let parts: Vec<String> = rel
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        (1..=parts.len()).any(|len| {
            is_default_skipped(&parts[len - 1])
                || self.ignore.matches(&parts[..len], len < parts.len())
        })
    }
}

pub fn collect_files_filtered(
    paths: &[PathBuf],
    ignore_file: Option<&Path>,
    collect_others: bool,
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, Filter), String> {
    let ignore = match ignore_file {
        Some(path) => {
            let text = fs::read_to_string(path)
                .map_err(|e| format!("cannot read ignore file {}: {e}", path.display()))?;
            IgnoreSet::parse(&text)
        }
        None => IgnoreSet::empty(),
    };
    let mut out = Vec::new();
    let mut others = Vec::new();
    for path in paths {
        if !path.exists() {
            return Err(format!("path does not exist: {}", path.display()));
        }
        if path.is_dir() {
            walk_dir(path, &[], &ignore, &mut out, &mut others, collect_others)?;
        } else if path.extension().is_some_and(|ext| ext == "nix") {
            out.push(path.clone());
        } else if collect_others {
            others.push(path.clone());
        }
    }
    out.sort();
    if collect_others {
        others.sort();
    }
    let roots = paths.iter().filter(|path| path.is_dir()).cloned().collect();
    Ok((out, others, Filter { ignore, roots }))
}

const MAX_REF_WALK_DIRS: usize = 50_000;

fn is_ref_walk_skipped(name: &str) -> bool {
    is_default_skipped(name)
        || matches!(
            name,
            "node_modules" | ".direnv" | ".venv" | "venv" | "__pycache__" | ".cache"
        )
}

pub fn collect_nix_files_best_effort(paths: &[PathBuf]) -> (Vec<PathBuf>, Option<String>) {
    let mut out = Vec::new();
    let mut visited_dirs = 0usize;
    let mut visited_canon_dirs: HashSet<PathBuf> = HashSet::new();
    let mut incomplete = None;
    for path in paths {
        if !path.is_dir() {
            if path.is_file() && path.extension().is_some_and(|ext| ext == "nix") {
                out.push(path.clone());
            }
            continue;
        }
        if let Ok(canon) = fs::canonicalize(path) {
            if !visited_canon_dirs.insert(canon) {
                continue;
            }
        }
        walk_nix_best_effort(
            path,
            &mut out,
            &mut visited_dirs,
            &mut visited_canon_dirs,
            &mut incomplete,
        );
    }
    out.sort();
    (out, incomplete)
}

fn walk_nix_best_effort(
    dir: &Path,
    out: &mut Vec<PathBuf>,
    visited_dirs: &mut usize,
    visited_canon_dirs: &mut HashSet<PathBuf>,
    incomplete: &mut Option<String>,
) {
    if *visited_dirs >= MAX_REF_WALK_DIRS {
        if incomplete.is_none() {
            *incomplete = Some(format!("directory limit reached at {}", dir.display()));
        }
        return;
    }
    *visited_dirs += 1;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if incomplete.is_none() {
                *incomplete = Some(format!("cannot read {}: {e}", dir.display()));
            }
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                if incomplete.is_none() {
                    *incomplete = Some(format!("cannot read {}: {e}", dir.display()));
                }
                continue;
            }
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_ref_walk_skipped(&name) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                if incomplete.is_none() {
                    *incomplete = Some(format!("cannot stat {}: {e}", path.display()));
                }
                continue;
            }
        };
        if file_type.is_dir() {
            if let Ok(canon) = fs::canonicalize(&path) {
                if !visited_canon_dirs.insert(canon) {
                    continue;
                }
            }
            walk_nix_best_effort(&path, out, visited_dirs, visited_canon_dirs, incomplete);
        } else if file_type.is_file() {
            if path.extension().is_some_and(|ext| ext == "nix") {
                out.push(path);
            }
        } else if file_type.is_symlink() {
            match fs::metadata(&path) {
                Ok(meta) if meta.is_dir() => match fs::canonicalize(&path) {
                    Ok(canon) => {
                        if visited_canon_dirs.insert(canon) {
                            walk_nix_best_effort(
                                &path,
                                out,
                                visited_dirs,
                                visited_canon_dirs,
                                incomplete,
                            );
                        }
                    }
                    Err(e) => {
                        if incomplete.is_none() {
                            *incomplete = Some(format!("cannot resolve {}: {e}", path.display()));
                        }
                    }
                },
                Ok(meta) if meta.is_file() => {
                    if path.extension().is_some_and(|ext| ext == "nix") {
                        out.push(path);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    if path.extension().is_some_and(|ext| ext == "nix") && incomplete.is_none() {
                        *incomplete = Some(format!("cannot stat {}: {e}", path.display()));
                    }
                }
            }
        }
    }
}

fn walk_dir(
    dir: &Path,
    rel: &[String],
    ignore: &IgnoreSet,
    out: &mut Vec<PathBuf>,
    others: &mut Vec<PathBuf>,
    collect_others: bool,
) -> Result<(), String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_default_skipped(&name) {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if !file_type.is_dir()
            && !collect_others
            && path.extension().is_none_or(|ext| ext != "nix")
        {
            continue;
        }
        let mut rel_owned: Vec<String> = rel.to_vec();
        rel_owned.push(name);
        if file_type.is_dir() {
            if ignore.matches(&rel_owned, true) {
                continue;
            }
            walk_dir(&path, &rel_owned, ignore, out, others, collect_others)?;
        } else if file_type.is_file() {
            if ignore.matches(&rel_owned, false) {
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "nix") {
                out.push(path);
            } else if collect_others {
                others.push(path);
            }
        }
    }
    Ok(())
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut star_ti) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}
