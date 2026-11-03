use tree_sitter::Node;

use crate::nix;

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let mut items = Vec::new();
    let mut text = String::new();
    let mut offset = 0;
    let mut header = 0;
    for line in source.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if !line.ends_with('\n') || !(bare.is_empty() || bare.starts_with('\'')) {
            break;
        }
        items.push(if bare.is_empty() {
            "blank".to_owned()
        } else {
            format!("(comment {})", nix::string(bare, holes, 0))
        });
        text.push_str(line);
        offset += line.len();
        header += 1;
    }
    let body = &source[offset..];
    if !body.is_empty() {
        let tree =
            crate::parse(&tree_sitter_vbscript::language(), body).map_err(|e| shift(&e, header))?;
        let importer = Importer {
            source: body,
            holes,
        };
        let (nix, lines) = importer
            .sequence(&named(tree.root_node()), None, 0)
            .map_err(|e| shift(&e, header))?;
        items.extend(nix);
        for line in lines {
            text.push_str(&line);
            text.push('\n');
        }
    }
    if items.is_empty() {
        return Err("the script is empty".into());
    }
    if text != source {
        let at = text
            .lines()
            .zip(source.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| text.lines().count().min(source.lines().count()))
            + 1;
        return Err(format!(
            "line {at}: the printer would not give this layout back"
        ));
    }
    Ok(format!(
        "[\n{}]",
        items
            .iter()
            .map(|item| format!("  {item}\n"))
            .collect::<String>()
    ))
}

fn shift(error: &str, by: usize) -> String {
    error
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(line, rest)| Some(format!("line {}:{rest}", line.parse::<usize>().ok()? + by)))
        .unwrap_or_else(|| error.to_owned())
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
}

struct Expression {
    nix: String,
    text: String,
    path: bool,
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(" "))
}

fn block(items: &[String]) -> String {
    format!(
        "[\n{}]",
        items
            .iter()
            .map(|item| format!("{item}\n"))
            .collect::<String>()
    )
}

impl Importer<'_> {
    fn at(&self, node: Node<'_>) -> String {
        format!("line {}", node.start_position().row + 1)
    }

    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.byte_range()]
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn unsupported(&self, node: Node<'_>) -> String {
        format!("{}: {} is not supported", self.at(node), node.kind())
    }

    fn sequence(
        &self,
        nodes: &[Node<'_>],
        mut row: Option<usize>,
        depth: usize,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let mut nix = Vec::new();
        let mut text = Vec::new();
        for node in nodes {
            if let Some(previous) = row {
                for _ in previous + 1..node.start_position().row {
                    nix.push("blank".to_owned());
                    text.push(String::new());
                }
            }
            let (item, lines) = self.statement(*node, depth)?;
            nix.push(item);
            text.extend(lines);
            row = Some(node.end_position().row);
        }
        Ok((nix, text))
    }

    fn statement(&self, node: Node<'_>, depth: usize) -> Result<(String, Vec<String>), String> {
        let tabs = "\t".repeat(depth);
        let line = |nix: String, text: String| Ok((nix, vec![format!("{tabs}{text}")]));
        let source = self.text(node);
        match node.kind() {
            "comment" => line(
                format!("(comment {})", self.string(source)),
                source.to_owned(),
            ),
            "option_directive" => {
                let rest = source.strip_prefix("Option ").unwrap_or(source);
                line(
                    format!("(option {})", self.string(rest)),
                    format!("Option {rest}"),
                )
            }
            "on_error_statement" => {
                let rest = source.strip_prefix("On Error ").unwrap_or(source);
                line(
                    format!("(onError {})", self.string(rest)),
                    format!("On Error {rest}"),
                )
            }
            "variable_declaration" => {
                let [names] = named(node)[..] else {
                    return Err(self.unsupported(node));
                };
                if names.kind() != "variable_list" {
                    return Err(self.unsupported(names));
                }
                let names = named(names)
                    .into_iter()
                    .map(|name| {
                        if name.kind() == "variable_declaration_identifier"
                            && name.named_child_count() == 1
                        {
                            Ok(self.text(name).to_owned())
                        } else {
                            Err(self.unsupported(name))
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let nix: Vec<_> = names.iter().map(|name| self.string(name)).collect();
                line(
                    format!("(dim {})", list(&nix)),
                    format!("Dim {}", names.join(", ")),
                )
            }
            "variable_assignment" => {
                let parts = children(node);
                let [target, _, value] = parts[..] else {
                    return Err(self.unsupported(node));
                };
                if target.kind() != "identifier" {
                    return Err(self.unsupported(target));
                }
                let value = self.expression(value)?;
                let name = self.text(target);
                line(
                    format!("(assign {} {})", self.string(name), value.nix),
                    format!("{name} = {}", value.text),
                )
            }
            "invocation_statement" => {
                let parts = named(node);
                let head = self.expression(parts[0])?;
                let arguments = match parts.get(1) {
                    Some(arguments) => self.arguments(*arguments)?,
                    None => Vec::new(),
                };
                if let Some(set) = self.set(&head, &parts)? {
                    return line(set.nix, set.text);
                }
                let mut nix = vec![head.nix];
                nix.extend(arguments.iter().map(|argument| argument.nix.clone()));
                let mut text = head.text;
                if !arguments.is_empty() {
                    text.push(' ');
                    text.push_str(
                        &arguments
                            .iter()
                            .map(|argument| argument.text.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                line(list(&nix), text)
            }
            "if_statement" => self.conditional(node, depth),
            _ => Err(self.unsupported(node)),
        }
    }

    fn set(&self, head: &Expression, parts: &[Node<'_>]) -> Result<Option<Expression>, String> {
        if head.text != "Set" {
            return Ok(None);
        }
        let assignment = parts
            .get(1)
            .and_then(|arguments| {
                (arguments.named_child_count() == 1)
                    .then(|| arguments.named_child(0))
                    .flatten()
            })
            .and_then(|argument| argument.named_child(0))
            .filter(|binary| binary.kind() == "binary_expression");
        let Some(binary) = assignment else {
            return Ok(None);
        };
        let parts = children(binary);
        match parts[..] {
            [target, operator, value]
                if target.kind() == "identifier" && operator.kind() == "=" =>
            {
                let value = self.expression(value)?;
                let name = self.text(target);
                Ok(Some(Expression {
                    nix: format!("(set {} {})", self.string(name), value.nix),
                    text: format!("Set {name} = {}", value.text),
                    path: false,
                }))
            }
            _ => Ok(None),
        }
    }

    fn conditional(&self, node: Node<'_>, depth: usize) -> Result<(String, Vec<String>), String> {
        let tabs = "\t".repeat(depth);
        let parts = named(node);
        let test = self.expression(parts[0])?;
        let mut body = Vec::new();
        let mut otherwise = None;
        for part in &parts[1..] {
            match part.kind() {
                "else_clause" => otherwise = Some(*part),
                "elseif_clause" => return Err(self.unsupported(*part)),
                _ => body.push(*part),
            }
        }
        let (body_nix, body_text) =
            self.sequence(&body, Some(parts[0].end_position().row), depth + 1)?;
        let mut text = vec![format!("{tabs}If {} Then", test.text)];
        text.extend(body_text);
        let nix = match otherwise {
            None => format!("(if' {} {})", test.nix, block(&body_nix)),
            Some(clause) => {
                let (else_nix, else_text) =
                    self.sequence(&named(clause), Some(clause.start_position().row), depth + 1)?;
                text.push(format!("{tabs}Else"));
                text.extend(else_text);
                format!(
                    "(ifElse {} {} {})",
                    test.nix,
                    block(&body_nix),
                    block(&else_nix)
                )
            }
        };
        text.push(format!("{tabs}End If"));
        Ok((nix, text))
    }

    fn arguments(&self, node: Node<'_>) -> Result<Vec<Expression>, String> {
        named(node)
            .into_iter()
            .map(|argument| match named(argument)[..] {
                [value] if argument.kind() == "argument" && value.kind() != "keyword_argument" => {
                    self.expression(value)
                }
                _ => Err(self.unsupported(argument)),
            })
            .collect()
    }

    fn leaf(&self, text: &str) -> Expression {
        Expression {
            nix: self.string(text),
            text: text.to_owned(),
            path: true,
        }
    }

    fn call(&self, callee: &str, node: Node<'_>) -> Result<Expression, String> {
        let parts = children(node);
        let arguments = match parts.iter().find(|part| part.kind() == "argument_list") {
            Some(list) => self.arguments(*list)?,
            None => Vec::new(),
        };
        let mut nix = vec![self.string(callee)];
        nix.extend(arguments.iter().map(|argument| argument.nix.clone()));
        let joined: Vec<_> = arguments
            .iter()
            .map(|argument| argument.text.as_str())
            .collect();
        Ok(Expression {
            nix: list(&nix),
            text: format!("{callee}({})", joined.join(", ")),
            path: false,
        })
    }

    fn expression(&self, node: Node<'_>) -> Result<Expression, String> {
        let source = self.text(node);
        match node.kind() {
            "identifier" | "number" | "boolean" => Ok(self.leaf(source)),
            "literal" => match named(node)[..] {
                [inner] => self.expression(inner),
                _ => Ok(self.leaf(source)),
            },
            "string_literal" => {
                let value = source[1..source.len() - 1].replace("\"\"", "\"");
                Ok(Expression {
                    nix: format!("(str {})", self.string(&value)),
                    text: format!("\"{}\"", value.replace('"', "\"\"")),
                    path: false,
                })
            }
            "member_expression" => {
                let parts = children(node);
                let [object, dot, property] = parts[..] else {
                    return Err(self.unsupported(node));
                };
                let object = self.expression(object)?;
                if !object.path || dot.kind() != "." {
                    return Err(self.unsupported(node));
                }
                match property.kind() {
                    "identifier" => {
                        Ok(self.leaf(&format!("{}.{}", object.text, self.text(property))))
                    }
                    "function_call" => self.function(property, &format!("{}.", object.text)),
                    _ => Err(self.unsupported(property)),
                }
            }
            "function_call" => self.function(node, ""),
            "binary_expression" => {
                let parts = children(node);
                let (Some(left), Some(right)) = (parts.first(), parts.last()) else {
                    return Err(self.unsupported(node));
                };
                let operator = self.source[left.end_byte()..right.start_byte()].trim();
                let left = self.expression(*left)?;
                let right = self.expression(*right)?;
                let operands = match left
                    .nix
                    .strip_prefix("(op [")
                    .and_then(|rest| rest.strip_suffix("])"))
                {
                    Some(inner) if !left.nix[5..].contains("(op [") => inner.to_owned(),
                    _ => left.nix,
                };
                Ok(Expression {
                    nix: format!("(op [{operands} {} {}])", self.string(operator), right.nix),
                    text: format!("{} {operator} {}", left.text, right.text),
                    path: false,
                })
            }
            "unary_expression" => {
                let Some(operand) = children(node).last().copied() else {
                    return Err(self.unsupported(node));
                };
                let operator = self.source[node.start_byte()..operand.start_byte()].trim();
                let operand = self.expression(operand)?;
                Ok(Expression {
                    nix: format!("(op [{} {}])", self.string(operator), operand.nix),
                    text: format!("{operator} {}", operand.text),
                    path: false,
                })
            }
            _ => Err(self.unsupported(node)),
        }
    }

    fn function(&self, node: Node<'_>, prefix: &str) -> Result<Expression, String> {
        let Some(name) = named(node)
            .into_iter()
            .find(|part| part.kind() == "identifier")
        else {
            return Err(self.unsupported(node));
        };
        if !self.text(node).starts_with(self.text(name)) {
            return Err(self.unsupported(node));
        }
        self.call(&format!("{prefix}{}", self.text(name)), node)
    }
}
