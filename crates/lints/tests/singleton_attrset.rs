use std::path::Path;

use strictix_core::{
    config::LintConfig, diagnostic::Diagnostic, fix::apply_fixes, rules::Rule,
    semantic::SemanticModel,
};
use strictix_lints::singleton_attrset::SingletonAttrset;
use strictix_syntax::parse;

fn check(source: &str, path: &str, config: &LintConfig) -> Vec<Diagnostic> {
    let tree = parse(source);
    let model = SemanticModel::new(source, &tree).with_path(Some(Path::new(path)));
    let mut diags = Vec::new();
    SingletonAttrset.check_file(&model, config, &mut diags);
    diags
}

fn fixed(source: &str) -> String {
    let edits: Vec<_> = check(source, "m.nix", &LintConfig::default())
        .into_iter()
        .filter_map(|d| d.fix)
        .flat_map(|f| f.edits)
        .collect();
    apply_fixes(source, &edits).unwrap()
}

#[test]
fn a_chain_of_one_binding_sets_becomes_one_path() {
    let source = "{\n  a = {\n    b = {\n      c = 1;\n    };\n  };\n  d = 2;\n}\n";
    let diags = check(source, "m.nix", &LintConfig::default());
    assert_eq!(diags.len(), 1);
    assert_eq!(
        diags[0].message,
        "this set holds one binding; write `a.b.c = ...;`"
    );
    assert_eq!(fixed(source), "{\n  a.b.c = 1;\n  d = 2;\n}\n");
}

#[test]
fn the_value_moves_out_by_the_levels_it_loses() {
    let source = "{\n  x = {\n    \"y z\" = {\n      p = 1;\n      q = ''\n        text\n\n      '';\n    };\n  };\n}\n";
    assert_eq!(
        fixed(source),
        "{\n  x.\"y z\" = {\n    p = 1;\n    q = ''\n      text\n\n    '';\n  };\n}\n"
    );
}

#[test]
fn a_multiline_quoted_string_keeps_its_text() {
    let source = "{\n  x = {\n    y = \"a\n      b\";\n  };\n}\n";
    assert_eq!(fixed(source), "{\n  x.y = \"a\n      b\";\n}\n");
}

#[test]
fn sets_the_rewrite_would_change_are_left_alone() {
    for source in [
        "{\n  a = { b = 1; };\n}\n",
        "let\n  a = {\n    b = 1;\n  };\nin a\n",
        "rec {\n  a = {\n    b = 1;\n  };\n}\n",
        "{\n  a = {\n    b = 1;\n  };\n  a.c = 2;\n}\n",
        "{\n  a = {\n    inherit b;\n  };\n}\n",
        "{\n  a = {\n    # why\n    b = 1;\n  };\n}\n",
        "{\n  a = {\n    b = rec {\n      c = 1;\n    };\n    d = 2;\n  };\n}\n",
    ] {
        assert!(
            check(source, "m.nix", &LintConfig::default()).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn a_finding_inside_another_waits_for_its_fix() {
    let source = "{\n  a = {\n    b = {\n      c = {\n        d = 1;\n      };\n      e = 2;\n    };\n  };\n}\n";
    let diags = check(source, "m.nix", &LintConfig::default());
    assert_eq!(diags.len(), 2);
    assert!(diags[0].fix.is_some());
    assert!(diags[1].fix.is_none());
    let once = fixed(source);
    assert_eq!(
        once,
        "{\n  a.b = {\n    c = {\n      d = 1;\n    };\n    e = 2;\n  };\n}\n"
    );
    assert_eq!(
        fixed(&once),
        "{\n  a.b = {\n    c.d = 1;\n    e = 2;\n  };\n}\n"
    );
}

#[test]
fn exempt_paths_are_skipped() {
    let source = "{\n  a = {\n    b = 1;\n  };\n}\n";
    let config = LintConfig::default().with_setting("singleton-attrset", "exempt", ".data.nix");
    assert!(check(source, "modules/x.data.nix", &config).is_empty());
    assert_eq!(check(source, "modules/x.nix", &config).len(), 1);
}
