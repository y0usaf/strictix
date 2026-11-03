use tree_sitter::Node;

use crate::{line_of, nix};

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_bash::LANGUAGE.into(), source)?;
    let root = tree.root_node();
    let importer = Importer { source, holes };
    let statements = named(root);
    let first = statements.first().ok_or("the script is empty")?;
    if first.start_byte() != 0 {
        return Err("the script must start with its first statement".into());
    }
    let last = statements.last().ok_or("the script is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the script must end with one newline after its last statement".into());
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

fn spaces(gap: &str) -> Option<(usize, usize)> {
    let lines = gap.bytes().take_while(|&b| b == b'\n').count();
    let rest = &gap[lines..];
    (lines > 0 && rest.bytes().all(|b| b == b' ')).then_some((lines, rest.len()))
}

impl Importer<'_> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn gap(&self, from: usize, to: usize) -> &str {
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

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn lines(&self, statements: &[Node<'_>], indent: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut group: Vec<String> = Vec::new();
        for (index, statement) in statements.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(statements[index - 1].end_byte(), statement.start_byte());
                if gap == "; " {
                    group.push(self.statement(*statement, indent)?);
                    continue;
                }
                let (lines, pad) = spaces(gap).ok_or_else(|| {
                    format!(
                        "{}: statements separated by {gap:?}",
                        self.at(statement.start_byte())
                    )
                })?;
                if pad != indent {
                    return Err(format!(
                        "{}: indented {pad}, expected {indent}",
                        self.at(statement.start_byte())
                    ));
                }
                out.push(flush(&mut group));
                for _ in 1..lines {
                    out.push("blank".into());
                }
            }
            group.push(self.statement(*statement, indent)?);
        }
        out.push(flush(&mut group));
        Ok(out)
    }

    fn body(
        &self,
        statements: &[Node<'_>],
        open: usize,
        close: usize,
        indent: usize,
        trail: &str,
        close_pad: usize,
    ) -> Result<String, String> {
        let first = statements
            .first()
            .ok_or_else(|| format!("{}: an empty body", self.at(open)))?;
        let last = statements.last().unwrap_or(first);
        let lead = self.gap(open, first.start_byte());
        if !lead.contains('\n') {
            self.expect(open, first.start_byte(), " ")?;
            for pair in statements.windows(2) {
                self.expect(pair[0].end_byte(), pair[1].start_byte(), "; ")?;
            }
            self.expect(last.end_byte(), close, trail)?;
            let items = statements
                .iter()
                .map(|s| self.statement(*s, indent))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("(inline [{}])", items.join(" ")));
        }
        self.expect(
            open,
            first.start_byte(),
            &format!("\n{}", " ".repeat(indent + 2)),
        )?;
        let tail = format!("\n{}", " ".repeat(close_pad));
        let close_gap = self.gap(last.end_byte(), close);
        if close_gap != tail && close_gap != format!(";{tail}") {
            self.expect(last.end_byte(), close, &tail)?;
        }
        if close_gap != tail {
            return Err(format!(
                "{}: a block ends with a stray semicolon",
                self.at(close)
            ));
        }
        let lines = self.lines(statements, indent + 2)?;
        Ok(format!(
            "[\n{}]",
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        ))
    }

    fn statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let start = node.start_byte();
        match node.kind() {
            "comment" => Ok(format!(
                "(comment {})",
                self.string(node.utf8_text(self.source.as_bytes()).unwrap_or(""))
            )),
            "command" | "declaration_command" => Ok(format!(
                "[{}]",
                self.words(&children(node), start)?.join(" ")
            )),
            "test_command" => Ok(format!("[{}]", self.test(node)?.join(" "))),
            "variable_assignment" => self.assign(node),
            "redirected_statement" => self.redirected(node, indent),
            "pipeline" => self.joined(node, &["|"], "pipe", indent, Vec::new()),
            "list" => self.joined(node, &["&&", "||"], "chain", indent, Vec::new()),
            "function_definition" => self.function(node, indent),
            "compound_statement" => Ok(format!("(braces {})", self.braces(node, indent)?)),
            "if_statement" => self.conditional(node, indent),
            "for_statement" => self.for_loop(node, indent),
            "while_statement" => self.while_loop(node, indent),
            "case_statement" => self.case(node, indent),
            kind => Err(format!("{}: {kind} is not supported", self.at(start))),
        }
    }

    fn words(&self, nodes: &[Node<'_>], start: usize) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut previous: Option<usize> = None;
        for node in nodes {
            if node.kind() == "command_name" {
                if let Some(end) = previous {
                    self.expect(end, node.start_byte(), " ")?;
                }
                out.extend(self.words(&children(*node), node.start_byte())?);
                previous = Some(node.end_byte());
                continue;
            }
            if let Some(end) = previous {
                self.expect(end, node.start_byte(), " ")?;
            } else if node.start_byte() != start {
                return Err(format!("{}: a command starts with a gap", self.at(start)));
            }
            out.push(match node.kind() {
                "variable_assignment" => self.assign(*node)?,
                "file_redirect" | "herestring_redirect" => self.redirect(*node)?,
                "heredoc_redirect" => self.heredoc(*node)?,
                kind if !node.is_named()
                    && matches!(
                        kind,
                        "local" | "export" | "declare" | "readonly" | "typeset"
                    ) =>
                {
                    self.string(kind)
                }
                _ => self.word(*node)?,
            });
            previous = Some(node.end_byte());
        }
        Ok(out)
    }

    fn test(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let mut leaves = Vec::new();
        flatten(node, &mut leaves);
        let mut groups: Vec<Vec<Node<'_>>> = Vec::new();
        for leaf in leaves {
            match groups.last_mut() {
                Some(group) if group[group.len() - 1].end_byte() == leaf.start_byte() => {
                    group.push(leaf)
                }
                Some(group) => {
                    self.expect(group[group.len() - 1].end_byte(), leaf.start_byte(), " ")?;
                    groups.push(vec![leaf]);
                }
                None => groups.push(vec![leaf]),
            }
        }
        groups
            .iter()
            .map(|group| match group.as_slice() {
                [leaf] if leaf.is_named() => self.word(*leaf),
                _ => {
                    Ok(self
                        .string(self.gap(group[0].start_byte(), group[group.len() - 1].end_byte())))
                }
            })
            .collect()
    }

    fn word(&self, node: Node<'_>) -> Result<String, String> {
        let text = &self.source[node.start_byte()..node.end_byte()];
        match node.kind() {
            "string" => {
                let parts = children(node);
                let (open, close) = (parts[0].end_byte(), parts[parts.len() - 1].start_byte());
                let mut out = Vec::new();
                let mut from = open;
                for part in &parts[1..parts.len() - 1] {
                    if part.kind() == "command_substitution" {
                        if from < part.start_byte() {
                            out.push(self.string(&self.source[from..part.start_byte()]));
                        }
                        out.push(self.substitution(*part)?);
                        from = part.end_byte();
                    }
                }
                if from < close || out.is_empty() {
                    out.push(self.string(&self.source[from..close]));
                }
                Ok(if out.len() == 1 && !out[0].starts_with('(') {
                    format!("(dq {})", out[0])
                } else {
                    format!("(dq [{}])", out.join(" "))
                })
            }
            "raw_string" => Ok(format!("(sq {})", self.string(&text[1..text.len() - 1]))),
            "ansi_c_string" => Ok(format!("(ansic {})", self.string(&text[2..text.len() - 1]))),
            "command_substitution" => self.substitution(node),
            "concatenation" => {
                let parts = children(node);
                for pair in parts.windows(2) {
                    self.expect(pair[0].end_byte(), pair[1].start_byte(), "")?;
                }
                let parts = parts
                    .iter()
                    .map(|p| self.word(*p))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("(word [{}])", parts.join(" ")))
            }
            "array" => {
                let items = named(node);
                if let Some(first) = items.first() {
                    self.expect(node.start_byte() + 1, first.start_byte(), "")?;
                }
                for pair in items.windows(2) {
                    self.expect(pair[0].end_byte(), pair[1].start_byte(), " ")?;
                }
                if let Some(last) = items.last() {
                    self.expect(last.end_byte(), node.end_byte() - 1, "")?;
                }
                let items = items
                    .iter()
                    .map(|i| self.word(*i))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("(array [{}])", items.join(" ")))
            }
            _ if text.contains('`') => Err(format!(
                "{}: backquote substitution is not supported",
                self.at(node.start_byte())
            )),
            _ => Ok(self.string(text)),
        }
    }

    fn substitution(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        if parts[0].kind() != "$(" {
            return Err(format!(
                "{}: only $(...) substitution is supported",
                self.at(node.start_byte())
            ));
        }
        let statements = named(node);
        let first = statements
            .first()
            .ok_or_else(|| format!("{}: an empty substitution", self.at(node.start_byte())))?;
        self.expect(parts[0].end_byte(), first.start_byte(), "")?;
        for pair in statements.windows(2) {
            self.expect(pair[0].end_byte(), pair[1].start_byte(), "; ")?;
        }
        self.expect(
            statements[statements.len() - 1].end_byte(),
            node.end_byte() - 1,
            "",
        )?;
        let items = statements
            .iter()
            .map(|s| self.statement(*s, 0))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("(sub [{}])", items.join(" ")))
    }

    fn assign(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let name = parts.first().ok_or("an empty assignment")?;
        let equals = parts.get(1).filter(|p| p.kind() == "=").ok_or_else(|| {
            format!(
                "{}: only = assignments are supported",
                self.at(node.start_byte())
            )
        })?;
        self.expect(name.end_byte(), equals.start_byte(), "")?;
        let name = self.string(name.utf8_text(self.source.as_bytes()).unwrap_or(""));
        let value = match parts.get(2) {
            Some(value) => {
                self.expect(equals.end_byte(), value.start_byte(), "")?;
                self.word(*value)?
            }
            None => self.string(""),
        };
        Ok(format!("(assign {name} {value})"))
    }

    fn redirect(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let target = parts.last().ok_or("an empty redirect")?;
        let operator = &self.source[node.start_byte()..parts[parts.len() - 2].end_byte()];
        let gap = self.gap(parts[parts.len() - 2].end_byte(), target.start_byte());
        let quoted = matches!(target.kind(), "string" | "raw_string" | "ansi_c_string");
        if gap != if quoted { " " } else { "" } {
            return Err(format!(
                "{}: a redirect spaced {gap:?} where the printer writes {:?}",
                self.at(node.start_byte()),
                if quoted { " " } else { "" }
            ));
        }
        Ok(format!(
            "(redir {} {})",
            self.string(operator),
            self.word(*target)?
        ))
    }

    fn heredoc(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let kinds: Vec<&str> = parts.iter().map(|p| p.kind()).collect();
        if kinds.first() != Some(&"<<")
            || kinds.get(1) != Some(&"heredoc_start")
            || kinds.last() != Some(&"heredoc_end")
        {
            return Err(format!(
                "{}: only a plain <<DELIMITER heredoc is supported",
                self.at(node.start_byte())
            ));
        }
        self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
        let start = parts[1].utf8_text(self.source.as_bytes()).unwrap_or("");
        let delimiter = start.trim_matches(['\'', '"']);
        let end = parts[parts.len() - 1];
        if end.utf8_text(self.source.as_bytes()).unwrap_or("") != delimiter {
            return Err(format!(
                "{}: the heredoc ends with a different delimiter",
                self.at(end.start_byte())
            ));
        }
        if self.gap(parts[1].end_byte(), parts[1].end_byte() + 1) != "\n" {
            return Err(format!(
                "{}: a heredoc must end its line",
                self.at(parts[1].start_byte())
            ));
        }
        let body = self.gap(parts[1].end_byte() + 1, end.start_byte());
        if !body.is_empty() && !body.ends_with('\n') {
            return Err(format!(
                "{}: a heredoc body must end with a newline",
                self.at(end.start_byte())
            ));
        }
        Ok(format!(
            "(heredoc {} {})",
            self.string(start),
            self.string(body)
        ))
    }

    fn redirected(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let command = parts[0];
        let mut redirects = Vec::new();
        let mut previous = command.end_byte();
        for redirect in &parts[1..] {
            self.expect(previous, redirect.start_byte(), " ")?;
            redirects.push(match redirect.kind() {
                "heredoc_redirect" => self.heredoc(*redirect)?,
                _ => self.redirect(*redirect)?,
            });
            previous = redirect.end_byte();
        }
        match command.kind() {
            "command" => {
                let mut words = self.words(&children(command), command.start_byte())?;
                words.extend(redirects);
                Ok(format!("[{}]", words.join(" ")))
            }
            "pipeline" => self.joined(command, &["|"], "pipe", indent, redirects),
            kind => Err(format!(
                "{}: redirecting a {kind} is not supported",
                self.at(node.start_byte())
            )),
        }
    }

    fn joined(
        &self,
        node: Node<'_>,
        operators: &[&str],
        name: &str,
        indent: usize,
        redirects: Vec<String>,
    ) -> Result<String, String> {
        let mut items: Vec<Node<'_>> = Vec::new();
        operands(node, node.kind(), &mut items);
        let mut out = Vec::new();
        let mut broken: Option<bool> = None;
        for (index, item) in items.iter().enumerate() {
            if item.is_named() {
                out.push(self.statement(*item, indent)?);
                continue;
            }
            if !operators.contains(&item.kind()) {
                return Err(format!(
                    "{}: {} is not supported",
                    self.at(item.start_byte()),
                    item.kind()
                ));
            }
            self.expect(items[index - 1].end_byte(), item.start_byte(), " ")?;
            let after = self.gap(item.end_byte(), items[index + 1].start_byte());
            let breaks = after == format!("\n{}", " ".repeat(indent + 2));
            if !breaks && after != " " {
                return Err(format!(
                    "{}: {:?} after {}",
                    self.at(item.end_byte()),
                    after,
                    item.kind()
                ));
            }
            if broken.is_some_and(|b| b != breaks) {
                return Err(format!(
                    "{}: a {name} breaks after some operators only",
                    self.at(item.start_byte())
                ));
            }
            broken = Some(breaks);
            if name == "chain" {
                out.push(self.string(item.kind()));
            }
        }
        if !redirects.is_empty() {
            let last = out
                .pop()
                .filter(|stage| stage.starts_with('[') && stage.ends_with(']'));
            let last = last.ok_or_else(|| {
                format!(
                    "{}: a redirect after a pipeline whose last stage is not a command",
                    self.at(node.end_byte())
                )
            })?;
            out.push(format!(
                "{} {}]",
                &last[..last.len() - 1],
                redirects.join(" ")
            ));
        }
        let list = format!("[{}]", out.join(" "));
        Ok(if broken == Some(true) {
            format!("({name} (rows {list}))")
        } else {
            format!("({name} {list})")
        })
    }

    fn function(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let kinds: Vec<&str> = parts.iter().map(|p| p.kind()).collect();
        if kinds.len() != 4
            || kinds[1] != "("
            || kinds[2] != ")"
            || kinds[3] != "compound_statement"
        {
            return Err(format!(
                "{}: only name() {{ ... }} functions are supported",
                self.at(node.start_byte())
            ));
        }
        self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
        self.expect(parts[2].end_byte(), parts[3].start_byte(), " ")?;
        let name = self.string(parts[0].utf8_text(self.source.as_bytes()).unwrap_or(""));
        Ok(format!("(fn {name} {})", self.braces(parts[3], indent)?))
    }

    fn braces(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let open = token(node, "{").ok_or("a brace group without {")?;
        let close = token(node, "}").ok_or("a brace group without }")?;
        self.body(
            &named(node),
            open.end_byte(),
            close.start_byte(),
            indent,
            "; ",
            indent,
        )
    }

    fn conditional(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let then = parts
            .iter()
            .position(|p| p.kind() == "then")
            .ok_or("if without then")?;
        let fi = parts
            .iter()
            .position(|p| p.kind() == "fi")
            .ok_or("if without fi")?;
        let condition: Vec<_> = parts[1..then]
            .iter()
            .filter(|p| p.is_named())
            .copied()
            .collect();
        let [test] = condition.as_slice() else {
            return Err(format!(
                "{}: an if needs exactly one condition statement",
                self.at(node.start_byte())
            ));
        };
        self.expect(parts[0].end_byte(), test.start_byte(), " ")?;
        self.expect(test.end_byte(), parts[then].start_byte(), "; ")?;
        let alternative = parts[then..fi]
            .iter()
            .position(|p| matches!(p.kind(), "else_clause" | "elif_clause"))
            .map(|i| i + then);
        let body_end = alternative.unwrap_or(fi);
        let body: Vec<_> = parts[then + 1..body_end]
            .iter()
            .filter(|p| p.is_named())
            .copied()
            .collect();
        let close = parts[body_end].start_byte();
        let test = self.statement(*test, indent)?;
        let body = self.body(&body, parts[then].end_byte(), close, indent, "; ", indent)?;
        match alternative {
            None => Ok(format!("(if' {test} {body})")),
            Some(index) if parts[index].kind() == "else_clause" => {
                let clause = parts[index];
                let keyword = token(clause, "else").ok_or("else without else")?;
                let otherwise = self.body(
                    &named(clause),
                    keyword.end_byte(),
                    parts[fi].start_byte(),
                    indent,
                    "; ",
                    indent,
                )?;
                Ok(format!("(ifElse {test} {body} {otherwise})"))
            }
            Some(index) => Err(format!(
                "{}: elif is not supported",
                self.at(parts[index].start_byte())
            )),
        }
    }

    fn do_group(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let open = token(node, "do").ok_or("a loop without do")?;
        let close = token(node, "done").ok_or("a loop without done")?;
        self.body(
            &named(node),
            open.end_byte(),
            close.start_byte(),
            indent,
            "; ",
            indent,
        )
    }

    fn for_loop(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let kinds: Vec<&str> = parts.iter().map(|p| p.kind()).collect();
        let body = parts
            .last()
            .filter(|p| p.kind() == "do_group")
            .ok_or("for without do")?;
        if kinds.get(2) != Some(&"in") {
            return Err(format!(
                "{}: only for NAME in WORDS loops are supported",
                self.at(node.start_byte())
            ));
        }
        self.expect(parts[0].end_byte(), parts[1].start_byte(), " ")?;
        self.expect(parts[1].end_byte(), parts[2].start_byte(), " ")?;
        let words: Vec<_> = parts[3..parts.len() - 1]
            .iter()
            .filter(|p| p.kind() != ";")
            .copied()
            .collect();
        let mut previous = parts[2].end_byte();
        for word in &words {
            self.expect(previous, word.start_byte(), " ")?;
            previous = word.end_byte();
        }
        self.expect(previous, body.start_byte(), "; ")?;
        let name = self.string(parts[1].utf8_text(self.source.as_bytes()).unwrap_or(""));
        let words = words
            .iter()
            .map(|w| self.word(*w))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!(
            "(for {name} [{}] {})",
            words.join(" "),
            self.do_group(*body, indent)?
        ))
    }

    fn while_loop(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let body = parts
            .last()
            .filter(|p| p.kind() == "do_group")
            .ok_or("while without do")?;
        let condition: Vec<_> = parts[1..parts.len() - 1]
            .iter()
            .filter(|p| p.is_named())
            .copied()
            .collect();
        let [test] = condition.as_slice() else {
            return Err(format!(
                "{}: a while needs exactly one condition statement",
                self.at(node.start_byte())
            ));
        };
        self.expect(parts[0].end_byte(), test.start_byte(), " ")?;
        self.expect(test.end_byte(), body.start_byte(), "; ")?;
        Ok(format!(
            "(while {} {})",
            self.statement(*test, indent)?,
            self.do_group(*body, indent)?
        ))
    }

    fn case(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let subject = parts.get(1).ok_or("case without a word")?;
        let keyword = parts
            .get(2)
            .filter(|p| p.kind() == "in")
            .ok_or("case without in")?;
        let esac = parts
            .last()
            .filter(|p| p.kind() == "esac")
            .ok_or("case without esac")?;
        self.expect(parts[0].end_byte(), subject.start_byte(), " ")?;
        self.expect(subject.end_byte(), keyword.start_byte(), " ")?;
        let arms: Vec<_> = parts[3..parts.len() - 1].to_vec();
        if arms.iter().any(|a| a.kind() != "case_item") {
            return Err(format!(
                "{}: unexpected case content",
                self.at(node.start_byte())
            ));
        }
        let inline = !self
            .gap(
                keyword.end_byte(),
                arms.first().map_or(esac.start_byte(), |a| a.start_byte()),
            )
            .contains('\n');
        let (lead, between, trail) = if inline {
            (" ".to_owned(), " ".to_owned(), " ".to_owned())
        } else {
            (
                format!("\n{}", " ".repeat(indent + 2)),
                format!("\n{}", " ".repeat(indent + 2)),
                format!("\n{}", " ".repeat(indent)),
            )
        };
        let mut previous = keyword.end_byte();
        let mut rendered = Vec::new();
        for (index, arm) in arms.iter().enumerate() {
            self.expect(
                previous,
                arm.start_byte(),
                if index == 0 { &lead } else { &between },
            )?;
            rendered.push(self.arm(*arm, indent + 2)?);
            previous = arm.end_byte();
        }
        self.expect(previous, esac.start_byte(), &trail)?;
        let arms = format!("[{}]", rendered.join(" "));
        let subject = self.word(*subject)?;
        Ok(if inline {
            format!("(case {subject} (inline {arms}))")
        } else {
            format!("(case {subject} {arms})")
        })
    }

    fn arm(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let close = parts
            .iter()
            .position(|p| p.kind() == ")")
            .ok_or("a case arm without )")?;
        let end = parts.last().filter(|p| p.kind() == ";;").ok_or_else(|| {
            format!(
                "{}: a case arm must end with ;;",
                self.at(node.start_byte())
            )
        })?;
        let patterns: Vec<_> = parts[..close]
            .iter()
            .filter(|p| p.kind() != "|")
            .copied()
            .collect();
        for (index, pair) in parts[..close].windows(2).enumerate() {
            let _ = index;
            self.expect(pair[0].end_byte(), pair[1].start_byte(), "")?;
        }
        self.expect(
            patterns[patterns.len() - 1].end_byte(),
            parts[close].start_byte(),
            "",
        )?;
        let statements: Vec<_> = parts[close + 1..parts.len() - 1]
            .iter()
            .filter(|p| p.is_named())
            .copied()
            .collect();
        let body = self.body(
            &statements,
            parts[close].end_byte(),
            end.start_byte(),
            indent,
            " ",
            indent + 2,
        )?;
        let patterns = patterns
            .iter()
            .map(|p| self.word(*p))
            .collect::<Result<Vec<_>, _>>()?;
        let patterns = if patterns.len() == 1 {
            patterns[0].clone()
        } else {
            format!("[{}]", patterns.join(" "))
        };
        Ok(format!("(arm {patterns} {body})"))
    }
}

fn flush(group: &mut Vec<String>) -> String {
    let items = std::mem::take(group);
    if items.len() == 1 {
        items.into_iter().next().unwrap_or_default()
    } else {
        format!("(inline [{}])", items.join(" "))
    }
}

fn operands<'t>(node: Node<'t>, kind: &str, out: &mut Vec<Node<'t>>) {
    for child in children(node) {
        if child.kind() == kind {
            operands(child, kind, out);
        } else {
            out.push(child);
        }
    }
}

fn flatten<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    for child in children(node) {
        if matches!(
            child.kind(),
            "binary_expression" | "unary_expression" | "parenthesized_expression"
        ) {
            flatten(child, out);
        } else {
            out.push(child);
        }
    }
}
