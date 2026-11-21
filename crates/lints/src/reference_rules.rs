use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    AstNode, AttrName, Expr, IndStringExpr, SelectExpr, StringPart, SyntaxKind, TextRange,
};

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

const VALID_BUILTINS: &[&str] = &[
    "abort",
    "add",
    "addDrvOutputDependencies",
    "addErrorContext",
    "all",
    "any",
    "appendContext",
    "attrNames",
    "attrValues",
    "baseNameOf",
    "bitAnd",
    "bitOr",
    "bitXor",
    "break",
    "builtins",
    "catAttrs",
    "ceil",
    "compareVersions",
    "concatLists",
    "concatMap",
    "concatStringsSep",
    "convertHash",
    "currentSystem",
    "currentTime",
    "deepSeq",
    "derivation",
    "derivationStrict",
    "dirOf",
    "div",
    "elem",
    "elemAt",
    "false",
    "fetchGit",
    "fetchMercurial",
    "fetchTarball",
    "fetchTree",
    "fetchurl",
    "filter",
    "filterSource",
    "findFile",
    "flakeRefToString",
    "floor",
    "foldl'",
    "fromJSON",
    "fromTOML",
    "functionArgs",
    "genericClosure",
    "genList",
    "getAttr",
    "getContext",
    "getEnv",
    "getFlake",
    "groupBy",
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
    "langVersion",
    "length",
    "lessThan",
    "listToAttrs",
    "map",
    "mapAttrs",
    "match",
    "mul",
    "nixPath",
    "nixVersion",
    "null",
    "parseDrvName",
    "parseFlakeRef",
    "partition",
    "path",
    "pathExists",
    "placeholder",
    "readDir",
    "readFile",
    "readFileType",
    "removeAttrs",
    "replaceStrings",
    "scopedImport",
    "seq",
    "sort",
    "split",
    "splitVersion",
    "storeDir",
    "storePath",
    "stringLength",
    "sub",
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
    "true",
    "tryEval",
    "typeOf",
    "unsafeDiscardOutputDependency",
    "unsafeDiscardStringContext",
    "unsafeGetAttrPos",
    "warn",
    "zipAttrsWith",
];

pub(crate) fn is_global(name: &str) -> bool {
    NIX_GLOBALS.contains(&name)
}

fn indented_string_fix_range(model: &SemanticModel<'_>, ref_range: TextRange) -> Option<TextRange> {
    if !model.source().contains("''") {
        return None;
    }
    let mut found = None;
    model.root().walk_containing(ref_range.start(), |node| {
        if node.kind() != SyntaxKind::IndStringExpr {
            return false;
        }
        let Some(ind) = IndStringExpr::cast(node) else {
            return false;
        };
        for part in ind.parts() {
            if let StringPart::Interp(interp) = part {
                if interp.syntax().range().contains(ref_range.start()) {
                    if let Some(Expr::Ident(token)) = interp.expr() {
                        if token.range() == ref_range {
                            found = Some(interp.syntax().range());
                            return true;
                        }
                    }
                }
            }
        }
        false
    });
    found
}

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
            if model.resolve(base).is_some() {
                continue;
            }
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
