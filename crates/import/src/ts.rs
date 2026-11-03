use tree_sitter::Node;

use crate::{line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source)?;
    let root = tree.root_node();
    let unit = if source.lines().any(|line| line.starts_with('\t')) {
        "\t"
    } else {
        "  "
    };
    let importer = Importer {
        source,
        holes,
        unit,
    };
    let items = named(root);
    let first = items.first().ok_or("the file is empty")?;
    if first.start_byte() != 0 {
        return Err("the file must start with its first statement".into());
    }
    let last = items.last().ok_or("the file is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the file must end with one newline after its last statement".into());
    }
    let lines = importer.statements(&items, 0)?;
    let list = format!(
        "[\n{}]",
        lines
            .iter()
            .map(|line| format!("  {line}\n"))
            .collect::<String>()
    );
    Ok(if unit == "\t" {
        list
    } else {
        format!("(spaces 2 {list})")
    })
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    unit: &'s str,
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn field<'t>(node: Node<'t>, name: &str) -> Option<Node<'t>> {
    node.child_by_field_name(name)
}

fn token<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children(node)
        .into_iter()
        .find(|child| !child.is_named() && child.kind() == kind)
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(" "))
}

fn rows(items: &[String]) -> String {
    format!(
        "(rows [\n{}])",
        items
            .iter()
            .map(|item| format!("{item}\n"))
            .collect::<String>()
    )
}

const WORDS: [&str; 17] = [
    "identifier",
    "property_identifier",
    "shorthand_property_identifier",
    "shorthand_property_identifier_pattern",
    "private_property_identifier",
    "type_identifier",
    "predefined_type",
    "number",
    "this",
    "super",
    "true",
    "false",
    "null",
    "undefined",
    "statement_identifier",
    "nested_type_identifier",
    "existential_type",
];

const ASSIGNING: [&str; 2] = ["assignment_expression", "augmented_assignment_expression"];

fn group(operator: &str) -> usize {
    match operator {
        "??" => 1,
        "||" => 2,
        "&&" => 3,
        "|" => 4,
        "^" => 5,
        "&" => 6,
        "==" | "!=" | "===" | "!==" => 7,
        "<" | ">" | "<=" | ">=" | "instanceof" | "in" => 8,
        "<<" | ">>" | ">>>" => 9,
        "+" | "-" => 10,
        "*" | "/" | "%" => 11,
        _ => 0,
    }
}

enum Link<'t> {
    Member(bool, Node<'t>, bool),
    Call(Node<'t>),
    Index(Node<'t>),
    Optional,
    NonNull,
    Types(Node<'t>),
}

impl<'s> Importer<'s> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn text(&self, node: Node<'_>) -> &'s str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn gap(&self, from: usize, to: usize) -> &'s str {
        &self.source[from..to]
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

    fn indent(&self, depth: usize) -> String {
        format!("\n{}", self.unit.repeat(depth))
    }

    fn broken(&self, from: usize, to: usize, depth: usize) -> Result<bool, String> {
        let gap = self.gap(from, to);
        if !gap.contains('\n') {
            return Ok(false);
        }
        let want = self.indent(depth);
        if gap == want {
            Ok(true)
        } else {
            Err(format!(
                "{}: expected a line break to {want:?}, found {gap:?}",
                self.at(to)
            ))
        }
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn word(&self, text: &str) -> String {
        match nix::int(text) {
            Some(int) if !text.starts_with('-') => int,
            _ => self.string(text),
        }
    }

    fn separated(
        &self,
        items: &[Node<'_>],
        depth: usize,
        mut item: impl FnMut(Node<'_>) -> Result<String, String>,
    ) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(items[index - 1].end_byte(), node.start_byte());
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 || gap[lines..] != self.unit.repeat(depth) {
                    return Err(format!(
                        "{}: expected a line break to indentation {depth}, found {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..lines {
                    out.push("blank".to_owned());
                }
            }
            out.push(item(*node)?);
        }
        Ok(out)
    }

    fn statements(&self, items: &[Node<'_>], depth: usize) -> Result<Vec<String>, String> {
        self.separated(items, depth, |node| self.statement(node, depth))
    }

    fn comment(&self, node: Node<'_>) -> String {
        format!("(comment {})", self.string(self.text(node)))
    }

    fn block(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if node.kind() != "statement_block" {
            return Err(format!(
                "{}: expected a block, found {}",
                self.at(node.start_byte()),
                node.kind()
            ));
        }
        let items = named(node);
        let close = node.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(node.start_byte() + 1, close, "")?;
            return Ok("[]".into());
        };
        let lead = self.gap(node.start_byte() + 1, first.start_byte());
        let blanks = lead
            .bytes()
            .take_while(|&b| b == b'\n')
            .count()
            .saturating_sub(1);
        self.expect(
            node.start_byte() + 1 + blanks,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        let last = items.last().unwrap_or(first);
        self.expect(last.end_byte(), close, &self.indent(depth))?;
        let mut lines = vec!["blank".to_owned(); blanks];
        lines.extend(self.statements(&items, depth + 1)?);
        Ok(format!(
            "[\n{}]",
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        ))
    }

    fn body(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if node.kind() == "statement_block" {
            self.block(node, depth)
        } else {
            self.statement(node, depth)
        }
    }

    fn semicolon(&self, node: Node<'_>, inner: Node<'_>) -> Result<(), String> {
        let semi = token(node, ";").ok_or_else(|| {
            format!(
                "{}: a statement without its semicolon",
                self.at(node.start_byte())
            )
        })?;
        self.expect(inner.end_byte(), semi.start_byte(), "")?;
        if semi.end_byte() != node.end_byte() {
            return Err(format!(
                "{}: text after a semicolon",
                self.at(semi.end_byte())
            ));
        }
        Ok(())
    }

    fn statement(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        match node.kind() {
            "comment" => Ok(self.comment(node)),
            "expression_statement" => {
                let inner = named(node)[0];
                self.semicolon(node, inner)?;
                self.expr(inner, depth)
            }
            "lexical_declaration" => {
                let declaration = self.declaration(node, depth)?;
                let last = *named(node).last().ok_or("an empty declaration")?;
                self.semicolon(node, last)?;
                Ok(declaration)
            }
            "return_statement" => match named(node).first() {
                None => {
                    self.expect(start + 6, node.end_byte(), ";")?;
                    Ok("return'".into())
                }
                Some(value) => {
                    self.expect(start + 6, value.start_byte(), " ")?;
                    self.semicolon(node, *value)?;
                    Ok(format!("(return {})", self.expr(*value, depth)?))
                }
            },
            "throw_statement" => {
                let value = named(node)[0];
                self.expect(start + 5, value.start_byte(), " ")?;
                self.semicolon(node, value)?;
                Ok(format!("(throw' {})", self.expr(value, depth)?))
            }
            "break_statement" | "continue_statement" => {
                let word = if node.kind() == "break_statement" {
                    "break"
                } else {
                    "continue"
                };
                if self.text(node) != format!("{word};") {
                    return Err(format!("{}: a labelled {word}", self.at(start)));
                }
                Ok(word.to_owned())
            }
            "if_statement" => self.conditional(node, depth),
            "for_statement" => self.for_loop(node, depth),
            "for_in_statement" => self.for_in(node, depth),
            "while_statement" => {
                let condition = field(node, "condition").ok_or("while without a condition")?;
                let body = field(node, "body").ok_or("while without a body")?;
                self.expect(start + 5, condition.start_byte(), " ")?;
                self.expect(condition.end_byte(), body.start_byte(), " ")?;
                Ok(format!(
                    "(while {} {})",
                    self.condition(condition, depth)?,
                    self.body(body, depth)?
                ))
            }
            "switch_statement" => self.switch(node, depth),
            "try_statement" => self.attempt(node, depth),
            "function_declaration" | "class_declaration" | "interface_declaration" => {
                self.expr(node, depth)
            }
            "type_alias_declaration" => {
                let name = field(node, "name").ok_or("type without a name")?;
                let value = field(node, "value").ok_or("type without a value")?;
                self.expect(start + 4, name.start_byte(), " ")?;
                self.expect(name.end_byte(), value.start_byte(), " = ")?;
                self.semicolon(node, value)?;
                Ok(format!(
                    "(type {} {})",
                    self.expr(name, depth)?,
                    self.expr(value, depth)?
                ))
            }
            "export_statement" => self.export(node, depth),
            "import_statement" => self.import(node),
            kind => Err(format!("{}: {kind} is not supported", self.at(start))),
        }
    }

    fn declaration(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let declarators = named(node);
        let [declarator] = declarators.as_slice() else {
            return Err(format!(
                "{}: one declarator per declaration",
                self.at(node.start_byte())
            ));
        };
        let kind = self.text(node.child(0).ok_or("no kind")?);
        let constructor = match kind {
            "const" => "const",
            "let" => "let'",
            other => {
                return Err(format!(
                    "{}: {other} is not supported",
                    self.at(node.start_byte())
                ))
            }
        };
        self.expect(node.start_byte() + kind.len(), declarator.start_byte(), " ")?;
        let name = field(*declarator, "name").ok_or("a declarator without a name")?;
        let mut target = self.expr(name, depth)?;
        let mut end = name.end_byte();
        if let Some(annotation) = field(*declarator, "type") {
            self.expect(end, annotation.start_byte(), "")?;
            target = format!("(typed {target} {})", self.annotation(annotation, depth)?);
            end = annotation.end_byte();
        }
        let value = match field(*declarator, "value") {
            None => "null".to_owned(),
            Some(value) => {
                let equals = token(*declarator, "=").ok_or("no =")?;
                self.expect(end, equals.start_byte(), " ")?;
                self.hang(equals.end_byte(), value, depth)?
            }
        };
        Ok(format!("({constructor} {target} {value})"))
    }

    fn hang(&self, after: usize, value: Node<'_>, depth: usize) -> Result<String, String> {
        if self.broken(after, value.start_byte(), depth + 1)? {
            Ok(format!("(below {})", self.expr(value, depth + 1)?))
        } else {
            self.expect(after, value.start_byte(), " ")?;
            self.expr(value, depth)
        }
    }

    fn annotation(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let inner = named(node);
        let [value] = inner.as_slice() else {
            return Err(format!(
                "{}: an annotation of {} parts",
                self.at(node.start_byte()),
                inner.len()
            ));
        };
        self.expect(node.start_byte() + 1, value.start_byte(), " ")?;
        self.expr(*value, depth)
    }

    fn condition(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let inner = named(node)[0];
        if self.broken(node.start_byte() + 1, inner.start_byte(), depth + 1)? {
            self.expect(inner.end_byte(), node.end_byte() - 1, &self.indent(depth))?;
            return Ok(format!("(below {})", self.expr(inner, depth + 1)?));
        }
        self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
        self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
        self.expr(inner, depth)
    }

    fn conditional(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let condition = field(node, "condition").ok_or("if without a condition")?;
        let consequence = field(node, "consequence").ok_or("if without a body")?;
        self.expect(node.start_byte() + 2, condition.start_byte(), " ")?;
        self.expect(condition.end_byte(), consequence.start_byte(), " ")?;
        let test = self.condition(condition, depth)?;
        let body = self.body(consequence, depth)?;
        let Some(alternative) = field(node, "alternative") else {
            return Ok(format!("(if' {test} {body})"));
        };
        let lead = if consequence.kind() == "statement_block" {
            " "
        } else {
            &self.indent(depth)
        };
        self.expect(consequence.end_byte(), alternative.start_byte(), lead)?;
        let other = named(alternative)[0];
        self.expect(alternative.start_byte() + 4, other.start_byte(), " ")?;
        Ok(format!(
            "(ifElse {test} {body} {})",
            self.body(other, depth)?
        ))
    }

    fn for_loop(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let body = field(node, "body").ok_or("for without a body")?;
        let open = token(node, "(").ok_or("no (")?;
        let close = token(node, ")").ok_or("no )")?;
        self.expect(node.start_byte() + 3, open.start_byte(), " ")?;
        self.expect(close.end_byte(), body.start_byte(), " ")?;
        let init = field(node, "initializer").filter(|init| init.kind() != "empty_statement");
        let condition = field(node, "condition")
            .filter(|condition| condition.kind() != "empty_statement" && condition.kind() != ";")
            .map(|condition| {
                if condition.kind() == "expression_statement" {
                    named(condition)[0]
                } else {
                    condition
                }
            });
        let increment = field(node, "increment");
        let mut want = String::new();
        let init = match init {
            None => "null".to_owned(),
            Some(init) if init.kind() == "lexical_declaration" => {
                let declarators = named(init);
                want.push_str(self.gap(init.start_byte(), declarators[0].end_byte()));
                self.declaration(init, depth)?
            }
            Some(init) => {
                want.push_str(self.text(init).trim_end_matches(';'));
                let inner = if init.kind() == "expression_statement" {
                    named(init)[0]
                } else {
                    init
                };
                self.expr(inner, depth)?
            }
        };
        want.push(';');
        let condition = match condition {
            None => "null".to_owned(),
            Some(condition) => {
                want.push(' ');
                want.push_str(self.text(condition));
                self.expr(condition, depth)?
            }
        };
        want.push(';');
        let increment = match increment {
            None => "null".to_owned(),
            Some(increment) => {
                want.push(' ');
                want.push_str(self.text(increment));
                self.expr(increment, depth)?
            }
        };
        self.expect(open.end_byte(), close.start_byte(), &want)?;
        Ok(format!(
            "(for {init} {condition} {increment} {})",
            self.body(body, depth)?
        ))
    }

    fn for_in(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let left = field(node, "left").ok_or("for without a target")?;
        let right = field(node, "right").ok_or("for without a subject")?;
        let body = field(node, "body").ok_or("for without a body")?;
        let kind = field(node, "kind").ok_or("for without const or let")?;
        let operator = field(node, "operator").ok_or("for without of or in")?;
        let open = token(node, "(").ok_or("no (")?;
        let waiting = token(node, "await");
        match waiting {
            Some(word) => {
                self.expect(node.start_byte() + 3, word.start_byte(), " ")?;
                self.expect(word.end_byte(), open.start_byte(), " ")?;
            }
            None => self.expect(node.start_byte() + 3, open.start_byte(), " ")?,
        }
        self.expect(open.end_byte(), kind.start_byte(), "")?;
        self.expect(kind.end_byte(), left.start_byte(), " ")?;
        self.expect(left.end_byte(), operator.start_byte(), " ")?;
        self.expect(operator.end_byte(), right.start_byte(), " ")?;
        let close = token(node, ")").ok_or("no )")?;
        self.expect(right.end_byte(), close.start_byte(), "")?;
        self.expect(close.end_byte(), body.start_byte(), " ")?;
        let constructor = match (self.text(kind), self.text(operator)) {
            ("const", "of") if waiting.is_some() => "forAwait \"const\"",
            ("const", "of") => "forOf \"const\"",
            ("let", "of") => "forOf \"let\"",
            ("const", "in") => "forIn \"const\"",
            ("let", "in") => "forIn \"let\"",
            (k, o) => {
                return Err(format!(
                    "{}: for ({k} .. {o}) is not supported",
                    self.at(node.start_byte())
                ))
            }
        };
        Ok(format!(
            "({constructor} {} {} {})",
            self.expr(left, depth)?,
            self.expr(right, depth)?,
            self.body(body, depth)?
        ))
    }

    fn switch(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let value = field(node, "value").ok_or("switch without a value")?;
        let body = field(node, "body").ok_or("switch without a body")?;
        self.expect(node.start_byte() + 6, value.start_byte(), " ")?;
        self.expect(value.end_byte(), body.start_byte(), " ")?;
        let cases = named(body);
        let first = cases.first().ok_or("an empty switch")?;
        self.expect(
            body.start_byte() + 1,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        self.expect(
            cases.last().unwrap_or(first).end_byte(),
            body.end_byte() - 1,
            &self.indent(depth),
        )?;
        let arms = self.separated(&cases, depth + 1, |case| {
            if case.kind() == "comment" {
                return Ok(self.comment(case));
            }
            let colon = token(case, ":").ok_or("a case without a colon")?;
            let test = match field(case, "value") {
                Some(test) => {
                    self.expect(case.start_byte() + 4, test.start_byte(), " ")?;
                    self.expect(test.end_byte(), colon.start_byte(), "")?;
                    self.expr(test, depth + 1)?
                }
                None => {
                    self.expect(case.start_byte() + 7, colon.start_byte(), "")?;
                    "null".to_owned()
                }
            };
            let statements: Vec<_> = named(case)
                .into_iter()
                .filter(|child| child.start_byte() > colon.start_byte())
                .collect();
            if let [block] = statements.as_slice() {
                if block.kind() == "statement_block"
                    && self.gap(colon.end_byte(), block.start_byte()) == " "
                {
                    return Ok(format!(
                        "(case {test} (braces {}))",
                        self.block(*block, depth + 1)?
                    ));
                }
            }
            if let Some(first) = statements.first() {
                self.expect(
                    colon.end_byte(),
                    first.start_byte(),
                    &self.indent(depth + 2),
                )?;
            }
            let lines = self.statements(&statements, depth + 2)?;
            Ok(format!("(case {test} {})", list(&lines)))
        })?;
        Ok(format!(
            "(switch {} [\n{}])",
            self.condition(value, depth)?,
            arms.iter()
                .map(|arm| format!("{arm}\n"))
                .collect::<String>()
        ))
    }

    fn attempt(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let body = field(node, "body").ok_or("try without a body")?;
        self.expect(node.start_byte() + 3, body.start_byte(), " ")?;
        let mut handlers = Vec::new();
        let mut end = body.end_byte();
        if let Some(handler) = field(node, "handler") {
            self.expect(end, handler.start_byte(), " ")?;
            let block = field(handler, "body").ok_or("catch without a body")?;
            let parameter = match field(handler, "parameter") {
                Some(parameter) => {
                    let open = token(handler, "(").ok_or("no (")?;
                    let close = token(handler, ")").ok_or("no )")?;
                    self.expect(handler.start_byte() + 5, open.start_byte(), " ")?;
                    self.expect(open.end_byte(), parameter.start_byte(), "")?;
                    self.expect(parameter.end_byte(), close.start_byte(), "")?;
                    self.expect(close.end_byte(), block.start_byte(), " ")?;
                    self.expr(parameter, depth)?
                }
                None => {
                    self.expect(handler.start_byte() + 5, block.start_byte(), " ")?;
                    "null".to_owned()
                }
            };
            handlers.push(format!("(catch {parameter} {})", self.block(block, depth)?));
            end = handler.end_byte();
        }
        if let Some(finalizer) = field(node, "finalizer") {
            self.expect(end, finalizer.start_byte(), " ")?;
            let block = field(finalizer, "body").ok_or("finally without a body")?;
            self.expect(finalizer.start_byte() + 7, block.start_byte(), " ")?;
            handlers.push(format!("(finally {})", self.block(block, depth)?));
        }
        Ok(format!(
            "(try {} {})",
            self.block(body, depth)?,
            list(&handlers)
        ))
    }

    fn export(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        if let Some(declaration) = field(node, "declaration") {
            if let Some(default) = token(node, "default") {
                self.expect(start + 6, default.start_byte(), " ")?;
                self.expect(default.end_byte(), declaration.start_byte(), " ")?;
                return Ok(format!(
                    "(pre \"export\" (pre \"default\" {}))",
                    self.statement(declaration, depth)?
                ));
            }
            self.expect(start + 6, declaration.start_byte(), " ")?;
            return Ok(format!(
                "(pre \"export\" {})",
                self.statement(declaration, depth)?
            ));
        }
        let value = field(node, "value")
            .ok_or_else(|| format!("{}: this export is not supported", self.at(start)))?;
        let default = token(node, "default").ok_or("an export without default")?;
        self.expect(start + 6, default.start_byte(), " ")?;
        self.expect(default.end_byte(), value.start_byte(), " ")?;
        if value.kind() != "function_expression" && value.kind() != "class" {
            self.semicolon(node, value)?;
        } else if value.end_byte() != node.end_byte() {
            return Err(format!(
                "{}: text after an export",
                self.at(value.end_byte())
            ));
        }
        Ok(format!(
            "(pre \"export\" (pre \"default\" {}))",
            self.expr(value, depth)?
        ))
    }

    fn import(&self, node: Node<'_>) -> Result<String, String> {
        let start = node.start_byte();
        let source = field(node, "source").ok_or("import without a source")?;
        let clause = named(node)
            .into_iter()
            .find(|child| child.kind() == "import_clause")
            .ok_or_else(|| format!("{}: an import without a clause", self.at(start)))?;
        let typed = token(node, "type");
        let lead = typed.map_or(start + 6, |word| word.end_byte());
        if let Some(word) = typed {
            self.expect(start + 6, word.start_byte(), " ")?;
        }
        self.expect(lead, clause.start_byte(), " ")?;
        let from = token(node, "from").ok_or("no from")?;
        self.expect(clause.end_byte(), from.start_byte(), " ")?;
        self.expect(from.end_byte(), source.start_byte(), " ")?;
        self.semicolon(node, source)?;
        let parts = named(clause);
        let [part] = parts.as_slice() else {
            return Err(format!(
                "{}: an import clause of {} parts",
                self.at(start),
                parts.len()
            ));
        };
        let names = match part.kind() {
            "named_imports" => {
                let specifiers = named(*part);
                self.delimited(*part, &specifiers, 0, Separator::Comma, true, |specifier| {
                    let name = field(specifier, "name").ok_or("no name")?;
                    let typed = token(specifier, "type");
                    match typed {
                        Some(word) => self.expect(word.end_byte(), name.start_byte(), " ")?,
                        None => self.expect(specifier.start_byte(), name.start_byte(), "")?,
                    }
                    let mut value = self.word(self.text(name));
                    let mut end = name.end_byte();
                    if let Some(alias) = field(specifier, "alias") {
                        self.expect(name.end_byte(), alias.start_byte(), " as ")?;
                        value = format!("(infix [{value} \"as\" {}])", self.word(self.text(alias)));
                        end = alias.end_byte();
                    }
                    if end != specifier.end_byte() {
                        return Err(format!("{}: text after an import specifier", self.at(end)));
                    }
                    Ok(if typed.is_some() {
                        format!("(pre \"type\" {value})")
                    } else {
                        value
                    })
                })?
            }
            "namespace_import" => {
                let name = named(*part)[0];
                self.expect(part.start_byte(), name.start_byte(), "* as ")?;
                format!("(infix [\"*\" \"as\" {}])", self.word(self.text(name)))
            }
            "identifier" => self.word(self.text(*part)),
            kind => return Err(format!("{}: {kind} in an import", self.at(start))),
        };
        let names = if typed.is_some() {
            format!("(pre \"type\" {names})")
        } else {
            names
        };
        Ok(format!("(import' {names} {})", self.expr(source, 0)?))
    }

    fn delimited(
        &self,
        node: Node<'_>,
        items: &[Node<'_>],
        depth: usize,
        separator: Separator,
        padded: bool,
        mut item: impl FnMut(Node<'_>) -> Result<String, String>,
    ) -> Result<String, String> {
        let open = node.start_byte() + 1;
        let close = node.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(open, close, "")?;
            return Ok("[]".into());
        };
        let last = *items.last().unwrap_or(first);
        let mark = separator.mark();
        if !self.gap(open, first.start_byte()).contains('\n') {
            let pad = if padded { " " } else { "" };
            self.expect(open, first.start_byte(), pad)?;
            for pair in items.windows(2) {
                self.expect(
                    pair[0].end_byte(),
                    pair[1].start_byte(),
                    &format!("{mark} "),
                )?;
            }
            let hugged = separator == Separator::Comma && !padded && self.hangs(last);
            let tail = if hugged {
                format!(",{}", self.indent(depth))
            } else {
                pad.to_owned()
            };
            self.expect(last.end_byte(), close, &tail)?;
            let values = items
                .iter()
                .map(|node| item(*node))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(list(&values));
        }
        self.expect(open, first.start_byte(), &self.indent(depth + 1))?;
        let mut out = Vec::new();
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let previous = items[index - 1];
                let mut gap = self.gap(previous.end_byte(), node.start_byte());
                if previous.kind() != "comment" {
                    gap = gap.strip_prefix(mark).ok_or_else(|| {
                        format!(
                            "{}: expected {mark:?} after an item",
                            self.at(previous.end_byte())
                        )
                    })?;
                }
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 || gap[lines..] != self.unit.repeat(depth + 1) {
                    return Err(format!(
                        "{}: items separated by {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..lines {
                    out.push("blank".to_owned());
                }
            }
            out.push(if node.kind() == "comment" {
                self.comment(*node)
            } else {
                item(*node)?
            });
        }
        let rest = last.kind() == "rest_pattern"
            || field(last, "pattern").is_some_and(|pattern| pattern.kind() == "rest_pattern");
        let trailing = if last.kind() == "comment" || (separator == Separator::Comma && rest) {
            ""
        } else {
            mark
        };
        self.expect(
            last.end_byte(),
            close,
            &format!("{trailing}{}", self.indent(depth)),
        )?;
        Ok(rows(&out))
    }

    fn expr(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let kind = node.kind();
        if WORDS.contains(&kind) || !node.is_named() {
            return Ok(self.word(self.text(node)));
        }
        match kind {
            "member_expression"
            | "call_expression"
            | "subscript_expression"
            | "non_null_expression" => self.postfix(node, depth),
            "string" => {
                let text = self.text(node);
                let raw = &text[1..text.len() - 1];
                let constructor = if text.starts_with('"') { "str" } else { "sq" };
                Ok(format!("({constructor} {})", self.string(raw)))
            }
            "template_string" => self.template(node, depth),
            "regex" => {
                let pattern = field(node, "pattern").ok_or("a regex without a pattern")?;
                let flags = field(node, "flags").map_or("", |flags| self.text(flags));
                Ok(format!(
                    "(regex {} {})",
                    self.string(self.text(pattern)),
                    self.string(flags)
                ))
            }
            "array" | "array_pattern" => {
                let items = named(node);
                self.delimited(node, &items, depth, Separator::Comma, false, |item| {
                    self.expr(item, depth + usize::from(self.multiline(node)))
                })
            }
            "object" | "object_pattern" => {
                let items = named(node);
                let inner = depth + usize::from(self.multiline(node));
                Ok(format!(
                    "(obj {})",
                    self.delimited(node, &items, depth, Separator::Comma, true, |item| self
                        .member(item, inner))?
                ))
            }
            "object_type" => {
                let items = named(node);
                let inner = depth + usize::from(self.multiline(node));
                let separator = if self.text(node).contains(',') && !self.text(node).contains(';') {
                    Separator::Comma
                } else {
                    Separator::Semicolon
                };
                if separator == Separator::Comma && !items.is_empty() {
                    return Err(format!(
                        "{}: an object type separated by commas",
                        self.at(start)
                    ));
                }
                Ok(format!(
                    "(shape {})",
                    self.delimited(node, &items, depth, separator, true, |item| self
                        .member(item, inner))?
                ))
            }
            "pair"
            | "pair_pattern"
            | "shorthand_property_identifier"
            | "spread_element"
            | "method_definition"
            | "object_assignment_pattern" => self.member(node, depth),
            "binary_expression"
            | "assignment_expression"
            | "augmented_assignment_expression"
            | "as_expression"
            | "satisfies_expression"
            | "union_type"
            | "type_predicate"
            | "assignment_pattern"
            | "intersection_type" => self.infix(node, depth),
            "unary_expression" | "await_expression" | "type_query" | "readonly_type"
            | "spread_element_" | "rest_pattern" | "keyof_type_query" => {
                let parts = children(node);
                let operator = parts[0];
                let argument = *parts.last().ok_or("no operand")?;
                let op = self.text(operator);
                let gap = if op.chars().all(|c| c.is_ascii_alphabetic()) {
                    " "
                } else {
                    ""
                };
                self.expect(operator.end_byte(), argument.start_byte(), gap)?;
                if op == "-" && argument.kind() == "number" {
                    return Ok(self.string(self.text(node)));
                }
                Ok(format!(
                    "(pre {} {})",
                    self.string(op),
                    self.expr(argument, depth)?
                ))
            }
            "update_expression" => {
                let parts = children(node);
                if parts[0].is_named() {
                    self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
                    Ok(format!(
                        "(post {} {})",
                        self.expr(parts[0], depth)?,
                        self.string(self.text(parts[1]))
                    ))
                } else {
                    self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
                    Ok(format!(
                        "(pre {} {})",
                        self.string(self.text(parts[0])),
                        self.expr(parts[1], depth)?
                    ))
                }
            }
            "array_type" => {
                let inner = named(node)[0];
                self.expect(inner.end_byte(), node.end_byte(), "[]")?;
                let value = self.expr(inner, depth)?;
                if WORDS.contains(&inner.kind()) {
                    Ok(self.string(self.text(node)))
                } else {
                    Ok(format!("(post {value} \"[]\")"))
                }
            }
            "literal_type" => self.expr(named(node)[0], depth),
            "parenthesized_expression" | "parenthesized_type" => {
                let inner = named(node)[0];
                if self.broken(start + 1, inner.start_byte(), depth + 1)? {
                    self.expect(inner.end_byte(), node.end_byte() - 1, &self.indent(depth))?;
                    return Ok(format!("(paren (below {}))", self.expr(inner, depth + 1)?));
                }
                self.expect(start + 1, inner.start_byte(), "")?;
                self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
                Ok(format!("(paren {})", self.expr(inner, depth)?))
            }
            "ternary_expression" => {
                let condition = field(node, "condition").ok_or("no condition")?;
                let consequence = field(node, "consequence").ok_or("no consequence")?;
                let alternative = field(node, "alternative").ok_or("no alternative")?;
                let question = token(node, "?").ok_or("no ?")?;
                let colon = token(node, ":").ok_or("no :")?;
                let branch =
                    |before: usize, mark: Node<'_>, value: Node<'_>| -> Result<String, String> {
                        self.expect(mark.end_byte(), value.start_byte(), " ")?;
                        if self.broken(before, mark.start_byte(), depth + 1)? {
                            Ok(format!("(below {})", self.expr(value, depth + 2)?))
                        } else {
                            self.expect(before, mark.start_byte(), " ")?;
                            self.expr(value, depth)
                        }
                    };
                Ok(format!(
                    "(ternary {} {} {})",
                    self.expr(condition, depth)?,
                    branch(condition.end_byte(), question, consequence)?,
                    branch(consequence.end_byte(), colon, alternative)?
                ))
            }
            "arrow_function" => self.arrow(node, depth),
            "function_expression" | "function_declaration" => self.function(node, depth),
            "new_expression" => {
                let constructor = field(node, "constructor").ok_or("new without a constructor")?;
                let arguments = field(node, "arguments")
                    .ok_or_else(|| format!("{}: new without arguments", self.at(start)))?;
                self.expect(start + 3, constructor.start_byte(), " ")?;
                let mut callee = self.expr(constructor, depth)?;
                let mut end = constructor.end_byte();
                if let Some(types) = field(node, "type_arguments") {
                    self.expect(end, types.start_byte(), "")?;
                    callee = format!("(generic {callee} {})", self.type_arguments(types, depth)?);
                    end = types.end_byte();
                }
                self.expect(end, arguments.start_byte(), "")?;
                Ok(format!(
                    "(new {callee} {})",
                    self.arguments(arguments, depth)?
                ))
            }
            "generic_type" => {
                let name = field(node, "name").ok_or("a generic without a name")?;
                let types = field(node, "type_arguments").ok_or("a generic without arguments")?;
                self.expect(name.end_byte(), types.start_byte(), "")?;
                Ok(format!(
                    "(generic {} {})",
                    self.expr(name, depth)?,
                    self.type_arguments(types, depth)?
                ))
            }
            "lookup_type" => {
                let parts = named(node);
                self.expect(parts[0].end_byte(), parts[1].start_byte(), "[")?;
                self.expect(parts[1].end_byte(), node.end_byte(), "]")?;
                Ok(format!(
                    "(idx {} {})",
                    self.expr(parts[0], depth)?,
                    self.expr(parts[1], depth)?
                ))
            }
            "function_type" => {
                let parameters =
                    field(node, "parameters").ok_or("a function type without parameters")?;
                let result =
                    field(node, "return_type").ok_or("a function type without a result")?;
                self.expect(parameters.end_byte(), result.start_byte(), " => ")?;
                Ok(format!(
                    "(arrow {} {})",
                    self.parameters(parameters, depth)?,
                    self.expr(result, depth)?
                ))
            }
            "class_declaration" | "class" => self.class(node, depth),
            "interface_declaration" => {
                let name = field(node, "name").ok_or("an interface without a name")?;
                let body = field(node, "body").ok_or("an interface without a body")?;
                self.expect(start + 9, name.start_byte(), " ")?;
                let mut head = self.expr(name, depth)?;
                let mut end = name.end_byte();
                if let Some(extends) = named(node)
                    .into_iter()
                    .find(|child| child.kind() == "extends_type_clause")
                {
                    self.expect(end, extends.start_byte(), " ")?;
                    let types = named(extends);
                    self.expect(extends.start_byte() + 7, types[0].start_byte(), " ")?;
                    if types.len() != 1 {
                        return Err(format!(
                            "{}: an interface extending several",
                            self.at(start)
                        ));
                    }
                    head = format!(
                        "(infix [{head} \"extends\" {}])",
                        self.expr(types[0], depth)?
                    );
                    end = extends.end_byte();
                }
                self.expect(end, body.start_byte(), " ")?;
                Ok(format!(
                    "(interface {head} {})",
                    self.members(body, depth, Separator::Semicolon)?
                ))
            }
            "property_signature"
            | "method_signature"
            | "public_field_definition"
            | "index_signature" => self.member(node, depth),
            "required_parameter" | "optional_parameter" => self.parameter(node, depth),
            "type_annotation" => self.annotation(node, depth),
            _ => Err(format!("{}: {kind} is not supported", self.at(start))),
        }
    }

    fn hangs(&self, node: Node<'_>) -> bool {
        node.kind() == "arrow_function"
            && field(node, "body").is_some_and(|body| {
                body.kind() != "statement_block"
                    && token(node, "=>").is_some_and(|arrow| {
                        self.gap(arrow.end_byte(), body.start_byte()).contains('\n')
                    })
            })
    }

    fn multiline(&self, node: Node<'_>) -> bool {
        let items = named(node);
        items.first().is_some_and(|first| {
            self.gap(node.start_byte() + 1, first.start_byte())
                .contains('\n')
        })
    }

    fn type_arguments(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.expr(item, depth)
        })
    }

    fn arguments(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        let inner = depth + usize::from(self.multiline(node));
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.expr(item, inner)
        })
    }

    fn parameters(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        let inner = depth + usize::from(self.multiline(node));
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.expr(item, inner)
        })
    }

    fn parameter(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let pattern = field(node, "pattern").ok_or("a parameter without a pattern")?;
        let mut modifiers = Vec::new();
        let mut at = node.start_byte();
        for child in children(node) {
            if child.start_byte() >= pattern.start_byte() {
                break;
            }
            self.expect(at, child.start_byte(), "")?;
            modifiers.push(self.text(child));
            at = child.end_byte() + 1;
            self.expect(child.end_byte(), at, " ")?;
        }
        self.expect(at, pattern.start_byte(), "")?;
        let mut value = self.expr(pattern, depth)?;
        let mut end = pattern.end_byte();
        if node.kind() == "optional_parameter" {
            self.expect(end, end + 1, "?")?;
            value = match pattern.kind() {
                "identifier" => self.string(&format!("{}?", self.text(pattern))),
                _ => return Err(format!("{}: an optional pattern", self.at(end))),
            };
            end += 1;
        }
        if let Some(annotation) = field(node, "type") {
            self.expect(end, annotation.start_byte(), "")?;
            value = format!("(typed {value} {})", self.annotation(annotation, depth)?);
            end = annotation.end_byte();
        }
        if let Some(default) = field(node, "value") {
            self.expect(end, default.start_byte(), " = ")?;
            value = format!("(infix [{value} \"=\" {}])", self.expr(default, depth)?);
        }
        for modifier in modifiers.iter().rev() {
            value = format!("(pre {} {value})", self.string(modifier));
        }
        Ok(value)
    }

    fn signature(
        &self,
        parameters: Node<'_>,
        result: Option<Node<'_>>,
        depth: usize,
    ) -> Result<(String, usize), String> {
        let mut value = self.parameters(parameters, depth)?;
        let mut end = parameters.end_byte();
        if let Some(result) = result {
            self.expect(end, result.start_byte(), "")?;
            value = format!("(typed {value} {})", self.annotation(result, depth)?);
            end = result.end_byte();
        }
        Ok((value, end))
    }

    fn function(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let parts = children(node);
        let asynchronous = parts[0].kind() == "async";
        let keyword = parts[usize::from(asynchronous)];
        if keyword.kind() != "function" {
            return Err(format!(
                "{}: {} functions are not supported",
                self.at(start),
                keyword.kind()
            ));
        }
        if asynchronous {
            self.expect(parts[0].end_byte(), keyword.start_byte(), " ")?;
        }
        if field(node, "type_parameters").is_some() || token(node, "*").is_some() {
            return Err(format!(
                "{}: generic and generator functions are not supported",
                self.at(start)
            ));
        }
        let parameters = field(node, "parameters").ok_or("a function without parameters")?;
        let name = match field(node, "name") {
            Some(name) => {
                self.expect(keyword.end_byte(), name.start_byte(), " ")?;
                self.expect(name.end_byte(), parameters.start_byte(), "")?;
                self.expr(name, depth)?
            }
            None => {
                self.expect(keyword.end_byte(), parameters.start_byte(), " ")?;
                "null".to_owned()
            }
        };
        let (signature, end) = self.signature(parameters, field(node, "return_type"), depth)?;
        let body = field(node, "body").ok_or("a function without a body")?;
        self.expect(end, body.start_byte(), " ")?;
        let value = format!("(fn {name} {signature} {})", self.block(body, depth)?);
        Ok(if asynchronous {
            format!("(pre \"async\" {value})")
        } else {
            value
        })
    }

    fn arrow(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let parts = children(node);
        let asynchronous = parts[0].kind() == "async";
        let (parameters, mut end) = match field(node, "parameter") {
            Some(parameter) => (self.expr(parameter, depth)?, parameter.end_byte()),
            None => {
                let parameters = field(node, "parameters").ok_or("an arrow without parameters")?;
                self.signature(parameters, field(node, "return_type"), depth)?
            }
        };
        if asynchronous {
            self.expect(parts[0].end_byte(), parts[1].start_byte(), " ")?;
        }
        if field(node, "type_parameters").is_some() {
            return Err(format!(
                "{}: generic arrows are not supported",
                self.at(node.start_byte())
            ));
        }
        let arrow = token(node, "=>").ok_or("no =>")?;
        self.expect(end, arrow.start_byte(), " ")?;
        end = arrow.end_byte();
        let body = field(node, "body").ok_or("an arrow without a body")?;
        let value = if body.kind() == "statement_block" {
            self.expect(end, body.start_byte(), " ")?;
            self.block(body, depth)?
        } else if self.broken(end, body.start_byte(), depth + 1)? {
            format!("(below {})", self.arrow_body(body, depth + 1)?)
        } else {
            self.expect(end, body.start_byte(), " ")?;
            self.arrow_body(body, depth)?
        };
        let value = format!("(arrow {parameters} {value})");
        Ok(if asynchronous {
            format!("(pre \"async\" {value})")
        } else {
            value
        })
    }

    fn arrow_body(&self, body: Node<'_>, depth: usize) -> Result<String, String> {
        let value = self.expr(body, depth)?;
        Ok(if body.kind() == "array" {
            format!("(array {value})")
        } else {
            value
        })
    }

    fn template(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut parts = Vec::new();
        let mut at = node.start_byte() + 1;
        for substitution in named(node)
            .into_iter()
            .filter(|child| child.kind() == "template_substitution")
        {
            if substitution.start_byte() > at {
                parts.push(self.string(self.gap(at, substitution.start_byte())));
            }
            let inner = named(substitution);
            let [value] = inner.as_slice() else {
                return Err(format!(
                    "{}: a substitution of {} parts",
                    self.at(substitution.start_byte()),
                    inner.len()
                ));
            };
            self.expect(substitution.start_byte() + 2, value.start_byte(), "")?;
            self.expect(value.end_byte(), substitution.end_byte() - 1, "")?;
            parts.push(format!("(sub {})", self.expr(*value, depth)?));
            at = substitution.end_byte();
        }
        let end = node.end_byte() - 1;
        if end > at {
            parts.push(self.string(self.gap(at, end)));
        }
        Ok(format!("(tpl {})", list(&parts)))
    }

    fn infix(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut operands = Vec::new();
        let mut operators = Vec::new();
        self.flatten(node, &mut operands, &mut operators)?;
        let mut gaps = Vec::new();
        for (index, operator) in operators.iter().enumerate() {
            self.expect(operands[index].end_byte(), operator.start_byte(), " ")?;
            gaps.push(self.gap(operator.end_byte(), operands[index + 1].start_byte()));
        }
        let assigning = ASSIGNING.contains(&node.kind()) || node.kind() == "assignment_pattern";
        if operators.len() == 1 && assigning {
            let value = self.hang(operators[0].end_byte(), operands[1], depth)?;
            return Ok(format!(
                "(infix [{} {} {value}])",
                self.expr(operands[0], depth)?,
                self.string(self.text(operators[0]))
            ));
        }
        let broken = gaps.iter().any(|gap| gap.contains('\n'));
        let want = if broken {
            self.indent(depth)
        } else {
            " ".to_owned()
        };
        for (index, gap) in gaps.iter().enumerate() {
            if *gap != want {
                return Err(format!(
                    "{}: an operator followed by {gap:?}, expected {want:?}",
                    self.at(operands[index + 1].start_byte())
                ));
            }
        }
        let mut items = vec![self.expr(operands[0], depth)?];
        for (index, operator) in operators.iter().enumerate() {
            items.push(self.string(self.text(*operator)));
            items.push(self.expr(operands[index + 1], depth)?);
        }
        Ok(if broken {
            format!("(infix {})", rows(&items))
        } else {
            format!("(infix {})", list(&items))
        })
    }

    fn flatten<'t>(
        &self,
        node: Node<'t>,
        operands: &mut Vec<Node<'t>>,
        operators: &mut Vec<Node<'t>>,
    ) -> Result<(), String> {
        let parts = children(node);
        let (left, operator, right) = match node.kind() {
            "as_expression" | "satisfies_expression" | "type_predicate" => {
                let named_parts = named(node);
                (
                    named_parts[0],
                    parts[1],
                    *parts.last().ok_or("no right operand")?,
                )
            }
            "union_type" | "intersection_type" if parts.len() == 3 => {
                (parts[0], parts[1], parts[2])
            }
            "union_type" | "intersection_type" => {
                return Err(format!(
                    "{}: a leading union bar",
                    self.at(node.start_byte())
                ))
            }
            _ => (
                field(node, "left").ok_or("no left operand")?,
                field(node, "operator")
                    .or_else(|| {
                        parts
                            .iter()
                            .copied()
                            .find(|part| !part.is_named() && part.kind() == "=")
                    })
                    .ok_or("no operator")?,
                field(node, "right").ok_or("no right operand")?,
            ),
        };
        let op = self.text(operator);
        let flat = left.kind() == node.kind()
            && node.kind() == "binary_expression"
            && group(op) != 0
            && field(left, "operator").is_some_and(|inner| group(self.text(inner)) == group(op))
            || (left.kind() == node.kind()
                && (node.kind() == "union_type" || node.kind() == "intersection_type"));
        if flat {
            self.flatten(left, operands, operators)?;
        } else {
            operands.push(left);
        }
        operators.push(operator);
        operands.push(right);
        Ok(())
    }

    fn chain<'t>(&self, node: Node<'t>, links: &mut Vec<Link<'t>>) -> Result<Node<'t>, String> {
        let mut current = node;
        loop {
            match current.kind() {
                "member_expression" => {
                    let object = field(current, "object").ok_or("no object")?;
                    let property = field(current, "property").ok_or("no property")?;
                    let optional = optional(current);
                    let accessor = if optional { "?." } else { "." };
                    let gap = self.gap(object.end_byte(), property.start_byte());
                    let broken = gap.contains('\n');
                    if !broken && gap != accessor {
                        return Err(format!(
                            "{}: member access spaced as {gap:?}",
                            self.at(property.start_byte())
                        ));
                    }
                    links.push(Link::Member(optional, property, broken));
                    current = object;
                }
                "call_expression" => {
                    let function = field(current, "function").ok_or("no callee")?;
                    let arguments = field(current, "arguments").ok_or("no arguments")?;
                    if arguments.kind() != "arguments" {
                        return Err(format!(
                            "{}: tagged templates are not supported",
                            self.at(arguments.start_byte())
                        ));
                    }
                    links.push(Link::Call(arguments));
                    let mut end = arguments.start_byte();
                    if let Some(types) = field(current, "type_arguments") {
                        self.expect(types.end_byte(), end, "")?;
                        links.push(Link::Types(types));
                        end = types.start_byte();
                    }
                    if optional(current) {
                        self.expect(function.end_byte(), end, "?.")?;
                        links.push(Link::Optional);
                    } else {
                        self.expect(function.end_byte(), end, "")?;
                    }
                    current = function;
                }
                "subscript_expression" => {
                    let object = field(current, "object").ok_or("no object")?;
                    let index = field(current, "index").ok_or("no index")?;
                    let optional = optional(current);
                    self.expect(
                        object.end_byte(),
                        index.start_byte(),
                        if optional { "?.[" } else { "[" },
                    )?;
                    self.expect(index.end_byte(), current.end_byte(), "]")?;
                    links.push(Link::Index(index));
                    if optional {
                        links.push(Link::Optional);
                    }
                    current = object;
                }
                "non_null_expression" => {
                    let inner = named(current)[0];
                    self.expect(inner.end_byte(), current.end_byte(), "!")?;
                    links.push(Link::NonNull);
                    current = inner;
                }
                _ => return Ok(current),
            }
        }
    }

    fn postfix(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut links = Vec::new();
        let base = self.chain(node, &mut links)?;
        links.reverse();
        let split = links
            .iter()
            .position(|link| matches!(link, Link::Member(_, _, true)));
        let Some(split) = split else {
            return self.fold(Value::from(self, base, depth)?, &links, depth);
        };
        let head = self.fold(Value::from(self, base, depth)?, &links[..split], depth)?;
        let mut out = vec![head];
        let mut index = split;
        while index < links.len() {
            let Link::Member(optional, property, true) = links[index] else {
                return Err("a chain link".into());
            };
            if optional {
                return Err(format!(
                    "{}: an optional link starting a chain row",
                    self.at(property.start_byte())
                ));
            }
            let gap = self.gap(property.start_byte() - 1, property.start_byte());
            if gap != "." {
                return Err(format!(
                    "{}: a chain row not starting with a dot",
                    self.at(property.start_byte())
                ));
            }
            let before = self.source[..property.start_byte() - 1]
                .trim_end_matches([' ', '\t']);
            let pad = &self.source[before.len()..property.start_byte() - 1];
            if !before.ends_with('\n')
                || pad != self.unit.repeat(depth + 1)
                || before[..before.len() - 1].ends_with('\n')
            {
                return Err(format!(
                    "{}: a chain row indented {pad:?}",
                    self.at(property.start_byte())
                ));
            }
            let end = links[index + 1..]
                .iter()
                .position(|link| matches!(link, Link::Member(_, _, true)))
                .map_or(links.len(), |at| index + 1 + at);
            out.push(self.fold(
                Value::Word(self.text(property).to_owned()),
                &links[index + 1..end],
                depth + 1,
            )?);
            index = end;
        }
        Ok(format!(
            "(chain [\n{}])",
            out.iter().map(|row| format!("{row}\n")).collect::<String>()
        ))
    }

    fn fold(&self, mut value: Value, links: &[Link<'_>], depth: usize) -> Result<String, String> {
        for link in links {
            value = match link {
                Link::Member(optional, property, _) => {
                    let name = self.text(*property);
                    let accessor = if *optional { "?." } else { "." };
                    match value {
                        Value::Word(word) => Value::Word(format!("{word}{accessor}{name}")),
                        Value::Dot(base, path) => {
                            Value::Dot(base, format!("{path}{accessor}{name}"))
                        }
                        Value::Node(node) => Value::Dot(
                            node,
                            if *optional {
                                format!("?.{name}")
                            } else {
                                name.to_owned()
                            },
                        ),
                    }
                }
                Link::Optional => match value {
                    Value::Word(word) => Value::Word(format!("{word}?.")),
                    Value::Dot(base, path) => Value::Dot(base, format!("{path}?.")),
                    Value::Node(node) => Value::Dot(node, "?.".to_owned()),
                },
                Link::Call(arguments) => Value::Node(format!(
                    "(call {} {})",
                    value.render(self),
                    self.arguments(*arguments, depth)?
                )),
                Link::Types(types) => Value::Node(format!(
                    "(generic {} {})",
                    value.render(self),
                    self.type_arguments(*types, depth)?
                )),
                Link::Index(index) => Value::Node(format!(
                    "(idx {} {})",
                    value.render(self),
                    self.expr(*index, depth)?
                )),
                Link::NonNull => Value::Node(format!("(post {} \"!\")", value.render(self))),
            };
        }
        Ok(value.render(self))
    }

    fn member(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        match node.kind() {
            "comment" => Ok(self.comment(node)),
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => {
                Ok(self.word(self.text(node)))
            }
            "spread_element" | "rest_pattern" => {
                let inner = named(node)[0];
                self.expect(start + 3, inner.start_byte(), "")?;
                Ok(format!("(pre \"...\" {})", self.expr(inner, depth)?))
            }
            "pair" | "pair_pattern" => {
                let key = field(node, "key").ok_or("a pair without a key")?;
                let value = field(node, "value").ok_or("a pair without a value")?;
                let colon = token(node, ":").ok_or("no :")?;
                self.expect(key.end_byte(), colon.start_byte(), "")?;
                Ok(format!(
                    "(prop {} {})",
                    self.key(key, depth)?,
                    self.hang(colon.end_byte(), value, depth)?
                ))
            }
            "object_assignment_pattern" => {
                let left = field(node, "left").ok_or("no left")?;
                let right = field(node, "right").ok_or("no right")?;
                self.expect(left.end_byte(), right.start_byte(), " = ")?;
                Ok(format!(
                    "(infix [{} \"=\" {}])",
                    self.expr(left, depth)?,
                    self.expr(right, depth)?
                ))
            }
            "method_definition" | "method_signature" => {
                let name = field(node, "name").ok_or("a method without a name")?;
                let mut modifiers = Vec::new();
                let mut at = start;
                for child in children(node) {
                    if child.start_byte() >= name.start_byte() {
                        break;
                    }
                    self.expect(at, child.start_byte(), "")?;
                    modifiers.push(self.text(child));
                    self.expect(child.end_byte(), child.end_byte() + 1, " ")?;
                    at = child.end_byte() + 1;
                }
                self.expect(at, name.start_byte(), "")?;
                let parameters = field(node, "parameters").ok_or("a method without parameters")?;
                let mut name_end = name.end_byte();
                let mut key = self.key(name, depth)?;
                if token(node, "?").is_some() {
                    self.expect(name_end, name_end + 1, "?")?;
                    key = self.string(&format!("{}?", self.text(name)));
                    name_end += 1;
                }
                self.expect(name_end, parameters.start_byte(), "")?;
                let (signature, end) =
                    self.signature(parameters, field(node, "return_type"), depth)?;
                let mut value = match field(node, "body") {
                    Some(body) => {
                        self.expect(end, body.start_byte(), " ")?;
                        format!("(method {key} {signature} {})", self.block(body, depth)?)
                    }
                    None => format!("(method {key} {signature} null)"),
                };
                for modifier in modifiers.iter().rev() {
                    value = format!("(pre {} {value})", self.string(modifier));
                }
                Ok(value)
            }
            "property_signature" | "public_field_definition" => {
                let name = field(node, "name").ok_or("a property without a name")?;
                let mut modifiers = Vec::new();
                let mut at = start;
                for child in children(node) {
                    if child.start_byte() >= name.start_byte() {
                        break;
                    }
                    self.expect(at, child.start_byte(), "")?;
                    modifiers.push(self.text(child));
                    self.expect(child.end_byte(), child.end_byte() + 1, " ")?;
                    at = child.end_byte() + 1;
                }
                self.expect(at, name.start_byte(), "")?;
                let mut value = self.key(name, depth)?;
                let mut end = name.end_byte();
                if token(node, "?").is_some() {
                    self.expect(end, end + 1, "?")?;
                    value = self.string(&format!("{}?", self.text(name)));
                    end += 1;
                }
                if let Some(annotation) = field(node, "type") {
                    self.expect(end, annotation.start_byte(), "")?;
                    value = format!("(typed {value} {})", self.annotation(annotation, depth)?);
                    end = annotation.end_byte();
                }
                if let Some(initial) = field(node, "value") {
                    let equals = token(node, "=").ok_or("no =")?;
                    self.expect(end, equals.start_byte(), " ")?;
                    value = format!(
                        "(infix [{value} \"=\" {}])",
                        self.hang(equals.end_byte(), initial, depth)?
                    );
                    end = initial.end_byte();
                }
                if end != node.end_byte() {
                    return Err(format!("{}: text after a property", self.at(end)));
                }
                for modifier in modifiers.iter().rev() {
                    value = format!("(pre {} {value})", self.string(modifier));
                }
                Ok(value)
            }
            _ => self.expr(node, depth),
        }
    }

    fn key(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if node.kind() == "computed_property_name" {
            let inner = named(node)[0];
            self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
            self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
            return Ok(format!("[{}]", self.expr(inner, depth)?));
        }
        self.expr(node, depth)
    }

    fn members(
        &self,
        body: Node<'_>,
        depth: usize,
        separator: Separator,
    ) -> Result<String, String> {
        let items = named(body);
        let close = body.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(body.start_byte() + 1, close, "")?;
            return Ok("[]".into());
        };
        self.expect(
            body.start_byte() + 1,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        let mark = separator.mark();
        let mut out = Vec::new();
        let mut end = body.start_byte() + 1;
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(end, node.start_byte());
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 || gap[lines..] != self.unit.repeat(depth + 1) {
                    return Err(format!(
                        "{}: members separated by {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..lines {
                    out.push("blank".to_owned());
                }
            }
            end = node.end_byte();
            let terminated = !matches!(node.kind(), "comment" | "method_definition");
            if terminated {
                let after = self.gap(end, end + mark.len());
                if after != mark {
                    return Err(format!("{}: a member without {mark:?}", self.at(end)));
                }
                end += mark.len();
            }
            out.push(self.member(*node, depth + 1)?);
        }
        self.expect(end, close, &self.indent(depth))?;
        Ok(format!(
            "[\n{}]",
            out.iter()
                .map(|item| format!("{item}\n"))
                .collect::<String>()
        ))
    }

    fn class(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let name = field(node, "name").ok_or("a class without a name")?;
        let body = field(node, "body").ok_or("a class without a body")?;
        self.expect(start + 5, name.start_byte(), " ")?;
        let mut head = self.expr(name, depth)?;
        let mut end = name.end_byte();
        if let Some(heritage) = named(node)
            .into_iter()
            .find(|child| child.kind() == "class_heritage")
        {
            self.expect(end, heritage.start_byte(), " ")?;
            let clauses = named(heritage);
            let [clause] = clauses.as_slice() else {
                return Err(format!(
                    "{}: a class with several heritage clauses",
                    self.at(start)
                ));
            };
            let types = named(*clause);
            if clause.kind() != "extends_clause" || types.len() != 1 {
                return Err(format!(
                    "{}: only a single extends is supported",
                    self.at(start)
                ));
            }
            self.expect(clause.start_byte() + 7, types[0].start_byte(), " ")?;
            head = format!(
                "(infix [{head} \"extends\" {}])",
                self.expr(types[0], depth)?
            );
            end = heritage.end_byte();
        }
        self.expect(end, body.start_byte(), " ")?;
        Ok(format!(
            "(class {head} {})",
            self.members(body, depth, Separator::Semicolon)?
        ))
    }
}

fn optional(node: Node<'_>) -> bool {
    children(node)
        .iter()
        .any(|child| child.kind() == "optional_chain" || child.kind() == "?.")
}

#[derive(Clone, Copy, PartialEq)]
enum Separator {
    Comma,
    Semicolon,
}

impl Separator {
    fn mark(self) -> &'static str {
        match self {
            Separator::Comma => ",",
            Separator::Semicolon => ";",
        }
    }
}

enum Value {
    Word(String),
    Node(String),
    Dot(String, String),
}

impl Value {
    fn from(importer: &Importer<'_>, node: Node<'_>, depth: usize) -> Result<Value, String> {
        if WORDS.contains(&node.kind()) {
            Ok(Value::Word(importer.text(node).to_owned()))
        } else {
            Ok(Value::Node(importer.expr(node, depth)?))
        }
    }

    fn render(self, importer: &Importer<'_>) -> String {
        match self {
            Value::Word(word) => importer.word(&word),
            Value::Node(node) => node,
            Value::Dot(base, path) => format!("(dot {base} {})", importer.string(&path)),
        }
    }
}
