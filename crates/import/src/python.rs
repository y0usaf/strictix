use tree_sitter::Node;

use crate::{column_of, line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let masked = mask(source);
    let tree = crate::parse(&tree_sitter_python::LANGUAGE.into(), &masked)?;
    let importer = Importer { source, holes };
    let nodes = named(tree.root_node());
    let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
        return Err("the source holds no statement".into());
    };
    if first.start_byte() != 0 {
        return Err("line 1: the source starts with blank space".into());
    }
    let tail = &source[last.end_byte()..];
    if tail != "\n" {
        return Err(format!(
            "the source must end with one newline after its last statement, not {tail:?}"
        ));
    }
    importer.statements(&nodes, 0, 0)
}

fn mask(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'@' {
            let name = bytes[at + 1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            let close = at + 1 + name;
            if name > 0 && !bytes[at + 1].is_ascii_digit() && bytes.get(close) == Some(&b'@') {
                bytes[at] = b'_';
                bytes[close] = b'_';
                at = close + 1;
                continue;
            }
        }
        at += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned())
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "line_continuation")
        .collect()
}

fn kids(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() != "line_continuation")
        .collect()
}

#[derive(Clone, Copy)]
struct Ctx {
    margin: usize,
    bare: bool,
}

enum Expr {
    Name(String),
    Nix(String),
    Rows(Vec<Vec<String>>),
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
}

fn row(items: &[String]) -> String {
    if items.len() == 1 && !items[0].starts_with('[') {
        items[0].clone()
    } else {
        format!("[{}]", items.join(" "))
    }
}

fn simple(nix: &str) -> bool {
    nix.starts_with('"') || nix.bytes().all(|b| b.is_ascii_digit()) || nix.starts_with("(-")
}

impl Importer<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn line(&self, at: usize) -> usize {
        line_of(self.source, at)
    }

    fn want(&self, from: usize, to: usize, expected: &str, what: &str) -> Result<(), String> {
        let gap = &self.source[from..to];
        if gap == expected {
            Ok(())
        } else {
            Err(format!(
                "line {}: {what}: {gap:?} where the printer writes {expected:?}",
                self.line(to)
            ))
        }
    }

    fn adjacent(&self, nodes: &[Node<'_>], gaps: &[&str], what: &str) -> Result<(), String> {
        for (pair, gap) in nodes.windows(2).zip(gaps) {
            self.want(pair[0].end_byte(), pair[1].start_byte(), gap, what)?;
        }
        Ok(())
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 4)
    }

    fn nix(&self, expr: Expr) -> String {
        match expr {
            Expr::Name(name) => nix::int(&name).unwrap_or_else(|| self.string(&name)),
            Expr::Nix(nix) => nix,
            Expr::Rows(rows) if rows.len() == 1 => row(&rows[0]),
            Expr::Rows(rows) => {
                let rows: Vec<String> = rows.iter().map(|r| row(r)).collect();
                format!("(rows [{}])", rows.join(" "))
            }
        }
    }

    fn value(&self, node: Node<'_>, ctx: Ctx) -> Result<String, String> {
        let expr = self.expr(node, ctx)?;
        Ok(self.nix(expr))
    }

    fn statements(
        &self,
        nodes: &[Node<'_>],
        indent: usize,
        depth: usize,
    ) -> Result<String, String> {
        let pad = " ".repeat(depth + 2);
        let mut out = String::from("[\n");
        for (index, node) in nodes.iter().enumerate() {
            if index > 0 {
                let gap = &self.source[nodes[index - 1].end_byte()..node.start_byte()];
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 || gap[lines..] != " ".repeat(indent) {
                    return Err(format!(
                        "line {}: statements separated by {gap:?}, not newlines and {indent} spaces",
                        self.line(node.start_byte())
                    ));
                }
                for _ in 1..lines {
                    out.push_str(&pad);
                    out.push_str("blank\n");
                }
            }
            out.push_str(&pad);
            out.push_str(&self.statement(*node, indent, depth + 2)?);
            out.push('\n');
        }
        out.push_str(&" ".repeat(depth));
        out.push(']');
        Ok(out)
    }

    fn block(
        &self,
        colon: Node<'_>,
        block: Node<'_>,
        indent: usize,
        depth: usize,
    ) -> Result<String, String> {
        if block.kind() != "block" {
            return Err(format!(
                "line {}: expected a block",
                self.line(block.start_byte())
            ));
        }
        let nodes = named(block);
        let first = nodes
            .first()
            .ok_or_else(|| format!("line {}: an empty block", self.line(block.start_byte())))?;
        self.want(
            colon.end_byte(),
            first.start_byte(),
            &format!("\n{}", " ".repeat(indent + 4)),
            "a block starts on the next line, four spaces in",
        )?;
        self.statements(&nodes, indent + 4, depth)
    }

    fn statement(&self, node: Node<'_>, indent: usize, depth: usize) -> Result<String, String> {
        let ctx = Ctx {
            margin: indent,
            bare: true,
        };
        let parts = kids(node);
        match node.kind() {
            "comment" => Ok(format!("(comment {})", self.string(self.text(node)))),
            "function_definition" => self.async_or(&parts, |parts| {
                if parts.len() != 5 {
                    return Err(format!(
                        "line {}: a def with a return annotation",
                        self.line(node.start_byte())
                    ));
                }
                self.adjacent(&parts[..4], &[" ", "", ""], "def header")?;
                let params = self.seq(
                    parts[2],
                    Ctx {
                        margin: indent,
                        bare: false,
                    },
                    false,
                )?;
                let body = self.block(parts[3], parts[4], indent, depth)?;
                Ok(format!(
                    "(def {} {params} {body})",
                    self.string(self.text(parts[1]))
                ))
            }),
            "class_definition" => {
                let (bases, colon) = match parts.len() {
                    4 => {
                        self.adjacent(&parts[..3], &[" ", ""], "class header")?;
                        ("null".to_owned(), 2)
                    }
                    5 => {
                        self.adjacent(&parts[..4], &[" ", "", ""], "class header")?;
                        (
                            self.seq(
                                parts[2],
                                Ctx {
                                    margin: indent,
                                    bare: false,
                                },
                                false,
                            )?,
                            3,
                        )
                    }
                    _ => {
                        return Err(format!(
                            "line {}: an unsupported class",
                            self.line(node.start_byte())
                        ))
                    }
                };
                let body = self.block(parts[colon], parts[colon + 1], indent, depth)?;
                Ok(format!(
                    "(class {} {bases} {body})",
                    self.string(self.text(parts[1]))
                ))
            }
            "if_statement" => {
                self.adjacent(&parts[..3], &[" ", ""], "if header")?;
                let test = self.value(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent, depth)?;
                self.branches(test, body, parts[3], &parts[4..], indent, depth)
            }
            "for_statement" => self.async_or(&parts, |parts| {
                if parts.len() != 6 {
                    return Err(format!(
                        "line {}: a for with an else",
                        self.line(node.start_byte())
                    ));
                }
                self.adjacent(&parts[..5], &[" ", " ", " ", ""], "for header")?;
                let target = self.value(parts[1], ctx)?;
                let iter = self.value(parts[3], ctx)?;
                let body = self.block(parts[4], parts[5], indent, depth)?;
                Ok(format!("(for {target} {iter} {body})"))
            }),
            "while_statement" => {
                if parts.len() != 4 {
                    return Err(format!(
                        "line {}: a while with an else",
                        self.line(node.start_byte())
                    ));
                }
                self.adjacent(&parts[..3], &[" ", ""], "while header")?;
                let test = self.value(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent, depth)?;
                Ok(format!("(while {test} {body})"))
            }
            "with_statement" => self.async_or(&parts, |parts| {
                if parts.len() != 4 || parts[1].kind() != "with_clause" {
                    return Err(format!(
                        "line {}: an unsupported with",
                        self.line(node.start_byte())
                    ));
                }
                self.adjacent(&parts[..3], &[" ", ""], "with header")?;
                let clause = kids(parts[1]);
                let items = self.commas(&clause, ctx, "with items")?;
                let body = self.block(parts[2], parts[3], indent, depth)?;
                Ok(format!("(with' [{}] {body})", items.join(" ")))
            }),
            "try_statement" => {
                self.adjacent(&parts[..2], &[""], "try header")?;
                let body = self.block(parts[1], parts[2], indent, depth)?;
                let mut clauses = Vec::new();
                let mut previous = parts[2];
                for clause in &parts[3..] {
                    self.want(
                        previous.end_byte(),
                        clause.start_byte(),
                        &format!("\n{}", " ".repeat(indent)),
                        "a clause on the line after its block",
                    )?;
                    clauses.push(self.clause(*clause, indent, depth)?);
                    previous = *clause;
                }
                Ok(format!("(try {body} [{}])", clauses.join(" ")))
            }
            "decorated_definition" => Err(format!(
                "line {}: decorators are not supported",
                self.line(node.start_byte())
            )),
            "expression_statement" if parts.len() == 1 => self.value(parts[0], ctx),
            _ => {
                let expr = self.spaced(&parts, ctx)?;
                Ok(self.nix(expr))
            }
        }
    }

    fn async_or(
        &self,
        parts: &[Node<'_>],
        statement: impl Fn(&[Node<'_>]) -> Result<String, String>,
    ) -> Result<String, String> {
        if parts.first().is_some_and(|part| part.kind() == "async") {
            self.adjacent(&parts[..2], &[" "], "async")?;
            Ok(format!("(async {})", statement(&parts[1..])?))
        } else {
            statement(parts)
        }
    }

    fn branches(
        &self,
        test: String,
        body: String,
        previous: Node<'_>,
        rest: &[Node<'_>],
        indent: usize,
        depth: usize,
    ) -> Result<String, String> {
        let Some(clause) = rest.first() else {
            return Ok(format!("(if' {test} {body})"));
        };
        self.want(
            previous.end_byte(),
            clause.start_byte(),
            &format!("\n{}", " ".repeat(indent)),
            "elif or else on the line after its block",
        )?;
        let parts = kids(*clause);
        let otherwise = match clause.kind() {
            "elif_clause" => {
                self.adjacent(&parts[..3], &[" ", ""], "elif header")?;
                let ctx = Ctx {
                    margin: indent,
                    bare: true,
                };
                let test = self.value(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent, depth)?;
                self.branches(test, body, parts[3], &rest[1..], indent, depth)?
            }
            "else_clause" if rest.len() == 1 => {
                self.adjacent(&parts[..2], &[""], "else header")?;
                self.block(parts[1], parts[2], indent, depth)?
            }
            _ => {
                return Err(format!(
                    "line {}: an unsupported if clause",
                    self.line(clause.start_byte())
                ))
            }
        };
        Ok(format!("(ifElse {test} {body} {otherwise})"))
    }

    fn clause(&self, clause: Node<'_>, indent: usize, depth: usize) -> Result<String, String> {
        let parts = kids(clause);
        match clause.kind() {
            "finally_clause" => {
                self.adjacent(&parts[..2], &[""], "finally header")?;
                Ok(format!(
                    "(finally {})",
                    self.block(parts[1], parts[2], indent, depth)?
                ))
            }
            "except_clause" => {
                let colon = parts.len() - 2;
                let ctx = Ctx {
                    margin: indent,
                    bare: true,
                };
                let caught = match colon {
                    1 => {
                        self.adjacent(&parts[..2], &[""], "except header")?;
                        "null".to_owned()
                    }
                    2 => {
                        self.adjacent(&parts[..3], &[" ", ""], "except header")?;
                        self.value(parts[1], ctx)?
                    }
                    4 if parts[2].kind() == "as" => {
                        self.adjacent(&parts[..5], &[" ", " ", " ", ""], "except header")?;
                        format!(
                            "(as {} {})",
                            self.value(parts[1], ctx)?,
                            self.value(parts[3], ctx)?
                        )
                    }
                    _ => {
                        return Err(format!(
                            "line {}: an unsupported except",
                            self.line(clause.start_byte())
                        ))
                    }
                };
                let body = self.block(parts[colon], parts[colon + 1], indent, depth)?;
                Ok(format!("(except {caught} {body})"))
            }
            _ => Err(format!(
                "line {}: {} is not supported",
                self.line(clause.start_byte()),
                clause.kind()
            )),
        }
    }

    fn commas(&self, parts: &[Node<'_>], ctx: Ctx, what: &str) -> Result<Vec<String>, String> {
        let mut items = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            if index % 2 == 1 {
                if part.kind() != "," {
                    return Err(format!(
                        "line {}: {what} without commas",
                        self.line(part.start_byte())
                    ));
                }
                self.want(parts[index - 1].end_byte(), part.start_byte(), "", what)?;
                if index + 1 == parts.len() {
                    return Err(format!(
                        "line {}: a trailing comma in {what}",
                        self.line(part.start_byte())
                    ));
                }
                self.want(part.end_byte(), parts[index + 1].start_byte(), " ", what)?;
            } else {
                items.push(self.value(*part, ctx)?);
            }
        }
        Ok(items)
    }

    fn spaced(&self, parts: &[Node<'_>], ctx: Ctx) -> Result<Expr, String> {
        let first_comma = parts.iter().position(|p| p.kind() == ",");
        let last_comma = parts.iter().rposition(|p| p.kind() == ",");
        let mut units: Vec<(usize, usize, Option<Node<'_>>, Vec<Node<'_>>)> = Vec::new();
        let mut index = 0;
        while index < parts.len() {
            match (first_comma, last_comma) {
                (Some(first), Some(last)) if index + 1 == first => {
                    if last + 1 >= parts.len() {
                        return Err(format!(
                            "line {}: a trailing comma",
                            self.line(parts[last].start_byte())
                        ));
                    }
                    let run = &parts[index..=last + 1];
                    units.push((
                        run[0].start_byte(),
                        run[run.len() - 1].end_byte(),
                        None,
                        run.to_vec(),
                    ));
                    index = last + 2;
                }
                _ => {
                    let part = parts[index];
                    units.push((part.start_byte(), part.end_byte(), Some(part), Vec::new()));
                    index += 1;
                }
            }
        }
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        let mut broke = false;
        let mut previous: Option<usize> = None;
        for (start, end, node, run) in units {
            if let Some(previous) = previous {
                let gap = &self.source[previous..start];
                if gap.contains('\n') {
                    let expected = if ctx.bare {
                        format!(" \\\n{}", " ".repeat(ctx.margin + 4))
                    } else {
                        format!("\n{}", " ".repeat(ctx.margin))
                    };
                    self.want(previous, start, &expected, "a continuation line")?;
                    rows.push(Vec::new());
                    broke = true;
                } else if gap != " " {
                    return Err(format!(
                        "line {}: tokens separated by {gap:?}, not one space",
                        self.line(start)
                    ));
                }
            }
            previous = Some(end);
            let inner = Ctx {
                margin: if ctx.bare && broke {
                    ctx.margin + 4
                } else {
                    ctx.margin
                },
                bare: ctx.bare,
            };
            let expr = match node {
                None => {
                    let items = self.commas(
                        &run,
                        Ctx {
                            bare: false,
                            ..inner
                        },
                        "a comma list",
                    )?;
                    Expr::Nix(format!("(commas [{}])", items.join(" ")))
                }
                Some(node) if !node.is_named() => Expr::Nix(self.string(self.text(node))),
                Some(node) => self.expr(node, inner)?,
            };
            match expr {
                Expr::Rows(more) => {
                    let mut more = more.into_iter();
                    if let Some(head) = more.next() {
                        if let Some(r) = rows.last_mut() {
                            r.extend(head);
                        }
                    }
                    for next in more {
                        rows.push(next);
                        broke = true;
                    }
                }
                other => {
                    let nix = self.nix(other);
                    if let Some(r) = rows.last_mut() {
                        r.push(nix);
                    }
                }
            }
        }
        Ok(Expr::Rows(rows))
    }

    fn expr(&self, node: Node<'_>, ctx: Ctx) -> Result<Expr, String> {
        let parts = kids(node);
        let line = self.line(node.start_byte());
        let flat = Ctx { bare: false, ..ctx };
        Ok(match node.kind() {
            "comment" => return Err(format!("line {line}: a comment inside a statement")),
            "identifier" | "true" | "false" | "none" | "ellipsis" | "integer" | "float"
            | "keyword_separator" => Expr::Name(self.text(node).to_owned()),
            "dotted_name" => {
                let text = self.text(node);
                if text.contains(char::is_whitespace) {
                    return Err(format!("line {line}: a dotted name with spaces"));
                }
                Expr::Name(text.to_owned())
            }
            "string" => Expr::Nix(self.literal(node, &parts)?),
            "attribute" => {
                self.adjacent(&parts, &["", ""], "attribute")?;
                let attribute = self.text(parts[2]).to_owned();
                match self.expr(parts[0], flat)? {
                    Expr::Name(name) if dotted(&name) => Expr::Name(format!("{name}.{attribute}")),
                    other => Expr::Nix(format!(
                        "(dot {} {})",
                        self.nix(other),
                        self.string(&attribute)
                    )),
                }
            }
            "call" => {
                self.adjacent(&parts, &[""], "call")?;
                let function = self.value(parts[0], flat)?;
                let arguments = match parts[1].kind() {
                    "generator_expression" => self.comprehension(parts[1], flat)?,
                    _ => self.seq(parts[1], flat, false)?,
                };
                Expr::Nix(format!("(call {function} {arguments})"))
            }
            "subscript" => {
                if parts.len() != 4 {
                    return Err(format!("line {line}: a subscript with several indices"));
                }
                self.adjacent(&parts, &["", "", ""], "subscript")?;
                let inner = Ctx {
                    margin: column_of(self.source, parts[1].end_byte()),
                    bare: false,
                };
                Expr::Nix(format!(
                    "(index {} {})",
                    self.value(parts[0], flat)?,
                    self.value(parts[2], inner)?
                ))
            }
            "slice" => self.slice(&parts, flat)?,
            "unary_operator" => {
                self.adjacent(&parts, &[""], "unary operator")?;
                let operand = self.text(parts[1]);
                match self.text(parts[0]) {
                    "-" if parts[1].kind() == "integer"
                        && nix::int(&format!("-{operand}")).is_some() =>
                    {
                        Expr::Nix(nix::int(&format!("-{operand}")).unwrap_or_default())
                    }
                    "-" => Expr::Nix(format!("(neg {})", self.value(parts[1], flat)?)),
                    "~" => Expr::Nix(format!("(invert {})", self.value(parts[1], flat)?)),
                    other => return Err(format!("line {line}: unary {other} is not supported")),
                }
            }
            "keyword_argument" | "default_parameter" => {
                self.adjacent(&parts, &["", ""], "keyword argument")?;
                Expr::Nix(format!(
                    "(kw {} {})",
                    self.string(self.text(parts[0])),
                    self.value(parts[2], flat)?
                ))
            }
            "list_splat"
            | "list_splat_pattern"
            | "dictionary_splat"
            | "dictionary_splat_pattern" => {
                if parts.len() != 2 {
                    return Err(format!("line {line}: a bare star"));
                }
                self.adjacent(&parts, &[""], "star")?;
                let constructor = if self.text(parts[0]) == "**" {
                    "starstar"
                } else {
                    "star"
                };
                Expr::Nix(format!("({constructor} {})", self.value(parts[1], flat)?))
            }
            "pair" => {
                self.adjacent(&parts, &["", " "], "dict pair")?;
                Expr::Nix(format!(
                    "(pair {} {})",
                    self.value(parts[0], flat)?,
                    self.value(parts[2], flat)?
                ))
            }
            "lambda" => {
                let (params, colon) = if parts.len() == 4 {
                    self.adjacent(&parts, &[" ", "", " "], "lambda")?;
                    (self.commas(&kids(parts[1]), flat, "lambda parameters")?, 2)
                } else {
                    self.adjacent(&parts, &["", " "], "lambda")?;
                    (Vec::new(), 1)
                };
                Expr::Nix(format!(
                    "(lambda [{}] {})",
                    params.join(" "),
                    self.value(parts[colon + 1], flat)?
                ))
            }
            "as_pattern" => {
                self.adjacent(&parts, &[" ", " "], "as")?;
                Expr::Nix(format!(
                    "(as {} {})",
                    self.value(parts[0], flat)?,
                    self.value(parts[2], flat)?
                ))
            }
            "as_pattern_target" | "with_item" if parts.len() == 1 => {
                return self.expr(parts[0], ctx)
            }
            "list" | "list_pattern" => {
                Expr::Nix(format!("(list {})", self.seq(node, flat, false)?))
            }
            "tuple" | "tuple_pattern" => {
                Expr::Nix(format!("(tuple {})", self.seq(node, flat, true)?))
            }
            "set" => Expr::Nix(format!("(set {})", self.seq(node, flat, false)?)),
            "dictionary" => Expr::Nix(format!("(dict {})", self.seq(node, flat, false)?)),
            "list_comprehension" => {
                Expr::Nix(format!("(list {})", self.comprehension(node, flat)?))
            }
            "set_comprehension" => Expr::Nix(format!("(set {})", self.comprehension(node, flat)?)),
            "dictionary_comprehension" => {
                Expr::Nix(format!("(dict {})", self.comprehension(node, flat)?))
            }
            "generator_expression" => {
                let content = self.comprehension(node, flat)?;
                Expr::Nix(format!("(paren {})", &content[1..content.len() - 1]))
            }
            "parenthesized_expression" => self.paren(node, &parts, flat)?,
            "expression_list" | "pattern_list" => {
                let items = self.commas(&parts, flat, "a comma list")?;
                Expr::Nix(format!("(commas [{}])", items.join(" ")))
            }
            _ if parts.is_empty() => {
                let text = self.text(node);
                if text.is_empty() || text.contains(char::is_whitespace) {
                    return Err(format!("line {line}: {} is not supported", node.kind()));
                }
                Expr::Name(text.to_owned())
            }
            _ => self.spaced(&parts, ctx)?,
        })
    }

    fn paren(&self, node: Node<'_>, parts: &[Node<'_>], ctx: Ctx) -> Result<Expr, String> {
        if parts.len() != 3 {
            return Err(format!(
                "line {}: an unsupported parenthesis",
                self.line(node.start_byte())
            ));
        }
        let gap = &self.source[parts[0].end_byte()..parts[1].start_byte()];
        if gap.contains('\n') {
            let inner = ctx.margin + 4;
            self.adjacent(
                parts,
                &[
                    &format!("\n{}", " ".repeat(inner)),
                    &format!("\n{}", " ".repeat(ctx.margin)),
                ],
                "a hanging parenthesis",
            )?;
            let value = self.value(
                parts[1],
                Ctx {
                    margin: inner,
                    bare: false,
                },
            )?;
            return Ok(Expr::Nix(format!("(hang {value})")));
        }
        self.adjacent(parts, &["", ""], "a parenthesis")?;
        let inner = Ctx {
            margin: column_of(self.source, parts[0].end_byte()),
            bare: false,
        };
        Ok(Expr::Nix(format!(
            "(paren {})",
            self.value(parts[1], inner)?
        )))
    }

    fn comprehension(&self, node: Node<'_>, _ctx: Ctx) -> Result<String, String> {
        let parts = kids(node);
        let (open, close) = (parts[0], parts[parts.len() - 1]);
        let inner = &parts[1..parts.len() - 1];
        self.want(
            open.end_byte(),
            inner[0].start_byte(),
            "",
            "a comprehension's opening bracket",
        )?;
        self.want(
            inner[inner.len() - 1].end_byte(),
            close.start_byte(),
            "",
            "a comprehension's closing bracket",
        )?;
        let ctx = Ctx {
            margin: column_of(self.source, open.end_byte()),
            bare: false,
        };
        let expr = self.spaced(inner, ctx)?;
        Ok(format!("[{}]", self.nix(expr)))
    }

    fn seq(&self, node: Node<'_>, ctx: Ctx, tuple: bool) -> Result<String, String> {
        let parts = kids(node);
        let line = self.line(node.start_byte());
        let (open, close) = match (parts.first(), parts.last()) {
            (Some(open), Some(close)) if parts.len() >= 2 => (*open, *close),
            _ => return Err(format!("line {line}: an unbracketed sequence")),
        };
        let middle = &parts[1..parts.len() - 1];
        if middle.is_empty() {
            self.want(open.end_byte(), close.start_byte(), "", "an empty bracket")?;
            return Ok("[]".into());
        }
        let mut items: Vec<Node<'_>> = Vec::new();
        let mut trailing = false;
        for (index, part) in middle.iter().enumerate() {
            if index % 2 == 1 {
                if part.kind() != "," {
                    return Err(format!(
                        "line {}: items without a comma",
                        self.line(part.start_byte())
                    ));
                }
                self.want(
                    middle[index - 1].end_byte(),
                    part.start_byte(),
                    "",
                    "a comma",
                )?;
                trailing = index + 1 == middle.len();
            } else {
                if part.kind() == "comment" {
                    return Err(format!(
                        "line {}: a comment inside brackets",
                        self.line(part.start_byte())
                    ));
                }
                items.push(*part);
            }
        }
        let last_end = middle[middle.len() - 1].end_byte();
        let hang = self.source[open.end_byte()..items[0].start_byte()].contains('\n');
        let column = if hang {
            ctx.margin + 4
        } else {
            column_of(self.source, open.end_byte())
        };
        let inner = Ctx {
            margin: column,
            bare: false,
        };
        let first_gap = if hang {
            format!("\n{}", " ".repeat(column))
        } else {
            String::new()
        };
        self.want(
            open.end_byte(),
            items[0].start_byte(),
            &first_gap,
            "an opening bracket",
        )?;
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                let comma = middle[2 * index - 1];
                let gap = &self.source[comma.end_byte()..item.start_byte()];
                if gap.contains('\n') {
                    self.want(
                        comma.end_byte(),
                        item.start_byte(),
                        &format!("\n{}", " ".repeat(column)),
                        "an item on a new line",
                    )?;
                    rows.push(Vec::new());
                } else {
                    self.want(
                        comma.end_byte(),
                        item.start_byte(),
                        " ",
                        "items after a comma",
                    )?;
                }
            }
            let value = self.value(*item, inner)?;
            if let Some(r) = rows.last_mut() {
                r.push(value);
            }
        }
        let closing = if hang {
            format!("\n{}", " ".repeat(ctx.margin))
        } else {
            String::new()
        };
        self.want(last_end, close.start_byte(), &closing, "a closing bracket")?;
        let expected_trailing = if hang {
            rows.len() > 1
        } else {
            tuple && items.len() == 1
        };
        if trailing != expected_trailing {
            return Err(format!(
                "line {line}: a trailing comma the printer {} write",
                if trailing { "would not" } else { "would" }
            ));
        }
        let rendered: Vec<String> = rows.iter().map(|r| row(r)).collect();
        Ok(if hang {
            format!("(rows [{}])", rendered.join(" "))
        } else if rows.len() > 1 {
            format!("(aligned [{}])", rendered.join(" "))
        } else {
            format!("[{}]", rows[0].join(" "))
        })
    }

    fn slice(&self, parts: &[Node<'_>], ctx: Ctx) -> Result<Expr, String> {
        let colons = parts.iter().filter(|p| p.kind() == ":").count();
        let line = parts.first().map_or(0, |p| self.line(p.start_byte()));
        if colons != 1 {
            return Err(format!("line {line}: a slice with a step"));
        }
        let colon = parts.iter().position(|p| p.kind() == ":").unwrap_or(0);
        let lower = match colon {
            0 => None,
            _ => Some(self.value(parts[0], ctx)?),
        };
        let upper = match parts.get(colon + 1) {
            None => None,
            Some(part) => Some(self.value(*part, ctx)?),
        };
        let spaced = !(lower.as_deref().is_none_or(simple) && upper.as_deref().is_none_or(simple));
        let pad = if spaced { " " } else { "" };
        if colon == 1 {
            self.want(parts[0].end_byte(), parts[1].start_byte(), pad, "a slice")?;
        }
        if let Some(part) = parts.get(colon + 1) {
            self.want(parts[colon].end_byte(), part.start_byte(), pad, "a slice")?;
        }
        Ok(Expr::Nix(format!(
            "(slice {} {})",
            lower.unwrap_or_else(|| "null".into()),
            upper.unwrap_or_else(|| "null".into())
        )))
    }

    fn literal(&self, node: Node<'_>, parts: &[Node<'_>]) -> Result<String, String> {
        let line = self.line(node.start_byte());
        let (start, end) = (parts[0], parts[parts.len() - 1]);
        let opening = self.text(start);
        let prefix: String = opening
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        let quote = &opening[prefix.len()..];
        if self.text(end) != quote {
            return Err(format!(
                "line {line}: a string closed differently than it opens"
            ));
        }
        let constructor = match quote {
            "\"" => "str",
            "'" => "sq",
            "\"\"\"" => "triple",
            _ => return Err(format!("line {line}: {quote} strings are not supported")),
        };
        let body = match prefix.as_str() {
            "f" => {
                let mut out = Vec::new();
                let mut from = start.end_byte();
                for part in &parts[1..parts.len() - 1] {
                    if part.kind() != "interpolation" {
                        continue;
                    }
                    if from < part.start_byte() {
                        out.push(self.string(&self.source[from..part.start_byte()]));
                    }
                    out.push(self.field(*part)?);
                    from = part.end_byte();
                }
                if from < end.start_byte() || out.is_empty() {
                    out.push(self.string(&self.source[from..end.start_byte()]));
                }
                format!("[{}]", out.join(" "))
            }
            "" | "r" | "b" => self.string(&self.source[start.end_byte()..end.start_byte()]),
            other => {
                return Err(format!(
                    "line {line}: the string prefix {other} is not supported"
                ))
            }
        };
        let literal = format!("({constructor} {body})");
        Ok(match prefix.as_str() {
            "r" => format!("(raw {literal})"),
            "b" => format!("(bytes {literal})"),
            _ => literal,
        })
    }

    fn field(&self, node: Node<'_>) -> Result<String, String> {
        let parts = kids(node);
        let line = self.line(node.start_byte());
        let ctx = Ctx {
            margin: column_of(self.source, node.start_byte()) + 1,
            bare: false,
        };
        match parts.as_slice() {
            [open, value, close] => {
                self.adjacent(&[*open, *value, *close], &["", ""], "an interpolation")?;
                Ok(format!("(field {})", self.value(*value, ctx)?))
            }
            [open, value, spec, close] if spec.kind() == "format_specifier" => {
                self.adjacent(
                    &[*open, *value, *spec, *close],
                    &["", "", ""],
                    "an interpolation",
                )?;
                let text = self.text(*spec);
                let Some(text) = text.strip_prefix(':').filter(|t| !t.contains(['{', '}'])) else {
                    return Err(format!("line {line}: an unsupported format specifier"));
                };
                Ok(format!(
                    "(spec {} {})",
                    self.value(*value, ctx)?,
                    self.string(text)
                ))
            }
            _ => Err(format!("line {line}: an unsupported interpolation")),
        }
    }
}

fn dotted(name: &str) -> bool {
    name.split('.').all(|part| {
        !part.is_empty()
            && !part.starts_with(|c: char| c.is_ascii_digit())
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'@')
    })
}
