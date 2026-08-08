//! Rule dispatch integration tests.
//!
//! These tests define their own dummy rule structs (node rules and file
//! rules) and exercise the machinery in `strictix_core::rules`:
//! node-vs-file dispatch, per-node firing order, config gating, config
//! pass-through, and the `rules!` registry macro. No real lint depends
//! on these tests; the semantics of each builtin rule are covered in
//! `strictix-lints`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{parse, SyntaxKind, SyntaxNode};

// --- dummy rules -----------------------------------------------------

/// Node rule: records every check_node call plus the node's range, and
/// records any check_file call (there should be none — node rules must
/// never touch the semantic model).
#[derive(Clone)]
struct CountingNodeRule {
    calls: Arc<AtomicUsize>,
    ranges: Arc<Mutex<Vec<(u32, u32)>>>,
    file_calls: Arc<AtomicUsize>,
    kind: SyntaxKind,
}

impl CountingNodeRule {
    fn new(kind: SyntaxKind) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            ranges: Arc::new(Mutex::new(Vec::new())),
            file_calls: Arc::new(AtomicUsize::new(0)),
            kind,
        }
    }
}

impl Rule for CountingNodeRule {
    fn code(&self) -> &'static str {
        "test-node-rule"
    }
    fn name(&self) -> &'static str {
        "Test node rule"
    }
    fn description(&self) -> &'static str {
        "Dummy node rule used by dispatch tests."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
    fn node_kind(&self) -> Option<SyntaxKind> {
        Some(self.kind)
    }
    fn check_node(&self, node: &SyntaxNode, _source: &str, _diags: &mut Vec<Diagnostic>) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.ranges
            .lock()
            .unwrap()
            .push((node.range().start(), node.range().end()));
    }
    fn check_file(
        &self,
        _model: &SemanticModel,
        _config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
        self.file_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// File rule: records every check_file call plus the config it received,
/// and records any check_node call (there should be none).
#[derive(Clone)]
struct CountingFileRule {
    calls: Arc<AtomicUsize>,
    seen_configs: Arc<Mutex<Vec<LintConfig>>>,
    node_calls: Arc<AtomicUsize>,
}

impl CountingFileRule {
    fn new() -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            seen_configs: Arc::new(Mutex::new(Vec::new())),
            node_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Rule for CountingFileRule {
    fn code(&self) -> &'static str {
        "test-file-rule"
    }
    fn name(&self) -> &'static str {
        "Test file rule"
    }
    fn description(&self) -> &'static str {
        "Dummy file rule used by dispatch tests."
    }
    fn severity(&self) -> Severity {
        Severity::Error
    }
    fn check_node(&self, _node: &SyntaxNode, _source: &str, _diags: &mut Vec<Diagnostic>) {
        self.node_calls.fetch_add(1, Ordering::SeqCst);
    }
    fn check_file(
        &self,
        _model: &SemanticModel,
        config: &LintConfig,
        _diags: &mut Vec<Diagnostic>,
    ) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen_configs.lock().unwrap().push(config.clone());
    }
}

// --- dispatch tests --------------------------------------------------

#[test]
fn node_rule_fires_once_per_matching_node_in_tree_order() {
    let source = "if true then 1 else 2\nif false then 3 else 4";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let config = LintConfig::default();

    let rule = CountingNodeRule::new(SyntaxKind::IfExpr);
    let calls = rule.calls.clone();
    let ranges = rule.ranges.clone();
    let file_calls = rule.file_calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];

    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);

    // 2 IfExpr nodes -> exactly 2 calls, in ascending source order.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let ranges = ranges.lock().unwrap();
    let starts: Vec<u32> = ranges.iter().map(|(s, _)| *s).collect();
    assert!(
        starts.windows(2).all(|w| w[0] < w[1]),
        "node rules must fire in tree (source) order"
    );
    // check_file must NOT be called for a node rule.
    assert_eq!(file_calls.load(Ordering::SeqCst), 0);
    assert!(diags.is_empty());
}

#[test]
fn node_rule_does_not_fire_for_other_kinds() {
    let source = "let x = 1 + 2; in [ x x ]";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let config = LintConfig::default();

    let rule = CountingNodeRule::new(SyntaxKind::IfExpr); // no IfExpr in source
    let calls = rule.calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];

    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);

    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn file_rule_fires_exactly_once_per_run() {
    let source = "1 + 2";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let config = LintConfig::default();

    let rule = CountingFileRule::new();
    let calls = rule.calls.clone();
    let seen = rule.seen_configs.clone();
    let node_calls = rule.node_calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];

    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);
    run_rules(&rules, &tree, &model, &config, source, &mut diags);

    // Exactly once per run_rules call: two calls -> two invocations.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(seen.lock().unwrap().len(), 2);
    // check_node must NOT be called for a file rule.
    assert_eq!(node_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn disabled_rule_does_not_fire() {
    let source = "if true then 1 else 2";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let mut diags = Vec::new();

    // Disabled node rule.
    let config = LintConfig::default().with_disabled(vec!["test-node-rule".to_string()]);
    let rule = CountingNodeRule::new(SyntaxKind::IfExpr);
    let calls = rule.calls.clone();
    let file_calls = rule.file_calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];
    run_rules(&rules, &tree, &model, &config, source, &mut diags);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(file_calls.load(Ordering::SeqCst), 0);

    // Disabled file rule.
    let config = LintConfig::default().with_disabled(vec!["test-file-rule".to_string()]);
    let rule = CountingFileRule::new();
    let calls = rule.calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];
    run_rules(&rules, &tree, &model, &config, source, &mut diags);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn run_rules_passes_config_through_unchanged() {
    let source = "1 + 2";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let config = LintConfig::default()
        .with_disabled(vec!["a".to_string(), "b".to_string()])
        .with_schema("options.json");

    let rule = CountingFileRule::new();
    let seen = rule.seen_configs.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(rule)];

    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].disabled, config.disabled);
    assert_eq!(seen[0].schema, config.schema);
}

#[test]
fn lint_config_builders_and_is_enabled() {
    let cfg = LintConfig::default();
    assert!(cfg.schema.is_none(), "schema defaults to None");
    assert!(cfg.is_enabled("anything"));

    let cfg = cfg.with_disabled(vec!["a".to_string(), "b".to_string()]);
    assert!(!cfg.is_enabled("a"));
    assert!(!cfg.is_enabled("b"));
    assert!(cfg.is_enabled("c"), "unlisted codes stay enabled");

    let cfg = cfg.with_schema("options.json");
    assert_eq!(cfg.schema.as_deref(), Some(Path::new("options.json")));

    // with_disabled replaces the list; it does not accumulate.
    let cfg = LintConfig::default()
        .with_disabled(vec!["a".to_string()])
        .with_disabled(vec!["b".to_string()]);
    assert!(cfg.is_enabled("a"));
    assert!(!cfg.is_enabled("b"));
}

// --- rules! macro ----------------------------------------------------

struct MacroRuleA;
struct MacroRuleB;

impl Rule for MacroRuleA {
    fn code(&self) -> &'static str {
        "macro-rule-a"
    }
    fn name(&self) -> &'static str {
        "Macro rule A"
    }
    fn description(&self) -> &'static str {
        "Dummy type for the rules! macro test."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
}

impl Rule for MacroRuleB {
    fn code(&self) -> &'static str {
        "macro-rule-b"
    }
    fn name(&self) -> &'static str {
        "Macro rule B"
    }
    fn description(&self) -> &'static str {
        "Dummy type for the rules! macro test."
    }
    fn severity(&self) -> Severity {
        Severity::Warning
    }
}

#[test]
fn rules_macro_expands_to_boxed_registry() {
    let registry = strictix_core::rules! { MacroRuleA, MacroRuleB };
    let codes: Vec<&str> = registry.iter().map(|r| r.code()).collect();
    assert_eq!(codes, ["macro-rule-a", "macro-rule-b"]);

    // A trailing comma is accepted.
    let registry: Vec<Box<dyn Rule>> = strictix_core::rules! { MacroRuleA, };
    assert_eq!(registry.len(), 1);
    assert_eq!(registry[0].code(), "macro-rule-a");
}

// --- multiple node rules ---------------------------------------------

#[test]
fn multiple_node_rules_of_different_kinds_fire_on_same_tree() {
    let source = "if true then [ 1 ] else [ 2 3 ]";
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let config = LintConfig::default();

    let if_rule = CountingNodeRule::new(SyntaxKind::IfExpr);
    let if_calls = if_rule.calls.clone();
    let list_rule = CountingNodeRule::new(SyntaxKind::ListExpr);
    let list_calls = list_rule.calls.clone();
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(if_rule), Box::new(list_rule)];

    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, &config, source, &mut diags);

    // One IfExpr, two ListExpr nodes; both rules fire on the same tree.
    assert_eq!(if_calls.load(Ordering::SeqCst), 1);
    assert_eq!(list_calls.load(Ordering::SeqCst), 2);
}
