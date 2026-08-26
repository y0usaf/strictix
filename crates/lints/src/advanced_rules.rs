//! Conservative higher-confidence rules for common generated-Nix traps.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, Expr, IfExpr, ListExpr, RecAttrsetExpr, SyntaxKind,
};

fn nodes(
    root: &strictix_syntax::SyntaxNode,
    kind: SyntaxKind,
) -> impl Iterator<Item = &strictix_syntax::SyntaxNode> {
    root.descendants().filter(move |n| n.kind() == kind)
}
fn expr_text<'a>(e: Expr<'a>, source: &'a str) -> &'a str {
    e.content_text(source)
}

pub struct DuplicateFunctionArgument;
impl Rule for DuplicateFunctionArgument {
    fn code(&self) -> &'static str {
        "duplicate-function-argument"
    }
    fn name(&self) -> &'static str {
        "Duplicate function argument"
    }
    fn description(&self) -> &'static str {
        "Flags a function application whose adjacent arguments are identical source expressions."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::ApplyExpr) {
            let Some(a) = ApplyExpr::cast(n) else {
                continue;
            };
            let Some(arg) = a.arg() else { continue };
            let Some(Expr::Apply(inner)) = a.func() else {
                continue;
            };
            let Some(prev) = inner.arg() else { continue };
            if expr_text(arg, s) == expr_text(prev, s) {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "function receives duplicate adjacent argument `{}`",
                        expr_text(arg, s)
                    ),
                    arg.content_range(),
                ));
            }
        }
    }
}

pub struct UnreachableBranch;
impl Rule for UnreachableBranch {
    fn code(&self) -> &'static str {
        "unreachable-branch"
    }
    fn name(&self) -> &'static str {
        "Unreachable branch"
    }
    fn description(&self) -> &'static str {
        "Flags the branch of an if-expression that a literal condition can never evaluate."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::IfExpr) {
            let Some(i) = IfExpr::cast(n) else { continue };
            let Some(Expr::Ident(t)) = i.cond() else {
                continue;
            };
            let dead = match t.text(s) {
                "true" => i.else_branch(),
                "false" => i.then_branch(),
                _ => None,
            };
            if let Some(e) = dead {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "branch is unreachable because the condition is constant",
                    e.content_range(),
                ));
            }
        }
    }
}

pub struct UnsafeWithShadowing;
impl Rule for UnsafeWithShadowing {
    fn code(&self) -> &'static str {
        "unsafe-with-shadowing"
    }
    fn name(&self) -> &'static str {
        "Unsafe with shadowing"
    }
    fn description(&self) -> &'static str {
        "Flags names resolved through with, because their meaning depends on the runtime attribute set."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        for r in m.references() {
            if r.via_with.is_some() {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "reference '{}' depends on a with-scope",
                        r.name.text(m.source())
                    ),
                    r.name.range(),
                ));
            }
        }
    }
}

pub struct DynamicImport;
impl Rule for DynamicImport {
    fn code(&self) -> &'static str {
        "dynamic-import"
    }
    fn name(&self) -> &'static str {
        "Dynamic import"
    }
    fn description(&self) -> &'static str {
        "Flags imports whose argument is not a static path or string."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        for site in m.import_sites() {
            let raw =
                m.source()[site.path_range.start() as usize..site.path_range.end() as usize].trim();
            let static_ok = raw.starts_with("./")
                || raw.starts_with("../")
                || raw.starts_with("/")
                || raw.starts_with("~")
                || raw.starts_with('"')
                || raw.starts_with("\'\'");
            if !static_ok {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "import path is dynamic and cannot be tracked",
                    site.path_range,
                ));
            }
        }
    }
}

pub struct ImportInList;
impl Rule for ImportInList {
    fn code(&self) -> &'static str {
        "import-in-list"
    }
    fn name(&self) -> &'static str {
        "Import in list"
    }
    fn description(&self) -> &'static str {
        "Flags a bare import application inside a list, where the imported value is often confused with its path."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::ListExpr) {
            let Some(list) = ListExpr::cast(n) else {
                continue;
            };
            let items: Vec<_> = list.items().collect();
            for pair in items.windows(2) {
                if let [Expr::Ident(name), arg] = pair {
                    if name.text(s) == "import" && m.resolve(name).is_none() {
                        d.push(Diagnostic::new(
                            self.code(),
                            self.severity(),
                            "import application appears directly in a list",
                            name.range(),
                        ));
                        let _ = arg;
                    }
                }
            }
        }
    }
}

pub struct BuiltinArity;
impl Rule for BuiltinArity {
    fn code(&self) -> &'static str {
        "builtin-arity"
    }
    fn name(&self) -> &'static str {
        "Builtin arity"
    }
    fn description(&self) -> &'static str {
        "Flags extra arguments passed to builtins with fixed arity."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::ApplyExpr) {
            let Some(a) = ApplyExpr::cast(n) else {
                continue;
            };
            let Some(Expr::Apply(inner)) = a.func() else {
                continue;
            };
            let Some(Expr::Select(sel)) = inner.func() else {
                continue;
            };
            let Some(Expr::Ident(base)) = sel.base() else {
                continue;
            };
            if base.text(s) != "builtins" {
                continue;
            };
            let Some(ap) = sel.attrpath() else { continue };
            let Some(strictix_syntax::AttrName::Ident(f)) = ap.elements().next() else {
                continue;
            };
            let name = f.text(s);
            let fixed = matches!(
                name,
                "length"
                    | "head"
                    | "tail"
                    | "isNull"
                    | "isAttrs"
                    | "isBool"
                    | "isInt"
                    | "isList"
                    | "isString"
                    | "isFunction"
            );
            if fixed {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("builtin '{name}' receives too many arguments"),
                    a.arg().map(|x| x.content_range()).unwrap_or(n.range()),
                ));
            }
        }
    }
}

pub struct SuspiciousRecursion;
impl Rule for SuspiciousRecursion {
    fn code(&self) -> &'static str {
        "suspicious-recursion"
    }
    fn name(&self) -> &'static str {
        "Suspicious recursion"
    }
    fn description(&self) -> &'static str {
        "Flags a recursive attribute whose value directly references its own name outside a function or data literal."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::RecAttrsetExpr) {
            let Some(r) = RecAttrsetExpr::cast(n) else {
                continue;
            };
            let Some(set) = r.attrset() else { continue };
            for item in set.items() {
                let AttrItem::Binding(b) = item else { continue };
                let Some(ek) = b.attrpath().and_then(|p| p.elements().next()) else {
                    continue;
                };
                let strictix_syntax::AttrName::Ident(name) = ek else {
                    continue;
                };
                let Some(v) = b.value() else { continue };
                if matches!(v,Expr::Ident(t) if t.text(s)==name.text(s)) {
                    d.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!(
                            "recursive attribute '{}' directly references itself",
                            name.text(s)
                        ),
                        name.range(),
                    ));
                }
            }
        }
    }
}

pub struct SuspiciousImportArgument;
impl Rule for SuspiciousImportArgument {
    fn code(&self) -> &'static str {
        "suspicious-import-argument"
    }
    fn name(&self) -> &'static str {
        "Suspicious import argument"
    }
    fn description(&self) -> &'static str {
        "Flags imports whose argument is a literal that cannot be a path."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        for site in m.import_sites() {
            let raw =
                m.source()[site.path_range.start() as usize..site.path_range.end() as usize].trim();
            if raw.chars().next().is_some_and(|c| {
                c.is_ascii_digit() || c.is_ascii_alphabetic() || c == '{' || c == '['
            }) {
                d.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    "import argument is not a path or string",
                    site.path_range,
                ));
            }
        }
    }
}
