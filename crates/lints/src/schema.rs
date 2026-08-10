//! Options-schema rule: flags NixOS option names that are not declared
//! in options.json (M8).
//!
//! NixOS modules consume their module arguments through the `config`
//! formal: `config.services.example.enable` reads an option. AI-generated
//! modules frequently hallucinate option names (`config.services.exmaple.enable`)
//! that exist nowhere in nixpkgs. This rule loads the options schema
//! (shape `{"options": {"a.b.c": {...}}}`) once per process, then walks
//! every select chain rooted at a `config` module argument and flags
//! full paths absent from the schema.
//!
//! The parsed schema lives in a process-wide [OnceLock] rather than a
//! field: rules are constructed by the registry macro as unit structs
//! (`Type {}`), so the struct itself carries no state. [OnceLock] is
//! Send + Sync, keeping the rule shareable across worker threads.

use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::json::JsonValue;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, Reference, SemanticModel};
use strictix_syntax::{AstNode, AttrName, Expr, SelectExpr};

/// The parsed options schema: the set of declared option paths, or the
/// reason loading/parsing failed. Loaded once per process.
type Schema = Result<HashSet<String>, String>;

static OPTIONS_SCHEMA: OnceLock<Schema> = OnceLock::new();

/// Read and parse options.json, extracting the `options` keys.
fn load_schema(path: &Path) -> Schema {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let parsed = JsonValue::parse(&text).map_err(|e| e.message)?;
    let options = parsed
        .get("options")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "expected a top-level \"options\" object".to_string())?;
    Ok(options.iter().map(|(key, _)| key.clone()).collect())
}

/// Whether a reference is a use of the `config` module argument: a
/// LambdaParam binding named `config`.
fn is_config_reference(model: &SemanticModel, reference: &Reference) -> bool {
    reference.name.text(model.source()) == "config"
        && model
            .resolve(reference.name)
            .is_some_and(|b| b.kind == BindingKind::LambdaParam)
}

/// The select chain rooted at `ref_range`: the innermost SelectExpr
/// whose base is the referenced ident, then every SelectExpr whose base
/// is the previous one (nested chains such as `Select(Select(config,a),b)`).
/// The parser usually flattens a path into one SelectExpr with a
/// multi-element attrpath, so the chain is typically a single node.
fn select_chain<'a>(
    model: &SemanticModel<'a>,
    ref_range: strictix_syntax::TextRange,
) -> Vec<SelectExpr<'a>> {
    let root = model.root();
    let mut chain = Vec::new();
    let Some(innermost) = root
        .descendants()
        .filter_map(SelectExpr::cast)
        .find(|s| matches!(s.base(), Some(Expr::Ident(t)) if t.range() == ref_range))
    else {
        return chain;
    };
    chain.push(innermost);
    let mut current_range = innermost.range();
    loop {
        let next = root
            .descendants()
            .filter_map(SelectExpr::cast)
            .find(|s| s.base().is_some_and(|b| b.range() == current_range));
        match next {
            Some(select) => {
                current_range = select.range();
                chain.push(select);
            }
            None => break,
        }
    }
    chain
}

/// Flags attribute paths read off `config` that options.json does not
/// declare.
///
/// Only fires when [LintConfig::schema] is set; the schema is loaded
/// lazily once. A load/parse failure is reported as a single diagnostic
/// on the first `config` reference, so a broken schema cannot crash
/// the run.
pub struct UnknownOption;

impl Rule for UnknownOption {
    fn code(&self) -> &'static str {
        "unknown-option"
    }

    fn name(&self) -> &'static str {
        "Unknown option"
    }

    fn description(&self) -> &'static str {
        "Flags option paths read off the config module argument that are not declared in options.json. Hallucinated option names are a classic failure mode of AI-generated NixOS modules."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(path) = &config.schema else {
            return; // schema rule off for this run
        };
        let schema = OPTIONS_SCHEMA.get_or_init(|| load_schema(path));
        let options = match schema {
            Ok(options) => options,
            Err(err) => {
                if let Some(first) = model
                    .references()
                    .iter()
                    .find(|r| is_config_reference(model, r))
                {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("could not load options schema: {err}"),
                        first.name.range(),
                    ));
                }
                return;
            }
        };
        for reference in model.references() {
            if !is_config_reference(model, reference) {
                continue;
            }
            let chain = select_chain(model, reference.name.range());
            if chain.is_empty() {
                continue;
            }
            // Collect the path segments innermost-out, skipping chains
            // with any non-ident segment (strings, interpolations).
            let mut segments = Vec::new();
            let mut valid = true;
            for select in &chain {
                let Some(attrpath) = select.attrpath() else {
                    valid = false;
                    break;
                };
                for element in attrpath.elements() {
                    match element {
                        AttrName::Ident(token) => {
                            segments.push(token.text(model.source()).to_string())
                        }
                        AttrName::Str(_) | AttrName::Interp(_) => valid = false,
                    }
                }
            }
            if !valid {
                continue;
            }
            let full_path = segments.join(".");
            if options.contains(&full_path) {
                continue;
            }
            let range = chain
                .last()
                .and_then(|s| s.attrpath())
                .map(|a| a.range())
                .unwrap_or_else(|| chain.last().expect("chain is non-empty").range());
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("option '{full_path}' is not declared in options.json"),
                range,
            ));
        }
    }
}
