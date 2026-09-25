//! `unused-option`: options the project declares that nothing in it reads.
//!
//! A project-wide rule. The first file to run builds one index of every
//! option path read anywhere in the project ([ProjectContext::memo]);
//! each file then reports its own `options.*` leaves that no read covers.
//! Reads are collected conservatively — anything that might read a
//! subtree counts as reading all of it — so a finding means no syntax
//! in the project can reach the option:
//!
//! - `config.a.b` / `options.a.b` off a module formal;
//! - aliases `cfg = config.a;` followed through `let`/`rec` bindings,
//!   so `cfg.b` reads `a.b` and a bare `cfg` reads all of `a`;
//! - `inherit (config.a) b;` and `inherit (cfg) b;`;
//! - `lib.attrByPath [ "a" "b" ] d config` and its siblings;
//! - `system.config.a.b` on an evaluated system from anywhere.

use std::collections::HashMap;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, Attrpath, Binding, Expr, InheritStmt,
    SelectExpr, StringPart, TextRange,
};

use crate::schema::{callee_name, module_attrset, static_segments, unwrap_parens};

/// How many `cfg = config.a; sub = cfg.b;` hops an alias is followed.
const ALIAS_DEPTH: usize = 4;

/// Every option path read anywhere in the project. A read covers the
/// subtree below it and every declaration above it.
struct OptionReads(Vec<Vec<String>>);

/// Whether read `read` can reach declaration `declared`: one is a
/// prefix of the other.
fn covers(read: &[String], declared: &[String]) -> bool {
    read.iter().zip(declared).all(|(r, d)| r == d)
}

fn key(range: TextRange) -> (u32, u32) {
    (range.start(), range.end())
}

/// Leading static segments of an attrpath, stopping at the first
/// dynamic one: `a.${x}.b` reads all of `a`.
fn leading_segments(path: Attrpath<'_>, source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for element in path.elements() {
        match element {
            AttrName::Ident(token) => out.push(token.text(source).to_owned()),
            AttrName::Str(string) => {
                let mut text = String::new();
                for part in string.parts() {
                    match part {
                        StringPart::Content(token) => text.push_str(token.text(source)),
                        StringPart::Interp(_) => return out,
                    }
                }
                out.push(text);
            }
            AttrName::Interp(_) => return out,
        }
    }
    out
}

/// One file's syntax, indexed by the range of the expression that sits
/// in each slot a read can flow through.
struct FileReads<'m, 'a> {
    model: &'m SemanticModel<'a>,
    /// select base range → the select
    selects: HashMap<(u32, u32), SelectExpr<'a>>,
    /// bound value range → the binding
    bindings: HashMap<(u32, u32), Binding<'a>>,
    /// inherit source range → the inherit
    inherits: HashMap<(u32, u32), InheritStmt<'a>>,
    /// argument range → the application
    applies: HashMap<(u32, u32), ApplyExpr<'a>>,
}

impl<'m, 'a> FileReads<'m, 'a> {
    fn new(model: &'m SemanticModel<'a>) -> Self {
        let mut this = Self {
            model,
            selects: HashMap::new(),
            bindings: HashMap::new(),
            inherits: HashMap::new(),
            applies: HashMap::new(),
        };
        for node in model.root().descendants() {
            if let Some(select) = SelectExpr::cast(node) {
                if let Some(base) = select.base() {
                    this.selects.insert(key(base.range()), select);
                }
            } else if let Some(binding) = Binding::cast(node) {
                if let Some(value) = binding.value() {
                    this.bindings.insert(key(value.range()), binding);
                }
            } else if let Some(inherit) = InheritStmt::cast(node) {
                if let Some(source) = inherit.source() {
                    this.inherits.insert(key(source.range()), inherit);
                }
            } else if let Some(apply) = ApplyExpr::cast(node) {
                if let Some(arg) = apply.arg() {
                    this.applies.insert(key(arg.range()), apply);
                }
            }
        }
        this
    }

    fn collect(&self, out: &mut Vec<Vec<String>>) {
        let source = self.model.source();
        for reference in self.model.references() {
            let formal = self
                .model
                .resolve(reference.name)
                .is_some_and(|b| b.kind == BindingKind::LambdaParam);
            if formal && matches!(reference.name.text(source), "config" | "options") {
                self.read_from(reference.name.range(), &[], ALIAS_DEPTH, out);
            }
        }
        // `system.config.a.b`: the options of an evaluated configuration.
        for select in self.selects.values() {
            if matches!(select.base(), Some(Expr::Ident(t)) if t.text(source) == "config") {
                continue; // a formal `config` was handled above
            }
            let Some(path) = select.attrpath() else {
                continue;
            };
            let segments = leading_segments(path, source);
            if let Some(at) = segments.iter().position(|s| s == "config") {
                let full = segments[at + 1..].to_vec();
                self.flow(select.range(), full, ALIAS_DEPTH, out);
            }
        }
    }

    /// Record what the expression at `range`, known to denote the
    /// option subtree `prefix`, reads.
    fn read_from(
        &self,
        range: TextRange,
        prefix: &[String],
        depth: usize,
        out: &mut Vec<Vec<String>>,
    ) {
        let source = self.model.source();
        let (full, whole) = match self.selects.get(&key(range)) {
            Some(select) => {
                let mut full = prefix.to_vec();
                if let Some(path) = select.attrpath() {
                    full.extend(leading_segments(path, source));
                }
                (full, select.range())
            }
            None => {
                if let Some(path) = self.attr_path_call(range) {
                    let mut full = prefix.to_vec();
                    full.extend(path);
                    out.push(full);
                    return;
                }
                (prefix.to_vec(), range)
            }
        };
        self.flow(whole, full, depth, out);
    }

    /// Record what the expression at `whole`, denoting the option
    /// subtree `full`, reads through the slot it sits in: the names an
    /// `inherit (...)` takes from it, the uses of a `let`/`rec` alias
    /// bound to it, or else the whole subtree.
    fn flow(&self, whole: TextRange, full: Vec<String>, depth: usize, out: &mut Vec<Vec<String>>) {
        let source = self.model.source();
        if let Some(inherit) = self.inherits.get(&key(whole)) {
            for name in inherit.names() {
                let mut path = full.clone();
                path.push(name.text(source).to_owned());
                match self.uses_of(name.range()).filter(|_| depth > 0) {
                    Some(uses) => {
                        for use_range in uses {
                            self.read_from(use_range, &path, depth - 1, out);
                        }
                    }
                    None => out.push(path),
                }
            }
            return;
        }
        if depth > 0 {
            if let Some(uses) = self.alias(whole) {
                for use_range in uses {
                    self.read_from(use_range, &full, depth - 1, out);
                }
                return;
            }
        }
        if !full.is_empty() {
            out.push(full);
        }
    }

    /// The uses of the `let`/`rec` name bound to exactly the expression
    /// at `range` (`cfg = config.a;`), or `None` when it is not an alias.
    fn alias(&self, range: TextRange) -> Option<Vec<TextRange>> {
        let binding = self.bindings.get(&key(range))?;
        let mut path = binding.attrpath()?.elements();
        let (Some(AttrName::Ident(name)), None) = (path.next(), path.next()) else {
            return None;
        };
        self.uses_of(name.range())
    }

    /// The uses of the `let`/`rec` name defined by the token at `name`.
    /// Other bindings (plain attrset keys) are not scopes: `None`.
    fn uses_of(&self, name: TextRange) -> Option<Vec<TextRange>> {
        let bound = self
            .model
            .bindings()
            .iter()
            .find(|b| b.name.range() == name)?;
        matches!(bound.kind, BindingKind::LetBinding | BindingKind::RecAttr)
            .then(|| bound.references.clone())
    }

    /// `lib.attrByPath [ "a" "b" ] d config` (and `getAttrFromPath`,
    /// `hasAttrByPath`): the literal path read off the value at `range`.
    fn attr_path_call(&self, range: TextRange) -> Option<Vec<String>> {
        let source = self.model.source();
        let apply = self.applies.get(&key(range))?;
        let mut func = unwrap_parens(apply.func()?);
        let mut first = None;
        while let Expr::Apply(inner) = func {
            first = inner.arg();
            func = unwrap_parens(inner.func()?);
        }
        if !matches!(
            callee_name(func, source)?,
            "attrByPath" | "getAttrFromPath" | "hasAttrByPath"
        ) {
            return None;
        }
        let Expr::List(list) = unwrap_parens(first?) else {
            return None;
        };
        list.items()
            .map(|item| match item {
                Expr::String(string) => {
                    let mut text = String::new();
                    for part in string.parts() {
                        match part {
                            StringPart::Content(token) => text.push_str(token.text(source)),
                            StringPart::Interp(_) => return None,
                        }
                    }
                    Some(text)
                }
                _ => None,
            })
            .collect()
    }
}

fn project_reads(project: &ProjectContext) -> OptionReads {
    let mut reads = Vec::new();
    for file in project.sources() {
        let tree = parse(&file.source);
        if tree.error_nodes().next().is_some() {
            continue;
        }
        let model = SemanticModel::new(&file.source, &tree);
        FileReads::new(&model).collect(&mut reads);
    }
    reads.sort();
    reads.dedup();
    OptionReads(reads)
}

/// Every option leaf this module declares: `options.<path>` bindings at
/// the module root, descending through plain attrset literals. Returns
/// each leaf's path and the attrpath range to report on.
fn declarations(model: &SemanticModel<'_>) -> Vec<(Vec<String>, TextRange)> {
    let source = model.source();
    let mut out = Vec::new();
    let Some(set) = module_attrset(model) else {
        return out;
    };
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
            source,
            value,
            &segments[1..],
            path.syntax().content_range(),
            &mut out,
        );
    }
    out
}

fn leaves(
    source: &str,
    value: Expr<'_>,
    prefix: &[String],
    range: TextRange,
    out: &mut Vec<(Vec<String>, TextRange)>,
) {
    let Expr::Attrset(set) = unwrap_parens(value) else {
        if !prefix.is_empty() {
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
        let Some(segments) = static_segments(path, source) else {
            continue;
        };
        let mut full = prefix.to_vec();
        full.extend(segments);
        leaves(source, value, &full, path.syntax().content_range(), out);
    }
}

/// Flags options declared in the project that nothing in the project
/// reads. Opt-in: a library meant for outside consumers declares options
/// it never reads, and linting a subset of a project hides its readers.
pub struct UnusedOption;

impl Rule for UnusedOption {
    fn code(&self) -> &'static str {
        "unused-option"
    }

    fn name(&self) -> &'static str {
        "Unused option"
    }

    fn description(&self) -> &'static str {
        "Flags `options.*` declarations that nothing in the linted project reads: no `config.<path>` (directly, through a `cfg` alias, `inherit`, or `lib.attrByPath`) reaches them. Opt-in, and only meaningful when the whole project is linted in one run."
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
        let declared = declarations(model);
        if declared.is_empty() {
            return;
        }
        let reads = project.memo(project_reads);
        for (path, range) in declared {
            if reads.0.iter().any(|read| covers(read, &path)) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("option '{}' is declared but never read", path.join(".")),
                range,
            ));
        }
    }
}
