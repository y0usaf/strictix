use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    project::{normalize, ProjectContext},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, NodeOrToken, SyntaxKind as K, SyntaxNode, SyntaxToken, TextRange,
};

pub struct DanglingPath;

impl Rule for DanglingPath {
    fn code(&self) -> &'static str {
        "dangling-path"
    }

    fn name(&self) -> &'static str {
        "Dangling path"
    }

    fn description(&self) -> &'static str {
        "Flags a path literal that does not exist on disk relative to the linted file — a hallucinated `imports = [ ./missing.nix ]` fails at evaluation. Paths probed with pathExists in the same file are skipped, and a file or directory of the linted project counts as existing (the files of a merged file live only in memory)."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        self.check_paths(model, None, diags);
    }

    fn check_file_project(
        &self,
        model: &SemanticModel,
        _config: &LintConfig,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        self.check_paths(model, project, diags);
    }
}

impl DanglingPath {
    fn check_paths(
        &self,
        model: &SemanticModel,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let Some(file) = model.path() else { return };
        let is_file = |path: &std::path::Path| {
            std::fs::metadata(path).is_ok_and(|meta| !meta.is_dir())
                || project.is_some_and(|p| p.contains(path))
        };
        let source = model.source();
        let root = model.root();
        let guarded = path_exists_arguments(root, source);
        let tokens = all_tokens(root);
        for (idx, tok) in tokens.iter().enumerate() {
            if tok.kind() != K::Path || !is_whole_path(&tokens, idx) {
                continue;
            }
            let text = tok.text(source);
            if text.starts_with('~') {
                continue;
            }
            if guarded.contains(&text) {
                continue;
            }
            let resolved = if text.starts_with('/') {
                std::path::PathBuf::from(text)
            } else {
                let Some(dir) = file.parent() else { continue };
                normalize(&dir.join(text))
            };
            let is_dir = match std::fs::metadata(&resolved) {
                Ok(meta) => Some(meta.is_dir()),
                Err(_) if project.is_some_and(|p| p.contains(&resolved)) => Some(false),
                Err(_) if project.is_some_and(|p| p.contains_dir(&resolved)) => Some(true),
                Err(_) => None,
            };
            match is_dir {
                None => {
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
                Some(is_dir) => {
                    if is_dir
                        && is_import_target(model, tok)
                        && !is_file(&resolved.join("default.nix"))
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
            if let Some(next) = tokens.get(idx + 1) {
                if tok.range().end() == next.range().start()
                    && matches!(next.kind(), K::Slash | K::InterpStart)
                {
                    continue;
                }
            }
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

fn all_tokens(node: &SyntaxNode) -> Vec<&SyntaxToken> {
    let mut tokens = Vec::new();
    walk_tokens(node, &mut |t| tokens.push(t));
    tokens
}

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

pub struct UnfilteredSrc;

impl Rule for UnfilteredSrc {
    fn code(&self) -> &'static str {
        "unfiltered-src"
    }

    fn name(&self) -> &'static str {
        "Unfiltered source directory"
    }

    fn description(&self) -> &'static str {
        "Flags `src = ./.` (or `src = ../.`), which copies the entire directory — including build artifacts and VCS metadata — into the Nix store on every evaluation. Opt-in."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn node_kind(&self) -> Option<K> {
        Some(K::Binding)
    }

    fn check_node(&self, node: &SyntaxNode, source: &str, diags: &mut Vec<Diagnostic>) {
        let Some(binding) = strictix_syntax::Binding::cast(node) else {
            return;
        };
        let Some(attrpath) = binding.attrpath() else {
            return;
        };
        let mut elems = attrpath.elements();
        let Some(strictix_syntax::AttrName::Ident(name)) = elems.next() else {
            return;
        };
        if elems.next().is_some() || name.text(source) != "src" {
            return;
        }
        let Some(val) = binding.value().and_then(crate::lib_helpers::unparen) else {
            return;
        };
        let strictix_syntax::Expr::Path(tok) = val else {
            return;
        };
        let text = tok.text(source);
        if text == "./." || text == "../." {
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "`src = {text};` copies the entire directory into the Nix store without filtering"
                    ),
                    binding.syntax().content_range(),
                )
                .with_help(
                    "filter the source tree with `lib.fileset.toSource` or `lib.cleanSource ./.`",
                ),
            );
        }
    }
}
