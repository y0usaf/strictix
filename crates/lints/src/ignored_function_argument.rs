//! ignored-function-argument: an attribute passed to a `{ a, ... }:` function
//! that neither names it nor binds the whole set with `@`, so nothing reads it.

use std::collections::HashSet;

use std::path::Path;

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    project::{normalize, ProjectContext},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, Expr, LambdaExpr, LambdaParam, Root, SyntaxKind,
    SyntaxToken, TextRange,
};

pub struct IgnoredFunctionArgument;

/// `range` widened to the whitespace before it, so removing it leaves no gap.
fn with_leading_space(range: TextRange, source: &str) -> TextRange {
    let start = source[..range.start() as usize]
        .trim_end_matches([' ', '\t', '\n', '\r'])
        .len();
    TextRange::new(u32::try_from(start).unwrap_or(0), range.end())
}

impl Rule for IgnoredFunctionArgument {
    fn code(&self) -> &'static str {
        "ignored-function-argument"
    }

    fn name(&self) -> &'static str {
        "Ignored function argument"
    }

    fn description(&self) -> &'static str {
        "Flags attributes of an attribute-set literal passed to a pattern function with `...` and no `@` name (`f = {lib, ...}: ...; f {inherit lib pkgs;}`) that the pattern does not declare: the function cannot read them, so passing them only says the function needs what it does not. The function is a lambda literal, a local binding of one, or `import` of a project file whose expression is one. The fix drops them, rewriting an `inherit` with the names that remain or removing it, and a binding, with the whitespace before it; it is skipped when a dropped value holds a comment."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file_project(
        &self,
        model: &SemanticModel,
        _: &LintConfig,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let source = model.source();
        for node in model
            .root()
            .descendants()
            .filter(|n| n.kind() == SyntaxKind::ApplyExpr)
        {
            let Some(apply) = ApplyExpr::cast(node) else {
                continue;
            };
            let Some(mut arg) = apply.arg() else { continue };
            while let Expr::Paren(p) = arg {
                let Some(inner) = p.expr() else { break };
                arg = inner;
            }
            let Expr::Attrset(attrset) = arg else {
                continue;
            };
            let declared: HashSet<String> =
                match resolve_lambda(apply.func(), model, &mut HashSet::new()) {
                    Some(lambda) => match open_pattern(lambda, source) {
                        Some(names) => names,
                        None => continue,
                    },
                    None => match imported_pattern(apply.func(), model, project) {
                        Some(names) => names,
                        None => continue,
                    },
                };
            let mut ignored = Vec::new();
            let mut fix = Fix::new("drop the arguments the function ignores");
            let mut fixable = true;
            for item in attrset.items() {
                match item {
                    AttrItem::Binding(binding) => {
                        let Some(path) = binding.attrpath() else {
                            continue;
                        };
                        let mut elements = path.elements();
                        let (Some(AttrName::Ident(name)), None) =
                            (elements.next(), elements.next())
                        else {
                            continue;
                        };
                        let name = name.text(source);
                        if declared.contains(name) {
                            continue;
                        }
                        ignored.push(name.to_owned());
                        let range = binding.syntax().content_range();
                        if source[range.start() as usize..range.end() as usize].contains('#') {
                            fixable = false;
                        }
                        fix = fix.edit(with_leading_space(range, source), "");
                    }
                    AttrItem::Inherit(inherit) => {
                        let names: Vec<_> = inherit.names().collect();
                        let (kept, dropped): (Vec<&SyntaxToken>, Vec<&SyntaxToken>) = names
                            .iter()
                            .copied()
                            .partition(|token| declared.contains(token.text(source)));
                        if dropped.is_empty() {
                            continue;
                        }
                        ignored.extend(dropped.iter().map(|t| t.text(source).to_owned()));
                        let range = inherit.syntax().content_range();
                        let text = &source[range.start() as usize..range.end() as usize];
                        if text.contains('#') {
                            fixable = false;
                        }
                        if kept.is_empty() {
                            fix = fix.edit(with_leading_space(range, source), "");
                        } else {
                            let head = match inherit.source() {
                                Some(from) => {
                                    let end = from.range().end();
                                    let close = source[end as usize..range.end() as usize]
                                        .find(')')
                                        .map_or(end, |i| end + i as u32 + 1);
                                    source[range.start() as usize..close as usize].to_owned()
                                }
                                None => "inherit".to_owned(),
                            };
                            let names: Vec<&str> = kept.iter().map(|t| t.text(source)).collect();
                            fix = fix.edit(range, format!("{head} {};", names.join(" ")));
                        }
                    }
                }
            }
            if ignored.is_empty() {
                continue;
            }
            let quoted: Vec<String> = ignored.iter().map(|n| format!("'{n}'")).collect();
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!(
                    "the function ignores {}: its pattern neither names {} nor binds the whole set with @; drop {}",
                    quoted.join(", "),
                    if ignored.len() == 1 { "it" } else { "them" },
                    if ignored.len() == 1 { "it" } else { "them" }
                ),
                attrset.syntax().content_range(),
            );
            if fixable {
                diag = diag.with_fix(fix);
            }
            diags.push(diag);
        }
    }
}

/// The names an open pattern (`{a, b, ...}:` with no `@` name) declares;
/// `None` for any other parameter.
fn open_pattern(lambda: LambdaExpr<'_>, source: &str) -> Option<HashSet<String>> {
    let LambdaParam::Formals(formals, None) = lambda.param() else {
        return None;
    };
    formals.has_ellipsis().then(|| {
        formals
            .params()
            .map(|p| p.name.text(source).to_owned())
            .collect()
    })
}

/// The open pattern of the file `import ./x.nix` reads, when it is a project
/// file whose whole expression is such a function.
fn imported_pattern(
    func: Option<Expr<'_>>,
    model: &SemanticModel<'_>,
    project: Option<&ProjectContext>,
) -> Option<HashSet<String>> {
    let source = model.source();
    let Expr::Apply(call) = unparen(func?)? else {
        return None;
    };
    let callee = unparen(call.func()?)?;
    if !matches!(callee.content_text(source), "import" | "builtins.import") {
        return None;
    }
    let Expr::Path(path) = unparen(call.arg()?)? else {
        return None;
    };
    let dir = model.path()?.parent().unwrap_or(Path::new("."));
    let target = normalize(&dir.join(path.text(source)));
    let file = project?.sources().iter().find(|file| {
        let path = normalize(&file.path);
        path == target || path == target.join("default.nix")
    })?;
    let tree = parse(&file.source);
    if tree.error_nodes().next().is_some() {
        return None;
    }
    let Expr::Lambda(lambda) = unparen(Root::cast(&tree)?.expr()?)? else {
        return None;
    };
    open_pattern(lambda, &file.source)
}

fn unparen(expr: Expr<'_>) -> Option<Expr<'_>> {
    match expr {
        Expr::Paren(paren) => unparen(paren.expr()?),
        other => Some(other),
    }
}

fn resolve_lambda<'a>(
    expr: Option<Expr<'a>>,
    model: &SemanticModel<'a>,
    seen: &mut HashSet<u32>,
) -> Option<LambdaExpr<'a>> {
    match expr? {
        Expr::Lambda(l) => Some(l),
        Expr::Paren(p) => resolve_lambda(p.expr(), model, seen),
        Expr::Ident(token) => {
            if !seen.insert(token.range().start()) {
                return None;
            }
            resolve_lambda(
                Some(crate::static_binding::value(token, model)?),
                model,
                seen,
            )
        }
        _ => None,
    }
}
