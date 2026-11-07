//! Conservative higher-confidence rules for common generated-Nix traps.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    ApplyExpr, AstNode, AttrItem, AttrName, Expr, IfExpr, InheritStmt, ListExpr, RecAttrsetExpr,
    SyntaxKind, SyntaxToken, TextRange, WithExpr,
};

fn is_trackable_flake_input(raw: &str) -> bool {
    let raw = raw.trim();
    let raw = raw.strip_prefix('(').unwrap_or(raw).trim();
    let raw = raw.strip_suffix(')').unwrap_or(raw).trim();
    let raw = raw.strip_prefix("toString").unwrap_or(raw).trim();
    let Some((root, rest)) = raw.split_once('.') else {
        return false;
    };
    if root != "inputs" && root != "flakeInputs" {
        return false;
    };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

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
        "Flags a function application whose adjacent arguments are identical source expressions. A generator syntax tree (the argument of a `generators.<language>.render` call) is exempt: a constructor takes the same name twice as often as the code does (`name = name`)."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let trees = crate::suggested_rules::tree_ranges(m);
        let s = m.source();
        for n in nodes(m.root(), SyntaxKind::ApplyExpr) {
            if crate::suggested_rules::in_ranges(&trees, n.range()) {
                continue;
            }
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
        "Flags names resolved through with, because their meaning depends on the runtime attribute set. Fixes a non-nested `with S;` over an identifier or static select chain by removing it and prefixing each with-resolved name with `S.`."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let mut seen = vec![false; m.with_sites().len()];
        for r in m.references() {
            let Some(site) = r.via_with else {
                continue;
            };
            let diagnostic = Diagnostic::new(
                self.code(),
                self.severity(),
                format!(
                    "reference '{}' depends on a with-scope",
                    r.name.text(m.source())
                ),
                r.name.range(),
            );
            if std::mem::replace(&mut seen[site], true) {
                d.push(diagnostic);
                continue;
            }
            d.push(match with_removal_fix(m, site) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            });
        }
    }
}

fn static_scope_chain<'a>(e: Expr<'a>, s: &str) -> Option<(&'a SyntaxToken, String)> {
    match e {
        Expr::Ident(t) => Some((t, t.text(s).to_string())),
        Expr::Select(select) if select.default().is_none() => {
            let (root, mut text) = static_scope_chain(select.base()?, s)?;
            for element in select.attrpath()?.elements() {
                let AttrName::Ident(t) = element else {
                    return None;
                };
                let name = t.text(s);
                if t.kind() != SyntaxKind::Ident || !is_bare_attr_name(name) {
                    return None;
                }
                text.push('.');
                text.push_str(name);
            }
            Some((root, text))
        }
        _ => None,
    }
}

fn is_bare_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-'))
        && name != "or"
}

fn with_removal_fix(m: &SemanticModel, site: usize) -> Option<Fix> {
    let s = m.source();
    let sites = m.with_sites();
    let range = sites[site].scope_range;
    if sites.iter().enumerate().any(|(j, w)| {
        j != site && w.scope_range.start() < range.end() && range.start() < w.scope_range.end()
    }) {
        return None;
    }
    let with_expr = m
        .root()
        .descendants()
        .filter_map(WithExpr::cast)
        .find(|w| w.range() == range)?;
    let body = with_expr.body()?;
    let (root, prefix) = static_scope_chain(with_expr.scope()?, s)?;
    let start = with_expr.syntax().content_range().start();
    let body_start = body.content_range().start();
    let gap = &s[start as usize..body_start as usize];
    if gap.contains('#') || gap.contains("/*") {
        return None;
    }
    let root_name = root.text(s);
    let root_binding = m.resolve_lexical(root_name, start).map(|b| b.name.range());
    for inherit in with_expr
        .syntax()
        .descendants()
        .filter_map(InheritStmt::cast)
    {
        if inherit.source().is_some() || inherit.range().start() < body_start {
            continue;
        }
        for name in inherit.names() {
            let text = name.text(s);
            let binding = m.resolve_lexical(text, name.range().start()).and_then(|b| {
                if b.name.range() == name.range() {
                    m.outer_shadow(b)
                } else {
                    Some(b)
                }
            });
            if binding.is_none() && !crate::reference_rules::is_global(text) {
                return None;
            }
        }
    }
    let mut fix = Fix::new(format!(
        "replace `with {prefix};` with explicit `{prefix}.` selects"
    ))
    .edit(TextRange::new(start, body_start), "");
    for r in m.references().iter().filter(|r| r.via_with == Some(site)) {
        let name = r.name.text(s);
        if crate::reference_rules::is_global(name) {
            continue;
        }
        if name.starts_with("__")
            || !is_bare_attr_name(name)
            || m.resolve_lexical(root_name, r.name.range().start())
                .map(|b| b.name.range())
                != root_binding
        {
            return None;
        }
        fix = fix.edit(r.name.range(), format!("{prefix}.{name}"));
    }
    Some(fix)
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
                || raw.starts_with("\'\'")
                || is_trackable_flake_input(raw);
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
        "Flags extra arguments passed to builtins whose results cannot be functions. Partial applications remain valid."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, m: &SemanticModel, _: &LintConfig, d: &mut Vec<Diagnostic>) {
        let source = m.source();
        let apply_nodes: Vec<_> = nodes(m.root(), SyntaxKind::ApplyExpr).collect();
        for n in apply_nodes.iter().copied() {
            if apply_nodes.iter().any(|candidate| {
                ApplyExpr::cast(candidate)
                    .and_then(|a| a.func())
                    .and_then(crate::lib_helpers::unparen)
                    .is_some_and(
                        |expr| matches!(expr, Expr::Apply(inner) if inner.range() == n.range()),
                    )
            }) {
                continue;
            }
            let Some(apply) = ApplyExpr::cast(n) else {
                continue;
            };
            let mut callee = apply.func().and_then(crate::lib_helpers::unparen);
            let mut args = 1;
            while let Some(Expr::Apply(inner)) = callee {
                args += 1;
                callee = inner.func().and_then(crate::lib_helpers::unparen);
            }
            let Some(Expr::Select(select)) = callee else {
                continue;
            };
            let Some(Expr::Ident(base)) = select.base() else {
                continue;
            };
            if base.text(source) != "builtins"
                || !crate::static_binding::is_unshadowed(base, m)
                || select.default().is_some()
            {
                continue;
            }
            let Some(path) = select.attrpath() else {
                continue;
            };
            let mut elements = path.elements();
            let Some(strictix_syntax::AttrName::Ident(function)) = elements.next() else {
                continue;
            };
            if elements.next().is_some() {
                continue;
            }
            let name = function.text(source);
            let Some(required) = builtin_arity(name) else {
                continue;
            };
            if args <= required {
                continue;
            }
            d.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("builtin '{name}' receives too many arguments"),
                apply
                    .arg()
                    .map(|arg| arg.content_range())
                    .unwrap_or(n.range()),
            ));
        }
    }
}

fn builtin_arity(name: &str) -> Option<usize> {
    Some(match name {
        "length" | "tail" | "isNull" | "isAttrs" | "isBool" | "isInt" | "isList" | "isString"
        | "isFunction" => 1,
        "add" | "sub" | "mul" | "div" | "mod" | "concatStringsSep" | "elem" | "hasAttr"
        | "genList" | "map" | "filter" | "all" | "any" | "compareVersions" | "match" | "split" => 2,
        "replaceStrings" | "substring" => 3,
        _ => return None,
    })
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
            let literal_is_invalid = raw.chars().next().is_some_and(|c| {
                c.is_ascii_digit() || c.is_ascii_alphabetic() || c == '{' || c == '['
            });
            let is_trackable = is_trackable_flake_input(raw);
            if literal_is_invalid && !is_trackable {
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
