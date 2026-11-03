use tree_sitter::Node;

use crate::{line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_luau::LANGUAGE.into(), source)?;
    let importer = Importer { source, holes };
    let nodes = named(tree.root_node());
    let Some(first) = nodes.first() else {
        if source.bytes().all(|b| b == b'\n') {
            return Ok(format!("[{}]", vec!["blank"; source.len()].join(" ")));
        }
        return Err("the file holds no statement".into());
    };
    if first.start_byte() != 0 {
        return Err("the file must start with its first statement".into());
    }
    let entries = importer.entries(&nodes, 0)?;
    let text: String = entries
        .iter()
        .map(|entry| format!("{}\n", entry.text))
        .collect();
    if text != source {
        return Err(mismatch(source, &text));
    }
    Ok(format!(
        "[\n{}]",
        entries
            .iter()
            .map(|entry| format!("  {}\n", entry.nix))
            .collect::<String>()
    ))
}

fn mismatch(source: &str, text: &str) -> String {
    let at = source
        .bytes()
        .zip(text.bytes())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| source.len().min(text.len()));
    let line = line_of(source, at.min(source.len()));
    let found = source.lines().nth(line - 1).unwrap_or("");
    let printed = text.lines().nth(line - 1).unwrap_or("");
    format!("line {line}: the printer writes {printed:?}, the source has {found:?}")
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
}

struct Out {
    nix: String,
    text: String,
}

impl Out {
    fn new(nix: String, text: String) -> Self {
        Self { nix, text }
    }
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|c| c.is_named())
        .collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn tabs(depth: usize) -> String {
    "\t".repeat(depth)
}

fn newline(depth: usize) -> String {
    format!("\n{}", tabs(depth))
}

fn join(outs: &[Out], nix: &str, text: &str) -> Out {
    Out::new(
        outs.iter()
            .map(|o| o.nix.as_str())
            .collect::<Vec<_>>()
            .join(nix),
        outs.iter()
            .map(|o| o.text.as_str())
            .collect::<Vec<_>>()
            .join(text),
    )
}

fn many(outs: Vec<Out>) -> Out {
    if outs.len() == 1 {
        outs.into_iter()
            .next()
            .unwrap_or_else(|| Out::new(String::new(), String::new()))
    } else {
        let joined = join(&outs, " ", ", ");
        Out::new(format!("(tuple [{}])", joined.nix), joined.text)
    }
}

fn names(outs: Vec<Out>) -> Out {
    if outs.len() == 1 {
        outs.into_iter()
            .next()
            .unwrap_or_else(|| Out::new(String::new(), String::new()))
    } else {
        let joined = join(&outs, " ", ", ");
        Out::new(format!("[{}]", joined.nix), joined.text)
    }
}

enum Part {
    Text(String),
    Code(String),
}

fn quote_mark(text: &str) -> char {
    if text.matches('"').count() > text.matches('\'').count() {
        '\''
    } else {
        '"'
    }
}

fn escape(text: &str, mark: char) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c == mark => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

fn left_associative(op: &str) -> bool {
    !matches!(op, ".." | "^")
}

impl Importer<'_> {
    fn at(&self, node: Node<'_>) -> String {
        format!("line {}", line_of(self.source, node.start_byte()))
    }

    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.byte_range()]
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn atom(&self, text: &str) -> Out {
        Out::new(
            nix::int(text).unwrap_or_else(|| self.string(text)),
            text.to_owned(),
        )
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!("{}: {what} ({})", self.at(node), node.kind()))
    }

    fn broken(&self, from: usize, to: usize) -> bool {
        self.source[from..to].contains('\n')
    }

    fn entries(&self, nodes: &[Node<'_>], depth: usize) -> Result<Vec<Out>, String> {
        let mut out: Vec<Out> = Vec::new();
        let mut previous: Option<Node<'_>> = None;
        for node in nodes {
            if let Some(before) = previous {
                let lines = self.source[before.end_byte()..node.start_byte()]
                    .matches('\n')
                    .count();
                if lines == 0 {
                    if node.kind() != "comment" {
                        return self.refuse(*node, "two statements share a line");
                    }
                    let last = out.pop().ok_or("a trailing comment without a statement")?;
                    let comment = self.text(*node);
                    out.push(Out::new(
                        format!("(trailing {} {})", last.nix, self.string(comment)),
                        format!("{} {comment}", last.text),
                    ));
                    previous = Some(*node);
                    continue;
                }
                for _ in 1..lines {
                    out.push(Out::new("blank".into(), String::new()));
                }
            }
            out.push(self.statement(*node, depth)?);
            previous = Some(*node);
        }
        Ok(out)
    }

    fn body(&self, nodes: &[Node<'_>], depth: usize) -> Result<Out, String> {
        let entries = self.entries(nodes, depth + 1)?;
        let nix: String = entries.iter().map(|e| format!("{}\n", e.nix)).collect();
        let text: String = entries
            .iter()
            .map(|e| {
                if e.nix == "blank" {
                    "\n".to_owned()
                } else {
                    format!("{}{}", newline(depth + 1), e.text)
                }
            })
            .collect();
        Ok(Out::new(
            format!("[\n{nix}]"),
            format!("{text}{}", newline(depth)),
        ))
    }

    fn between<'t>(&self, node: Node<'t>, from: usize, to: usize) -> Vec<Node<'t>> {
        let mut out = Vec::new();
        for child in children(node) {
            if child.start_byte() < from || child.end_byte() > to {
                continue;
            }
            match child.kind() {
                "block" => out.extend(named(child)),
                "comment" => out.push(child),
                _ => {}
            }
        }
        out
    }

    fn token<'t>(&self, node: Node<'t>, kind: &str) -> Option<Node<'t>> {
        children(node)
            .into_iter()
            .find(|c| !c.is_named() && c.kind() == kind)
    }

    fn statement(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        match node.kind() {
            "comment" => {
                let text = self.text(node);
                Ok(Out::new(
                    format!("(comment {})", self.string(text)),
                    text.to_owned(),
                ))
            }
            "break_statement" | "continue_statement" => {
                let word = self.text(node);
                Ok(Out::new(self.string(word), word.to_owned()))
            }
            "function_call" => self.expression(node, depth),
            "return_statement" => match named(node).as_slice() {
                [] => Ok(Out::new("\"return\"".into(), "return".into())),
                [list] if list.kind() == "expression_list" => {
                    let values = self.list(*list, depth)?;
                    Ok(Out::new(
                        format!("(return {})", values.nix),
                        format!("return {}", values.text),
                    ))
                }
                _ => self.refuse(node, "a return the printer does not write"),
            },
            "variable_declaration" => self.local(node, depth),
            "assignment_statement" => {
                let (targets, values) = self.assignment(node, depth)?;
                Ok(Out::new(
                    format!("(assign {} {})", targets.nix, values.nix),
                    format!("{} = {}", targets.text, values.text),
                ))
            }
            "update_statement" => {
                let op = children(node)
                    .into_iter()
                    .find(|c| !c.is_named())
                    .map(|c| self.text(c).to_owned())
                    .unwrap_or_default();
                let (targets, values) = self.assignment(node, depth)?;
                Ok(Out::new(
                    format!(
                        "(update {} {} {})",
                        self.string(&op),
                        targets.nix,
                        values.nix
                    ),
                    format!("{} {op} {}", targets.text, values.text),
                ))
            }
            "function_declaration" => self.declaration(node, depth),
            "if_statement" => {
                let clause = self.clause(node, depth)?;
                Ok(Out::new(clause.nix, format!("if {}", clause.text)))
            }
            "for_statement" => self.for_statement(node, depth),
            "while_statement" => {
                let condition = self.field(node, "condition")?;
                let test = self.expression(condition, depth)?;
                let body = self.loop_body(node, depth)?;
                Ok(Out::new(
                    format!("(while {} {})", test.nix, body.nix),
                    format!("while {} do{}end", test.text, body.text),
                ))
            }
            "type_definition" => {
                let parts = named(node);
                let [name, value] = parts.as_slice() else {
                    return self.refuse(node, "a type definition the printer does not write");
                };
                if name.kind() != "identifier"
                    || children(node).first().map(|c| c.kind()) != Some("type")
                {
                    return self.refuse(node, "a type definition the printer does not write");
                }
                let value = self.type_(*value, depth)?;
                let name = self.text(*name);
                Ok(Out::new(
                    format!("(type {} {})", self.string(name), value.nix),
                    format!("type {name} = {}", value.text),
                ))
            }
            _ => self.refuse(node, "a statement the printer does not write"),
        }
    }

    fn field<'t>(&self, node: Node<'t>, name: &str) -> Result<Node<'t>, String> {
        node.child_by_field_name(name)
            .ok_or_else(|| format!("{}: {} without its {name}", self.at(node), node.kind()))
    }

    fn list(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let values = named(node)
            .into_iter()
            .map(|value| self.expression(value, depth))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(many(values))
    }

    fn variables(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let mut outs: Vec<Out> = Vec::new();
        let mut typed = false;
        for child in children(node) {
            match child.kind() {
                "," => {}
                ":" => typed = true,
                "attribute" => {
                    return self.refuse(child, "an attribute the printer does not write")
                }
                _ if typed => {
                    typed = false;
                    let name = outs.pop().ok_or("a type without a name")?;
                    let kind = self.type_(child, depth)?;
                    outs.push(Out::new(
                        format!("(typed {} {})", name.nix, kind.nix),
                        format!("{}: {}", name.text, kind.text),
                    ));
                }
                _ => outs.push(self.expression(child, depth)?),
            }
        }
        Ok(names(outs))
    }

    fn assignment(&self, node: Node<'_>, depth: usize) -> Result<(Out, Out), String> {
        let parts = named(node);
        let [targets, values] = parts.as_slice() else {
            return self.refuse(node, "an assignment the printer does not write");
        };
        Ok((self.variables(*targets, depth)?, self.list(*values, depth)?))
    }

    fn local(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let parts = named(node);
        let [assignment] = parts.as_slice() else {
            return self.refuse(node, "a local the printer does not write");
        };
        if assignment.kind() != "assignment_statement" {
            return self.refuse(node, "a local without a value");
        }
        let (targets, values) = self.assignment(*assignment, depth)?;
        Ok(Out::new(
            format!("(local {} {})", targets.nix, values.nix),
            format!("local {} = {}", targets.text, values.text),
        ))
    }

    fn signature(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        if named(node).iter().any(|c| c.kind() == "generic_type_list") {
            return self.refuse(node, "generic parameters the printer does not write");
        }
        let parameters = self.field(node, "parameters")?;
        let mut outs = Vec::new();
        for parameter in named(parameters) {
            let parts = named(parameter);
            outs.push(match parts.as_slice() {
                [name] => self.expression(*name, depth)?,
                [name, kind] => {
                    let name = self.expression(*name, depth)?;
                    let kind = self.type_(*kind, depth)?;
                    Out::new(
                        format!("(typed {} {})", name.nix, kind.nix),
                        format!("{}: {}", name.text, kind.text),
                    )
                }
                _ => return self.refuse(parameter, "a parameter the printer does not write"),
            });
        }
        let joined = join(&outs, " ", ", ");
        let params = Out::new(format!("[{}]", joined.nix), format!("({})", joined.text));
        let returns = children(node)
            .into_iter()
            .skip_while(|c| c.id() != parameters.id())
            .skip(1)
            .find(|c| c.is_named() && c.kind() != "block" && c.kind() != "comment");
        match returns {
            None => Ok(params),
            Some(kind) => {
                let kind = self.type_(kind, depth)?;
                Ok(Out::new(
                    format!("(typed {} {})", params.nix, kind.nix),
                    format!("{}: {}", params.text, kind.text),
                ))
            }
        }
    }

    fn function_body(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let parameters = self.field(node, "parameters")?;
        let end = self.token(node, "end").ok_or("a function without end")?;
        let nodes = self.between(node, parameters.end_byte(), end.start_byte());
        if nodes.is_empty() {
            return Ok(Out::new("[]".into(), " ".into()));
        }
        self.body(&nodes, depth)
    }

    fn declaration(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let local = self.token(node, "local").is_some();
        let name = self.field(node, "name")?;
        let name = self.path(name).ok_or_else(|| {
            format!(
                "{}: a function name the printer does not write",
                self.at(name)
            )
        })?;
        let signature = self.signature(node, depth)?;
        let body = self.function_body(node, depth)?;
        let (constructor, keyword) = if local {
            ("localFunction", "local function")
        } else {
            ("function", "function")
        };
        Ok(Out::new(
            format!(
                "({constructor} {} {} {})",
                self.string(&name),
                signature.nix,
                body.nix
            ),
            format!("{keyword} {name}{}{}end", signature.text, body.text),
        ))
    }

    fn path(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "identifier" => Some(self.text(node).to_owned()),
            "dot_index_expression" | "method_index_expression" => {
                let table = node.child_by_field_name("table")?;
                let name = node
                    .child_by_field_name("field")
                    .or_else(|| node.child_by_field_name("method"))?;
                let mark = if node.kind() == "dot_index_expression" {
                    "."
                } else {
                    ":"
                };
                let base = self.path(table)?;
                if base.contains(':') {
                    return None;
                }
                Some(format!("{base}{mark}{}", self.text(name)))
            }
            _ => None,
        }
    }

    fn clause(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let condition = self.field(node, "condition")?;
        let test = self.expression(condition, depth)?;
        let then = self.token(node, "then").ok_or("an if without then")?;
        let alternatives: Vec<Node<'_>> = children(node)
            .into_iter()
            .filter(|c| matches!(c.kind(), "elseif_statement" | "else_statement"))
            .collect();
        let end = match alternatives.first() {
            Some(alternative) => alternative.start_byte(),
            None => self
                .token(node, "end")
                .map_or(node.end_byte(), |e| e.start_byte()),
        };
        let body = self.body(&self.between(node, then.end_byte(), end), depth)?;
        let head = format!("{} then{}", test.text, body.text);
        let (nix, tail) = match alternatives.as_slice() {
            [] => (format!("(if' {} {})", test.nix, body.nix), "end".to_owned()),
            [alternative, ..] if alternative.kind() == "elseif_statement" => {
                let rest = self.clause_chain(node, &alternatives, depth)?;
                (
                    format!("(ifElse {} {} {})", test.nix, body.nix, rest.nix),
                    format!("elseif {}", rest.text),
                )
            }
            [_] => {
                let otherwise = self.body(&self.region(node, &alternatives, 0), depth)?;
                (
                    format!("(ifElse {} {} {})", test.nix, body.nix, otherwise.nix),
                    format!("else{}end", otherwise.text),
                )
            }
            _ => return self.refuse(node, "an if the printer does not write"),
        };
        Ok(Out::new(nix, format!("{head}{tail}")))
    }

    fn region<'t>(&self, node: Node<'t>, alternatives: &[Node<'t>], from: usize) -> Vec<Node<'t>> {
        let alternative = alternatives[0];
        let bound = match alternatives.get(1) {
            Some(next) => next.start_byte(),
            None => self
                .token(node, "end")
                .map_or(node.end_byte(), |e| e.start_byte()),
        };
        let mut nodes = self.between(alternative, from, alternative.end_byte());
        nodes.extend(children(node).into_iter().filter(|c| {
            c.kind() == "comment"
                && c.start_byte() >= alternative.end_byte()
                && c.end_byte() <= bound
        }));
        nodes
    }

    fn clause_chain(
        &self,
        node: Node<'_>,
        alternatives: &[Node<'_>],
        depth: usize,
    ) -> Result<Out, String> {
        let first = alternatives[0];
        let rest = &alternatives[1..];
        let condition = self.field(first, "condition")?;
        let test = self.expression(condition, depth)?;
        let then = self.token(first, "then").ok_or("an elseif without then")?;
        let body = self.body(&self.region(node, alternatives, then.end_byte()), depth)?;
        let head = format!("{} then{}", test.text, body.text);
        let (nix, tail) = match rest {
            [] => (format!("(if' {} {})", test.nix, body.nix), "end".to_owned()),
            [next, ..] if next.kind() == "elseif_statement" => {
                let more = self.clause_chain(node, rest, depth)?;
                (
                    format!("(ifElse {} {} {})", test.nix, body.nix, more.nix),
                    format!("elseif {}", more.text),
                )
            }
            [_] => {
                let otherwise = self.body(&self.region(node, rest, 0), depth)?;
                (
                    format!("(ifElse {} {} {})", test.nix, body.nix, otherwise.nix),
                    format!("else{}end", otherwise.text),
                )
            }
            _ => return self.refuse(node, "an if the printer does not write"),
        };
        Ok(Out::new(nix, format!("{head}{tail}")))
    }

    fn loop_body(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let open = self.token(node, "do").ok_or("a loop without do")?;
        let end = self.token(node, "end").ok_or("a loop without end")?;
        self.body(
            &self.between(node, open.end_byte(), end.start_byte()),
            depth,
        )
    }

    fn for_statement(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let clause = self.field(node, "clause")?;
        let body = self.loop_body(node, depth)?;
        if clause.kind() == "for_numeric_clause" {
            let name = self.text(self.field(clause, "name")?).to_owned();
            let mut bounds = vec![
                self.expression(self.field(clause, "start")?, depth)?,
                self.expression(self.field(clause, "end")?, depth)?,
            ];
            if let Some(step) = clause.child_by_field_name("step") {
                bounds.push(self.expression(step, depth)?);
            }
            let bounds = many(bounds);
            return Ok(Out::new(
                format!(
                    "(forRange {} {} {})",
                    self.string(&name),
                    bounds.nix,
                    body.nix
                ),
                format!("for {name} = {} do{}end", bounds.text, body.text),
            ));
        }
        let (targets, values) = self.assignment(clause, depth)?;
        Ok(Out::new(
            format!("(for {} {} {})", targets.nix, values.nix, body.nix),
            format!("for {} in {} do{}end", targets.text, values.text, body.text),
        ))
    }

    fn expression(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        match node.kind() {
            "identifier" | "number" | "nil" | "true" | "false" | "vararg_expression" => {
                Ok(self.atom(self.text(node)))
            }
            "dot_index_expression" | "method_index_expression" => {
                if let Some(path) = self.path(node) {
                    return Ok(Out::new(self.string(&path), path));
                }
                let table = self.expression(self.field(node, "table")?, depth)?;
                let (name, constructor, mark) = match node.kind() {
                    "dot_index_expression" => (self.field(node, "field")?, "dot", "."),
                    _ => (self.field(node, "method")?, "colon", ":"),
                };
                let name = self.text(name);
                let base = Out::new(
                    format!("({constructor} {} {})", table.nix, self.string(name)),
                    format!("{}{mark}{name}", table.text),
                );
                let op = self.token(node, mark).ok_or("an index without its mark")?;
                if node.kind() == "method_index_expression"
                    && self.broken(self.field(node, "table")?.end_byte(), op.start_byte())
                {
                    return Ok(Out::new(
                        format!("(rows {})", base.nix),
                        format!("{}{}:{name}", table.text, newline(depth + 1)),
                    ));
                }
                Ok(base)
            }
            "bracket_index_expression" => {
                let table = self.expression(self.field(node, "table")?, depth)?;
                let key = self.expression(self.field(node, "field")?, depth)?;
                Ok(Out::new(
                    format!("(idx {} {})", table.nix, key.nix),
                    format!("{}[{}]", table.text, key.text),
                ))
            }
            "function_call" => self.call(node, depth),
            "string" => self.string_literal(node),
            "table_constructor" => self.table(node, depth, false),
            "binary_expression" => self.binary(node, depth),
            "unary_expression" => {
                let op = children(node)
                    .first()
                    .map(|c| self.text(*c).to_owned())
                    .unwrap_or_default();
                let operand = self.expression(self.field(node, "operand")?, depth)?;
                let space = if op == "not" { " " } else { "" };
                Ok(Out::new(
                    format!("(unary {} {})", self.string(&op), operand.nix),
                    format!("{op}{space}{}", operand.text),
                ))
            }
            "parenthesized_expression" => {
                let parts = named(node);
                let [inner] = parts.as_slice() else {
                    return self.refuse(node, "parentheses the printer does not write");
                };
                let inner = self.expression(*inner, depth)?;
                Ok(Out::new(
                    format!("(paren {})", inner.nix),
                    format!("({})", inner.text),
                ))
            }
            "function_definition" => {
                let signature = self.signature(node, depth)?;
                let body = self.function_body(node, depth)?;
                Ok(Out::new(
                    format!("(fn {} {})", signature.nix, body.nix),
                    format!("function{}{}end", signature.text, body.text),
                ))
            }
            "if_expression" => self.if_expression(node, depth),
            "cast_expression" => {
                let parts = named(node);
                let [value, kind] = parts.as_slice() else {
                    return self.refuse(node, "a cast the printer does not write");
                };
                let value = self.expression(*value, depth)?;
                let kind = self.type_(*kind, depth)?;
                Ok(Out::new(
                    format!("(cast {} {})", value.nix, kind.nix),
                    format!("{} :: {}", value.text, kind.text),
                ))
            }
            _ => self.refuse(node, "an expression the printer does not write"),
        }
    }

    fn call(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let callee_node = self.field(node, "name")?;
        let callee = self.expression(callee_node, depth)?;
        let arguments = self.field(node, "arguments")?;
        let open = children(arguments).into_iter().next();
        if open.map(|o| o.kind()) != Some("(") {
            return self.refuse(node, "a call without parentheses");
        }
        let inner = if callee.nix.starts_with("(rows (colon") {
            depth + 1
        } else {
            depth
        };
        let values = named(arguments);
        let broken = values
            .first()
            .is_some_and(|first| self.broken(arguments.start_byte(), first.start_byte()));
        let args = values
            .iter()
            .map(|v| self.expression(*v, if broken { inner + 1 } else { inner }))
            .collect::<Result<Vec<_>, _>>()?;
        let nix: String = std::iter::once(callee.nix.clone())
            .chain(args.iter().map(|a| a.nix.clone()))
            .collect::<Vec<_>>()
            .join(" ");
        if broken {
            let lines = args
                .iter()
                .map(|a| format!("{}{}", newline(inner + 1), a.text))
                .collect::<Vec<_>>()
                .join(",");
            return Ok(Out::new(
                format!("(rows [{nix}])"),
                format!("{}({lines}{})", callee.text, newline(inner)),
            ));
        }
        let joined = join(&args, " ", ", ");
        Ok(Out::new(
            format!("[{nix}]"),
            format!("{}({})", callee.text, joined.text),
        ))
    }

    fn string_literal(&self, node: Node<'_>) -> Result<Out, String> {
        if named(node).iter().any(|c| c.kind() == "interpolation") {
            return self.refuse(node, "an interpolated string");
        }
        let raw = self.text(node);
        if let Some(inner) = raw.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
            return Ok(Out::new(
                format!("(long {})", self.string(inner)),
                raw.to_owned(),
            ));
        }
        let inner = &raw[1..raw.len() - 1];
        let mut parts: Vec<Part> = vec![Part::Text(String::new())];
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            let decoded = if c == '\\' {
                match chars.next() {
                    Some('\\') => '\\',
                    Some('"') => '"',
                    Some('\'') => '\'',
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    Some('u') if chars.next() == Some('{') => {
                        let code: String = chars.by_ref().take_while(|&c| c != '}').collect();
                        if code.is_empty() || !code.bytes().all(|b| b.is_ascii_hexdigit()) {
                            return self.refuse(node, "an escape the printer does not write");
                        }
                        parts.push(Part::Code(code));
                        parts.push(Part::Text(String::new()));
                        continue;
                    }
                    _ => return self.refuse(node, "an escape the printer does not write"),
                }
            } else {
                c
            };
            if let Some(Part::Text(text)) = parts.last_mut() {
                text.push(decoded);
            }
        }
        parts.retain(|part| !matches!(part, Part::Text(text) if text.is_empty()));
        let plain: String = parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.as_str()),
                Part::Code(_) => None,
            })
            .collect();
        let mark = quote_mark(&plain);
        let text: String = parts
            .iter()
            .map(|part| match part {
                Part::Text(text) => escape(text, mark),
                Part::Code(code) => format!("\\u{{{code}}}"),
            })
            .collect();
        let nix = match parts.as_slice() {
            [] => self.string(""),
            [Part::Text(text)] => self.string(text),
            _ => format!(
                "[{}]",
                parts
                    .iter()
                    .map(|part| match part {
                        Part::Text(text) => self.string(text),
                        Part::Code(code) => format!("(unicode {})", self.string(code)),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        };
        Ok(Out::new(
            format!("(str {nix})"),
            format!("{mark}{text}{mark}"),
        ))
    }

    fn table(&self, node: Node<'_>, depth: usize, types: bool) -> Result<Out, String> {
        let parts: Vec<Node<'_>> = named(node);
        let broken = parts
            .first()
            .is_some_and(|first| self.broken(node.start_byte(), first.start_byte()));
        let inner = if broken { depth + 1 } else { depth };
        let entries = if types {
            self.type_entries(node, inner)?
        } else {
            let mut entries = Vec::new();
            for (index, part) in parts.iter().enumerate() {
                if index > 0 {
                    let lines = self.source[parts[index - 1].end_byte()..part.start_byte()]
                        .matches('\n')
                        .count();
                    for _ in 1..lines {
                        entries.push(Out::new("blank".into(), String::new()));
                    }
                }
                entries.push(if part.kind() == "comment" {
                    let text = self.text(*part);
                    Out::new(format!("(comment {})", self.string(text)), text.to_owned())
                } else {
                    self.entry(*part, inner)?
                });
            }
            entries
        };
        let nix: String = entries.iter().map(|e| format!("{}\n", e.nix)).collect();
        if entries.is_empty() {
            return Ok(Out::new("(table [])".into(), "{}".into()));
        }
        if broken {
            let text: String = entries
                .iter()
                .map(|e| {
                    if e.nix == "blank" {
                        return "\n".to_owned();
                    }
                    let comma = if e.nix.starts_with("(comment") {
                        ""
                    } else {
                        ","
                    };
                    format!("{}{}{comma}", newline(inner), e.text)
                })
                .collect();
            return Ok(Out::new(
                format!("(rows (table [\n{nix}]))"),
                format!("{{{text}{}}}", newline(depth)),
            ));
        }
        let joined = join(&entries, " ", ", ");
        Ok(Out::new(
            format!("(table [{}])", joined.nix),
            format!("{{ {} }}", joined.text),
        ))
    }

    fn entry(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        if node.kind() != "field" {
            return self.refuse(node, "a table entry the printer does not write");
        }
        let value = self.expression(self.field(node, "value")?, depth)?;
        match node.child_by_field_name("name") {
            None => Ok(value),
            Some(name) if self.token(node, "[").is_some() => {
                let key = self.expression(name, depth)?;
                Ok(Out::new(
                    format!("(pair {} {})", key.nix, value.nix),
                    format!("[{}] = {}", key.text, value.text),
                ))
            }
            Some(name) => {
                let name = self.text(name);
                Ok(Out::new(
                    format!("(field {} {})", self.string(name), value.nix),
                    format!("{name} = {}", value.text),
                ))
            }
        }
    }

    fn type_entries(&self, node: Node<'_>, depth: usize) -> Result<Vec<Out>, String> {
        let mut entries = Vec::new();
        let mut name: Option<String> = None;
        let mut key_type: Option<Out> = None;
        let mut colon = false;
        for child in children(node) {
            match child.kind() {
                "{" | "}" | "," => {}
                ":" => colon = true,
                "comment" => {
                    let text = self.text(child);
                    entries.push(Out::new(
                        format!("(comment {})", self.string(text)),
                        text.to_owned(),
                    ));
                }
                "identifier" if !colon && name.is_none() && self.next_is_colon(child) => {
                    name = Some(self.text(child).to_owned());
                }
                "object_field_type" => {
                    let parts = named(child);
                    let [key] = parts.as_slice() else {
                        return self.refuse(child, "an indexer the printer does not write");
                    };
                    key_type = Some(self.type_(*key, depth)?);
                }
                _ if child.is_named() => {
                    let kind = self.type_(child, depth)?;
                    if let Some(key) = key_type.take() {
                        colon = false;
                        entries.push(Out::new(
                            format!("(pair {} {})", key.nix, kind.nix),
                            format!("[{}]: {}", key.text, kind.text),
                        ));
                        continue;
                    }
                    match name.take() {
                        Some(field) => {
                            colon = false;
                            entries.push(Out::new(
                                format!("(field {} {})", self.string(&field), kind.nix),
                                format!("{field}: {}", kind.text),
                            ));
                        }
                        None => entries.push(kind),
                    }
                }
                _ => return self.refuse(child, "a table type the printer does not write"),
            }
        }
        Ok(entries)
    }

    fn next_is_colon(&self, node: Node<'_>) -> bool {
        node.next_sibling().is_some_and(|n| n.kind() == ":")
    }

    fn type_(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        match node.kind() {
            "identifier" | "builtin_type" => Ok(self.atom(self.text(node))),
            "optional_type" => {
                let parts = named(node);
                let [inner] = parts.as_slice() else {
                    return self.refuse(node, "an optional type the printer does not write");
                };
                let inner = self.type_(*inner, depth)?;
                Ok(Out::new(
                    format!("(opt {})", inner.nix),
                    format!("{}?", inner.text),
                ))
            }
            "object_type" => self.table(node, depth, true),
            "tuple_type" => {
                let parts = named(node)
                    .into_iter()
                    .map(|p| self.type_(p, depth))
                    .collect::<Result<Vec<_>, _>>()?;
                let joined = join(&parts, " ", ", ");
                Ok(Out::new(
                    format!("(tuple [{}])", joined.nix),
                    format!("({})", joined.text),
                ))
            }
            _ => self.refuse(node, "a type the printer does not write"),
        }
    }

    fn binary(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let op = self.operator(node);
        let mut operands = Vec::new();
        self.flatten(node, &op, &mut operands);
        let broken = operands
            .windows(2)
            .any(|pair| self.broken(pair[0].end_byte(), pair[1].start_byte()));
        let inner = if broken { depth + 1 } else { depth };
        let outs = operands
            .iter()
            .map(|o| self.expression(*o, inner))
            .collect::<Result<Vec<_>, _>>()?;
        let quoted = self.string(&op);
        let nix = outs
            .iter()
            .map(|o| o.nix.as_str())
            .collect::<Vec<_>>()
            .join(&format!(" {quoted} "));
        if broken {
            let rest: String = outs[1..]
                .iter()
                .map(|o| format!("{}{op} {}", newline(inner), o.text))
                .collect();
            return Ok(Out::new(
                format!("(rows (infix [{nix}]))"),
                format!("{}{rest}", outs[0].text),
            ));
        }
        let text = outs
            .iter()
            .map(|o| o.text.as_str())
            .collect::<Vec<_>>()
            .join(&format!(" {op} "));
        Ok(Out::new(format!("(infix [{nix}])"), text))
    }

    fn operator(&self, node: Node<'_>) -> String {
        children(node)
            .into_iter()
            .find(|c| !c.is_named() || c.kind() == "and" || c.kind() == "or")
            .map(|c| self.text(c).to_owned())
            .unwrap_or_default()
    }

    fn flatten<'t>(&self, node: Node<'t>, op: &str, out: &mut Vec<Node<'t>>) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            out.push(node);
            return;
        };
        let same = |n: Node<'_>| n.kind() == "binary_expression" && self.operator(n) == op;
        if left_associative(op) && same(left) {
            self.flatten(left, op, out);
        } else {
            out.push(left);
        }
        if !left_associative(op) && same(right) {
            self.flatten(right, op, out);
        } else {
            out.push(right);
        }
    }

    fn if_expression(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let mut parts: Vec<(&str, Node<'_>, Node<'_>)> = Vec::new();
        let then = self.token(node, "then").ok_or("an if without then")?;
        parts.push(("if", children(node)[0], self.field(node, "condition")?));
        parts.push(("then", then, self.field(node, "consequence")?));
        for clause in named(node) {
            match clause.kind() {
                "elseif_clause" => {
                    parts.push((
                        "elseif",
                        children(clause)[0],
                        self.field(clause, "condition")?,
                    ));
                    let then = self.token(clause, "then").ok_or("an elseif without then")?;
                    parts.push(("then", then, self.field(clause, "consequence")?));
                }
                "else_clause" => parts.push((
                    "else",
                    children(clause)[0],
                    self.field(clause, "consequence")?,
                )),
                _ => {}
            }
        }
        let broken = parts
            .windows(2)
            .any(|pair| self.broken(pair[0].2.end_byte(), pair[1].1.start_byte()));
        let inner = if broken { depth + 1 } else { depth };
        let mut rows: Vec<Vec<Out>> = Vec::new();
        for (index, (word, mark, value)) in parts.iter().enumerate() {
            let value = self.expression(*value, inner)?;
            let out = Out::new(value.nix, format!("{word} {}", value.text));
            let fresh = index == 0 || self.broken(parts[index - 1].2.end_byte(), mark.start_byte());
            match rows.last_mut() {
                Some(row) if !fresh => row.push(out),
                _ => rows.push(vec![out]),
            }
        }
        if broken {
            let nix = rows
                .iter()
                .map(|row| {
                    format!(
                        "[{}]",
                        row.iter()
                            .map(|o| o.nix.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            let text = rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|o| o.text.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(&newline(inner));
            return Ok(Out::new(format!("(rows (ifExpr [{nix}]))"), text));
        }
        let all: Vec<Out> = rows.into_iter().flatten().collect();
        let joined = join(&all, " ", " ");
        Ok(Out::new(format!("(ifExpr [{}])", joined.nix), joined.text))
    }
}
