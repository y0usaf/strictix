use std::collections::{BTreeMap, BTreeSet, VecDeque};

use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    rules::Rule,
    semantic::SemanticModel,
};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Attrpath, Expr, LambdaParam, Root, SelectExpr, StringPart,
    SyntaxKind, TextRange,
};

use crate::schema::unwrap_parens;

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

pub struct DeadFlakeInput;

impl Rule for DeadFlakeInput {
    fn code(&self) -> &'static str {
        "dead-flake-input"
    }

    fn name(&self) -> &'static str {
        "Dead flake input"
    }

    fn description(&self) -> &'static str {
        "Flags a flake input declared in `inputs` that is never used in `outputs`, either directly or transitively through another live input's `follows`."
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

        let mut declared: BTreeMap<String, TextRange> = BTreeMap::new();
        let mut follows_edges: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut outputs_expr: Option<Expr<'_>> = None;

        for item in flake.items() {
            let AttrItem::Binding(binding) = item else {
                continue;
            };
            let Some(path) = static_path(binding.attrpath(), source) else {
                continue;
            };
            match path.as_slice() {
                [head] if head == "outputs" => {
                    outputs_expr = binding.value().map(unwrap_parens);
                }
                [head] if head == "inputs" => {
                    if let Some(Expr::Attrset(inputs_set)) = binding.value().map(unwrap_parens) {
                        for sub in inputs_set.items() {
                            let AttrItem::Binding(sub_binding) = sub else {
                                continue;
                            };
                            let Some(sub_path) = static_path(sub_binding.attrpath(), source) else {
                                continue;
                            };
                            let [name, rest @ ..] = sub_path.as_slice() else {
                                continue;
                            };
                            declared
                                .entry(name.clone())
                                .or_insert_with(|| sub_binding.syntax().content_range());
                            collect_follows(
                                name,
                                rest,
                                sub_binding.value(),
                                source,
                                &mut follows_edges,
                            );
                        }
                    }
                }
                [head, name, rest @ ..] if head == "inputs" => {
                    declared
                        .entry(name.clone())
                        .or_insert_with(|| binding.syntax().content_range());
                    collect_follows(name, rest, binding.value(), source, &mut follows_edges);
                }
                _ => {}
            }
        }

        if declared.is_empty() {
            return;
        }
        let Some(Expr::Lambda(outputs)) = outputs_expr else {
            return;
        };

        let mut live: BTreeSet<String> = BTreeSet::new();

        let mut collect_attr_uses = |param_range: TextRange| -> bool {
            let Some(binding) = model.binding_at(param_range) else {
                return true;
            };
            for &ref_range in &binding.references {
                let mut selected_attr: Option<String> = None;
                model.root().walk_containing(ref_range.start(), |node| {
                    if node.kind() != SyntaxKind::SelectExpr {
                        return false;
                    }
                    let Some(select) = SelectExpr::cast(node) else {
                        return false;
                    };
                    let Some(base) = select.base() else {
                        return false;
                    };
                    if unwrap_parens(base).range() != ref_range {
                        return false;
                    }
                    if let Some(first) = select.attrpath().and_then(|ap| ap.elements().next()) {
                        selected_attr = static_attr_name(first, source);
                    }
                    true
                });
                match selected_attr {
                    Some(attr) => {
                        live.insert(attr);
                    }
                    None => return false,
                }
            }
            true
        };

        match outputs.param() {
            LambdaParam::Ident(param) => {
                if !collect_attr_uses(param.range()) {
                    return;
                }
            }
            LambdaParam::Formals(formals, at_name) => {
                if let Some(at) = at_name {
                    if !collect_attr_uses(at.range()) {
                        return;
                    }
                }
                for param in formals.params() {
                    let name = param.name.text(source);
                    if name == "self" {
                        continue;
                    }
                    let is_used = model
                        .binding_at(param.name.range())
                        .is_some_and(|b| !b.references.is_empty());
                    if is_used {
                        live.insert(name.to_string());
                    }
                }
            }
        }

        let mut queue: VecDeque<String> = live.iter().cloned().collect();
        while let Some(curr) = queue.pop_front() {
            if let Some(targets) = follows_edges.get(&curr) {
                for target in targets {
                    if live.insert(target.clone()) {
                        queue.push_back(target.clone());
                    }
                }
            }
        }

        let allowed: Vec<&str> = config.setting_list(self.code(), "allow").collect();
        for (name, range) in declared {
            if live.contains(&name) || allowed.contains(&name.as_str()) {
                continue;
            }
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("flake input '{name}' is never used in outputs"),
                    range,
                )
                .with_help("remove the unused input from `inputs`, or reference it in `outputs`"),
            );
        }
    }
}

fn collect_follows(
    input_name: &str,
    path: &[String],
    value: Option<Expr<'_>>,
    source: &str,
    edges: &mut BTreeMap<String, Vec<String>>,
) {
    if path.last().map(String::as_str) == Some("follows") {
        if let Some(target) = value.and_then(|v| static_string(unwrap_parens(v), source)) {
            if let Some(root_target) = target.split('/').next().filter(|s| !s.is_empty()) {
                edges
                    .entry(input_name.to_string())
                    .or_default()
                    .push(root_target.to_string());
            }
        }
        return;
    }
    if let Some(Expr::Attrset(nested)) = value.map(unwrap_parens) {
        for item in nested.items() {
            let AttrItem::Binding(binding) = item else {
                continue;
            };
            let Some(sub_path) = static_path(binding.attrpath(), source) else {
                continue;
            };
            let mut full = path.to_vec();
            full.extend(sub_path);
            collect_follows(input_name, &full, binding.value(), source, edges);
        }
    }
}

fn static_string(expr: Expr<'_>, source: &str) -> Option<String> {
    let Expr::String(string) = expr else {
        return None;
    };
    if string.parts().any(|p| matches!(p, StringPart::Interp(_))) {
        return None;
    }
    let range = string.syntax().content_range();
    let text = &source[range.start() as usize..range.end() as usize];
    if text.contains('\\') {
        return None;
    }
    Some(text.strip_prefix('"')?.strip_suffix('"')?.to_string())
}

fn static_attr_name(element: AttrName<'_>, source: &str) -> Option<String> {
    match element {
        AttrName::Ident(token) => Some(token.text(source).to_string()),
        AttrName::Str(string) => static_string(Expr::String(string), source),
        AttrName::Interp(_) => None,
    }
}
