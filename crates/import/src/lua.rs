use tree_sitter::Node;

use crate::{line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_lua::LANGUAGE.into(), source)?;
    let root = tree.root_node();
    let importer = Importer { source, holes };
    let statements = named(root);
    let first = statements.first().ok_or("the chunk is empty")?;
    if first.start_byte() != 0 {
        return Err("the chunk must start with its first statement".into());
    }
    let last = statements.last().ok_or("the chunk is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the chunk must end with one newline after its last statement".into());
    }
    let lines = importer.lines(&statements, 0)?;
    Ok(format!(
        "[\n{}]",
        lines
            .iter()
            .map(|line| format!("  {line}\n"))
            .collect::<String>()
    ))
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn token<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children(node)
        .into_iter()
        .find(|child| !child.is_named() && child.kind() == kind)
}

fn right_associative(operator: &str) -> bool {
    matches!(operator, ".." | "^")
}

fn pad(indent: usize) -> String {
    format!("\n{}", " ".repeat(indent))
}

impl Importer<'_> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn gap(&self, from: usize, to: usize) -> &str {
        &self.source[from..to]
    }

    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn expect(&self, from: usize, to: usize, want: &str) -> Result<(), String> {
        let gap = self.gap(from, to);
        if gap == want {
            Ok(())
        } else {
            Err(format!(
                "{}: expected {want:?} here, found {gap:?}",
                self.at(to)
            ))
        }
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn keyword<'t>(&self, node: Node<'t>, kind: &str) -> Result<Node<'t>, String> {
        token(node, kind).ok_or_else(|| format!("{}: no {kind} here", self.at(node.start_byte())))
    }

    fn unsupported<T>(&self, node: Node<'_>) -> Result<T, String> {
        Err(format!(
            "{}: {} is not supported",
            self.at(node.start_byte()),
            node.kind()
        ))
    }

    fn lines(&self, statements: &[Node<'_>], indent: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for (index, statement) in statements.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(statements[index - 1].end_byte(), statement.start_byte());
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 {
                    return Err(format!(
                        "{}: statements share a line in a block written over several lines",
                        self.at(statement.start_byte())
                    ));
                }
                self.expect(
                    statements[index - 1].end_byte() + lines - 1,
                    statement.start_byte(),
                    &pad(indent),
                )?;
                for _ in 1..lines {
                    out.push("blank".into());
                }
            }
            out.push(self.statement(*statement, indent)?);
        }
        Ok(out)
    }

    fn body(
        &self,
        block: Option<Node<'_>>,
        open: usize,
        close: usize,
        indent: usize,
    ) -> Result<String, String> {
        let statements = block.map(named).unwrap_or_default();
        let (Some(first), Some(last)) = (statements.first(), statements.last()) else {
            self.expect(open, close, " ")?;
            return Ok("(inline [])".into());
        };
        if !self.gap(open, first.start_byte()).contains('\n') {
            self.expect(open, first.start_byte(), " ")?;
            for pair in statements.windows(2) {
                let want = if pair[1].kind() == "empty_statement" {
                    ""
                } else {
                    " "
                };
                self.expect(pair[0].end_byte(), pair[1].start_byte(), want)?;
            }
            self.expect(last.end_byte(), close, " ")?;
            let items = statements
                .iter()
                .map(|s| self.statement(*s, indent))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("(inline [{}])", items.join(" ")));
        }
        self.expect(open, first.start_byte(), &pad(indent + 2))?;
        self.expect(last.end_byte(), close, &pad(indent))?;
        let lines = self.lines(&statements, indent + 2)?;
        Ok(format!(
            "[\n{}]",
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        ))
    }

    fn statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        match node.kind() {
            "comment" => Ok(format!("(comment {})", self.string(self.text(node)))),
            "empty_statement" => Ok(self.string(";")),
            "break_statement" => Ok(self.string("break")),
            "function_call" => self.expression(node, indent),
            "assignment_statement" => {
                let (targets, values) = self.assignment(node, indent)?;
                Ok(format!("(assign {targets} {values})"))
            }
            "variable_declaration" => self.local(node, indent),
            "function_declaration" => self.declaration(node, indent),
            "return_statement" => self.return_statement(node, indent),
            "if_statement" => self.conditional(node, indent),
            "for_statement" => self.for_loop(node, indent),
            "while_statement" => {
                let start = self.keyword(node, "while")?;
                let test = self.field(node, "condition")?;
                let open = self.keyword(node, "do")?;
                self.expect(start.end_byte(), test.start_byte(), " ")?;
                self.expect(test.end_byte(), open.start_byte(), " ")?;
                let body = self.closed(node, open.end_byte(), "end", indent)?;
                Ok(format!("(while {} {body})", self.expression(test, indent)?))
            }
            "repeat_statement" => {
                let start = self.keyword(node, "repeat")?;
                let until = self.keyword(node, "until")?;
                let test = self.field(node, "condition")?;
                let body = self.body(
                    node.child_by_field_name("body"),
                    start.end_byte(),
                    until.start_byte(),
                    indent,
                )?;
                self.expect(until.end_byte(), test.start_byte(), " ")?;
                Ok(format!(
                    "(repeat {body} {})",
                    self.expression(test, indent)?
                ))
            }
            "do_statement" => {
                let start = self.keyword(node, "do")?;
                let body = self.closed(node, start.end_byte(), "end", indent)?;
                Ok(format!("(do {body})"))
            }
            _ => self.unsupported(node),
        }
    }

    fn field<'t>(&self, node: Node<'t>, name: &str) -> Result<Node<'t>, String> {
        node.child_by_field_name(name)
            .ok_or_else(|| format!("{}: no {name} here", self.at(node.start_byte())))
    }

    fn closed(
        &self,
        node: Node<'_>,
        open: usize,
        close: &str,
        indent: usize,
    ) -> Result<String, String> {
        let end = self.keyword(node, close)?;
        if end.end_byte() != node.end_byte() {
            return Err(format!("{}: text after {close}", self.at(end.end_byte())));
        }
        self.body(
            node.child_by_field_name("body"),
            open,
            end.start_byte(),
            indent,
        )
    }

    fn separated(&self, nodes: &[Node<'_>]) -> Result<(), String> {
        for pair in nodes.windows(2) {
            self.expect(pair[0].end_byte(), pair[1].start_byte(), ", ")?;
        }
        Ok(())
    }

    fn expressions(&self, nodes: &[Node<'_>], indent: usize) -> Result<Vec<String>, String> {
        self.separated(nodes)?;
        nodes.iter().map(|n| self.expression(*n, indent)).collect()
    }

    fn one_or_list(items: Vec<String>) -> String {
        match items.as_slice() {
            [one] => one.clone(),
            _ => format!("[{}]", items.join(" ")),
        }
    }

    fn assignment(&self, node: Node<'_>, indent: usize) -> Result<(String, String), String> {
        let parts = children(node);
        let [targets, equals, values] = parts.as_slice() else {
            return self.unsupported(node);
        };
        if equals.kind() != "=" {
            return self.unsupported(node);
        }
        self.expect(targets.end_byte(), equals.start_byte(), " ")?;
        self.expect(equals.end_byte(), values.start_byte(), " ")?;
        let targets = self.targets(*targets, indent)?;
        let values = self.expressions(&named(*values), indent)?;
        Ok((targets, format!("[{}]", values.join(" "))))
    }

    fn targets(&self, list: Node<'_>, indent: usize) -> Result<String, String> {
        let names = named(list);
        if names.iter().any(|n| n.kind() == "attribute") {
            return Err(format!(
                "{}: a local attribute is not supported",
                self.at(list.start_byte())
            ));
        }
        let names = self.expressions(&names, indent)?;
        Ok(Self::one_or_list(names))
    }

    fn local(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let local = self.keyword(node, "local")?;
        let rest = named(node);
        let [rest] = rest.as_slice() else {
            return self.unsupported(node);
        };
        self.expect(local.end_byte(), rest.start_byte(), " ")?;
        if rest.kind() == "variable_list" {
            return Ok(format!("(local {} [])", self.targets(*rest, indent)?));
        }
        let (names, values) = self.assignment(*rest, indent)?;
        Ok(format!("(local {names} {values})"))
    }

    fn parameters(&self, node: Node<'_>) -> Result<String, String> {
        let names = named(node);
        if let (Some(first), Some(last)) = (names.first(), names.last()) {
            self.expect(node.start_byte() + 1, first.start_byte(), "")?;
            self.expect(last.end_byte(), node.end_byte() - 1, "")?;
        } else {
            self.expect(node.start_byte(), node.end_byte(), "()")?;
        }
        self.separated(&names)?;
        let names: Vec<String> = names.iter().map(|n| self.string(self.text(*n))).collect();
        Ok(format!("[{}]", names.join(" ")))
    }

    fn function_body(&self, node: Node<'_>, after: usize, indent: usize) -> Result<String, String> {
        let parameters = self.field(node, "parameters")?;
        self.expect(after, parameters.start_byte(), "")?;
        let body = self.closed(node, parameters.end_byte(), "end", indent)?;
        Ok(format!("{} {body}", self.parameters(parameters)?))
    }

    fn declaration(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let function = self.keyword(node, "function")?;
        let name = self.field(node, "name")?;
        self.expect(function.end_byte(), name.start_byte(), " ")?;
        let path = self.chain(name).ok_or_else(|| {
            format!(
                "{}: a function name must be a plain path",
                self.at(name.start_byte())
            )
        })?;
        let rest = self.function_body(node, name.end_byte(), indent)?;
        match token(node, "local") {
            Some(local) => {
                self.expect(local.end_byte(), function.start_byte(), " ")?;
                Ok(format!("(localFunction {} {rest})", self.string(path)))
            }
            None if function.start_byte() == node.start_byte() => {
                Ok(format!("(function {} {rest})", self.string(path)))
            }
            None => self.unsupported(node),
        }
    }

    fn return_statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        if token(node, ";").is_some() {
            return Err(format!(
                "{}: a semicolon after return is not supported",
                self.at(node.start_byte())
            ));
        }
        let start = self.keyword(node, "return")?;
        match named(node).as_slice() {
            [] => Ok("(return [])".into()),
            [list] => {
                self.expect(start.end_byte(), list.start_byte(), " ")?;
                let values = self.expressions(&named(*list), indent)?;
                Ok(format!("(return [{}])", values.join(" ")))
            }
            _ => self.unsupported(node),
        }
    }

    fn test(&self, node: Node<'_>, word: &str, indent: usize) -> Result<(String, usize), String> {
        let start = self.keyword(node, word)?;
        let test = self.field(node, "condition")?;
        let then = self.keyword(node, "then")?;
        self.expect(start.end_byte(), test.start_byte(), " ")?;
        self.expect(test.end_byte(), then.start_byte(), " ")?;
        Ok((self.expression(test, indent)?, then.end_byte()))
    }

    fn conditional(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let end = self.keyword(node, "end")?;
        let alternatives: Vec<Node<'_>> = {
            let mut cursor = node.walk();
            node.children_by_field_name("alternative", &mut cursor)
                .collect()
        };
        let close_of = |index: usize| {
            alternatives
                .get(index)
                .map_or(end.start_byte(), |n| n.start_byte())
        };
        let mut otherwise: Option<String> = None;
        for (index, alternative) in alternatives.iter().enumerate().rev() {
            let close = close_of(index + 1);
            if alternative.end_byte() > close {
                return self.unsupported(*alternative);
            }
            otherwise = Some(match alternative.kind() {
                "else_statement" => {
                    if otherwise.is_some() {
                        return self.unsupported(*alternative);
                    }
                    let word = self.keyword(*alternative, "else")?;
                    self.body(
                        alternative.child_by_field_name("body"),
                        word.end_byte(),
                        close,
                        indent,
                    )?
                }
                "elseif_statement" => {
                    let (test, open) = self.test(*alternative, "elseif", indent)?;
                    let body = self.body(
                        alternative.child_by_field_name("consequence"),
                        open,
                        close,
                        indent,
                    )?;
                    Self::branch(test, body, otherwise.take())
                }
                _ => return self.unsupported(*alternative),
            });
        }
        let (test, open) = self.test(node, "if", indent)?;
        let body = self.body(
            node.child_by_field_name("consequence"),
            open,
            close_of(0),
            indent,
        )?;
        Ok(Self::branch(test, body, otherwise))
    }

    fn branch(test: String, body: String, otherwise: Option<String>) -> String {
        match otherwise {
            Some(otherwise) => format!("(ifElse {test} {body} {otherwise})"),
            None => format!("(if' {test} {body})"),
        }
    }

    fn for_loop(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let start = self.keyword(node, "for")?;
        let clause = self.field(node, "clause")?;
        let open = self.keyword(node, "do")?;
        self.expect(start.end_byte(), clause.start_byte(), " ")?;
        self.expect(clause.end_byte(), open.start_byte(), " ")?;
        let body = self.closed(node, open.end_byte(), "end", indent)?;
        match clause.kind() {
            "for_generic_clause" => {
                let parts = children(clause);
                let [names, word, values] = parts.as_slice() else {
                    return self.unsupported(clause);
                };
                if word.kind() != "in" {
                    return self.unsupported(clause);
                }
                self.expect(names.end_byte(), word.start_byte(), " ")?;
                self.expect(word.end_byte(), values.start_byte(), " ")?;
                let names = self.targets(*names, indent)?;
                let values = self.expressions(&named(*values), indent)?;
                Ok(format!("(forIn {names} [{}] {body})", values.join(" ")))
            }
            "for_numeric_clause" => {
                let name = self.field(clause, "name")?;
                let equals = self.keyword(clause, "=")?;
                let first = self.field(clause, "start")?;
                self.expect(name.end_byte(), equals.start_byte(), " ")?;
                self.expect(equals.end_byte(), first.start_byte(), " ")?;
                let range: Vec<Node<'_>> = ["start", "end", "step"]
                    .iter()
                    .filter_map(|f| clause.child_by_field_name(f))
                    .collect();
                let range = self.expressions(&range, indent)?;
                Ok(format!(
                    "(forNum {} [{}] {body})",
                    self.string(self.text(name)),
                    range.join(" ")
                ))
            }
            _ => self.unsupported(clause),
        }
    }

    fn chain(&self, node: Node<'_>) -> Option<&str> {
        let pure = match node.kind() {
            "identifier" => true,
            "dot_index_expression" | "method_index_expression" => {
                node.child_by_field_name("table").is_some_and(|table| {
                    table.kind() != "method_index_expression" && self.chain(table).is_some()
                })
            }
            _ => false,
        };
        let text = self.text(node);
        (pure
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':')))
        .then_some(text)
    }

    fn expression(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        if let Some(path) = self.chain(node) {
            return Ok(self.string(path));
        }
        let text = self.text(node);
        match node.kind() {
            "number" => Ok(nix::int(text).unwrap_or_else(|| self.string(text))),
            "nil" => Ok("null".into()),
            "true" | "false" => Ok(text.to_owned()),
            "vararg_expression" => Ok(self.string(text)),
            "string" => self.literal(node),
            "dot_index_expression" | "method_index_expression" => {
                let table = self.field(node, "table")?;
                let (mark, kind, name) = if node.kind() == "dot_index_expression" {
                    (".", "dot", self.field(node, "field")?)
                } else {
                    (":", "method", self.field(node, "method")?)
                };
                self.expect(table.end_byte(), name.start_byte(), mark)?;
                Ok(format!(
                    "({kind} {} {})",
                    self.expression(table, indent)?,
                    self.string(self.text(name))
                ))
            }
            "bracket_index_expression" => {
                let table = self.field(node, "table")?;
                let key = self.field(node, "field")?;
                self.expect(table.end_byte(), key.start_byte(), "[")?;
                self.expect(key.end_byte(), node.end_byte(), "]")?;
                Ok(format!(
                    "(index {} {})",
                    self.expression(table, indent)?,
                    self.expression(key, indent)?
                ))
            }
            "function_call" => {
                let name = self.field(node, "name")?;
                let arguments = self.field(node, "arguments")?;
                self.expect(name.end_byte(), arguments.start_byte(), "")?;
                let values = named(arguments);
                if self.text(arguments).as_bytes().first() != Some(&b'(') {
                    let [value] = values.as_slice() else {
                        return self.unsupported(arguments);
                    };
                    return Ok(format!(
                        "(apply {} {})",
                        self.expression(name, indent)?,
                        self.expression(*value, indent)?
                    ));
                }
                if let (Some(first), Some(last)) = (values.first(), values.last()) {
                    self.expect(arguments.start_byte() + 1, first.start_byte(), "")?;
                    self.expect(last.end_byte(), arguments.end_byte() - 1, "")?;
                } else {
                    self.expect(arguments.start_byte(), arguments.end_byte(), "()")?;
                }
                let mut items = vec![self.expression(name, indent)?];
                items.extend(self.expressions(&values, indent)?);
                Ok(format!("[{}]", items.join(" ")))
            }
            "parenthesized_expression" => match named(node).as_slice() {
                [inner] => {
                    self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
                    self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
                    Ok(format!("(paren {})", self.expression(*inner, indent)?))
                }
                _ => self.unsupported(node),
            },
            "unary_expression" => {
                let operator = self.field(node, "operator")?;
                let operand = self.field(node, "operand")?;
                let word = operator.kind();
                self.expect(
                    operator.end_byte(),
                    operand.start_byte(),
                    if word == "not" { " " } else { "" },
                )?;
                if word == "-" && operand.kind() == "number" {
                    if let Some(int) = nix::int(&format!("-{}", self.text(operand))) {
                        return Ok(int);
                    }
                }
                let kind = match word {
                    "not" => "not",
                    "-" => "neg",
                    "#" => "len",
                    "~" => "bnot",
                    _ => return self.unsupported(node),
                };
                Ok(format!("({kind} {})", self.expression(operand, indent)?))
            }
            "binary_expression" => self.operation(node, indent),
            "table_constructor" => self.table(node, indent),
            "function_definition" => {
                let word = self.keyword(node, "function")?;
                Ok(format!(
                    "(fn {})",
                    self.function_body(node, word.end_byte(), indent)?
                ))
            }
            _ => self.unsupported(node),
        }
    }

    fn literal(&self, node: Node<'_>) -> Result<String, String> {
        let start = self.field(node, "start")?;
        let end = self.field(node, "end")?;
        let content = self.gap(start.end_byte(), end.start_byte());
        match (start.kind(), end.kind()) {
            ("\"", "\"") => Ok(format!("(str {})", self.string(content))),
            ("'", "'") => Ok(format!("(sq {})", self.string(content))),
            _ => Err(format!(
                "{}: a long bracket string is not supported",
                self.at(node.start_byte())
            )),
        }
    }

    fn operation(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let operator = self.field(node, "operator")?;
        let word = operator.kind();
        let mut items = Vec::new();
        Self::operands(node, word, &mut items);
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        let mut margin = indent;
        for (index, item) in items.iter().enumerate() {
            if index % 2 == 1 {
                let before = self.gap(items[index - 1].end_byte(), item.start_byte());
                if before == pad(indent + 2) {
                    rows.push(Vec::new());
                    margin = indent + 2;
                } else {
                    self.expect(items[index - 1].end_byte(), item.start_byte(), " ")?;
                }
                self.expect(item.end_byte(), items[index + 1].start_byte(), " ")?;
                rows.last_mut().unwrap().push(self.string(item.kind()));
            } else {
                let value = self.expression(*item, margin)?;
                rows.last_mut().unwrap().push(value);
            }
        }
        if rows.len() == 1 {
            return Ok(format!("(op [{}])", rows[0].join(" ")));
        }
        let rows: Vec<String> = rows
            .iter()
            .map(|row| format!("[{}]", row.join(" ")))
            .collect();
        Ok(format!("(op (rows [{}]))", rows.join(" ")))
    }

    fn operands<'t>(node: Node<'t>, word: &str, out: &mut Vec<Node<'t>>) {
        let same = |side: Option<Node<'t>>| {
            side.filter(|child| {
                child.kind() == "binary_expression"
                    && child
                        .child_by_field_name("operator")
                        .is_some_and(|op| op.kind() == word)
            })
        };
        let (Some(left), Some(operator), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("operator"),
            node.child_by_field_name("right"),
        ) else {
            out.push(node);
            return;
        };
        match same(Some(left)) {
            Some(left) if !right_associative(word) => Self::operands(left, word, out),
            _ => out.push(left),
        }
        out.push(operator);
        match same(Some(right)) {
            Some(right) if right_associative(word) => Self::operands(right, word, out),
            _ => out.push(right),
        }
    }

    fn table(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let fields = named(node);
        let (Some(first), Some(last)) = (fields.first(), fields.last()) else {
            self.expect(node.start_byte(), node.end_byte(), "{}")?;
            return Ok("(table [])".into());
        };
        let open = node.start_byte() + 1;
        let close = node.end_byte() - 1;
        if self.gap(open, first.start_byte()) == " " {
            self.separated(&fields)?;
            self.expect(last.end_byte(), close, " ")?;
            let fields = fields
                .iter()
                .map(|f| self.table_field(*f, indent))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("(table [{}])", fields.join(" ")));
        }
        let row = pad(indent + 2);
        self.expect(open, first.start_byte(), &row)?;
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(fields[index - 1].end_byte(), field.start_byte());
                if gap == format!(",{row}") {
                    rows.push(Vec::new());
                } else {
                    self.expect(fields[index - 1].end_byte(), field.start_byte(), ", ")?;
                }
            }
            let value = self.table_field(*field, indent + 2)?;
            rows.last_mut().unwrap().push(value);
        }
        self.expect(last.end_byte(), close, &format!(",{}", pad(indent)))?;
        let rows: Vec<String> = rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [one] if !one.starts_with('[') => one.clone(),
                _ => format!("[{}]", row.join(" ")),
            })
            .collect();
        Ok(format!("(table (rows [\n{}\n]))", rows.join("\n")))
    }

    fn table_field(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        if node.kind() != "field" {
            return self.unsupported(node);
        }
        let value = self.field(node, "value")?;
        let Some(name) = node.child_by_field_name("name") else {
            return self.expression(value, indent);
        };
        let equals = self.keyword(node, "=")?;
        self.expect(equals.end_byte(), value.start_byte(), " ")?;
        let value = self.expression(value, indent)?;
        if name.start_byte() == node.start_byte() && name.kind() == "identifier" {
            self.expect(name.end_byte(), equals.start_byte(), " ")?;
            return Ok(format!("(field {} {value})", self.string(self.text(name))));
        }
        self.expect(node.start_byte(), name.start_byte(), "[")?;
        self.expect(name.end_byte(), equals.start_byte(), "] ")?;
        Ok(format!("(key {} {value})", self.expression(name, indent)?))
    }
}
