//! Directory walking with gitignore-style exclusion.
//!
//! Collects every `.nix` file under the given paths, recursively, with
//! two layers of exclusion: a hard-coded skip list (VCS/build/result
//! directories) and an optional ignore file (default
//! `./.strictixignore`, overridable with `--ignore-file`).
//!
//! Ignore patterns follow gitignore's simple rules as far as strictix
//! needs them: a pattern without `/` matches any path component at any
//! depth ("build" prunes every directory named build), a pattern with
//! `/` matches the relative path from the walk root exactly
//! ("foo/bar" prunes ./foo/bar), a trailing `/` restricts a pattern to
//! directories, and `*` is a wildcard inside one component (no
//! `**`). Ignore rules apply to directories (pruned before
//! descending) and to files alike. Comments and blank lines are
//! skipped.

use std::fs;
use std::path::{Path, PathBuf};

/// The component names that are always skipped during a walk,
/// regardless of the ignore file: VCS metadata, build output, and Nix
/// build results.
fn is_default_skipped(name: &str) -> bool {
    name == ".git" || name == "target" || name == "result" || name.starts_with("result-")
}

/// One parsed ignore pattern.
struct Pattern {
    /// The pattern split on `/` (a pattern without `/` has one
    /// component and matches any component at any depth).
    components: Vec<String>,
    /// A trailing `/` restricts the pattern to directories.
    dir_only: bool,
    /// Whether the pattern contained a `/` (then it matches the
    /// whole relative path, not single components).
    anchored: bool,
}

impl Pattern {
    /// Whether a pattern matches a path with the given relative
    /// components (which may be a directory or a file).
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

/// All ignore patterns in effect for one walk.
struct IgnoreSet {
    patterns: Vec<Pattern>,
}

impl IgnoreSet {
    fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    /// Parse an ignore file: one pattern per line, `#` comments and
    /// blank lines skipped. A trailing `/` marks a directory-only
    /// pattern; any other `/` anchors the pattern to the walk root.
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

    /// Whether any pattern covers the path with the given relative
    /// components.
    fn matches(&self, rel: &[String], is_dir: bool) -> bool {
        self.patterns.iter().any(|p| p.matches(rel, is_dir))
    }
}

/// Collect every `.nix` file under `paths`, sorted.
///
/// Directories are walked recursively. `ignore_file` is the parsed
/// ignore file when one was found (an explicitly requested file that
/// cannot be read is an error; a missing default is not). Explicitly
/// given paths are always honored: the skip list and ignore patterns
/// apply to entries discovered during the walk, not to the roots the
/// user asked for. A nonexistent path is an error.
///
/// # Errors
///
/// - A path that does not exist.
/// - An explicitly requested ignore file that cannot be read.
pub fn collect_files(
    paths: &[PathBuf],
    ignore_file: Option<&Path>,
) -> Result<Vec<PathBuf>, String> {
    let ignore = match ignore_file {
        Some(path) => {
            let text = fs::read_to_string(path)
                .map_err(|e| format!("cannot read ignore file {}: {e}", path.display()))?;
            IgnoreSet::parse(&text)
        }
        None => IgnoreSet::empty(),
    };
    let mut out = Vec::new();
    for path in paths {
        if !path.exists() {
            return Err(format!("path does not exist: {}", path.display()));
        }
        if path.is_dir() {
            walk_dir(path, &[], &ignore, &mut out)?;
        } else if path.extension().is_some_and(|ext| ext == "nix") {
            out.push(path.clone());
        }
    }
    out.sort();
    Ok(out)
}

/// Recursively collect `.nix` files under `dir` (the walk root is
/// `root`, used to compute relative paths for anchored patterns).
fn walk_dir(
    dir: &Path,
    rel: &[String],
    ignore: &IgnoreSet,
    out: &mut Vec<PathBuf>,
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
        let mut rel_owned: Vec<String> = rel.to_vec();
        rel_owned.push(name.clone());
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            if ignore.matches(&rel_owned, true) {
                continue; // prune the directory
            }
            walk_dir(&path, &rel_owned, ignore, out)?;
        } else if file_type.is_file() {
            if ignore.matches(&rel_owned, false) {
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "nix") {
                out.push(path);
            }
        }
    }
    Ok(())
}

/// Match one path component against a component pattern where `*`
/// matches any run of characters (including the empty run). Literal
/// characters match themselves; there is no `**` and no character
/// class support.
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
