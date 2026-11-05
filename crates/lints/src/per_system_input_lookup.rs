//! per-system-input-lookup: `inputs.NAME.packages.${pkgs.stdenv.hostPlatform.system}`
//! spelled out where one per-system argument would say `inputPackages.NAME`.

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AstNode, AttrName, Expr, SelectExpr, SyntaxKind, TextRange};

const CODE: &str = "per-system-input-lookup";
const INPUTS: [&str; 2] = ["inputs", "flakeInputs"];

/// Whether an interpolated segment names the build platform: an attribute
/// path or name ending in `system` (`system`, `pkgs.system`,
/// `pkgs.stdenv.hostPlatform.system`).
fn is_system(name: AttrName<'_>, source: &str) -> bool {
    let AttrName::Interp(interp) = name else {
        return false;
    };
    match interp.expr() {
        Some(Expr::Ident(token)) => token.text(source) == "system",
        Some(Expr::Select(select)) => {
            select.default().is_none()
                && select
                    .attrpath()
                    .and_then(|path| path.elements().last())
                    .is_some_and(
                        |last| matches!(last, AttrName::Ident(t) if t.text(source) == "system"),
                    )
        }
        _ => false,
    }
}

fn ident(name: AttrName<'_>, source: &str) -> Option<String> {
    match name {
        AttrName::Ident(token) => Some(token.text(source).to_owned()),
        _ => None,
    }
}

pub struct PerSystemInputLookup;

impl Rule for PerSystemInputLookup {
    fn code(&self) -> &'static str {
        CODE
    }

    fn name(&self) -> &'static str {
        "Per-system input lookup"
    }

    fn description(&self) -> &'static str {
        "Flags `INPUTS.NAME.packages.${SYSTEM}` (also through `.outputs`, and `legacyPackages`), where INPUTS is a name in the `inputs` setting (default `inputs` and `flakeInputs`) and SYSTEM an interpolated `system` or attribute path ending in `.system` (`pkgs.stdenv.hostPlatform.system`): every module repeating the platform lookup for each input it uses. Bind the lookup once, `inputPackages = builtins.mapAttrs (_: input: input.packages.${system} or {}) inputs;`, pass it as a module argument and write `inputPackages.NAME`. With the `argument` setting naming that binding, a `packages` lookup where the name is lexically bound is fixed to `ARGUMENT.NAME`; elsewhere the diagnostic says to take it as an argument first. The fix trusts the setting that the argument maps each input to its packages for the same system."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let configured: Vec<&str> = config.setting_list(CODE, "inputs").collect();
        let inputs: &[&str] = if configured.is_empty() {
            &INPUTS
        } else {
            &configured
        };
        let argument = config.setting_str(CODE, "argument");
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
            if !inputs.contains(&base.text(source)) {
                continue;
            }
            let Some(path) = select.attrpath() else {
                continue;
            };
            let elements: Vec<AttrName<'_>> = path.elements().collect();
            let Some(input) = elements.first().copied() else {
                continue;
            };
            let mut at = 1;
            if elements.get(at).and_then(|e| ident(*e, source)).as_deref() == Some("outputs") {
                at += 1;
            }
            let Some(set) = elements.get(at).and_then(|e| ident(*e, source)) else {
                continue;
            };
            if set != "packages" && set != "legacyPackages" {
                continue;
            }
            let Some(system) = elements.get(at + 1).copied() else {
                continue;
            };
            if !is_system(system, source) {
                continue;
            }
            let input_text = match input {
                AttrName::Ident(token) => token.text(source),
                AttrName::Str(string) => string.syntax().text(source),
                AttrName::Interp(_) => continue,
            };
            let start = select.syntax().content_range().start();
            let AttrName::Interp(system) = system else {
                continue;
            };
            let lookup = TextRange::new(start, system.syntax().content_range().end());
            let mut diag = Diagnostic::new(
                CODE,
                self.severity(),
                format!(
                    "`{}` looks the platform up for one input; bind every input's {set} for the system once and select `{input_text}` from it",
                    &source[lookup.start() as usize..lookup.end() as usize]
                ),
                lookup,
            );
            match argument {
                Some(name) if set == "packages" => {
                    if model.resolve_lexical(name, start).is_some() {
                        diag = diag.with_fix(
                            Fix::new(format!("select from {name}"))
                                .edit(lookup, format!("{name}.{input_text}")),
                        );
                    } else {
                        diag = diag.with_help(format!(
                            "take `{name}` as an argument here, then write `{name}.{input_text}`"
                        ));
                    }
                }
                _ => {
                    diag = diag.with_help("bind `inputPackages = builtins.mapAttrs (_: input: input.packages.${system} or {}) inputs;` once, pass it to modules, and set `argument` so fix rewrites the lookups");
                }
            }
            diags.push(diag);
        }
    }
}
