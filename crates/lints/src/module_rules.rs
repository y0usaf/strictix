//! Module structure mistakes, restricted to the root module expression.
//!
//! A root `imports` key establishes module context. Otherwise require an
//! explicit `config`/`options` section together with conventional module
//! formals; a `lib` formal alone only qualifies an `options` declaration.
//! Plain attrsets and package functions remain ambiguous and are skipped.

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, AttrsetExpr, Expr, LambdaParam, Root, StringPart, SyntaxKind,
    SyntaxToken, TextRange,
};

use crate::lib_helpers::{library_member, unparen};

struct Module<'a> {
    set: AttrsetExpr<'a>,
    config: Option<&'a SyntaxToken>,
}

fn static_name<'s>(name: AttrName<'_>, source: &'s str) -> Option<&'s str> {
    match name {
        AttrName::Ident(token) => Some(token.text(source)),
        AttrName::Str(string) => {
            let mut parts = string.parts();
            let StringPart::Content(token) = parts.next()? else {
                return None;
            };
            let text = token.text(source);
            (parts.next().is_none() && !text.contains('\\')).then_some(text)
        }
        AttrName::Interp(_) => None,
    }
}

fn heads<'s>(set: AttrsetExpr<'_>, source: &'s str) -> Vec<(&'s str, TextRange)> {
    let mut names = Vec::new();
    for item in set.items() {
        match item {
            AttrItem::Binding(binding) => {
                if let Some(path) = binding.attrpath() {
                    if let Some(name) = path.elements().next().and_then(|n| static_name(n, source))
                    {
                        names.push((name, path.syntax().content_range()));
                    }
                }
            }
            AttrItem::Inherit(inherit) => names.extend(
                inherit
                    .names()
                    .map(|token| (token.text(source), token.range())),
            ),
        }
    }
    names
}

fn root_module<'a>(model: &SemanticModel<'a>) -> Option<Module<'a>> {
    let mut expr = Root::cast(model.root())?.expr()?;
    let mut config = None;
    let mut module_formal = false;
    let mut lib_formal = false;
    let mut seen_lambda = false;
    loop {
        expr = match unparen(expr)? {
            Expr::Let(let_expr) => let_expr.body()?,
            Expr::With(with) => with.body()?,
            Expr::Lambda(lambda) if !seen_lambda => {
                seen_lambda = true;
                let LambdaParam::Formals(formals, _) = lambda.param() else {
                    return None;
                };
                for param in formals.params() {
                    match param.name.text(model.source()) {
                        "config" => {
                            config = Some(param.name);
                            module_formal = true;
                        }
                        "options" => module_formal = true,
                        "lib" => lib_formal = true,
                        _ => {}
                    }
                }
                lambda.body()?
            }
            Expr::Attrset(set) => {
                let names = heads(set, model.source());
                let has = |key| names.iter().any(|(name, _)| *name == key);
                return (has("imports")
                    || (module_formal && (has("config") || has("options")))
                    || (lib_formal && has("options")))
                .then_some(Module { set, config });
            }
            _ => return None,
        };
    }
}

pub struct MixedModuleSyntax;

impl Rule for MixedModuleSyntax {
    fn code(&self) -> &'static str {
        "mixed-module-syntax"
    }
    fn name(&self) -> &'static str {
        "Mixed module syntax"
    }
    fn description(&self) -> &'static str {
        "Flags ordinary option definitions beside an explicit config or options section in a root module. Module context requires imports or conventional module formals with explicit sections. Module metadata is allowed; ambiguous plain data and nested modules are skipped."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(module) = root_module(model) else {
            return;
        };
        let names = heads(module.set, model.source());
        if !names
            .iter()
            .any(|(name, _)| matches!(*name, "config" | "options"))
        {
            return;
        }
        for (name, range) in names {
            if matches!(
                name,
                "_class"
                    | "_file"
                    | "key"
                    | "disabledModules"
                    | "imports"
                    | "options"
                    | "config"
                    | "meta"
                    | "freeformType"
            ) {
                continue;
            }
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("module definition '{name}' is outside the explicit config section"),
                    range,
                )
                .with_help("move ordinary option definitions into config when config or options is present"),
            );
        }
    }
}

pub struct ConfigDependentImports;

impl Rule for ConfigDependentImports {
    fn code(&self) -> &'static str {
        "config-dependent-imports"
    }
    fn name(&self) -> &'static str {
        "Config-dependent imports"
    }
    fn description(&self) -> &'static str {
        "Flags root module imports that demand the module config argument to select their list or paths. Follows local aliases and recognized optional/optionals calls, while skipping unknown calls, deferred module bodies, and branches not known to execute. No automatic fix."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn check_file(&self, model: &SemanticModel, _: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(module) = root_module(model) else {
            return;
        };
        let Some(config) = module.config else { return };
        for item in module.set.items() {
            let AttrItem::Binding(binding) = item else {
                continue;
            };
            let Some(path) = binding.attrpath() else {
                continue;
            };
            let mut elements = path.elements();
            if elements.next().and_then(|n| static_name(n, model.source())) != Some("imports")
                || elements.next().is_some()
            {
                continue;
            }
            let Some(value) = binding.value() else {
                continue;
            };
            if let Some(range) = demanded_config(value, Demand::Imports, config, model, 32) {
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "imports require config before the module import graph has been collected",
                        range,
                    )
                    .with_help("import the module unconditionally and use lib.mkIf to control its option definitions; import selection can use specialArgs"),
                );
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Demand {
    Value,
    Imports,
}

fn literal_bool(expr: Expr<'_>, model: &SemanticModel<'_>) -> Option<bool> {
    let Expr::Ident(token) = unparen(expr)? else {
        return None;
    };
    if !crate::static_binding::is_unshadowed(token, model) {
        return None;
    }
    match token.text(model.source()) {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

// Inspect only syntax known to demand a value. List elements are demanded when
// collecting imports, but remain lazy inside conditions and other values.
// Attrsets/lambdas are module payloads: descending into them would flag valid
// inline modules whose option definitions refer to the outer configuration.
fn demanded_config(
    expr: Expr<'_>,
    demand: Demand,
    config: &SyntaxToken,
    model: &SemanticModel<'_>,
    fuel: usize,
) -> Option<TextRange> {
    let fuel = fuel.checked_sub(1)?;
    let visit = |expr, demand| demanded_config(expr, demand, config, model, fuel);
    match unparen(expr)? {
        Expr::Ident(token) => {
            if crate::static_binding::resolves_to(token, config, model) {
                Some(token.range())
            } else {
                visit(crate::static_binding::value(token, model)?, demand)
            }
        }
        Expr::Let(expr) => visit(expr.body()?, demand),
        Expr::With(expr) => visit(expr.body()?, demand),
        Expr::Select(expr) => visit(expr.base()?, Demand::Value),
        Expr::HasAttr(expr) => visit(expr.base()?, Demand::Value),
        Expr::Unary(expr) => visit(expr.operand()?, Demand::Value),
        Expr::Assert(expr) => visit(expr.cond()?, Demand::Value).or_else(|| {
            (literal_bool(expr.cond()?, model) == Some(true))
                .then(|| visit(expr.body()?, demand))
                .flatten()
        }),
        Expr::If(expr) => visit(expr.cond()?, Demand::Value).or_else(|| {
            visit(
                if literal_bool(expr.cond()?, model)? {
                    expr.then_branch()?
                } else {
                    expr.else_branch()?
                },
                demand,
            )
        }),
        Expr::List(list) if demand == Demand::Imports => {
            list.items().find_map(|item| visit(item, Demand::Value))
        }
        Expr::String(string) => string.parts().find_map(|part| match part {
            StringPart::Interp(interp) => visit(interp.expr()?, Demand::Value),
            StringPart::Content(_) => None,
        }),
        Expr::IndString(string) => string.parts().find_map(|part| match part {
            StringPart::Interp(interp) => visit(interp.expr()?, Demand::Value),
            StringPart::Content(_) => None,
        }),
        Expr::Bin(expr) => {
            let op = expr.op()?;
            let child_demand = if op == SyntaxKind::PlusPlus {
                demand
            } else {
                Demand::Value
            };
            visit(expr.lhs()?, child_demand).or_else(|| {
                // The right side of boolean operators is conditional.
                if matches!(
                    op,
                    SyntaxKind::AndAnd | SyntaxKind::OrOr | SyntaxKind::Arrow
                ) {
                    let lhs = literal_bool(expr.lhs()?, model)?;
                    if (op == SyntaxKind::OrOr && lhs) || (op != SyntaxKind::OrOr && !lhs) {
                        return None;
                    }
                }
                visit(expr.rhs()?, child_demand)
            })
        }
        Expr::Apply(apply) => {
            let func = unparen(apply.func()?)?;
            if let Expr::Apply(inner) = func {
                let helper = library_member(model, inner.func()?);
                if matches!(helper, Some("optional" | "optionals")) {
                    let condition = inner.arg()?;
                    return visit(condition, Demand::Value).or_else(|| {
                        if demand != Demand::Imports || literal_bool(condition, model) != Some(true)
                        {
                            return None;
                        }
                        visit(
                            apply.arg()?,
                            if helper == Some("optionals") {
                                Demand::Imports
                            } else {
                                Demand::Value
                            },
                        )
                    });
                }
            }
            // A function value is forced, but arbitrary functions need not
            // force their arguments. In particular mkIf defers its condition.
            visit(func, Demand::Value)
        }
        _ => None,
    }
}
