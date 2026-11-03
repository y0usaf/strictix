use crate::{column_of, line_of, nix};

const PREFIXES: [(&str, &str); 5] = [
    ("quoting_lit", "quote"),
    ("syn_quoting_lit", "quasi"),
    ("unquoting_lit", "unquote"),
    ("unquote_splicing_lit", "splice"),
    ("var_quoting_lit", "function"),
];

const BODIES: [(&str, usize); 38] = [
    ("block", 1),
    ("case", 1),
    ("define-extension", 3),
    ("defconstant", 1),
    ("defgeneric", 2),
    ("defmacro", 2),
    ("defmethod", 2),
    ("defpackage", 1),
    ("defparameter", 1),
    ("defstruct", 1),
    ("defun", 2),
    ("defvar", 1),
    ("destructuring-bind", 2),
    ("dolist", 1),
    ("dotimes", 1),
    ("ecase", 1),
    ("etypecase", 1),
    ("flet", 1),
    ("handler-bind", 1),
    ("handler-case", 1),
    ("ignore-errors", 0),
    ("labels", 1),
    ("lambda", 1),
    ("let", 1),
    ("let*", 1),
    ("macrolet", 1),
    ("multiple-value-bind", 2),
    ("prog1", 1),
    ("progn", 0),
    ("typecase", 1),
    ("unless", 1),
    ("unwind-protect", 1),
    ("when", 1),
    ("with-input-from-string", 1),
    ("with-open-file", 1),
    ("with-output-to-string", 1),
    ("with-slots", 2),
    ("loop", usize::MAX),
];

const LOCALS: [&str; 3] = ["flet", "labels", "macrolet"];

enum Form {
    Atom(String),
    Str(String),
    Prefix(&'static str, Box<Item>),
    Comment(String),
    List(Vec<Item>),
}

struct Item {
    form: Form,
    start: usize,
    end: usize,
}

struct Row<'a> {
    items: &'a [Item],
    first: usize,
    at: Option<usize>,
    blank: bool,
}

struct Emitter<'s> {
    source: &'s str,
    holes: &'s [String],
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into(), source)?;
    let root = tree.root_node();
    let mut items = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect(child, source, &mut items)?;
    }
    let items = merge_atoms(items, source)?;
    let emitter = Emitter { source, holes };
    let mut out = String::from("[\n");
    let mut previous = 0;
    for item in &items {
        let gap = &source[previous..item.start];
        if previous == 0 {
            if !gap.is_empty() {
                return Err(format!("line 1: the source starts with {gap:?}"));
            }
        } else {
            let lines = newlines(gap).ok_or_else(|| {
                format!(
                    "line {}: two top-level forms share a line or are separated by {gap:?}",
                    line_of(source, item.start)
                )
            })?;
            for _ in 1..lines {
                out.push_str("  blank\n");
            }
        }
        out.push_str("  ");
        out.push_str(&emitter.form(item, false, 2)?);
        out.push('\n');
        previous = item.end;
    }
    let tail = &source[previous..];
    if tail != "\n" {
        return Err(format!(
            "the source must end with one newline after its last form, not {tail:?}"
        ));
    }
    out.push(']');
    Ok(out)
}

fn newlines(gap: &str) -> Option<usize> {
    let lines = gap.bytes().take_while(|&b| b == b'\n').count();
    (lines > 0 && gap[lines..].bytes().all(|b| b == b' ') && !gap[lines..].contains('\n'))
        .then_some(lines)
}

fn is_list(node: tree_sitter::Node<'_>) -> bool {
    let mut cursor = node.walk();
    let first = node.children(&mut cursor).next();
    first.is_some_and(|child| !child.is_named() && child.kind() == "(")
}

fn structured(node: tree_sitter::Node<'_>) -> bool {
    if is_list(node)
        || matches!(node.kind(), "str_lit" | "comment" | "block_comment")
        || prefix(node.kind()).is_some()
    {
        return true;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.into_iter().any(structured)
}

fn prefix(kind: &str) -> Option<&'static str> {
    PREFIXES
        .iter()
        .find(|(node, _)| *node == kind)
        .map(|(_, name)| *name)
}

fn collect(node: tree_sitter::Node<'_>, source: &str, out: &mut Vec<Item>) -> Result<(), String> {
    let (start, end) = (node.start_byte(), node.end_byte());
    let line = line_of(source, start);
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    if is_list(node) {
        let close = children
            .last()
            .filter(|c| c.kind() == ")")
            .ok_or(format!("line {line}: unclosed list"))?;
        let mut items = Vec::new();
        for child in &children[1..children.len() - 1] {
            collect(*child, source, &mut items)?;
        }
        out.push(Item {
            form: Form::List(merge_atoms(items, source)?),
            start,
            end: close.end_byte(),
        });
        return Ok(());
    }
    let form = match node.kind() {
        "str_lit" => Form::Str(unescape(&source[start + 1..end - 1])),
        "comment" => {
            let text = source[start..end].trim_end_matches('\n');
            out.push(Item {
                form: Form::Comment(text.to_owned()),
                start,
                end: start + text.len(),
            });
            return Ok(());
        }
        "block_comment" => return Err(format!("line {line}: block comments are not supported")),
        kind => match prefix(kind) {
            Some(name) => {
                let mut inner = Vec::new();
                for child in &children[1..] {
                    collect(*child, source, &mut inner)?;
                }
                let mut inner = merge_atoms(inner, source)?;
                if inner.len() != 1 || inner[0].start != children[0].end_byte() {
                    return Err(format!(
                        "line {line}: a reader prefix must be followed directly by one form"
                    ));
                }
                Form::Prefix(name, Box::new(inner.remove(0)))
            }
            None if structured(node) => {
                for child in children {
                    collect(child, source, out)?;
                }
                return Ok(());
            }
            None => Form::Atom(source[start..end].to_owned()),
        },
    };
    out.push(Item { form, start, end });
    Ok(())
}

fn merge_atoms(items: Vec<Item>, source: &str) -> Result<Vec<Item>, String> {
    let mut out: Vec<Item> = Vec::new();
    for item in items {
        if let Some(last) = out.last_mut() {
            if last.end == item.start {
                match (&mut last.form, &item.form) {
                    (Form::Atom(text), Form::Atom(next)) => {
                        text.push_str(next);
                        last.end = item.end;
                        continue;
                    }
                    _ => {
                        return Err(format!(
                            "line {}: forms without a space between them",
                            line_of(source, item.start)
                        ))
                    }
                }
            }
        }
        out.push(item);
    }
    Ok(out)
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or('\\')
        } else {
            c
        });
    }
    out
}

fn number(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let mut parts = body.splitn(2, ['.', '/']);
    let whole = parts.next().unwrap_or("");
    let rest = parts.next();
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    match rest {
        None => !whole.is_empty() && digits(whole),
        Some(fraction) => {
            digits(whole)
                && !(whole.is_empty() && fraction.is_empty())
                && digits(fraction.split(['e', 'E', 'd', 'D']).next().unwrap_or(""))
        }
    }
}

fn symbol(text: &str) -> bool {
    !text.starts_with([':', '#', '"']) && !number(text)
}

pub fn offset(head: Option<&str>, local: bool, index: usize, first_row: usize) -> usize {
    if local {
        return if index > 1 { 2 } else { 4 };
    }
    let Some(head) = head else { return 1 };
    if let Some((_, body)) = BODIES.iter().find(|(name, _)| *name == head) {
        if *body != usize::MAX {
            return if index > *body { 2 } else { 4 };
        }
    }
    if symbol(head) && first_row >= 2 {
        head.len() + 2
    } else if head == "cond" {
        2
    } else {
        1
    }
}

impl Emitter<'_> {
    fn string(&self, text: &str, indent: usize) -> String {
        nix::string(text, self.holes, indent)
    }

    fn form(&self, item: &Item, local: bool, indent: usize) -> Result<String, String> {
        Ok(match &item.form {
            Form::Atom(text) => nix::int(text).unwrap_or_else(|| self.string(text, indent)),
            Form::Str(text) => format!("(str {})", self.string(text, indent)),
            Form::Comment(text) => format!("(comment {})", self.string(text, indent)),
            Form::Prefix(name, inner) => format!("({name} {})", self.form(inner, false, indent)?),
            Form::List(items) => self.list(item.start, items, local, false, indent)?,
        })
    }

    fn list(
        &self,
        open: usize,
        items: &[Item],
        local: bool,
        children_local: bool,
        indent: usize,
    ) -> Result<String, String> {
        let source = self.source;
        let line = |at: usize| line_of(source, at);
        if let Some(first) = items.first() {
            if first.start != open + 1 {
                return Err(format!(
                    "line {}: space after an opening parenthesis",
                    line(open)
                ));
            }
        }
        let head = match items.first().map(|item| &item.form) {
            Some(Form::Atom(text)) => Some(text.as_str()),
            _ => None,
        };
        let mut rows: Vec<Row<'_>> = Vec::new();
        let mut row_start = 0;
        for index in 1..=items.len() {
            let gap = if index < items.len() {
                &source[items[index - 1].end..items[index].start]
            } else {
                ""
            };
            if index < items.len() && gap == " " {
                if matches!(items[index - 1].form, Form::Comment(_)) {
                    return Err(format!(
                        "line {}: a comment runs to the end of its line",
                        line(items[index].start)
                    ));
                }
                continue;
            }
            rows.push(Row {
                items: &items[row_start..index],
                first: row_start,
                at: None,
                blank: false,
            });
            if index == items.len() {
                break;
            }
            let lines = newlines(gap).ok_or_else(|| {
                format!(
                    "line {}: items separated by {gap:?}",
                    line(items[index].start)
                )
            })?;
            for _ in 1..lines {
                rows.push(Row {
                    items: &[],
                    first: index,
                    at: None,
                    blank: true,
                });
            }
            row_start = index;
        }
        if let Some(last) = items.last() {
            if matches!(last.form, Form::Comment(_)) {
                return Err(format!(
                    "line {}: a comment before a closing parenthesis",
                    line(last.start)
                ));
            }
        }
        let first_row = rows.first().map_or(0, |row| row.items.len());
        let base = column_of(source, open);
        for row in rows.iter_mut().skip(1).filter(|row| !row.blank) {
            let actual = column_of(source, row.items[0].start) - base;
            if actual != offset(head, local, row.first, first_row) {
                row.at = Some(actual);
            }
        }
        let locals = head.is_some_and(|h| LOCALS.contains(&h));
        let item = |index: usize, indent: usize| -> Result<String, String> {
            let item = &items[index];
            if locals && index == 1 {
                if let Form::List(definitions) = &item.form {
                    return self.list(item.start, definitions, false, true, indent);
                }
            }
            self.form(item, children_local, indent)
        };
        if rows.len() <= 1 {
            let parts = (0..items.len())
                .map(|i| item(i, indent))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("[{}]", parts.join(" ")));
        }
        let pad = " ".repeat(indent + 2);
        let mut out = String::from("(rows [\n");
        for row in &rows {
            out.push_str(&pad);
            if row.blank {
                out.push_str("blank");
            } else {
                let parts = (row.first..row.first + row.items.len())
                    .map(|i| item(i, indent + 2))
                    .collect::<Result<Vec<_>, _>>()?;
                let lone = parts.len() == 1 && !parts[0].starts_with('[');
                let body = if lone {
                    parts[0].clone()
                } else {
                    format!("[{}]", parts.join(" "))
                };
                match row.at {
                    Some(at) => out.push_str(&format!("(at {at} {body})")),
                    None => out.push_str(&body),
                }
            }
            out.push('\n');
        }
        out.push_str(&" ".repeat(indent));
        out.push_str("])");
        Ok(out)
    }
}
