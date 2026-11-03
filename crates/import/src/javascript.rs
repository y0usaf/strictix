use tree_sitter::Node;

use crate::{line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_javascript::LANGUAGE.into(), source)?;
    let importer = Importer { source, holes };
    let statements = named(tree.root_node());
    if statements.is_empty() {
        return Err("the program is empty".into());
    }
    let mut lines = Vec::new();
    let mut text = String::new();
    for (index, statement) in statements.iter().enumerate() {
        if index > 0 {
            for _ in 1..importer.newlines(statements[index - 1], *statement) {
                lines.push("blank".to_owned());
                text.push('\n');
            }
        }
        let (nix, written) = importer.statement(*statement, 0)?;
        lines.push(nix);
        text.push_str(&written);
        text.push('\n');
    }
    importer.compare(&text)?;
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

type Out = (String, String);

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn has_token(node: Node<'_>, kind: &str) -> bool {
    children(node).iter().any(|child| child.kind() == kind)
}

fn pad(indent: usize) -> String {
    " ".repeat(indent)
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(" "))
}

fn lines(items: &[String]) -> String {
    format!("[\n{}\n]", items.join("\n"))
}

impl Importer<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn at(&self, node: Node<'_>) -> String {
        format!("line {}", line_of(self.source, node.start_byte()))
    }

    fn unsupported(&self, node: Node<'_>, what: &str) -> String {
        format!("{}: {what} is not supported", self.at(node))
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn field<'t>(&self, node: Node<'t>, name: &str) -> Result<Node<'t>, String> {
        node.child_by_field_name(name)
            .ok_or_else(|| format!("{}: {} without {name}", self.at(node), node.kind()))
    }

    fn newlines(&self, before: Node<'_>, after: Node<'_>) -> usize {
        self.source[before.end_byte()..after.start_byte()]
            .matches('\n')
            .count()
    }

    fn compare(&self, text: &str) -> Result<(), String> {
        if text == self.source {
            return Ok(());
        }
        let at = text
            .bytes()
            .zip(self.source.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(text.len().min(self.source.len()));
        let line = line_of(self.source, at);
        let row = |s: &str| s.lines().nth(line - 1).unwrap_or("").to_owned();
        Err(format!(
            "line {line}: the printer writes {:?} where the source has {:?}",
            row(text),
            row(self.source)
        ))
    }

    fn broken(&self, node: Node<'_>) -> bool {
        let entries = named(node);
        matches!(node.kind(), "object" | "array" | "object_pattern")
            && entries.first().is_some_and(|first| {
                self.source[node.start_byte()..first.start_byte()].contains('\n')
            })
    }

    fn statements(
        &self,
        nodes: &[Node<'_>],
        indent: usize,
    ) -> Result<(Vec<String>, String), String> {
        let mut items = Vec::new();
        let mut text = String::new();
        for (index, node) in nodes.iter().enumerate() {
            if index > 0 {
                for _ in 1..self.newlines(nodes[index - 1], *node) {
                    items.push("blank".to_owned());
                    text.push('\n');
                }
            }
            let (nix, written) = self.statement(*node, indent)?;
            items.push(nix);
            text.push('\n');
            text.push_str(&pad(indent));
            text.push_str(&written);
        }
        Ok((items, text))
    }

    fn block(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        if node.kind() != "statement_block" {
            return Err(self.unsupported(node, node.kind()));
        }
        let statements = named(node);
        if !self.text(node).contains('\n') {
            let items = statements
                .iter()
                .map(|s| self.statement(*s, indent))
                .collect::<Result<Vec<_>, _>>()?;
            let nix: Vec<String> = items.iter().map(|(n, _)| n.clone()).collect();
            let text: Vec<String> = items.iter().map(|(_, t)| t.clone()).collect();
            let written = if text.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{ {} }}", text.join(" "))
            };
            return Ok((format!("(inline {})", list(&nix)), written));
        }
        let (items, text) = self.statements(&statements, indent + 2)?;
        Ok((lines(&items), format!("{{{text}\n{}}}", pad(indent))))
    }

    fn clause(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let (nix, text) = match node.kind() {
            "statement_block" => self.block(node, indent)?,
            "expression_statement" => {
                return Err(format!(
                    "{}: a body without braces must be a statement, not an expression",
                    self.at(node)
                ))
            }
            _ => self.statement(node, indent)?,
        };
        Ok((nix, format!(" {text}")))
    }

    fn parenthesized(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let inner = named(node);
        match inner.as_slice() {
            [one] if node.kind() == "parenthesized_expression" => self.expression(*one, indent),
            _ => Err(self.unsupported(node, "this condition")),
        }
    }

    fn statement(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        match node.kind() {
            "comment" | "hash_bang_line" => {
                let text = self.text(node);
                Ok((format!("(comment {})", self.string(text)), text.to_owned()))
            }
            "import_statement" => self.import(node),
            "lexical_declaration" => self.declaration(node, indent),
            "function_declaration" => self.function(node, indent),
            "expression_statement" => {
                let inner = named(node);
                let [expression] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this statement"));
                };
                let (nix, text) = self.expression(*expression, indent)?;
                Ok((nix, format!("{text};")))
            }
            "return_statement" => match named(node).as_slice() {
                [] => Ok(("(return null)".to_owned(), "return;".to_owned())),
                [value] => {
                    let (nix, text) = self.expression(*value, indent)?;
                    Ok((format!("(return {nix})"), format!("return {text};")))
                }
                _ => Err(self.unsupported(node, "this return")),
            },
            "throw_statement" => {
                let inner = named(node);
                let [value] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this throw"));
                };
                let (nix, text) = self.expression(*value, indent)?;
                Ok((format!("(throw' {nix})"), format!("throw {text};")))
            }
            "continue_statement" if named(node).is_empty() => {
                Ok(("continue".to_owned(), "continue;".to_owned()))
            }
            "if_statement" => {
                let (test, test_text) =
                    self.parenthesized(self.field(node, "condition")?, indent)?;
                let (body, body_text) = self.clause(self.field(node, "consequence")?, indent)?;
                match node.child_by_field_name("alternative") {
                    None => Ok((
                        format!("(if' {test} {body})"),
                        format!("if ({test_text}){body_text}"),
                    )),
                    Some(alternative) => {
                        let inner = named(alternative);
                        let [otherwise] = inner.as_slice() else {
                            return Err(self.unsupported(alternative, "this else"));
                        };
                        let (other, other_text) = self.clause(*otherwise, indent)?;
                        Ok((
                            format!("(ifElse {test} {body} {other})"),
                            format!("if ({test_text}){body_text} else{other_text}"),
                        ))
                    }
                }
            }
            "while_statement" => {
                let (test, test_text) =
                    self.parenthesized(self.field(node, "condition")?, indent)?;
                let (body, body_text) = self.clause(self.field(node, "body")?, indent)?;
                Ok((
                    format!("(while {test} {body})"),
                    format!("while ({test_text}){body_text}"),
                ))
            }
            "try_statement" => {
                if node.child_by_field_name("finalizer").is_some() {
                    return Err(self.unsupported(node, "finally"));
                }
                let (body, body_text) = self.block(self.field(node, "body")?, indent)?;
                let handler = self.field(node, "handler")?;
                let (param, param_text) = match handler.child_by_field_name("parameter") {
                    None => ("null".to_owned(), String::new()),
                    Some(p) if p.kind() == "identifier" => {
                        (self.string(self.text(p)), format!(" ({})", self.text(p)))
                    }
                    Some(p) => return Err(self.unsupported(p, "this catch binding")),
                };
                let (caught, caught_text) = self.block(self.field(handler, "body")?, indent)?;
                Ok((
                    format!("(try' {body} {param} {caught})"),
                    format!("try {body_text} catch{param_text} {caught_text}"),
                ))
            }
            other => Err(self.unsupported(node, other)),
        }
    }

    fn import(&self, node: Node<'_>) -> Result<Out, String> {
        let clause = named(node)
            .into_iter()
            .find(|n| n.kind() == "import_clause")
            .ok_or_else(|| self.unsupported(node, "an import without names"))?;
        let inner = named(clause);
        let [specifiers] = inner.as_slice() else {
            return Err(self.unsupported(clause, "this import clause"));
        };
        if specifiers.kind() != "named_imports" {
            return Err(self.unsupported(clause, "this import clause"));
        }
        let mut names = Vec::new();
        for specifier in named(*specifiers) {
            if specifier.kind() != "import_specifier" || named(specifier).len() != 1 {
                return Err(self.unsupported(specifier, "a renamed import"));
            }
            names.push(self.text(specifier).to_owned());
        }
        let source = self.field(node, "source")?;
        let quoted = self.text(source);
        if !quoted.starts_with('"') {
            return Err(self.unsupported(source, "a single-quoted string"));
        }
        let path = &quoted[1..quoted.len() - 1];
        let nix: Vec<String> = names.iter().map(|n| self.string(n)).collect();
        Ok((
            format!("(import' {} {})", list(&nix), self.string(path)),
            format!("import {{ {} }} from \"{path}\";", names.join(", ")),
        ))
    }

    fn declaration(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let kind = self.text(self.field(node, "kind")?).to_owned();
        let declarators = named(node);
        let [declarator] = declarators.as_slice() else {
            return Err(self.unsupported(node, "more than one declarator"));
        };
        let (name, name_text) = self.expression(self.field(*declarator, "name")?, indent)?;
        let value = declarator
            .child_by_field_name("value")
            .map(|v| self.expression(v, indent))
            .transpose()?;
        match (kind.as_str(), value) {
            ("const", Some((value, value_text))) => Ok((
                format!("(const {name} {value})"),
                format!("const {name_text} = {value_text};"),
            )),
            ("let", Some((value, value_text))) => Ok((
                format!("(let' {name} {value})"),
                format!("let {name_text} = {value_text};"),
            )),
            ("let", None) => Ok((format!("(let' {name} null)"), format!("let {name_text};"))),
            _ => Err(self.unsupported(node, "this declaration")),
        }
    }

    fn params(&self, node: Node<'_>) -> Result<(String, String), String> {
        let mut names = Vec::new();
        for param in named(node) {
            if param.kind() != "identifier" {
                return Err(self.unsupported(param, "this parameter"));
            }
            names.push(self.text(param).to_owned());
        }
        let nix: Vec<String> = names.iter().map(|n| self.string(n)).collect();
        Ok((list(&nix), names.join(", ")))
    }

    fn asynchronous(node: Node<'_>, out: Out) -> Out {
        if has_token(node, "async") {
            (format!("(async {})", out.0), format!("async {}", out.1))
        } else {
            out
        }
    }

    fn function(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let name = self.text(self.field(node, "name")?).to_owned();
        let (params, params_text) = self.params(self.field(node, "parameters")?)?;
        let (body, body_text) = self.block(self.field(node, "body")?, indent)?;
        Ok(Self::asynchronous(
            node,
            (
                format!("(function {} {params} {body})", self.string(&name)),
                format!("function {name}({params_text}) {body_text}"),
            ),
        ))
    }

    fn arrow(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let (params, params_text) = match node.child_by_field_name("parameters") {
            Some(p) => self.params(p)?,
            None => {
                let p = self.field(node, "parameter")?;
                (list(&[self.string(self.text(p))]), self.text(p).to_owned())
            }
        };
        let body = self.field(node, "body")?;
        let (nix, text) = if body.kind() == "statement_block" {
            self.block(body, indent)?
        } else {
            let (nix, text) = self.expression(body, indent)?;
            if nix.starts_with('[') {
                return Err(format!(
                    "{}: an arrow returning a call needs a block body",
                    self.at(body)
                ));
            }
            (nix, text)
        };
        Ok(Self::asynchronous(
            node,
            (
                format!("(arrow {params} {nix})"),
                format!("({params_text}) => {text}"),
            ),
        ))
    }

    fn path(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "identifier" | "this" => Some(self.text(node).to_owned()),
            "member_expression" => {
                let object = self.path(node.child_by_field_name("object")?)?;
                let property = node.child_by_field_name("property")?;
                (property.kind() == "property_identifier").then(|| {
                    let dot = if has_token(node, "optional_chain") {
                        "?."
                    } else {
                        "."
                    };
                    format!("{object}{dot}{}", self.text(property))
                })
            }
            _ => None,
        }
    }

    fn expression(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        if let Some(path) = self.path(node) {
            return Ok((self.string(&path), path));
        }
        let text = self.text(node);
        match node.kind() {
            "null" | "true" | "false" | "undefined" | "shorthand_property_identifier_pattern" => {
                Ok((self.string(text), text.to_owned()))
            }
            "number" => Ok(match nix::int(text) {
                Some(int) => (int, text.to_owned()),
                None => (self.string(text), text.to_owned()),
            }),
            "string" => {
                if !text.starts_with('"') {
                    return Err(self.unsupported(node, "a single-quoted string"));
                }
                let content = &text[1..text.len() - 1];
                Ok((format!("(str {})", self.string(content)), text.to_owned()))
            }
            "member_expression" => {
                if has_token(node, "optional_chain") {
                    return Err(self.unsupported(node, "?. after a call"));
                }
                let property = self.field(node, "property")?;
                if property.kind() != "property_identifier" {
                    return Err(self.unsupported(property, property.kind()));
                }
                let (object, object_text) = self.expression(self.field(node, "object")?, indent)?;
                let name = self.text(property);
                Ok((
                    format!("(dot {object} {})", self.string(name)),
                    format!("{object_text}.{name}"),
                ))
            }
            "call_expression" => {
                if has_token(node, "optional_chain") {
                    return Err(self.unsupported(node, "an optional call"));
                }
                let (callee, callee_text) =
                    self.expression(self.field(node, "function")?, indent)?;
                let (args, args_text) = self.arguments(self.field(node, "arguments")?, indent)?;
                let mut items = vec![callee];
                items.extend(args);
                Ok((list(&items), format!("{callee_text}({args_text})")))
            }
            "new_expression" => {
                let (callee, callee_text) =
                    self.expression(self.field(node, "constructor")?, indent)?;
                let arguments = node
                    .child_by_field_name("arguments")
                    .ok_or_else(|| self.unsupported(node, "new without arguments"))?;
                let (args, args_text) = self.arguments(arguments, indent)?;
                let mut items = vec![callee];
                items.extend(args);
                Ok((
                    format!("(new {})", list(&items)),
                    format!("new {callee_text}({args_text})"),
                ))
            }
            "await_expression" => {
                let inner = named(node);
                let [value] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this await"));
                };
                let (nix, text) = self.expression(*value, indent)?;
                Ok((format!("(await {nix})"), format!("await {text}")))
            }
            "unary_expression" => {
                let operator = self.text(self.field(node, "operator")?);
                let argument = self.field(node, "argument")?;
                if operator == "-" && argument.kind() == "number" {
                    if let Some(int) = nix::int(&format!("-{}", self.text(argument))) {
                        return Ok((int, format!("-{}", self.text(argument))));
                    }
                }
                let (nix, text) = self.expression(argument, indent)?;
                match operator {
                    "!" => Ok((format!("(not {nix})"), format!("!{text}"))),
                    "typeof" => Ok((format!("(typeof {nix})"), format!("typeof {text}"))),
                    other => Err(self.unsupported(node, &format!("the {other} operator"))),
                }
            }
            "binary_expression" | "assignment_expression" | "augmented_assignment_expression" => {
                self.operation(node, indent)
            }
            "parenthesized_expression" => {
                let (nix, text) = self.parenthesized(node, indent)?;
                Ok((format!("(paren {nix})"), format!("({text})")))
            }
            "arrow_function" => self.arrow(node, indent),
            "object" | "object_pattern" => self.collection(node, indent, "obj", "{", "}"),
            "array" => self.collection(node, indent, "array", "[", "]"),
            other => Err(self.unsupported(node, other)),
        }
    }

    fn arguments(&self, node: Node<'_>, indent: usize) -> Result<(Vec<String>, String), String> {
        if node.kind() != "arguments" {
            return Err(self.unsupported(node, node.kind()));
        }
        let items = named(node)
            .iter()
            .map(|a| self.expression(*a, indent))
            .collect::<Result<Vec<_>, _>>()?;
        let text: Vec<String> = items.iter().map(|(_, t)| t.clone()).collect();
        Ok((items.into_iter().map(|(n, _)| n).collect(), text.join(", ")))
    }

    fn flatten<'t>(&self, node: Node<'t>, out: &mut Vec<Node<'t>>) -> Result<(), String> {
        if !matches!(
            node.kind(),
            "binary_expression" | "assignment_expression" | "augmented_assignment_expression"
        ) {
            out.push(node);
            return Ok(());
        }
        let left = self.field(node, "left")?;
        let right = self.field(node, "right")?;
        let operator = children(node)
            .into_iter()
            .find(|c| c.start_byte() >= left.end_byte() && c.end_byte() <= right.start_byte())
            .ok_or_else(|| self.unsupported(node, "an expression without its operator"))?;
        self.flatten(left, out)?;
        out.push(operator);
        self.flatten(right, out)
    }

    fn operation(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let mut parts = Vec::new();
        self.flatten(node, &mut parts)?;
        let rows = parts
            .windows(2)
            .skip(1)
            .step_by(2)
            .any(|pair| self.newlines(pair[0], pair[1]) > 0);
        let inner = if rows { indent + 2 } else { indent };
        let (first, first_text) = self.expression(parts[0], indent)?;
        let mut nix = vec![first];
        let mut text = first_text;
        for pair in parts[1..].chunks(2) {
            let operator = self.text(pair[0]);
            let (n, t) = self.expression(pair[1], inner)?;
            nix.push(self.string(operator));
            nix.push(n);
            if rows {
                text.push_str(&format!(" {operator}\n{}{t}", pad(inner)));
            } else {
                text.push_str(&format!(" {operator} {t}"));
            }
        }
        if rows {
            Ok((format!("(op (rows {}))", lines(&nix)), text))
        } else {
            Ok((format!("(op {})", list(&nix)), text))
        }
    }

    fn entry(&self, node: Node<'_>, indent: usize) -> Result<(Out, bool), String> {
        match node.kind() {
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => {
                let text = self.text(node);
                Ok(((self.string(text), text.to_owned()), false))
            }
            "pair" => {
                let key = self.field(node, "key")?;
                let (key, key_text) = match key.kind() {
                    "property_identifier" => {
                        (self.string(self.text(key)), self.text(key).to_owned())
                    }
                    "string" => self.expression(key, indent)?,
                    _ => return Err(self.unsupported(key, "a computed key")),
                };
                let value = self.field(node, "value")?;
                let broken = self.broken(value);
                let (value, value_text) = self.expression(value, indent)?;
                Ok((
                    (list(&[key, value]), format!("{key_text}: {value_text}")),
                    broken,
                ))
            }
            other => Err(self.unsupported(node, other)),
        }
    }

    fn collection(
        &self,
        node: Node<'_>,
        indent: usize,
        name: &str,
        open: &str,
        close: &str,
    ) -> Result<Out, String> {
        let object = name == "obj";
        let broken = self.broken(node);
        let inner = if broken { indent + 2 } else { indent };
        let mut nix = Vec::new();
        let mut text = Vec::new();
        let mut hug = false;
        for item in named(node) {
            let ((n, t), b) = if object {
                self.entry(item, inner)?
            } else {
                (self.expression(item, inner)?, false)
            };
            nix.push(n);
            text.push(t);
            hug = b;
        }
        if broken {
            let rows: Vec<String> = text
                .iter()
                .map(|t| format!("\n{}{t}", pad(inner)))
                .collect();
            return Ok((
                format!("({name} (rows {}))", lines(&nix)),
                format!("{open}{}\n{}{close}", rows.join(","), pad(indent)),
            ));
        }
        let space = if object && !text.is_empty() { " " } else { "" };
        let closing = if hug { "" } else { space };
        Ok((
            format!("({name} {})", list(&nix)),
            format!("{open}{space}{}{closing}{close}", text.join(", ")),
        ))
    }
}
