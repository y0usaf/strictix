use std::collections::{HashMap, HashSet};

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, AstNode, AttrItem, AttrName, AttrsetExpr, Expr, Root, StringPart, SyntaxKind,
    SyntaxNode, TextRange,
};

use crate::config_file_rules::relative;

const MIN_LEAVES: usize = 8;

type Leaf = (Box<[u32]>, u32);

struct Module {
    file: usize,
    name: Option<String>,
}

struct Set {
    module: usize,
    anchor: TextRange,
    parent: Option<(usize, Box<[u32]>)>,
    children: Vec<(Box<[u32]>, usize)>,
    leaves: Vec<Leaf>,
}

#[derive(Default)]
struct Interner {
    ids: HashMap<String, u32>,
    names: Vec<String>,
}

impl Interner {
    fn id(&mut self, text: &str) -> u32 {
        if let Some(&id) = self.ids.get(text) {
            return id;
        }
        let id = u32::try_from(self.names.len()).expect("fewer than 2^32 distinct texts");
        self.ids.insert(text.to_owned(), id);
        self.names.push(text.to_owned());
        id
    }
}

struct Builder<'s, 'i> {
    source: &'s str,
    segments: &'i mut Interner,
    values: &'i mut Interner,
    sets: &'i mut Vec<Set>,
    seen: HashSet<u32>,
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn attrset_of(value: Expr<'_>) -> Option<AttrsetExpr<'_>> {
    match value {
        Expr::Paren(paren) => paren.expr().and_then(attrset_of),
        Expr::Attrset(set) => Some(set),
        Expr::RecAttrset(set) => set.attrset(),
        _ => None,
    }
}

impl Builder<'_, '_> {
    fn segment(&mut self, name: AttrName<'_>) -> u32 {
        let source = self.source;
        let text = match name {
            AttrName::Ident(token) => token.text(source).to_owned(),
            AttrName::Str(string) => string
                .parts()
                .map(|part| match part {
                    StringPart::Content(token) => token.text(source),
                    StringPart::Interp(interp) => interp.syntax().text(source).trim(),
                })
                .collect(),
            AttrName::Interp(interp) => interp.syntax().text(source).trim().to_owned(),
        };
        self.segments.id(&text)
    }

    fn value(&mut self, text: &str) -> u32 {
        self.values.id(&normalized(text))
    }

    fn build(&mut self, set: AttrsetExpr<'_>, module: usize, anchor: TextRange) -> usize {
        let source = self.source;
        self.seen.insert(set.syntax().range().start());
        let index = self.sets.len();
        self.sets.push(Set {
            module,
            anchor,
            parent: None,
            children: Vec::new(),
            leaves: Vec::new(),
        });
        let mut leaves: Vec<Leaf> = Vec::new();
        let mut children = Vec::new();
        for item in set.items() {
            match item {
                AttrItem::Binding(binding) => {
                    let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
                        continue;
                    };
                    let key: Box<[u32]> = path.elements().map(|e| self.segment(e)).collect();
                    let child = attrset_of(value)
                        .filter(|inner| inner.items().next().is_some())
                        .map(|inner| self.build(inner, module, path.syntax().content_range()));
                    match child {
                        Some(child) => {
                            self.sets[child].parent = Some((index, key.clone()));
                            leaves.extend(
                                self.sets[child].leaves.iter().map(|(p, v)| {
                                    (key.iter().chain(p.iter()).copied().collect(), *v)
                                }),
                            );
                            children.push((key, child));
                        }
                        None => {
                            let value = self.value(value.content_text(source));
                            leaves.push((key, value));
                        }
                    }
                }
                AttrItem::Inherit(inherit) => {
                    let text = inherit.source().map_or_else(
                        || "inherit".to_owned(),
                        |from| format!("inherit ({})", from.content_text(source)),
                    );
                    let value = self.value(&text);
                    for name in inherit.names() {
                        let key = Box::new([self.segments.id(name.text(source))]);
                        leaves.push((key, value));
                    }
                }
            }
        }
        leaves.sort_unstable();
        let entry = &mut self.sets[index];
        entry.leaves = leaves;
        entry.children = children;
        index
    }

    fn module(&mut self, root: &SyntaxNode, module: usize) {
        for node in root.descendants() {
            if node.kind() != SyntaxKind::AttrsetExpr || self.seen.contains(&node.range().start()) {
                continue;
            }
            let Some(set) = AttrsetExpr::cast(node) else {
                continue;
            };
            let start = node.content_range().start();
            self.build(set, module, TextRange::new(start, start + 1));
        }
    }
}

fn merged_modules<'t>(tree: &'t SyntaxNode, source: &str) -> Option<Vec<(String, &'t SyntaxNode)>> {
    let Expr::Let(top) = Root::cast(tree)?.expr()? else {
        return None;
    };
    let mut body = top.body()?;
    while let Expr::Paren(paren) = body {
        body = paren.expr()?;
    }
    let Expr::Apply(outer) = body else {
        return None;
    };
    let Expr::Apply(inner) = outer.func()? else {
        return None;
    };
    if inner.func()?.content_text(source) != "builtins.seq" {
        return None;
    }
    let Expr::Ident(manifest) = inner.arg()? else {
        return None;
    };
    let manifest = manifest.text(source);
    Some(
        top.bindings()?
            .items()
            .filter_map(|item| {
                let AttrItem::Binding(binding) = item else {
                    return None;
                };
                let mut elements = binding.attrpath()?.elements();
                let (Some(AttrName::Ident(name)), None) = (elements.next(), elements.next()) else {
                    return None;
                };
                let name = name.text(source);
                (name != manifest).then(|| (name.to_owned(), binding.syntax()))
            })
            .collect(),
    )
}

struct Findings(Vec<Vec<(TextRange, String)>>);

struct Index<'p> {
    project: &'p ProjectContext,
    lines: Vec<Vec<u32>>,
    modules: Vec<Module>,
    sets: Vec<Set>,
    segments: Interner,
}

impl Index<'_> {
    fn line(&self, file: usize, offset: u32) -> usize {
        self.lines[file].partition_point(|&start| start <= offset)
    }

    fn site(&self, set: usize, from: usize) -> String {
        let set = &self.sets[set];
        let module = &self.modules[set.module];
        let line = self.line(module.file, set.anchor.start());
        let files = self.project.sources();
        if module.file == from {
            return match &module.name {
                Some(name) => format!("{name} (line {line})"),
                None => format!("line {line}"),
            };
        }
        let path = relative(&files[from].path, &files[module.file].path);
        match &module.name {
            Some(name) => format!("{name} ({}:{line})", path.display()),
            None => format!("{}:{line}", path.display()),
        }
    }

    fn sites(&self, sets: impl Iterator<Item = usize>, from: usize) -> String {
        sets.map(|set| self.site(set, from))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn path(&self, key: &[u32]) -> String {
        key.iter()
            .map(|&segment| self.segments.names[segment as usize].as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

fn index(project: &ProjectContext) -> Findings {
    let files = project.sources();
    let mut segments = Interner::default();
    let mut values = Interner::default();
    let mut sets = Vec::new();
    let mut modules = Vec::new();
    let mut lines = Vec::new();
    for (file, entry) in files.iter().enumerate() {
        let source = entry.source.as_str();
        lines.push(
            std::iter::once(0)
                .chain(
                    source
                        .match_indices('\n')
                        .map(|(i, _)| u32::try_from(i + 1).unwrap_or(u32::MAX)),
                )
                .collect(),
        );
        let tree = parse(source);
        if tree.error_nodes().next().is_some() {
            continue;
        }
        let mut builder = Builder {
            source,
            segments: &mut segments,
            values: &mut values,
            sets: &mut sets,
            seen: HashSet::new(),
        };
        match merged_modules(&tree, source) {
            Some(bindings) => {
                for (name, node) in bindings {
                    modules.push(Module {
                        file,
                        name: Some(name),
                    });
                    builder.module(node, modules.len() - 1);
                }
            }
            None => {
                modules.push(Module { file, name: None });
                builder.module(&tree, modules.len() - 1);
            }
        }
    }
    let index = Index {
        project,
        lines,
        modules,
        sets,
        segments,
    };
    let mut findings = vec![Vec::new(); files.len()];
    identical(&index, &mut findings);
    overlapping(&index, &mut findings);
    for file in &mut findings {
        file.sort_by_key(|(range, _)| (range.start(), range.end()));
    }
    Findings(findings)
}

fn identical(index: &Index<'_>, findings: &mut [Vec<(TextRange, String)>]) {
    let sets = &index.sets;
    let mut ids: HashMap<&[Leaf], usize> = HashMap::new();
    let mut classes: Vec<Vec<usize>> = Vec::new();
    let mut class_of = vec![None; sets.len()];
    for (i, set) in sets.iter().enumerate() {
        if set.leaves.len() < MIN_LEAVES {
            continue;
        }
        let id = *ids.entry(&set.leaves).or_insert_with(|| {
            classes.push(Vec::new());
            classes.len() - 1
        });
        classes[id].push(i);
        class_of[i] = Some(id);
    }
    let mut order: Vec<usize> = (0..classes.len()).collect();
    order.sort_by_key(|&c| {
        (
            std::cmp::Reverse(sets[classes[c][0]].leaves.len()),
            classes[c][0],
        )
    });
    let mut reported = vec![false; classes.len()];
    for id in order {
        let class = &classes[id];
        let modules: HashSet<usize> = class.iter().map(|&s| sets[s].module).collect();
        if modules.len() < 2 {
            continue;
        }
        let covered = class.iter().all(|&site| {
            let mut at = sets[site].parent.as_ref().map(|(p, _)| *p);
            while let Some(p) = at {
                if class_of[p].is_some_and(|c| reported[c]) {
                    return true;
                }
                at = sets[p].parent.as_ref().map(|(q, _)| *q);
            }
            false
        });
        if covered {
            continue;
        }
        reported[id] = true;
        let count = sets[class[0]].leaves.len();
        for &site in class {
            let file = index.modules[sets[site].module].file;
            let others = index.sites(class.iter().copied().filter(|&o| o != site), file);
            findings[file].push((
                sets[site].anchor,
                format!(
                    "these {count} leaf bindings are identical to the attrset in {others}; define them once and reuse them"
                ),
            ));
        }
    }
}

fn shared<'l>(a: &'l [Leaf], b: &[Leaf]) -> Vec<&'l Leaf> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(&a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn overlapping(index: &Index<'_>, findings: &mut [Vec<(TextRange, String)>]) {
    let sets = &index.sets;
    let mut postings: HashMap<&Leaf, Vec<usize>> = HashMap::new();
    for (i, set) in sets.iter().enumerate() {
        if set.parent.is_none() {
            for leaf in &set.leaves {
                postings.entry(leaf).or_default().push(i);
            }
        }
    }
    let mut counts: HashMap<(usize, usize), usize> = HashMap::new();
    for roots in postings.values() {
        for (n, &a) in roots.iter().enumerate() {
            for &b in &roots[n + 1..] {
                if sets[a].module != sets[b].module {
                    *counts.entry((a, b)).or_default() += 1;
                }
            }
        }
    }
    let mut pairs: Vec<(usize, usize)> = counts
        .into_iter()
        .filter(|&(_, count)| count >= MIN_LEAVES)
        .map(|(pair, _)| pair)
        .collect();
    pairs.sort_unstable();
    for (mut a, mut b) in pairs {
        let common = shared(&sets[a].leaves, &sets[b].leaves);
        let mut depth = 0;
        while let Some((ca, cb, len)) = sets[a].children.iter().find_map(|(key, ca)| {
            let end = depth + key.len();
            let under = common
                .iter()
                .all(|(path, _)| path.len() > end && path[depth..end] == key[..]);
            if !under {
                return None;
            }
            let cb = sets[b]
                .children
                .iter()
                .find(|(other, _)| other == key)
                .map(|(_, cb)| *cb)?;
            Some((*ca, cb, key.len()))
        }) {
            a = ca;
            b = cb;
            depth += len;
        }
        if sets[a].leaves == sets[b].leaves {
            continue;
        }
        let count = common.len();
        let path = index.path(&common[0].0[..depth]);
        let target = if path.is_empty() {
            "an attrset".to_owned()
        } else {
            format!("`{path}`")
        };
        let mut sections: Vec<(u32, usize)> = Vec::new();
        for (leaf, _) in &common {
            match sections.iter_mut().find(|(key, _)| *key == leaf[depth]) {
                Some((_, n)) => *n += 1,
                None => sections.push((leaf[depth], 1)),
            }
        }
        sections.sort_by_key(|&(key, n)| (std::cmp::Reverse(n), key));
        let breakdown = if sections.len() > 1 {
            let parts: Vec<String> = sections
                .iter()
                .map(|&(key, n)| format!("`{}` {n}", index.path(&[key])))
                .collect();
            format!(", by key: {}", parts.join(", "))
        } else {
            String::new()
        };
        for (site, other) in [(a, b), (b, a)] {
            let file = index.modules[sets[site].module].file;
            findings[file].push((
                sets[site].anchor,
                format!(
                    "{count} key/value leaves here are identical (same key path and value) to {target} in {}, though the attrsets differ ({} leaves here, {} there{breakdown}); define the shared settings once and merge each module's own on top",
                    index.site(other, file),
                    sets[site].leaves.len(),
                    sets[other].leaves.len(),
                ),
            ));
        }
    }
}

pub struct DuplicateDefinitions;

impl Rule for DuplicateDefinitions {
    fn code(&self) -> &'static str {
        "duplicate-definitions"
    }

    fn name(&self) -> &'static str {
        "Duplicate definitions"
    }

    fn description(&self) -> &'static str {
        "Flags attrset subtrees with at least 8 leaf bindings that are identical (every relative key path and whitespace-normalized value) in two or more modules: different files, or different top-level bindings of a `strictix merge` file. Each group is reported once at every site, naming the others. When whole subtrees differ, two root attrsets (a module body, a `let` value, a list element; never one nested under a different key) that still share at least 8 identical key-path/value leaves are reported at the deepest pair holding all of them, with the count and a per-key breakdown. Keys are compared by full path, so a setting under a different section never matches."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
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
        let files = project.sources();
        let Some(current) = files
            .iter()
            .position(|f| model.path() == Some(f.path.as_path()))
            .or_else(|| files.iter().position(|f| f.source == model.source()))
        else {
            return;
        };
        let findings = project.memo(index);
        for (range, message) in &findings.0[current] {
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                message.clone(),
                *range,
            ));
        }
    }
}
