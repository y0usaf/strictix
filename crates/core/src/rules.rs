use crate::config::LintConfig;
use crate::context::Context;
use crate::diagnostic::{Diagnostic, Severity};
use crate::fix::{FixError, TextEdit};
use crate::semantic::SemanticModel;
use strictix_syntax::{parse, SyntaxKind, SyntaxNode};

const SUPPRESSION_PREFIX: &str = "# strictix: disable-next-line=";

fn apply_suppressions(source: &str, diags: &mut Vec<Diagnostic>) {
    if diags.is_empty() || !source.contains(SUPPRESSION_PREFIX) {
        return;
    }
    let mut suppressed_lines = std::collections::HashMap::<usize, Vec<&str>>::new();
    for (line_no, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let Some(code) = trimmed.strip_prefix(SUPPRESSION_PREFIX) else {
            continue;
        };
        if !code.is_empty() && !code.chars().any(char::is_whitespace) {
            suppressed_lines.entry(line_no + 1).or_default().push(code);
        }
    }
    let mut line_starts = vec![0usize];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(offset + 1);
        }
    }
    diags.retain(|diag| {
        let line = line_starts
            .partition_point(|&start| start <= diag.range.start() as usize)
            .saturating_sub(1);
        !suppressed_lines
            .get(&line)
            .is_some_and(|codes| codes.contains(&diag.code))
    });
}

pub trait Rule: Send + Sync {
        fn code(&self) -> &'static str;

        fn name(&self) -> &'static str;

        fn description(&self) -> &'static str;

        fn severity(&self) -> Severity;

        fn default_enabled(&self) -> bool {
        true
    }

        fn node_kind(&self) -> Option<SyntaxKind> {
        None
    }

        fn check_node(&self, _node: &SyntaxNode, _source: &str, _diags: &mut Vec<Diagnostic>) {}

    fn check_file(
        &self,
        _model: &SemanticModel,
        _config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
    }

        fn check_file_project(
        &self,
        model: &SemanticModel,
        config: &LintConfig,
        _project: Option<&crate::project::ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        self.check_file(model, config, diags);
    }

        fn check_path(
        &self,
        _path: &std::path::Path,
        _source: &str,
        _config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
    }
}

pub fn run_path_rules(
    rules: &[Box<dyn Rule>],
    path: &std::path::Path,
    source: &str,
    config: &LintConfig,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules {
        if config.is_enabled(rule.code())
            && (rule.default_enabled() || config.enabled.iter().any(|code| code == rule.code()))
        {
            rule.check_path(path, source, config, &mut diags);
        }
    }
    diags.sort_by_key(|d| d.range.start());
    diags
}

pub fn run_rules(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    diags: &mut Vec<Diagnostic>,
) {
    run_rules_project_inner(rules, tree, model, config, source, None, false, diags);
}

pub fn run_rules_project(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    project: Option<&crate::project::ProjectContext>,
    diags: &mut Vec<Diagnostic>,
) {
    run_rules_project_inner(rules, tree, model, config, source, project, true, diags);
}

#[allow(clippy::too_many_arguments)]
fn run_rules_project_inner(
    rules: &[Box<dyn Rule>],
    tree: &SyntaxNode,
    model: &SemanticModel,
    config: &LintConfig,
    source: &str,
    project: Option<&crate::project::ProjectContext>,
    include_syntax: bool,
    diags: &mut Vec<Diagnostic>,
) {
    if include_syntax && syntax_diagnostics(tree, diags) {
        apply_suppressions(source, diags);
        return;
    }
    const NUM_KINDS: usize = SyntaxKind::Eof as usize + 1;
    let mut kind_buckets: [Vec<(usize, &dyn Rule)>; NUM_KINDS] =
        std::array::from_fn(|_| Vec::new());
    let mut node_slots: Vec<Option<usize>> = Vec::with_capacity(rules.len());
    let mut active_node_count = 0usize;
    for rule in rules {
        let enabled = config.is_enabled(rule.code())
            && (rule.default_enabled() || config.enabled.iter().any(|code| code == rule.code()));
        if !enabled {
            node_slots.push(None);
            continue;
        }
        if let Some(kind) = rule.node_kind() {
            let slot = active_node_count;
            active_node_count += 1;
            kind_buckets[kind as usize].push((slot, rule.as_ref()));
            node_slots.push(Some(slot));
        } else {
            node_slots.push(Some(usize::MAX));
        }
    }
    let mut node_diags = if active_node_count > 0 {
        let mut buckets: Vec<Vec<Diagnostic>> = vec![Vec::new(); active_node_count];
        for node in tree.descendants() {
            for &(slot, rule) in &kind_buckets[node.kind() as usize] {
                rule.check_node(node, source, &mut buckets[slot]);
            }
        }
        buckets
    } else {
        Vec::new()
    };
    for (rule, slot) in rules.iter().zip(node_slots) {
        match slot {
            None => {}
            Some(usize::MAX) => rule.check_file_project(model, config, project, diags),
            Some(slot) => diags.append(&mut node_diags[slot]),
        }
    }
    if !diags.is_empty() {
        let trees = crate::generated::tree_ranges(model);
        if !trees.is_empty() {
            diags.retain(|diag| !crate::generated::inside(&trees, diag.range));
        }
    }
    apply_suppressions(source, diags);
}

fn syntax_diagnostics(tree: &SyntaxNode, diags: &mut Vec<Diagnostic>) -> bool {
    let mut fatal = false;
    for node in tree.error_nodes() {
        fatal = true;
        diags.push(Diagnostic::new(
            "syntax-error",
            Severity::Error,
            "syntax error: malformed syntax",
            node.content_range(),
        ));
    }
    fatal
}

pub const MAX_FIX_PASSES: usize = 10;

pub struct LintRun {
        pub diagnostics: Vec<Diagnostic>,
        pub fixed: Option<String>,
        pub passes: usize,
        pub error: Option<FixError>,
}

pub fn lint(
    rules: &[Box<dyn Rule>],
    source: &str,
    path: Option<&std::path::Path>,
    config: &LintConfig,
    fix: bool,
) -> LintRun {
    lint_project(rules, source, path, config, fix, None)
}

pub fn lint_project(
    rules: &[Box<dyn Rule>],
    source: &str,
    path: Option<&std::path::Path>,
    config: &LintConfig,
    fix: bool,
    project: Option<&crate::project::ProjectContext>,
) -> LintRun {
    if !fix {
        let tree = parse(source);
        let mut diags = Vec::new();
        let model = SemanticModel::new(source, &tree).with_path(path);
        run_rules_project(rules, &tree, &model, config, source, project, &mut diags);
        diags.sort_by_key(|d| d.range.start());
        return LintRun {
            diagnostics: diags,
            fixed: None,
            passes: 0,
            error: None,
        };
    }

    let mut context = Context::new(source.to_string());
    let mut diagnostics = Vec::new();
    let mut passes = 0usize;
    let mut error = None;

    loop {
        let tree = parse(context.source());
        let mut diags = Vec::new();
        let model = SemanticModel::new(context.source(), &tree).with_path(path);
        run_rules_project(
            rules,
            &tree,
            &model,
            config,
            context.source(),
            project,
            &mut diags,
        );

        let edits: Vec<TextEdit> = if fix {
            select_pass_edits(&diags)
        } else {
            Vec::new()
        };

        if passes == 0 {
            diags.sort_by_key(|d| d.range.start());
            diagnostics = diags;
        }

        if !fix || edits.is_empty() {
            break;
        }
        match context.commit(&edits) {
            Ok(()) => {}
            Err(err) => {
                context.rollback_all();
                error = Some(err);
                break;
            }
        }
        passes += 1;
        if passes >= MAX_FIX_PASSES {
            break;
        }
    }

    let final_text = context.source().to_string();
    let fixed = (final_text != source).then_some(final_text);

    LintRun {
        diagnostics,
        fixed,
        passes,
        error,
    }
}

fn select_pass_edits(diags: &[Diagnostic]) -> Vec<TextEdit> {
    let groups: Vec<&[TextEdit]> = diags
        .iter()
        .filter_map(|d| d.fix.as_ref())
        .map(|f| f.edits.as_slice())
        .filter(|edits| !edits.is_empty())
        .collect();

    let strictly_contains = |outer: &TextEdit, inner: &TextEdit| -> bool {
        outer.range.start() <= inner.range.start()
            && inner.range.end() <= outer.range.end()
            && outer.range != inner.range
    };

    let mut selected = Vec::new();
    for (i, group) in groups.iter().enumerate() {
        let nested_in_other = groups.iter().enumerate().any(|(j, other)| {
            i != j
                && group
                    .iter()
                    .all(|e_in| other.iter().any(|e_out| strictly_contains(e_out, e_in)))
        });
        if !nested_in_other {
            selected.extend(group.iter().cloned());
        }
    }
    selected
}

#[macro_export]
macro_rules! rules {
    ($($rule:ident),* $(,)?) => {
        vec![$(Box::new($rule {}) as Box<dyn $crate::rules::Rule>),*]
    };
}
