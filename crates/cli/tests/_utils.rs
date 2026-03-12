use std::{fs, io::Write, path::Path, process::Command};

use anyhow::anyhow;
use lib::{LINTS, Lint, Report};
use proptest::{collection::vec, prelude::*};
use rnix::{Parse, Root, SyntaxElement, WalkEvent};
use strictix::LintMap;
use tempfile::NamedTempFile;

pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

fn create_fixture(expression: &str) -> anyhow::Result<NamedTempFile> {
    let mut fixture = NamedTempFile::with_suffix(".nix")?;
    fixture.write_all(expression.as_bytes())?;
    fixture.write_all(b"\n")?; // otherwise diff says there's no newline at end of file
    Ok(fixture)
}

fn sanitize_output(output: Vec<u8>, path: &Path) -> anyhow::Result<String> {
    let output = strip_ansi_escapes::strip(output)?;
    let output = String::from_utf8(output)?;
    Ok(output.replace(path.to_str().unwrap(), "<temp_file_path>"))
}

pub fn run_cli(path: &Path, args: &[&str]) -> anyhow::Result<CliOutput> {
    let output = Command::new(env!("CARGO_BIN_EXE_strictix"))
        .args(args)
        .arg(path)
        .output()?;

    Ok(CliOutput {
        success: output.status.success(),
        stdout: sanitize_output(output.stdout, path)?,
        stderr: sanitize_output(output.stderr, path)?,
    })
}

pub fn test_cli(expression: &str, args: &[&str]) -> anyhow::Result<String> {
    let fixture = create_fixture(expression)?;
    Ok(run_cli(fixture.path(), args)?.stdout)
}

pub fn assert_fix_roundtrip(expression: &str) -> anyhow::Result<()> {
    let fixture = create_fixture(expression)?;
    let original = fs::read_to_string(fixture.path())?;

    let first_fix = run_cli(fixture.path(), &["fix"])?;
    assert!(
        first_fix.success,
        "strictix fix failed\nstdout:\n{}\nstderr:\n{}",
        first_fix.stdout, first_fix.stderr,
    );

    let first_after = fs::read_to_string(fixture.path())?;

    if first_after != original {
        let check = run_cli(fixture.path(), &["check"])?;
        assert!(
            check.success,
            "strictix check failed after fix\nstdout:\n{}\nstderr:\n{}",
            check.stdout, check.stderr,
        );
    }

    let second_fix = run_cli(fixture.path(), &["fix"])?;
    assert!(
        second_fix.success,
        "second strictix fix failed\nstdout:\n{}\nstderr:\n{}",
        second_fix.stdout, second_fix.stderr,
    );

    let second_after = fs::read_to_string(fixture.path())?;
    assert_eq!(
        first_after, second_after,
        "strictix fix is not idempotent\nfirst run stdout:\n{}\nfirst run stderr:\n{}\nsecond run stdout:\n{}\nsecond run stderr:\n{}",
        first_fix.stdout, first_fix.stderr, second_fix.stdout, second_fix.stderr,
    );

    let dry_run = run_cli(fixture.path(), &["fix", "--dry-run"])?;
    assert!(
        dry_run.success,
        "strictix fix --dry-run failed after convergence\nstdout:\n{}\nstderr:\n{}",
        dry_run.stdout, dry_run.stderr,
    );
    assert!(
        dry_run.stdout.trim().is_empty(),
        "strictix fix --dry-run still reports changes after convergence\nstdout:\n{}\nstderr:\n{}",
        dry_run.stdout,
        dry_run.stderr,
    );

    Ok(())
}

pub fn trivia_strategy() -> impl Strategy<Value = String> {
    vec(
        prop_oneof![
            Just(String::new()),
            Just(" ".to_string()),
            Just("\n".to_string()),
            Just("\n\n".to_string()),
            Just("# pad\n".to_string()),
            Just("# alpha\n# beta\n".to_string()),
            Just("  # indented\n".to_string()),
            Just(" \n".to_string()),
        ],
        0..4,
    )
    .prop_map(|parts| parts.concat())
}

pub fn assert_rewrite_invariants(
    lint_name: &str,
    expression: &str,
    prefix: &str,
    suffix: &str,
) -> anyhow::Result<()> {
    let source = format!("{prefix}{expression}{suffix}");
    assert_valid_syntax(&source, "generated source must parse before rewrite")?;

    let first_after = apply_rule_fixes(lint_name, &source)?;
    assert_valid_syntax(&first_after, "rewrite produced invalid syntax")?;

    let second_after = apply_rule_fixes(lint_name, &first_after)?;
    assert_eq!(
        first_after, second_after,
        "rewrite is not idempotent for lint `{lint_name}`\nsource:\n{source}\nfirst:\n{first_after}\nsecond:\n{second_after}",
    );

    Ok(())
}

fn apply_rule_fixes(lint_name: &str, source: &str) -> anyhow::Result<String> {
    let lints = lint_map_for(lint_name)?;
    let mut rewritten = source.to_owned();

    loop {
        let parsed = Root::parse(&rewritten);
        let _ = parsed.clone().ok().map_err(|err| anyhow!("{err}"))?;
        let reports = collect_filtered_reports(&parsed, &lints, |report| {
            report.total_suggestion_range().is_some()
        });

        if reports.is_empty() {
            break;
        }

        let before_pass = rewritten.clone();
        for report in reorder(reports) {
            let before = rewritten.clone();
            let range = report.range();
            report.apply(&mut rewritten);
            assert_report_stability(&before, &rewritten, range, lint_name);
        }

        if rewritten == before_pass {
            return Err(anyhow!(
                "rewrite for lint `{lint_name}` did not converge despite reporting fixes\nsource:\n{source}"
            ));
        }
    }

    Ok(rewritten)
}

fn assert_valid_syntax(source: &str, context: &str) -> anyhow::Result<()> {
    Root::parse(source)
        .ok()
        .map(|_| ())
        .map_err(|err| anyhow!("{context}: {err}\nsource:\n{source}"))
}

fn lint_map_for(lint_name: &str) -> anyhow::Result<LintMap> {
    let selected = LINTS
        .iter()
        .copied()
        .filter(|lint| lint.name() == lint_name)
        .collect::<Vec<_>>();

    if selected.is_empty() {
        return Err(anyhow!("unknown lint `{lint_name}`"));
    }

    Ok(lint_map_of(&selected))
}

fn lint_map_of(lints: &[&'static dyn Lint]) -> LintMap {
    let mut map = LintMap::new();
    for lint in lints {
        for &kind in lint.match_kind() {
            map.entry(kind)
                .and_modify(|rules: &mut Vec<_>| rules.push(*lint))
                .or_insert_with(|| vec![*lint]);
        }
    }
    map
}

fn collect_filtered_reports(
    root: &Parse<Root>,
    lints: &LintMap,
    predicate: impl Fn(&Report) -> bool,
) -> Vec<Report> {
    root.syntax()
        .preorder_with_tokens()
        .filter_map(|event| match event {
            WalkEvent::Enter(child) => Some(child),
            WalkEvent::Leave(_) => None,
        })
        .flat_map(|child| reports_for_element(child, lints))
        .filter(predicate)
        .collect()
}

fn reports_for_element<'a>(
    child: SyntaxElement,
    lints: &'a LintMap,
) -> impl Iterator<Item = Report> + 'a {
    lints.get(&child.kind()).into_iter().flat_map(move |rules| {
        let child = child.clone();
        rules.iter().filter_map(move |rule| rule.validate(&child))
    })
}

fn reorder(mut reports: Vec<Report>) -> Vec<Report> {
    use std::collections::VecDeque;

    reports.sort_by(|a, b| {
        let a_range = a.range();
        let b_range = b.range();
        a_range.end().partial_cmp(&b_range.end()).unwrap()
    });

    reports
        .into_iter()
        .fold(VecDeque::new(), |mut deque: VecDeque<Report>, report| {
            let report_range = report.range();
            if let Some(front_range) = deque.front().map(Report::range) {
                if report_range.start() > front_range.end() {
                    deque.push_front(report);
                }
            } else {
                deque.push_front(report);
            }
            deque
        })
        .into()
}

fn assert_report_stability(before: &str, after: &str, range: rnix::TextRange, lint_name: &str) {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    let before_bytes = before.as_bytes();
    let after_bytes = after.as_bytes();

    assert_eq!(
        &before_bytes[..start],
        &after_bytes[..start],
        "rewrite changed bytes before its advertised range for lint `{lint_name}`\nrange: {range:?}\nbefore:\n{before}\nafter:\n{after}",
    );

    let suffix_len = before_bytes.len() - end;
    assert!(
        after_bytes.len() >= suffix_len,
        "rewrite shrank past the unchanged suffix for lint `{lint_name}`\nrange: {range:?}\nbefore:\n{before}\nafter:\n{after}",
    );
    assert_eq!(
        &before_bytes[end..],
        &after_bytes[after_bytes.len() - suffix_len..],
        "rewrite changed bytes after its advertised range for lint `{lint_name}`\nrange: {range:?}\nbefore:\n{before}\nafter:\n{after}",
    );
}
