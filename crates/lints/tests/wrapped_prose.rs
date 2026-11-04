use std::path::Path;

use strictix_core::config::LintConfig;
use strictix_core::rules::Rule;
use strictix_lints::wrapped_prose::WrappedProse;

fn lines(path: &str, source: &str) -> Vec<usize> {
    let mut diags = Vec::new();
    WrappedProse {}.check_path(Path::new(path), source, &LintConfig::default(), &mut diags);
    diags
        .iter()
        .map(|d| source[..d.range.start() as usize].matches('\n').count() + 1)
        .collect()
}

#[test]
fn a_paragraph_or_item_on_several_lines_is_reported_once() {
    let source = "# Title\n\nOne paragraph\nthat goes on\nand on.\n\n- an item\n  continued\n- one line\n\nOne line.\n";
    assert_eq!(lines("README.md", source), [4, 8]);
    assert!(lines("notes.txt", source).is_empty());
    assert_eq!(lines("guide.MARKDOWN", source), [4, 8]);
}

#[test]
fn blocks_that_keep_their_lines_are_left_alone() {
    let source = concat!(
        "```nix\nlet\n  x = 1;\nin\n  x\n```\n\n",
        "~~~~\na\nb\n~~~~\n\n",
        "    indented\n    code\n\n",
        "<details>\n<summary>s</summary>\nmore html\n\n",
        "<pre>\na\n\nb\n</pre>\n\n",
        "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n",
        "> quoted\n> lines\n\n",
        "Heading\n=======\n\n",
        "text\n***\n\n",
        "[a]: https://a\n[b]: https://b\n\n",
        "hard  \nbreak\\\nkept\n",
    );
    assert!(
        lines("README.md", source).is_empty(),
        "{:?}",
        lines("README.md", source)
    );
}

#[test]
fn a_list_item_after_text_starts_a_new_block() {
    assert!(lines("a.md", "Text:\n- item\n- item\n1. first\n2) second\n").is_empty());
    assert_eq!(lines("a.md", "Text\n-not an item\n"), [2]);
}
