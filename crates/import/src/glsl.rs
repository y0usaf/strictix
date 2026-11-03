use crate::{column_of, line_of, nix};

type Node<'t> = tree_sitter::Node<'t>;

const CONSTRUCTORS: [&str; 5] = ["bool", "double", "float", "int", "uint"];

const ASSIGNMENTS: [&str; 11] = [
    "=", "+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|=",
];

const ATOMS: [&str; 8] = [
    "identifier",
    "number_literal",
    "primitive_type",
    "type_identifier",
    "field_identifier",
    "true",
    "false",
    "type_qualifier",
];

const CHAINS: [&str; 6] = [
    "binary_expression",
    "assignment_expression",
    "conditional_expression",
    "comma_expression",
    "init_declarator",
    "parameter_declaration",
];

const SUFFIXES: [&str; 3] = ["field_expression", "unary_expression", "update_expression"];

enum Chain {
    Row(Vec<String>),
    Rows(String),
}

struct Emitter<'s> {
    source: &'s str,
    holes: &'s [String],
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let masked = mask(source);
    let tree = crate::parse(&tree_sitter_glsl::LANGUAGE_GLSL.into(), &masked)?;
    let emitter = Emitter { source, holes };
    let mut out = String::from("[\n");
    let mut previous = 0;
    for node in children(tree.root_node()) {
        let gap = &source[previous..node.start_byte()];
        if previous == 0 {
            if !gap.is_empty() {
                return Err(format!("line 1: the source starts with {gap:?}"));
            }
        } else {
            let lines = newlines(gap, 0).ok_or_else(|| {
                format!(
                    "line {}: two top-level items share a line or are separated by {gap:?}",
                    emitter.line(node)
                )
            })?;
            for _ in 1..lines {
                out.push_str("  blank\n");
            }
        }
        out.push_str("  ");
        out.push_str(&emitter.statement(node, 0, 2)?);
        out.push('\n');
        previous = emitter.end(node);
    }
    let tail = &source[previous..];
    if tail != "\n" {
        return Err(format!(
            "the source must end with one newline after its last item, not {tail:?}"
        ));
    }
    out.push(']');
    Ok(out)
}

fn mask(source: &str) -> String {
    let mut out = source.as_bytes().to_vec();
    for name in CONSTRUCTORS {
        for (at, _) in source.match_indices(name) {
            let after = &source[at + name.len()..];
            let joined = source[..at]
                .bytes()
                .last()
                .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
            if after.starts_with('(') && !joined {
                out[at] = b'_';
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| source.to_owned())
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn newlines(gap: &str, indent: usize) -> Option<usize> {
    let lines = gap.bytes().take_while(|&b| b == b'\n').count();
    let rest = &gap[lines..];
    (lines > 0 && rest.len() == indent && rest.bytes().all(|b| b == b' ')).then_some(lines)
}

fn flatten<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    if CHAINS.contains(&node.kind()) {
        for child in children(node) {
            flatten(child, out);
        }
    } else {
        out.push(node);
    }
}

fn atomic(node: Node<'_>) -> bool {
    if ATOMS.contains(&node.kind()) || !node.is_named() {
        return true;
    }
    let parts = children(node);
    SUFFIXES.contains(&node.kind())
        && parts.iter().all(|part| atomic(*part))
        && parts
            .windows(2)
            .all(|pair| pair[0].end_byte() == pair[1].start_byte())
}

impl Emitter<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.byte_range()]
    }

    fn line(&self, node: Node<'_>) -> usize {
        line_of(self.source, node.start_byte())
    }

    fn column(&self, node: Node<'_>) -> usize {
        column_of(self.source, node.start_byte())
    }

    fn end(&self, node: Node<'_>) -> usize {
        node.start_byte() + self.text(node).trim_end_matches('\n').len()
    }

    fn gap(&self, before: Node<'_>, after: Node<'_>) -> &str {
        &self.source[self.end(before)..after.start_byte()]
    }

    fn expect(&self, before: Node<'_>, after: Node<'_>, gap: &str) -> Result<(), String> {
        let actual = self.gap(before, after);
        if actual == gap {
            Ok(())
        } else {
            Err(format!(
                "line {}: expected {gap:?} before `{}`, not {actual:?}",
                self.line(after),
                self.text(after)
            ))
        }
    }

    fn string(&self, text: &str, indent: usize) -> String {
        nix::string(text, self.holes, indent)
    }

    fn atom(&self, node: Node<'_>) -> String {
        let text = self.text(node);
        nix::int(text).unwrap_or_else(|| self.string(text, 0))
    }

    fn statement(&self, node: Node<'_>, margin: usize, indent: usize) -> Result<String, String> {
        let parts = children(node);
        match node.kind() {
            "comment" => {
                let text = self.text(node).trim_end_matches('\n');
                if text.contains('\n') {
                    return Err(format!(
                        "line {}: a comment over several lines",
                        self.line(node)
                    ));
                }
                Ok(format!("(comment {})", self.string(text, indent)))
            }
            "declaration"
            | "expression_statement"
            | "return_statement"
            | "continue_statement"
            | "break_statement" => {
                let (last, rest) = parts.split_last().ok_or("an empty statement")?;
                let previous = rest.last().ok_or("a statement without an expression")?;
                if last.kind() != ";" {
                    return Err(format!("line {}: a statement without `;`", self.line(node)));
                }
                self.expect(*previous, *last, "")?;
                self.expression(rest, margin, indent)
            }
            "function_definition" => {
                let (body, signature) = parts.split_last().ok_or("an empty function")?;
                self.expect(signature[signature.len() - 1], *body, " ")?;
                Ok(format!(
                    "(fn {} {})",
                    self.expression(signature, margin, indent)?,
                    self.block(*body, margin, indent)?
                ))
            }
            "if_statement" => self.conditional(&parts, margin, indent),
            "for_statement" => self.for_loop(&parts, margin, indent),
            "while_statement" => {
                let [keyword, test, body] = parts.as_slice() else {
                    return Err(format!("line {}: an unexpected while", self.line(node)));
                };
                self.expect(*keyword, *test, " ")?;
                self.expect(*test, *body, " ")?;
                Ok(format!(
                    "(while {} {})",
                    self.parenthesized(*test, margin, indent)?,
                    self.body(*body, margin, indent)?
                ))
            }
            "preproc_def" | "preproc_function_def" => self.define(&parts, indent),
            kind => Err(format!(
                "line {}: no GLSL statement for {kind}",
                self.line(node)
            )),
        }
    }

    fn conditional(
        &self,
        parts: &[Node<'_>],
        margin: usize,
        indent: usize,
    ) -> Result<String, String> {
        let [keyword, test, body, rest @ ..] = parts else {
            return Err("an if without a body".into());
        };
        self.expect(*keyword, *test, " ")?;
        self.expect(*test, *body, " ")?;
        let test = self.parenthesized(*test, margin, indent)?;
        let then = self.body(*body, margin, indent)?;
        let Some(clause) = rest.first() else {
            return Ok(format!("(if' {test} {then})"));
        };
        let braced = body.kind() == "compound_statement";
        let before = if braced {
            " ".to_owned()
        } else {
            format!("\n{}", " ".repeat(margin))
        };
        self.expect(*body, *clause, &before)?;
        let [word, other] = children(*clause)[..] else {
            return Err(format!("line {}: an unexpected else", self.line(*clause)));
        };
        self.expect(word, other, " ")?;
        let otherwise = if other.kind() == "if_statement" {
            self.statement(other, margin, indent)?
        } else {
            self.body(other, margin, indent)?
        };
        Ok(format!("(ifElse {test} {then} {otherwise})"))
    }

    fn for_loop(&self, parts: &[Node<'_>], margin: usize, indent: usize) -> Result<String, String> {
        let [keyword, open, header @ .., close, body] = parts else {
            return Err("an unexpected for".into());
        };
        self.expect(*keyword, *open, " ")?;
        self.expect(*close, *body, " ")?;
        let mut tokens = Vec::new();
        for part in header {
            if part.kind() == "declaration" {
                tokens.extend(children(*part));
            } else {
                tokens.push(*part);
            }
        }
        let segments: Vec<&[Node<'_>]> = tokens.split(|token| token.kind() == ";").collect();
        if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
            return Err(format!(
                "line {}: a for header needs all three clauses",
                self.line(*keyword)
            ));
        }
        let mut previous = *open;
        let mut clauses = Vec::new();
        for (index, segment) in segments.iter().enumerate() {
            self.expect(previous, segment[0], if index == 0 { "" } else { " " })?;
            let last = segment[segment.len() - 1];
            let next = tokens
                .iter()
                .find(|token| token.start_byte() >= last.end_byte())
                .copied()
                .unwrap_or(*close);
            self.expect(last, next, "")?;
            previous = next;
            clauses.push(self.expression(segment, margin, indent)?);
        }
        Ok(format!(
            "(for {} {})",
            clauses.join(" "),
            self.body(*body, margin, indent)?
        ))
    }

    fn define(&self, parts: &[Node<'_>], indent: usize) -> Result<String, String> {
        let (keyword, name, params, value) = match parts {
            [keyword, name, value] => (*keyword, *name, None, *value),
            [keyword, name, params, value] => (*keyword, *name, Some(*params), *value),
            _ => return Err("a #define needs a name and a value".into()),
        };
        if self.text(keyword) != "#define" || value.kind() != "preproc_arg" {
            return Err(format!(
                "line {}: only #define NAME VALUE is supported",
                self.line(keyword)
            ));
        }
        self.expect(keyword, name, " ")?;
        let head = match params {
            None => self.atom(name),
            Some(params) => {
                self.expect(name, params, "")?;
                self.expect(params, value, " ")?;
                format!(
                    "(call {} {})",
                    self.atom(name),
                    self.arguments(params, 0, indent)?
                )
            }
        };
        if params.is_none() {
            self.expect(name, value, " ")?;
        }
        let text = self.text(value).trim_end_matches('\n');
        if text.contains('\n') || text.ends_with([' ', '\t']) {
            return Err(format!(
                "line {}: a macro over several lines",
                self.line(value)
            ));
        }
        let wrapper = format!("int _ = {text};\n");
        let tree = crate::parse(&tree_sitter_glsl::LANGUAGE_GLSL.into(), &mask(&wrapper)).map_err(
            |e| {
                format!(
                    "line {}: the macro body is not an expression: {e}",
                    self.line(value)
                )
            },
        )?;
        let declaration = tree.root_node().child(0).ok_or("an empty macro")?;
        let init = declaration
            .child_by_field_name("declarator")
            .and_then(|declarator| declarator.child_by_field_name("value"))
            .ok_or_else(|| {
                format!(
                    "line {}: the macro body is not an expression",
                    self.line(value)
                )
            })?;
        let inner = Emitter {
            source: &wrapper,
            holes: self.holes,
        };
        Ok(format!(
            "(define {head} {})",
            inner.expression(&[init], 0, indent)?
        ))
    }

    fn body(&self, node: Node<'_>, margin: usize, indent: usize) -> Result<String, String> {
        if node.kind() == "compound_statement" {
            self.block(node, margin, indent)
        } else {
            Ok(format!("(bare {})", self.statement(node, margin, indent)?))
        }
    }

    fn block(&self, node: Node<'_>, margin: usize, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let [open, statements @ .., close] = parts.as_slice() else {
            return Err(format!("line {}: an unexpected block", self.line(node)));
        };
        if open.kind() != "{" || close.kind() != "}" {
            return Err(format!("line {}: an unexpected block", self.line(node)));
        }
        let broken = self
            .gap(*open, *statements.first().unwrap_or(close))
            .starts_with('\n');
        if !broken {
            if statements.is_empty() {
                return Err(format!("line {}: an empty one-line block", self.line(node)));
            }
            let mut previous = *open;
            let mut out = Vec::new();
            for statement in statements {
                self.expect(previous, *statement, " ")?;
                out.push(self.statement(*statement, margin, indent)?);
                previous = *statement;
            }
            self.expect(previous, *close, " ")?;
            return Ok(format!("(inline [{}])", out.join(" ")));
        }
        let pad = " ".repeat(indent + 2);
        let mut out = String::from("[\n");
        let mut previous = *open;
        for statement in statements {
            let gap = self.gap(previous, *statement);
            let lines = newlines(gap, margin + 4).ok_or_else(|| {
                format!(
                    "line {}: statements share a line or are separated by {gap:?}",
                    self.line(*statement)
                )
            })?;
            for _ in 1..lines {
                out.push_str(&pad);
                out.push_str("blank\n");
            }
            out.push_str(&pad);
            out.push_str(&self.statement(*statement, margin + 4, indent + 2)?);
            out.push('\n');
            previous = *statement;
        }
        self.expect(previous, *close, &format!("\n{}", " ".repeat(margin)))?;
        out.push_str(&" ".repeat(indent));
        out.push(']');
        Ok(out)
    }

    fn parenthesized(
        &self,
        node: Node<'_>,
        margin: usize,
        indent: usize,
    ) -> Result<String, String> {
        let [open, inner, close] = children(node)[..] else {
            return Err(format!(
                "line {}: an unexpected parenthesis",
                self.line(node)
            ));
        };
        self.expect(open, inner, "")?;
        self.expect(inner, close, "")?;
        self.expression(&[inner], margin, indent)
    }

    fn expression(
        &self,
        nodes: &[Node<'_>],
        margin: usize,
        indent: usize,
    ) -> Result<String, String> {
        Ok(match self.chain(nodes, margin, indent)? {
            Chain::Rows(rows) => rows,
            Chain::Row(items) if items.len() == 1 && !items[0].starts_with('[') => items[0].clone(),
            Chain::Row(items) => format!("[{}]", items.join(" ")),
        })
    }

    fn chain(&self, nodes: &[Node<'_>], margin: usize, indent: usize) -> Result<Chain, String> {
        let mut leaves = Vec::new();
        for node in nodes {
            flatten(*node, &mut leaves);
        }
        let mut starts = vec![0];
        for index in 1..leaves.len() {
            let gap = self.gap(leaves[index - 1], leaves[index]);
            let comma = self.text(leaves[index]) == ",";
            if (comma && gap.is_empty()) || (!comma && gap == " ") {
                continue;
            }
            if comma || newlines(gap, gap.len().saturating_sub(1)) != Some(1) {
                return Err(format!(
                    "line {}: `{}` follows {gap:?}",
                    self.line(leaves[index]),
                    self.text(leaves[index])
                ));
            }
            starts.push(index);
        }
        if starts.len() == 1 {
            return Ok(Chain::Row(
                leaves
                    .iter()
                    .map(|leaf| self.item(*leaf, margin, indent))
                    .collect::<Result<_, _>>()?,
            ));
        }
        let start = self.column(leaves[0]);
        let first = &leaves[..starts[1]];
        let default = first
            .iter()
            .position(|leaf| !leaf.is_named() && ASSIGNMENTS.contains(&self.text(*leaf)))
            .filter(|&at| first[..at].iter().all(|leaf| atomic(*leaf)))
            .map(|at| self.column(first[at]) - start);
        let pad = " ".repeat(indent + 2);
        let mut out = String::from("(rows [\n");
        starts.push(leaves.len());
        for (row, bounds) in starts.windows(2).enumerate() {
            let row_leaves = &leaves[bounds[0]..bounds[1]];
            let column = self.column(row_leaves[0]);
            if column < start {
                return Err(format!(
                    "line {}: a row starts left of its expression",
                    self.line(row_leaves[0])
                ));
            }
            let by = column - start;
            let items = row_leaves
                .iter()
                .map(|leaf| self.item(*leaf, start + by, indent + 2))
                .collect::<Result<Vec<_>, _>>()?;
            let body = format!("[{}]", items.join(" "));
            out.push_str(&pad);
            if row > 0 && default != Some(by) {
                out.push_str(&format!("(at {by} {body})"));
            } else {
                out.push_str(&body);
            }
            out.push('\n');
        }
        out.push_str(&" ".repeat(indent));
        out.push_str("])");
        Ok(Chain::Rows(out))
    }

    fn item(&self, node: Node<'_>, margin: usize, indent: usize) -> Result<String, String> {
        if atomic(node) {
            return Ok(self.atom(node));
        }
        let parts = children(node);
        match (node.kind(), parts.as_slice()) {
            ("parenthesized_expression", [open, inner, close]) => {
                self.expect(*open, *inner, "")?;
                self.expect(*inner, *close, "")?;
                Ok(match self.chain(&[*inner], margin, indent)? {
                    Chain::Row(items) => format!("[{}]", items.join(" ")),
                    Chain::Rows(rows) => format!("[{rows}]"),
                })
            }
            ("call_expression" | "function_declarator", [function, arguments]) => {
                self.expect(*function, *arguments, "")?;
                Ok(format!(
                    "(call {} {})",
                    self.item(*function, margin, indent)?,
                    self.arguments(*arguments, margin, indent)?
                ))
            }
            ("subscript_expression" | "array_declarator", [base, open, at, close]) => {
                self.expect(*base, *open, "")?;
                self.expect(*open, *at, "")?;
                self.expect(*at, *close, "")?;
                Ok(format!(
                    "(index {} {})",
                    self.item(*base, margin, indent)?,
                    self.expression(&[*at], margin, indent)?
                ))
            }
            ("field_expression", [base, dot, name]) => {
                self.expect(*base, *dot, "")?;
                self.expect(*dot, *name, "")?;
                Ok(format!(
                    "(field {} {})",
                    self.item(*base, margin, indent)?,
                    self.atom(*name)
                ))
            }
            ("unary_expression", [operator, operand]) => {
                self.expect(*operator, *operand, "")?;
                Ok(format!(
                    "(unary {} {})",
                    self.atom(*operator),
                    self.item(*operand, margin, indent)?
                ))
            }
            (kind, _) => Err(format!(
                "line {}: no GLSL expression for {kind}",
                self.line(node)
            )),
        }
    }

    fn arguments(&self, node: Node<'_>, margin: usize, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let [open, inner @ .., close] = parts.as_slice() else {
            return Err(format!(
                "line {}: an unexpected argument list",
                self.line(node)
            ));
        };
        if inner.is_empty() {
            self.expect(*open, *close, "")?;
            return Ok("[]".into());
        }
        let values: Vec<Node<'_>> = inner.iter().step_by(2).copied().collect();
        for comma in inner.iter().skip(1).step_by(2) {
            if comma.kind() != "," {
                return Err(format!(
                    "line {}: arguments without a comma",
                    self.line(*comma)
                ));
            }
        }
        if inner.len() % 2 == 0 {
            return Err(format!("line {}: a trailing comma", self.line(*close)));
        }
        let block = self.gap(*open, values[0]).starts_with('\n');
        let column = if block {
            margin + 4
        } else {
            self.column(*open) + 1
        };
        let mut rows: Vec<Vec<Node<'_>>> = vec![Vec::new()];
        let mut previous = *open;
        for (index, value) in values.iter().enumerate() {
            if index > 0 {
                self.expect(previous, inner[2 * index - 1], "")?;
                previous = inner[2 * index - 1];
            }
            let gap = self.gap(previous, *value);
            let same = if index == 0 { "" } else { " " };
            if (block && index == 0) || gap != same {
                if newlines(gap, column) != Some(1) {
                    return Err(format!(
                        "line {}: an argument follows {gap:?}",
                        self.line(*value)
                    ));
                }
                rows.push(Vec::new());
            }
            if let Some(row) = rows.last_mut() {
                row.push(*value);
            }
            previous = *value;
        }
        let closing = if block {
            format!("\n{}", " ".repeat(margin))
        } else {
            String::new()
        };
        self.expect(previous, *close, &closing)?;
        let broken = rows.len() > 1;
        let inner_margin = if broken { column } else { margin };
        let row = |values: &[Node<'_>], indent: usize| -> Result<String, String> {
            let values = values
                .iter()
                .map(|value| self.expression(&[*value], inner_margin, indent))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", values.join(" ")))
        };
        if !broken {
            return row(&rows[0], indent);
        }
        let pad = " ".repeat(indent + 2);
        let mut out = String::from("(rows [\n");
        for values in &rows {
            out.push_str(&pad);
            out.push_str(&row(values, indent + 2)?);
            out.push('\n');
        }
        out.push_str(&" ".repeat(indent));
        out.push_str("])");
        Ok(out)
    }
}
