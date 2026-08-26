//! Path-literal lints: hallucinated (nonexistent) paths, the `a/b`
//! division trap, and impure `<...>` search paths.
//!
//! All three are file rules over the raw token stream: Path and
//! SearchPath are single tokens, but the lexer splits interpolated
//! paths (`./x/${v}`) and numeric division traps (`4/2`) into adjacent
//! fragments, so classifying one Path token requires its physical
//! neighbors — parent-node context alone cannot tell a whole path
//! literal from a fragment.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange,
};

/// Flags path literals that do not exist on disk.
pub struct DanglingPath;

impl Rule for DanglingPath {
    fn code(&self) -> &'static str {
        "dangling-path"
    }

    fn name(&self) -> &'static str {
        "Dangling path"
    }

    fn description(&self) -> &'static str {
        "Flags a path literal that does not exist on disk relative to the linted file — a hallucinated `imports = [ ./missing.nix ]` fails at evaluation. Paths probed with pathExists in the same file are skipped."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        // Anonymous sources (tests, stdin) have no on-disk location to
        // resolve relative paths against; without one every existence
        // check would be a guess, so the rule stays silent entirely.
        let Some(file) = model.path() else { return };
        let source = model.source();
        let root = model.root();
        let guarded = path_exists_arguments(root, source);
        let tokens = all_tokens(root);
        for (idx, tok) in tokens.iter().enumerate() {
            if tok.kind() != K::Path || !is_whole_path(&tokens, idx) {
                continue;
            }
            let text = tok.text(source);
            // `~` expands against $HOME at evaluation time; existence
            // depends on the environment, not the repository, so
            // nothing can be proven about it.
            if text.starts_with('~') {
                continue;
            }
            // A path probed with pathExists is *expected* to possibly
            // not exist — that is the whole point of the probe.
            if guarded.contains(&text) {
                continue;
            }
            let resolved = if text.starts_with('/') {
                std::path::PathBuf::from(text)
            } else {
                // Relative to the directory containing the linted file.
                let Some(dir) = file.parent() else { continue };
                dir.join(text)
            };
            // metadata follows symlinks: a live symlink target counts
            // as existing, a broken one does not — matching what Nix
            // evaluation would see.
            match std::fs::metadata(&resolved) {
                Err(_) => {
                    diags.push(
                        Diagnostic::new(
                            "dangling-path",
                            Severity::Error,
                            format!(
                                "path '{text}' does not exist (resolved to {})",
                                resolved.display()
                            ),
                            tok.range(),
                        )
                        .with_help("create the file or correct the path"),
                    );
                }
                Ok(meta) => {
                    // Importing a directory loads its default.nix; a
                    // directory that exists but lacks one still fails
                    // at evaluation. Only import targets are held to
                    // this — a bare directory path is legal data.
                    if meta.is_dir()
                        && is_import_target(model, tok)
                        && std::fs::metadata(resolved.join("default.nix")).is_err()
                    {
                        diags.push(
                            Diagnostic::new(
                                "dangling-path",
                                Severity::Error,
                                format!("directory '{text}' has no default.nix"),
                                tok.range(),
                            )
                            .with_help("importing a directory loads its default.nix"),
                        );
                    }
                }
            }
        }
    }
}

/// Flags `4/2`-style path literals that look like intended division.
pub struct AccidentalPathDivision;

impl Rule for AccidentalPathDivision {
    fn code(&self) -> &'static str {
        "accidental-path-division"
    }

    fn name(&self) -> &'static str {
        "Path that looks like division"
    }

    fn description(&self) -> &'static str {
        "Flags a bare numeric path literal like `4/2`: Nix lexes digits around an unspaced slash as a relative path, not division. Add spaces around `/` to divide."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let tokens = all_tokens(model.root());
        for (idx, tok) in tokens.iter().enumerate() {
            if tok.kind() != K::Path {
                continue;
            }
            // A trailing Slash/InterpStart fragment means this is part
            // of an interpolated path composite, never a division trap.
            if let Some(next) = tokens.get(idx + 1) {
                if tok.range().end() == next.range().start()
                    && matches!(next.kind(), K::Slash | K::InterpStart)
                {
                    continue;
                }
            }
            // Our lexer starts numbers before paths, so `4/2` arrives
            // as Int `4` + adjacent Path `/2`; glue the fragments back
            // together to test the literal the user actually wrote.
            let range = match tokens.get(idx.wrapping_sub(1)) {
                Some(prev)
                    if idx > 0
                        && prev.kind() == K::Int
                        && prev.range().end() == tok.range().start() =>
                {
                    TextRange::new(prev.range().start(), tok.range().end())
                }
                _ => tok.range(),
            };
            let text = &source[range.start() as usize..range.end() as usize];
            // Exactly `digits / digits`: both split halves being all
            // ASCII digits also enforces the single slash and rules
            // out `./`, `../`, `/`, and `~` prefixes.
            let Some((num, den)) = text.split_once('/') else {
                continue;
            };
            if num.is_empty()
                || den.is_empty()
                || !num.bytes().all(|b| b.is_ascii_digit())
                || !den.bytes().all(|b| b.is_ascii_digit())
            {
                continue;
            }
            // The fix deliberately changes semantics (path literal ->
            // integer division): that is the entire point of the rule.
            let fix = Fix::new("interpret as division").edit(range, format!("{num} / {den}"));
            diags.push(
                Diagnostic::new(
                    "accidental-path-division",
                    Severity::Warning,
                    format!("Nix parses '{text}' as a relative path, not division"),
                    range,
                )
                .with_help("add spaces around `/` to divide")
                .with_fix(fix),
            );
        }
    }
}

/// Flags `<nixpkgs>`-style search-path references.
///
/// A `<...>` literal is resolved by scanning NIX_PATH when the
/// expression is *evaluated*, so the result depends on the caller's
/// environment: two machines evaluating the same file can get different
/// nixpkgs. Pure evaluation (flakes) forbids NIX_PATH lookups outright,
/// so the reference is also a hard error there. There is no safe
/// mechanical rewrite — the right replacement (flake input, pinned
/// fetcher, function argument) is a design decision — hence no fix.
pub struct SearchPathReference;

impl Rule for SearchPathReference {
    fn code(&self) -> &'static str {
        "search-path-reference"
    }

    fn name(&self) -> &'static str {
        "Search path reference"
    }

    fn description(&self) -> &'static str {
        "Flags `<...>` search-path literals: they resolve through NIX_PATH at evaluation time, so the build is impure and breaks under flakes' pure evaluation."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        walk_tokens(model.root(), &mut |tok| {
            if tok.kind() != K::SearchPath {
                return;
            }
            let text = tok.text(source);
            diags.push(
                Diagnostic::new(
                    "search-path-reference",
                    Severity::Warning,
                    format!(
                        "'{text}' resolves through NIX_PATH at evaluation time; impure and incompatible with pure flake evaluation"
                    ),
                    tok.range(),
                )
                .with_help(
                    "pin the dependency explicitly (flake input, fetcher, or function argument) instead of NIX_PATH",
                ),
            );
        });
    }
}

// --- helpers ----------------------------------------------------------

/// Every token in source order, trivia included. The lexer splits
/// composite paths into physically adjacent fragments, so path rules
/// need each Path token's flat-stream neighbors — which tree-shaped
/// walks cannot provide across node boundaries.
fn all_tokens(node: &SyntaxNode) -> Vec<&SyntaxToken> {
    let mut tokens = Vec::new();
    walk_tokens(node, &mut |t| tokens.push(t));
    tokens
}

/// Whether the Path token at `idx` is a whole path literal on its own.
///
/// Fragments betray themselves by touching a neighbor with no gap:
/// `./x/${v}` lexes as Path `./x` + Slash + InterpStart..., `4/2` as
/// Int `4` + Path `/2`, and `/a/${v}/b` puts InterpEnd right before the
/// trailing Path `/b`. Trivia between tokens breaks adjacency, which is
/// exactly right — `./x /2` really is two separate expressions.
fn is_whole_path(tokens: &[&SyntaxToken], idx: usize) -> bool {
    let tok = tokens[idx];
    if idx > 0 {
        let prev = tokens[idx - 1];
        if prev.range().end() == tok.range().start()
            && matches!(prev.kind(), K::Int | K::Float | K::InterpEnd | K::Slash)
        {
            return false;
        }
    }
    if let Some(next) = tokens.get(idx + 1) {
        if tok.range().end() == next.range().start()
            && matches!(next.kind(), K::Slash | K::InterpStart)
        {
            return false;
        }
    }
    true
}

/// The trimmed argument text of every `pathExists` application in the
/// file. Matched by callee suffix so the bare builtin and every
/// namespaced spelling (`builtins.pathExists`, `lib.pathExists`,
/// `lib.filesystem.pathExists`) all count.
fn path_exists_arguments<'a>(root: &'a SyntaxNode, source: &'a str) -> Vec<&'a str> {
    let mut args = Vec::new();
    walk_nodes(root, &mut |node| {
        if node.kind() != K::ApplyExpr {
            return;
        }
        let Some(apply) = ApplyExpr::cast(node) else {
            return;
        };
        let Some(func) = apply.func() else { return };
        // Node ranges flush leading trivia into the node; trim it away
        // before comparing spellings.
        let callee = func.text(source).trim();
        if callee != "pathExists" && !callee.ends_with(".pathExists") {
            return;
        }
        if let Some(arg) = apply.arg() {
            args.push(arg.text(source).trim());
        }
    });
    args
}

/// Whether `tok` is the argument of an `import` call. Import sites are
/// recorded by the semantic model; containment (rather than equality)
/// tolerates the leading trivia that node ranges flush into the
/// argument expression.
fn is_import_target(model: &SemanticModel, tok: &SyntaxToken) -> bool {
    model.import_sites().iter().any(|site| {
        site.path_range.start() <= tok.range().start() && tok.range().end() <= site.path_range.end()
    })
}

fn walk_nodes<'a>(node: &'a SyntaxNode, f: &mut impl FnMut(&'a SyntaxNode)) {
    f(node);
    for child in node.child_nodes() {
        walk_nodes(child, f);
    }
}

fn walk_tokens<'a>(node: &'a SyntaxNode, f: &mut impl FnMut(&'a SyntaxToken)) {
    for child in node.children() {
        match child {
            NodeOrToken::Token(t) => f(t),
            NodeOrToken::Node(n) => walk_tokens(n, f),
        }
    }
}
