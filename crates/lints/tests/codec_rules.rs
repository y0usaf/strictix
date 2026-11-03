use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{CodecKind, CodecLoader};
use strictix_core::rules::{run_rules, Rule};
use strictix_core::semantic::SemanticModel;
use strictix_lints::config_file_rules::CodecCandidate;
use strictix_syntax::parse;

fn config() -> LintConfig {
    LintConfig {
        codecs: vec![("lisp".to_owned(), vec!["lisp".to_owned()])],
        codec_generators: vec![("lisp".to_owned(), "lispSource".to_owned())],
        ..LintConfig::default()
    }
}

fn candidates(source: &str, config: &LintConfig) -> Vec<strictix_core::diagnostic::Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree);
    let rules: Vec<Box<dyn Rule>> = vec![Box::new(CodecCandidate)];
    let mut diags = Vec::new();
    run_rules(&rules, &tree, &model, config, source, &mut diags);
    diags
}

#[test]
fn a_line_list_passed_to_a_codec_generator_is_a_lines_candidate() {
    let source = "{generators}: generators.lispSource (import ./lisp/bar.data.nix)";
    let diags = candidates(source, &config());
    assert_eq!(diags.len(), 1);
    let codec = diags[0].codec.as_ref().expect("a codec target");
    assert_eq!(codec.id, "lisp");
    assert_eq!(codec.kind, CodecKind::File);
    assert_eq!(codec.loader, Some(CodecLoader::Lines));
    assert_eq!(codec.target.as_deref(), Some("lisp/bar.data.nix"));
    assert_eq!(
        &source[codec.replace.start() as usize..codec.replace.end() as usize],
        "generators.lispSource (import ./lisp/bar.data.nix)"
    );
}

#[test]
fn other_generators_and_non_data_files_are_not_candidates() {
    let config = config();
    assert!(candidates(
        "{generators}: generators.tsSource (import ./x.data.nix)",
        &config
    )
    .is_empty());
    assert!(candidates(
        "{generators}: generators.lispSource (import ./x.nix)",
        &config
    )
    .is_empty());
    assert!(candidates("{generators}: generators.lispSource [\"(a)\"]", &config).is_empty());
}

#[test]
fn a_codec_without_a_generator_flags_no_line_lists() {
    let config = LintConfig {
        codecs: vec![("lisp".to_owned(), vec!["lisp".to_owned()])],
        ..LintConfig::default()
    };
    assert!(candidates(
        "{generators}: generators.lispSource (import ./x.data.nix)",
        &config
    )
    .is_empty());
}
