use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AttrItem, Expr, StringPart};

use crate::never_enabled_option::{declarations, display, is_enable_option, project_writes};
use crate::schema::{callee_name, static_segments, unwrap_parens};

fn literal(model: &SemanticModel<'_>, expr: Expr<'_>) -> bool {
    match unwrap_parens(expr) {
        Expr::Int(_) | Expr::Float(_) => true,
        Expr::Ident(token) => {
            let name = token.text(model.source());
            matches!(name, "true" | "false" | "null")
                && !model.is_bound(name, token.range().start())
        }
        Expr::String(string) => string
            .parts()
            .all(|part| matches!(part, StringPart::Content(_))),
        Expr::IndString(string) => string
            .parts()
            .all(|part| matches!(part, StringPart::Content(_))),
        Expr::Unary(unary) => unary
            .operand()
            .is_some_and(|operand| literal(model, operand)),
        Expr::List(list) => list.items().all(|item| literal(model, item)),
        Expr::Attrset(set) => set.items().all(|item| match item {
            AttrItem::Binding(binding) => {
                binding
                    .attrpath()
                    .is_some_and(|path| static_segments(path, model.source()).is_some())
                    && binding.value().is_some_and(|value| literal(model, value))
            }
            AttrItem::Inherit(_) => false,
        }),
        _ => false,
    }
}

fn literal_default(model: &SemanticModel<'_>, value: Expr<'_>) -> bool {
    let Expr::Apply(apply) = unwrap_parens(value) else {
        return false;
    };
    let (Some(func), Some(arg)) = (apply.func(), apply.arg()) else {
        return false;
    };
    if callee_name(func, model.source()) != Some("mkOption") || is_enable_option(model, value) {
        return false;
    }
    let Expr::Attrset(set) = unwrap_parens(arg) else {
        return false;
    };
    set.items().any(|item| {
        let AttrItem::Binding(binding) = item else {
            return false;
        };
        binding
            .attrpath()
            .and_then(|path| static_segments(path, model.source()))
            .is_some_and(|segments| segments == ["default"])
            && binding.value().is_some_and(|value| literal(model, value))
    })
}

pub struct ConstantOption;

impl Rule for ConstantOption {
    fn code(&self) -> &'static str {
        "constant-option"
    }

    fn name(&self) -> &'static str {
        "Constant option"
    }

    fn description(&self) -> &'static str {
        "Flags `mkOption` declarations with a literal default that no file in the linted project ever defines, so every configuration uses the default: a constant dressed as an option. Definitions are matched as in `never-enabled-option`, which already covers bool options defaulting to false. Opt-in, and only meaningful when the whole project is linted in one run."
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
        let declared = declarations(model, literal_default);
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
                    "option '{}' is never set anywhere in the project, so it always has its default; replace it with a let value in the module that reads it, or delete it if nothing does",
                    display(&path)
                ),
                range,
            ));
        }
    }
}
