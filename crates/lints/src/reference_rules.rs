//! Reference and builtin rules: `undefined-variable` and `unknown-builtin`.
//!
//! Both are file rules that read the lazy [SemanticModel] once. They
//! target the classic failure modes of AI-generated Nix: referencing a
//! name that binds to nothing (often a `$HOME`/`$PATH` shell variable
//! that the model meant literally), and reaching for a wrong member on
//! the `builtins` constant (conflating lib helpers with their builtin
//! counterparts, e.g. `builtins.filterAttrs`).

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    AstNode, AttrName, Expr, IndStringExpr, SelectExpr, StringPart, SyntaxKind, TextRange,
};

/// Names Nix exposes as global (bare) values: keywords, builtin
/// functions, and the polymorphic `builtins` set. A reference to one of
/// these never means "undefined variable" — the evaluator resolves them
/// implicitly, so the linter must skip them.
const NIX_GLOBALS: &[&str] = &[
    "true",
    "false",
    "null",
    "import",
    "abort",
    "baseNameOf",
    "break",
    "dirOf",
    "derivation",
    "derivationStrict",
    "fetchGit",
    "fetchMercurial",
    "fetchTarball",
    "fetchTree",
    "fromTOML",
    "isNull",
    "map",
    "placeholder",
    "removeAttrs",
    "scopedImport",
    "throw",
    "toString",
    "builtins",
];

/// The exhaustive set of real Nix 2.x `builtins` members. Only these
/// names are allowed after a single `builtins.` hop; anything else is a
/// mistake (usually AI invented a lib helper as a builtin).
const VALID_BUILTINS: &[&str] = &[
    "add",
    "addDrvOutputDependencies",
    "addErrorContext",
    "all",
    "any",
    "attrNames",
    "attrValues",
    "baseNameOf",
    "bitAnd",
    "bitOr",
    "bitXor",
    "break",
    "compareVersions",
    "concatLists",
    "concatMap",
    "concatStringsSep",
    "currentSystem",
    "currentTime",
    "deepSeq",
    "derivation",
    "derivationStrict",
    "dirOf",
    "elem",
    "elemAt",
    "fetchGit",
    "fetchMercurial",
    "fetchTarball",
    "fetchTree",
    "filter",
    "filterSource",
    "fromJSON",
    "fromTOML",
    "functionArgs",
    "genList",
    "getAttr",
    "getContext",
    "getEnv",
    "getFlake",
    "hasAttr",
    "hasContext",
    "hashFile",
    "hashString",
    "head",
    "import",
    "intersectAttrs",
    "isAttrs",
    "isBool",
    "isFloat",
    "isFunction",
    "isInt",
    "isList",
    "isNull",
    "isPath",
    "isString",
    "length",
    "lessThan",
    "listToAttrs",
    "map",
    "mapAttrs",
    "path",
    "pathExists",
    "placeholder",
    "parseDrvName",
    "readDir",
    "readFile",
    "removeAttrs",
    "replaceStrings",
    "scopedImport",
    "seq",
    "sort",
    "split",
    "stringLength",
    "substring",
    "tail",
    "throw",
    "toFile",
    "toJSON",
    "toPath",
    "toString",
    "toXML",
    "trace",
    "traceVerbose",
    "typeOf",
];

/// Whether `name` is the Nix `builtins` constant or one of the values
/// it exposes in scope implicitly — a reference never flags these.
fn is_global(name: &str) -> bool {
    NIX_GLOBALS.contains(&name)
}

/// The full `${name}` interpolation range when `ref_range` (the ident
/// token) is the entire interpolated expression of an *indented* string
/// literal. Indented strings are the shell-script wrapper where an
/// unbound `$HOME`-style name is almost always meant literally; a
/// double-quoted string has no escape-a-literal idiom we can safely
/// invent, so no fix there.
///
/// Returns the whole `${...}` span (delimiters included) so the fix can
/// rewrite it to `''${...}` — the Nix escape that emits a literal
/// `${name}` instead of interpolating it.
fn indented_string_fix_range(model: &SemanticModel<'_>, ref_range: TextRange) -> Option<TextRange> {
    for node in model.root().descendants() {
        if node.kind() != SyntaxKind::IndStringExpr {
            continue;
        }
        let Some(ind) = IndStringExpr::cast(node) else {
            continue;
        };
        for part in ind.parts() {
            if let StringPart::Interp(interp) = part {
                if let Some(Expr::Ident(token)) = interp.expr() {
                    if token.range() == ref_range {
                        return Some(interp.syntax().range());
                    }
                }
            }
        }
    }
    None
}

/// Flags a reference that resolves to nothing in scope.
///
/// The classic AI-slop failure: a shell `$HOME`/`$PATH` written as a
/// Nix interpolation (`''${HOME}''`) meant as shell text, which the Nix
/// evaluator rejects with `undefined variable 'HOME'`. Also catches
/// general dead/id references. A name is "in scope" when it resolves to
/// a binding, is covered by an enclosing `with`, or is part of the Nix
/// globals set.
pub struct UndefinedVariable;

impl Rule for UndefinedVariable {
    fn code(&self) -> &'static str {
        "undefined-variable"
    }

    fn name(&self) -> &'static str {
        "Undefined variable"
    }

    fn description(&self) -> &'static str {
        "Flags a reference to a name that resolves to nothing in scope. The Nix evaluator aborts on such a reference, so it is an error; when it sits inside an indented-string interpolation (the classic shell-text mistake) this rule offers to escape it."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for reference in model.references() {
            // A lexical binding or an enclosing with covers the name;
            // it is not undefined.
            if reference.resolved.is_some() || reference.via_with.is_some() {
                continue;
            }
            let name = reference.name.text(model.source());
            if is_global(name) {
                continue;
            }
            let range = reference.name.range();
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!("undefined variable '{name}' in expression position is never bound"),
                range,
            );
            // Only inside an indented-string `${name}` is there a safe
            // rewrite: escape it so the shell keeps the literal. A
            // double-quoted string or plain expression has no generic
            // fix, so none is offered.
            if let Some(interp_range) = indented_string_fix_range(model, range) {
                let replacement = format!(
                    "''{}",
                    &model.source()[interp_range.start() as usize..interp_range.end() as usize]
                );
                diag = diag.with_fix(
                    Fix::new("escape literal with ''$ (only in indented string)")
                        .edit(interp_range, replacement),
                );
            }
            diags.push(diag);
        }
    }
}

/// Flags `builtins.<name>` where `<name>` is not a real Nix builtin.
///
/// AI often reaches into `builtins` for a lib helper
/// (`builtins.filterAttrs` for `builtins.filter`) or an invented name.
/// Fires only on a single-hop select (`builtins.X`) on the global
/// `builtins` constant — never a shadowed `builtins`, never an
/// attribute path past the first hop (`` `builtins.map.<something>`),
/// and never a `builtins ? attr` feature-probe.
pub struct UnknownBuiltin;

impl Rule for UnknownBuiltin {
    fn code(&self) -> &'static str {
        "unknown-builtin"
    }

    fn name(&self) -> &'static str {
        "Unknown builtin"
    }

    fn description(&self) -> &'static str {
        "Flags a member of the builtins constant that does not exist. AI-generated Nix frequently writes `builtins.filterAttrs` or similar, reaching for a lib helper on the builtins set. Only the real builtins are accepted."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            if node.kind() != SyntaxKind::SelectExpr {
                continue;
            }
            let Some(select) = SelectExpr::cast(node) else {
                continue;
            };
            let Some(Expr::Ident(base)) = select.base() else {
                continue;
            };
            if base.text(source) != "builtins" {
                continue;
            }
            // A `builtins` that resolves to a binding is a shadowed
            // name, not the constant — skip.
            if model.resolve(&base).is_some() {
                continue;
            }
            // Only the single-hop form `builtins.<name>` is this rule's
            // business; deeper attrpaths (`builtins.map.<Y>`) are not.
            let Some(attrpath) = select.attrpath() else {
                continue;
            };
            let mut elements = attrpath.elements();
            let Some(first) = elements.next() else {
                continue;
            };
            let AttrName::Ident(attr) = first else {
                continue;
            };
            if elements.next().is_some() {
                continue;
            }
            let name = attr.text(source);
            if VALID_BUILTINS.contains(&name) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("builtin '{name}' does not exist"),
                attr.range(),
            ));
        }
    }
}