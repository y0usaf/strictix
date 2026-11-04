use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::rules::Rule;
use strictix_lints::scattered_documentation::ScatteredDocumentation;

fn reported(path: &str, config: &LintConfig) -> usize {
    let mut diags = Vec::new();
    ScatteredDocumentation {}.check_path(Path::new(path), config, &mut diags);
    diags.len()
}

#[test]
fn markdown_other_than_the_document_is_reported() {
    let default = LintConfig::default();
    assert_eq!(reported("./README.md", &default), 0);
    assert_eq!(reported("docs/guide.md", &default), 1);
    assert_eq!(reported("docs/guide.MD", &default), 1);
    assert_eq!(reported("docs/guide.nix", &default), 0);
    assert_eq!(reported("docs/notes.txt", &default), 0);
    let config =
        LintConfig::default().with_setting("scattered-documentation", "file", "modules/README.md");
    assert_eq!(reported("./modules/README.md", &config), 0);
    assert_eq!(reported("modules/README.md", &config), 0);
    assert_eq!(reported("README.md", &config), 1);
    assert_eq!(reported("modules/a/README.md", &config), 1);
}
