//! Flags flake inputs that would build from a second nixpkgs.
//!
//! An input that does not follow the root `nixpkgs` brings its own package
//! set: its exported packages link a different glibc, and for graphics or
//! compositor inputs a different mesa and wayland, so the system closure
//! holds two of each and `nix flake update` moves them independently.
//! `inputs.<name>.inputs.nixpkgs.follows = "nixpkgs"` is the fix. An input
//! marked `flake = false` is a source tree only — it has no outputs and so
//! no `pkgs` to follow — and is exempt.

use std::collections::BTreeMap;

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{AstNode, AttrItem, AttrName, Attrpath, Expr, Root, StringPart, TextRange};

use crate::schema::unwrap_parens;

/// Flags a flake input that pins its own nixpkgs.
pub struct FlakeInputWithoutNixpkgsFollows;

#[derive(Default)]
struct Input {
    range: Option<TextRange>,
    url: bool,
    source_only: bool,
    follows: bool,
}

impl Rule for FlakeInputWithoutNixpkgsFollows {
    fn code(&self) -> &'static str {
        "flake-input-without-nixpkgs-follows"
    }

    fn name(&self) -> &'static str {
        "Flake input without nixpkgs follows"
    }

    fn description(&self) -> &'static str {
        "Flags a flake input that pins its own nixpkgs instead of setting `inputs.nixpkgs.follows = \"nixpkgs\"`, which duplicates the package set in the closure. Inputs marked `flake = false` are sources only and exempt."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let Some(Expr::Attrset(flake)) = Root::cast(model.root()).and_then(|root| root.expr())
        else {
            return;
        };
        let Some(declared) = flake.items().find_map(|item| match item {
            AttrItem::Binding(binding) => static_path(binding.attrpath(), source)
                .filter(|path| path.as_slice() == ["inputs"])
                .and_then(|_| binding.value())
                .map(unwrap_parens),
            AttrItem::Inherit(_) => None,
        }) else {
            return;
        };
        let Expr::Attrset(declared) = declared else {
            return;
        };
        let mut inputs: BTreeMap<String, Input> = BTreeMap::new();
        for item in declared.items() {
            let AttrItem::Binding(binding) = item else {
                continue;
            };
            let Some(path) = static_path(binding.attrpath(), source) else {
                continue;
            };
            let [name, rest @ ..] = path.as_slice() else {
                continue;
            };
            let entry = inputs.entry(name.clone()).or_default();
            entry
                .range
                .get_or_insert_with(|| binding.syntax().content_range());
            record(entry, rest, binding.value(), source);
            if let Some(Expr::Attrset(nested)) = binding.value().map(unwrap_parens) {
                collect(entry, nested.items(), source, rest.to_vec());
            }
        }
        if !inputs.contains_key("nixpkgs") {
            return;
        }
        let allowed: Vec<&str> = config.setting_list(self.code(), "allow").collect();
        for (name, input) in inputs {
            if name == "nixpkgs" || !input.url || input.source_only || input.follows {
                continue;
            }
            if allowed.contains(&name.as_str()) {
                continue;
            }
            let Some(range) = input.range else {
                continue;
            };
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "input '{name}' does not follow the root nixpkgs, so it builds its exported packages from a second package set"
                    ),
                    range,
                )
                .with_help(format!(
                    "add `inputs.{name}.inputs.nixpkgs.follows = \"nixpkgs\";` after its url, or `flake = false;` when only its source tree is used"
                )),
            );
        }
    }
}

fn collect<'a>(
    entry: &mut Input,
    items: impl Iterator<Item = AttrItem<'a>>,
    source: &str,
    prefix: Vec<String>,
) {
    for item in items {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let Some(path) = static_path(binding.attrpath(), source) else {
            continue;
        };
        let mut full = prefix.clone();
        full.extend(path);
        record(entry, &full, binding.value(), source);
        if let Some(Expr::Attrset(nested)) = binding.value().map(unwrap_parens) {
            collect(entry, nested.items(), source, full);
        }
    }
}

fn record(entry: &mut Input, path: &[String], value: Option<Expr<'_>>, source: &str) {
    let tail: Vec<&str> = path.iter().map(String::as_str).collect();
    match tail.last() {
        Some(&"url") => entry.url = true,
        Some(&"follows") if tail.contains(&"nixpkgs") => entry.follows = true,
        Some(&"flake") if value.is_some_and(|v| is_false(v, source)) => entry.source_only = true,
        _ => {}
    }
}

fn is_false(expr: Expr<'_>, source: &str) -> bool {
    matches!(expr, Expr::Ident(token) if token.text(source) == "false")
}

fn static_path(path: Option<Attrpath<'_>>, source: &str) -> Option<Vec<String>> {
    path?
        .elements()
        .map(|element| match element {
            AttrName::Ident(token) => Some(token.text(source).to_string()),
            AttrName::Str(string)
                if !string.parts().any(|p| matches!(p, StringPart::Interp(_))) =>
            {
                let range = string.syntax().content_range();
                let text = &source[range.start() as usize..range.end() as usize];
                if text.contains('\\') {
                    return None;
                }
                Some(text.strip_prefix('"')?.strip_suffix('"')?.to_string())
            }
            AttrName::Str(_) | AttrName::Interp(_) => None,
        })
        .collect()
}
