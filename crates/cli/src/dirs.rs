use std::{
    fs,
    io::{self, Error, ErrorKind},
    path::{Path, PathBuf},
};

use ignore::{
    Error as IgnoreError, Match,
    gitignore::{Gitignore, GitignoreBuilder},
};

#[derive(Debug)]
pub struct Walker {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    unrestricted: bool,
    extra_ignores: Vec<String>,
}

impl Walker {
    pub fn new<P: AsRef<Path>>(
        target: P,
        unrestricted: bool,
        extra_ignores: Vec<String>,
    ) -> io::Result<Self> {
        let target = target.as_ref().to_path_buf();
        if !target.exists() {
            Err(Error::new(
                ErrorKind::NotFound,
                format!("file not found: {}", target.display()),
            ))
        } else if target.is_dir() {
            Ok(Self {
                dirs: vec![target],
                files: vec![],
                unrestricted,
                extra_ignores,
            })
        } else {
            Ok(Self {
                dirs: vec![],
                files: vec![target],
                unrestricted,
                extra_ignores,
            })
        }
    }
}

impl Iterator for Walker {
    type Item = PathBuf;
    fn next(&mut self) -> Option<Self::Item> {
        self.files.pop().or_else(|| {
            while let Some(dir) = self.dirs.pop() {
                // Rebuild ignores per directory so nested `.gitignore` files participate in
                // traversal decisions.
                let Ok(ignore) = build_ignore_set(&self.extra_ignores, &dir, self.unrestricted)
                else {
                    continue;
                };
                if dir.is_dir()
                    && let Match::None | Match::Whitelist(_) = ignore.matched(&dir, true)
                {
                    let mut found = false;
                    let Ok(read_dir) = fs::read_dir(&dir) else {
                        continue;
                    };
                    for entry in read_dir {
                        let Ok(entry) = entry else {
                            continue;
                        };
                        let path = entry.path();
                        if path.is_dir() {
                            self.dirs.push(path);
                        } else if path.is_file()
                            && let Match::None | Match::Whitelist(_) = ignore.matched(&path, false)
                        {
                            found = true;
                            self.files.push(path);
                        }
                    }
                    if found {
                        break;
                    }
                }
            }
            self.files.pop()
        })
    }
}

pub fn build_ignore_set<P: AsRef<Path>>(
    ignore: &[String],
    target: P,
    unrestricted: bool,
) -> Result<Gitignore, IgnoreError> {
    // Extra ignore patterns are applied with gitignore semantics from the currently visited
    // directory, alongside any `.gitignore` found there.
    let target = target.as_ref();
    let gitignore_dir = if target.is_dir() {
        target.to_path_buf()
    } else {
        target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let gitignore_path = gitignore_dir.join(".gitignore");

    let mut gitignore = GitignoreBuilder::new(&gitignore_dir);

    // if we are to "restrict" aka "respect" .gitignore, then
    // add globs from gitignore path as well
    if !unrestricted {
        gitignore.add(&gitignore_path);

        // ignore .git by default, nobody cares about .git, i'm sure
        gitignore.add_line(None, ".git")?;

        // ignore npins by default, it's auto-generated
        gitignore.add_line(None, "npins")?;
    }

    for i in ignore {
        gitignore.add_line(None, i.as_str())?;
    }

    gitignore.build()
}

pub fn walk_nix_files<P: AsRef<Path>>(
    target: P,
    ignore: &[String],
    unrestricted: bool,
) -> Result<impl Iterator<Item = PathBuf>, io::Error> {
    let walker = Walker::new(target, unrestricted, ignore.to_vec())?;
    Ok(walker.filter(|path: &PathBuf| matches!(path.extension(), Some(e) if e == "nix")))
}
