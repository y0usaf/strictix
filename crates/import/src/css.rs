enum Node {
    Rule {
        select: String,
        set: Vec<(String, String)>,
    },
    Block {
        at: String,
        rules: Vec<Node>,
    },
    Statement {
        at: String,
    },
}

impl Node {
    fn render(&self) -> String {
        match self {
            Self::Rule { select, set } => {
                let declarations: String = set
                    .iter()
                    .map(|(property, value)| format!("  {property}: {value};\n"))
                    .collect();
                format!("{select} {{\n{declarations}}}")
            }
            Self::Block { at, rules } => format!("{at} {{\n{}\n}}", indent(&render_nodes(rules))),
            Self::Statement { at } => format!("{at};"),
        }
    }
}

fn render_nodes(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(Node::render)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn indent(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn import(source: &str) -> Result<String, String> {
    let mut parser = Parser {
        lines: source.lines().collect(),
        at: 0,
    };
    let nodes = parser.nodes("", None)?;
    if parser.at < parser.lines.len() {
        return Err(parser.unexpected("content after the last rule"));
    }
    let printed = format!("{}\n", render_nodes(&nodes));
    if printed != source {
        return Err(format!(
            "line {}: the CSS generator would print this file differently",
            first_difference(&printed, source)
        ));
    }
    Ok(literal(&nodes))
}

fn first_difference(printed: &str, source: &str) -> usize {
    printed
        .lines()
        .zip(source.lines())
        .position(|(a, b)| a != b)
        .map_or(1, |line| line + 1)
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    at: usize,
}

impl Parser<'_> {
    fn unexpected(&self, what: &str) -> String {
        format!(
            "line {}: {what}: `{}`",
            self.at + 1,
            self.lines.get(self.at).copied().unwrap_or("")
        )
    }

        fn nodes(&mut self, margin: &str, closing: Option<&str>) -> Result<Vec<Node>, String> {
        let own = format!("{margin}}}");
        let mut nodes = Vec::new();
        while let Some(line) = self.lines.get(self.at).copied() {
            if Some(line) == closing {
                break;
            }
            if line.is_empty() {
                self.at += 1;
                continue;
            }
            let Some(head) = line.strip_prefix(margin) else {
                return Err(self.unexpected("this line is not indented where the printer puts it"));
            };
            if let Some(at) = head.strip_suffix(" {") {
                if at.starts_with('@') {
                    self.at += 1;
                    let inner = format!("{margin}  ");
                    let rules = self.nodes(&inner, Some(&own))?;
                    if self.lines.get(self.at).copied() != Some(own.as_str()) {
                        return Err(self.unexpected("a block at-rule must close with `}`"));
                    }
                    self.at += 1;
                    nodes.push(Node::Block {
                        at: at.to_owned(),
                        rules,
                    });
                } else {
                    self.at += 1;
                    let set = self.declarations(&format!("{margin}  "), &own)?;
                    nodes.push(Node::Rule {
                        select: at.to_owned(),
                        set,
                    });
                }
                continue;
            }
            if let Some(at) = head.strip_suffix(';') {
                if !at.starts_with('@') {
                    return Err(self.unexpected("a statement must be an at-rule"));
                }
                self.at += 1;
                nodes.push(Node::Statement { at: at.to_owned() });
                continue;
            }
            return Err(self.unexpected("a node is a rule, a block at-rule or a statement at-rule"));
        }
        Ok(nodes)
    }

        fn declarations(
        &mut self,
        margin: &str,
        closing: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let mut set = Vec::new();
        loop {
            let Some(line) = self.lines.get(self.at).copied() else {
                return Err(self.unexpected("the rule never closes"));
            };
            if line == closing {
                self.at += 1;
                return Ok(set);
            }
            let Some(body) = line.strip_prefix(margin) else {
                return Err(self.unexpected("a declaration sits two columns into its rule"));
            };
            let mut declaration = body.to_owned();
            self.at += 1;
            while !declaration.ends_with(';') {
                let Some(next) = self.lines.get(self.at).copied() else {
                    return Err(self.unexpected("a declaration never ends"));
                };
                declaration.push('\n');
                declaration.push_str(next);
                self.at += 1;
            }
            declaration.pop();
            let Some((property, value)) = declaration.split_once(": ") else {
                return Err(self.unexpected("a declaration is `property: value`"));
            };
            if property.trim() != property || property.is_empty() {
                return Err(self.unexpected("a declaration's property has stray spaces"));
            }
            set.push((property.to_owned(), value.to_owned()));
        }
    }
}

fn literal(nodes: &[Node]) -> String {
    if nodes.is_empty() {
        return "[ ]".to_owned();
    }
    let mut lines = vec!["[".to_owned()];
    for node in nodes {
        lines.extend(node_lines(node).iter().map(|line| format!("  {line}")));
    }
    lines.push("]".to_owned());
    lines.join("\n")
}

fn node_lines(node: &Node) -> Vec<String> {
    match node {
        Node::Rule { select, set } => {
            let mut out = vec!["{".to_owned(), format!("  select = {};", quote(select))];
            if set.is_empty() {
                out.push("  set = [ ];".to_owned());
            } else {
                out.push("  set = [".to_owned());
                for (property, value) in set {
                    out.push(format!("    [{} {}]", quote(property), quote(value)));
                }
                out.push("  ];".to_owned());
            }
            out.push("}".to_owned());
            out
        }
        Node::Block { at, rules } => {
            let mut out = vec!["{".to_owned(), format!("  at = {};", quote(at))];
            if rules.is_empty() {
                out.push("  rules = [ ];".to_owned());
            } else {
                out.push("  rules = [".to_owned());
                for rule in rules {
                    out.extend(node_lines(rule).iter().map(|line| format!("    {line}")));
                }
                out.push("  ];".to_owned());
            }
            out.push("}".to_owned());
            out
        }
        Node::Statement { at } => vec![
            "{".to_owned(),
            format!("  at = {};", quote(at)),
            "}".to_owned(),
        ],
    }
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),

            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
