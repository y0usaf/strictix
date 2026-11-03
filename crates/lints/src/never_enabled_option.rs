use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, AstNode, AttrItem, AttrName, Attrpath, AttrsetExpr, Binding, Expr, Root, StringPart,
    SyntaxKind, TextRange,
};

use crate::schema::{callee_name, module_attrset, static_segments, unwrap_parens};

type Segment = Option<String>;

pub(crate) struct OptionWrites(Vec<Vec<Segment>>);

impl OptionWrites {
    pub(crate) fn reach(&self, declared: &[String]) -> bool {
        self.0.iter().any(|write| reaches(write, declared))
    }
}

const RESERVED_HEADS: [&str; 5] = ["imports", "options", "_module", "disabledModules", "key"];

fn reaches(write: &[Segment], declared: &[String]) -> bool {
    write
        .iter()
        .zip(declared)
        .all(|(w, d)| w.as_ref().is_none_or(|w| w == d))
}

fn segments(path: Attrpath<'_>, source: &str) -> Vec<Segment> {
    path.elements()
        .map(|element| match element {
            AttrName::Ident(token) => Some(token.text(source).to_owned()),
            AttrName::Str(string) => {
                let mut text = String::new();
                for part in string.parts() {
                    match part {
                        StringPart::Content(token) => text.push_str(token.text(source)),
                        StringPart::Interp(_) => return None,
                    }
                }
                (!text.contains('\\')).then_some(text)
            }
            AttrName::Interp(_) => None,
        })
        .collect()
}

fn walk(source: &str, value: Expr<'_>, path: &[Segment], root: bool, out: &mut Vec<Vec<Segment>>) {
    match unwrap_parens(value) {
        Expr::Apply(apply) => {
            let (Some(func), Some(arg)) = (apply.func(), apply.arg()) else {
                return out.push(path.to_vec());
            };
            match callee_name(func, source) {
                Some("mkDefault" | "mkForce" | "mkBefore" | "mkAfter" | "mkOptionDefault") => {
                    walk(source, arg, path, root, out);
                }
                Some("mkMerge") => match unwrap_parens(arg) {
                    Expr::List(list) => {
                        for item in list.items() {
                            walk(source, item, path, root, out);
                        }
                    }
                    _ => out.push(path.to_vec()),
                },
                _ => {
                    let wrapper = match unwrap_parens(func) {
                        Expr::Apply(inner) => inner
                            .func()
                            .and_then(|f| callee_name(f, source))
                            .is_some_and(|n| matches!(n, "mkIf" | "mkOverride" | "mkOrder")),
                        _ => false,
                    };
                    if wrapper {
                        walk(source, arg, path, root, out);
                    } else {
                        out.push(path.to_vec());
                    }
                }
            }
        }
        Expr::If(expr) => {
            for branch in [expr.then_branch(), expr.else_branch()]
                .into_iter()
                .flatten()
            {
                walk(source, branch, path, root, out);
            }
        }
        Expr::Let(expr) => {
            if let Some(body) = expr.body() {
                walk(source, body, path, root, out);
            }
        }
        Expr::With(expr) => {
            if let Some(body) = expr.body() {
                walk(source, body, path, root, out);
            }
        }
        Expr::Assert(expr) => {
            if let Some(body) = expr.body() {
                walk(source, body, path, root, out);
            }
        }
        Expr::Bin(expr) if expr.op() == Some(SyntaxKind::SlashSlash) => {
            for side in [expr.lhs(), expr.rhs()].into_iter().flatten() {
                walk(source, side, path, root, out);
            }
        }
        Expr::Attrset(set) => attrset(source, set, path, root, out),
        Expr::RecAttrset(rec) => {
            if let Some(set) = rec.attrset() {
                attrset(source, set, path, root, out);
            }
        }
        _ => out.push(path.to_vec()),
    }
}

fn attrset(
    source: &str,
    set: AttrsetExpr<'_>,
    path: &[Segment],
    root: bool,
    out: &mut Vec<Vec<Segment>>,
) {
    for item in set.items() {
        match item {
            AttrItem::Inherit(inherit) => {
                for name in inherit.names() {
                    let mut full = path.to_vec();
                    full.push(Some(name.text(source).to_owned()));
                    out.push(full);
                }
            }
            AttrItem::Binding(binding) => {
                let (Some(attrpath), Some(value)) = (binding.attrpath(), binding.value()) else {
                    continue;
                };
                let mut written = segments(attrpath, source);
                if root {
                    if let Some(Some(head)) = written.first() {
                        if RESERVED_HEADS.contains(&head.as_str()) {
                            continue;
                        }
                        if head == "config" {
                            written.remove(0);
                        }
                    }
                }
                let mut full = path.to_vec();
                full.extend(written);
                walk(source, value, &full, root && full.is_empty(), out);
            }
        }
    }
}

fn module_root(expr: Expr<'_>, source: &str, out: &mut Vec<Vec<Segment>>) {
    let mut expr = expr;
    loop {
        expr = match unwrap_parens(expr) {
            Expr::Lambda(lambda) => match lambda.body() {
                Some(body) => body,
                None => return,
            },
            Expr::Let(let_expr) => match let_expr.body() {
                Some(body) => body,
                None => return,
            },
            Expr::With(with) => match with.body() {
                Some(body) => body,
                None => return,
            },
            Expr::Attrset(_) | Expr::RecAttrset(_) => break,
            _ => return,
        };
    }
    walk(source, expr, &[], true, out);
}

pub(crate) fn project_writes(project: &ProjectContext) -> OptionWrites {
    let mut writes = Vec::new();
    for file in project.sources() {
        let source = file.source.as_str();
        let root = parse(source);
        let root = &root;
        if let Some(expr) = Root::cast(root).and_then(|r| r.expr()) {
            module_root(expr, source, &mut writes);
            if let Some(bindings) = match unwrap_parens(expr) {
                Expr::Let(let_expr) => let_expr.bindings(),
                _ => None,
            } {
                for item in bindings.items() {
                    if let AttrItem::Binding(binding) = item {
                        if let Some(value) = binding.value() {
                            module_root(value, source, &mut writes);
                        }
                    }
                }
            }
        }
        for node in root.descendants() {
            let Some(binding) = Binding::cast(node) else {
                continue;
            };
            let named = binding
                .attrpath()
                .and_then(|p| p.elements().last())
                .is_some_and(|last| {
                    matches!(last, AttrName::Ident(t) if matches!(t.text(source), "imports" | "modules"))
                });
            if !named {
                continue;
            }
            if let Some(Expr::List(list)) = binding.value().map(unwrap_parens) {
                for item in list.items() {
                    module_root(item, source, &mut writes);
                }
            }
        }
    }
    writes.sort();
    writes.dedup();
    OptionWrites(writes)
}

pub(crate) fn is_false(model: &SemanticModel<'_>, expr: Expr<'_>) -> bool {
    matches!(unwrap_parens(expr), Expr::Ident(t)
        if t.text(model.source()) == "false" && !model.is_bound("false", t.range().start()))
}

fn bool_default_false(model: &SemanticModel<'_>, arg: Expr<'_>) -> bool {
    let source = model.source();
    let Expr::Attrset(set) = unwrap_parens(arg) else {
        return false;
    };
    let (mut boolean, mut off) = (false, false);
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
            continue;
        };
        match static_segments(path, source).as_deref() {
            Some([name]) if name == "type" => {
                boolean = callee_name(unwrap_parens(value), source) == Some("bool");
            }
            Some([name]) if name == "default" => off = is_false(model, value),
            _ => {}
        }
    }
    boolean && off
}

pub(crate) fn is_enable_option(model: &SemanticModel<'_>, value: Expr<'_>) -> bool {
    let Expr::Apply(apply) = unwrap_parens(value) else {
        return false;
    };
    let (Some(func), Some(arg)) = (apply.func(), apply.arg()) else {
        return false;
    };
    match callee_name(func, model.source()) {
        Some("mkEnableOption") => true,
        Some("mkOption") => bool_default_false(model, arg),
        _ => false,
    }
}

fn module_sets<'a>(model: &SemanticModel<'a>) -> Vec<AttrsetExpr<'a>> {
    if let Some(set) = module_attrset(model) {
        return vec![set];
    }
    let Some(Expr::Let(let_expr)) = Root::cast(model.root())
        .and_then(|r| r.expr())
        .map(unwrap_parens)
    else {
        return Vec::new();
    };
    let Some(bindings) = let_expr.bindings() else {
        return Vec::new();
    };
    bindings
        .items()
        .filter_map(|item| {
            let AttrItem::Binding(binding) = item else {
                return None;
            };
            let mut expr = binding.value()?;
            loop {
                expr = match unwrap_parens(expr) {
                    Expr::Lambda(lambda) => lambda.body()?,
                    Expr::Let(inner) => inner.body()?,
                    Expr::With(with) => with.body()?,
                    Expr::Attrset(set) => return Some(set),
                    _ => return None,
                };
            }
        })
        .collect()
}

pub(crate) fn declarations(
    model: &SemanticModel<'_>,
    accept: fn(&SemanticModel<'_>, Expr<'_>) -> bool,
) -> Vec<(Vec<String>, TextRange)> {
    let source = model.source();
    let mut out = Vec::new();
    for set in module_sets(model) {
        module_declarations(model, set, source, accept, &mut out);
    }
    out
}

fn module_declarations(
    model: &SemanticModel<'_>,
    set: AttrsetExpr<'_>,
    source: &str,
    accept: fn(&SemanticModel<'_>, Expr<'_>) -> bool,
    out: &mut Vec<(Vec<String>, TextRange)>,
) {
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
            continue;
        };
        let Some(segments) = static_segments(path, source) else {
            continue;
        };
        if segments.first().map(String::as_str) != Some("options") {
            continue;
        }
        leaves(
            model,
            value,
            &segments[1..],
            path.syntax().content_range(),
            accept,
            out,
        );
    }
}

fn leaves(
    model: &SemanticModel<'_>,
    value: Expr<'_>,
    prefix: &[String],
    range: TextRange,
    accept: fn(&SemanticModel<'_>, Expr<'_>) -> bool,
    out: &mut Vec<(Vec<String>, TextRange)>,
) {
    let Expr::Attrset(set) = unwrap_parens(value) else {
        if !prefix.is_empty() && accept(model, value) {
            out.push((prefix.to_vec(), range));
        }
        return;
    };
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
            continue;
        };
        let Some(segments) = static_segments(path, model.source()) else {
            continue;
        };
        let mut full = prefix.to_vec();
        full.extend(segments);
        leaves(
            model,
            value,
            &full,
            path.syntax().content_range(),
            accept,
            out,
        );
    }
}

pub(crate) fn display(path: &[String]) -> String {
    path.iter()
        .map(|segment| {
            let plain = segment
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '\''));
            if plain {
                segment.clone()
            } else {
                format!("\"{segment}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

pub struct NeverEnabledOption;

impl Rule for NeverEnabledOption {
    fn code(&self) -> &'static str {
        "never-enabled-option"
    }

    fn name(&self) -> &'static str {
        "Never-enabled option"
    }

    fn description(&self) -> &'static str {
        "Flags enable options (`mkEnableOption`, or `mkOption` of type bool defaulting to false) that no file in the linted project ever defines, so everything gated on them is dead. Any definition of the option or a parent attrset, through `config`, `mkIf`/`mkMerge`/`mkDefault`, counts; dynamic segments and `inherit` that could reach it count as set. Opt-in, and only meaningful when the whole project is linted in one run."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_file_project(
        &self,
        model: &SemanticModel,
        _config: &LintConfig,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let Some(project) = project else {
            return;
        };
        let declared = declarations(model, is_enable_option);
        if declared.is_empty() {
            return;
        }
        let writes = project.memo(project_writes);
        for (path, range) in declared {
            if writes.reach(&path) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!(
                    "option '{}' is never set anywhere in the project; the code it gates never runs",
                    display(&path)
                ),
                range,
            ));
        }
    }
}
