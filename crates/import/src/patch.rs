use crate::{line_of, nix};

const HEADERS: [&str; 9] = [
    "command",
    "file_change",
    "binary_change",
    "index",
    "similarity",
    "old_file",
    "new_file",
    "location",
    "comment",
];

struct Hunk<'s> {
    start: usize,
    heading: Option<&'s str>,
    lines: Vec<&'s str>,
}

struct Lines<'s> {
    rows: Vec<&'s str>,
    kinds: Vec<Option<&'static str>>,
    at: usize,
}

impl<'s> Lines<'s> {
    fn peek(&self) -> Option<(&'s str, Option<&'static str>)> {
        self.rows
            .get(self.at)
            .map(|row| (*row, self.kinds[self.at]))
    }

    fn take(&mut self, kind: &str, prefix: &str) -> Result<&'s str, String> {
        match self.peek() {
            Some((row, Some(found))) if found == kind => {
                self.at += 1;
                row.strip_prefix(prefix)
                    .ok_or_else(|| self.error(&format!("expected {prefix:?}")))
            }
            _ => Err(self.error(&format!("expected a {kind} line"))),
        }
    }

    fn error(&self, message: &str) -> String {
        format!("line {}: {message}", self.at.min(self.rows.len()) + 1)
    }
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_diff::LANGUAGE.into(), source)?;
    if source.contains('\r') {
        return Err(format!(
            "line {}: a carriage return",
            line_of(source, source.find('\r').unwrap_or(0))
        ));
    }
    let Some(body) = source.strip_suffix('\n') else {
        return Err("the patch must end with a newline".into());
    };
    let rows: Vec<&str> = body.split('\n').collect();
    let mut kinds = vec![None; rows.len()];
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if let Some(kind) = HEADERS.iter().find(|kind| **kind == node.kind()) {
            let row = node.start_position().row;
            if row < kinds.len() && kinds[row].is_none() {
                kinds[row] = Some(*kind);
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    let mut lines = Lines { rows, kinds, at: 0 };
    let mut out = String::from("[\n");
    while lines.peek().is_some() {
        out.push_str(&file(&mut lines, holes)?);
    }
    out.push(']');
    Ok(out)
}

fn file(lines: &mut Lines<'_>, holes: &[String]) -> Result<String, String> {
    let index = match lines.peek() {
        Some((_, Some("command"))) => {
            let command = lines.take("command", "diff --git a/")?;
            let line = lines.at;
            let index = lines.take("index", "index ")?;
            Some((command, index, line))
        }
        _ => None,
    };
    let path = lines.take("old_file", "--- a/")?;
    let new = lines.take("new_file", "+++ b/")?;
    if new != path {
        return Err(lines.error("the old and new paths differ"));
    }
    if let Some((command, _, line)) = index {
        if command != format!("{path} b/{path}") {
            return Err(format!(
                "line {line}: the diff --git paths differ from ---/+++"
            ));
        }
    }
    let mut hunks = Vec::new();
    let mut delta: isize = 0;
    while let Some((_, Some("location"))) = lines.peek() {
        let (hunk, change) = hunk(lines, delta)?;
        delta += change;
        hunks.push(hunk);
    }
    if hunks.is_empty() {
        return Err(lines.error("a file without hunks"));
    }
    let head = match index {
        Some((_, index, _)) => format!(
            "(git {} {} [\n",
            nix::string(path, holes, 0),
            nix::string(index, holes, 0)
        ),
        None => format!("(file {} [\n", nix::string(path, holes, 0)),
    };
    let mut out = format!("  {head}");
    for hunk in &hunks {
        out.push_str(&match hunk.heading {
            Some(heading) => format!(
                "    (section {} {} [\n",
                hunk.start,
                nix::string(heading, holes, 0)
            ),
            None => format!("    (hunk {} [\n", hunk.start),
        });
        for line in &hunk.lines {
            out.push_str("      ");
            out.push_str(&change(line, holes));
            out.push('\n');
        }
        out.push_str("    ])\n");
    }
    out.push_str("  ])\n");
    Ok(out)
}

fn change(line: &str, holes: &[String]) -> String {
    let (mark, text) = line.split_at(1);
    match mark {
        "+" => format!("(add {})", nix::string(text, holes, 0)),
        "-" => format!("(del {})", nix::string(text, holes, 0)),
        _ => nix::string(text, holes, 0),
    }
}

fn span(start: usize, count: usize) -> String {
    if count == 1 {
        start.to_string()
    } else {
        format!("{start},{count}")
    }
}

fn hunk<'s>(lines: &mut Lines<'s>, delta: isize) -> Result<(Hunk<'s>, isize), String> {
    let location = lines.take("location", "@@ -")?;
    let line = lines.at;
    let malformed = || format!("line {line}: a hunk header this printer does not write");
    let (ranges, heading) = match location.split_once(" @@") {
        Some((ranges, "")) => (ranges, None),
        Some((ranges, rest)) => (ranges, Some(rest.strip_prefix(' ').ok_or_else(malformed)?)),
        None => return Err(malformed()),
    };
    let (old, new) = ranges.split_once(" +").ok_or_else(malformed)?;
    let range = |text: &str| -> Result<(usize, usize), String> {
        let (start, count) = text.split_once(',').unwrap_or((text, "1"));
        let parse = |n: &str| {
            n.parse::<usize>()
                .ok()
                .filter(|_| n == "0" || !n.starts_with('0'))
        };
        Ok((
            parse(start).ok_or_else(malformed)?,
            parse(count).ok_or_else(malformed)?,
        ))
    };
    let (start, old_count) = range(old)?;
    let (new_start, new_count) = range(new)?;
    let mut lines_in = Vec::new();
    let (mut olds, mut news) = (0, 0);
    while olds < old_count || news < new_count {
        let Some((row, _)) = lines.peek() else {
            return Err(lines.error("the patch ends inside a hunk"));
        };
        match row.as_bytes().first() {
            Some(b' ') => {
                olds += 1;
                news += 1;
            }
            Some(b'-') => olds += 1,
            Some(b'+') => news += 1,
            _ => {
                return Err(
                    lines.error("a hunk line that is not context, an addition or a deletion")
                )
            }
        }
        lines_in.push(row);
        lines.at += 1;
    }
    let hunk = Hunk {
        start,
        heading,
        lines: lines_in,
    };
    let header = format!(
        "-{} +{} @@",
        span(start, old_count),
        span(derived(start, old_count, new_count, delta), new_count)
    );
    if olds != old_count
        || news != new_count
        || format!("-{ranges} @@") != header
        || new_start != derived(start, old_count, new_count, delta)
    {
        return Err(format!("line {line}: the hunk's counts or new start are not the ones its lines and earlier hunks give"));
    }
    Ok((hunk, new_count as isize - old_count as isize))
}

fn derived(start: usize, old: usize, new: usize, delta: isize) -> usize {
    let position = start as isize + isize::from(old == 0) + delta - isize::from(new == 0);
    usize::try_from(position).unwrap_or(0)
}
