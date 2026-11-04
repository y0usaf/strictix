use std::cell::RefCell;
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::{line_of, nix};

const NIX_KEYWORDS: [&str; 13] = [
    "assert", "else", "false", "if", "in", "inherit", "let", "null", "or", "rec", "then", "true",
    "with",
];

const CONSTRUCTORS: [&str; 37] = [
    "assign",
    "blank",
    "cast",
    "comment",
    "dot",
    "field",
    "fn",
    "for",
    "forRange",
    "hex",
    "idx",
    "if'",
    "ifElse",
    "ifExpr",
    "infix",
    "len",
    "local",
    "localFunction",
    "long",
    "neg",
    "nil",
    "not",
    "opt",
    "pair",
    "paren",
    "render",
    "return",
    "rows",
    "str",
    "table",
    "trailing",
    "tuple",
    "type",
    "typed",
    "unicode",
    "update",
    "while",
];

const GLOBALS: [&str; 30] = [
    "AgentBlock",
    "AgentFinishedEvent",
    "AgentStateEvent",
    "CommandFinishedEvent",
    "FocusEvent",
    "PaneInfo",
    "PickItem",
    "ProcessResult",
    "StatusPaneInfo",
    "StatusSegment",
    "TabEvent",
    "TabInfo",
    "WindowCx",
    "any",
    "boolean",
    "ipairs",
    "math",
    "number",
    "os",
    "pairs",
    "pcall",
    "proproblems",
    "require",
    "string",
    "table",
    "tern",
    "tonumber",
    "tostring",
    "type",
    "utf8",
];

fn precedence(operator: &str) -> Option<u8> {
    Some(match operator {
        "or" => 1,
        "and" => 2,
        "<" | ">" | "<=" | ">=" | "~=" | "==" => 3,
        ".." => 4,
        "+" | "-" => 5,
        "*" | "/" | "//" | "%" => 6,
        "^" => 8,
        _ => return None,
    })
}

const UNARY: u8 = 7;

fn right_associative(operator: &str) -> bool {
    matches!(operator, ".." | "^")
}

fn spelled(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_luau::LANGUAGE.into(), source)?;
    let importer = Importer {
        source,
        holes,
        scopes: RefCell::new(Vec::new()),
        globals: RefCell::new(BTreeSet::new()),
    };
    let nodes = named(tree.root_node());
    let body = match nodes.first() {
        None if source.bytes().all(|b| b == b'\n') => {
            format!("[{}]", vec!["blank"; source.len()].join(" "))
        }
        None => return Err("the file holds no statement".into()),
        Some(first) if first.start_byte() != 0 => {
            return Err("the file must start with its first statement".into())
        }
        Some(_) => {
            let (formals, entries) = importer.scoped(&nodes, Vec::new(), 0)?;
            let text: String = entries
                .iter()
                .map(|entry| format!("{}\n", entry.text))
                .collect();
            if text != source {
                return Err(mismatch(source, &text));
            }
            let list: String = entries
                .iter()
                .map(|entry| format!("  {}\n", entry.nix))
                .collect();
            lambda(&formals, format!("[\n{list}]"))
        }
    };
    let mut formals: BTreeSet<String> = nix::identifiers(&body)
        .into_iter()
        .filter(|name| CONSTRUCTORS.contains(&name.as_str()))
        .collect();
    formals.extend(importer.globals.into_inner());
    let formals: String = formals.iter().map(|name| format!("  {name},\n")).collect();
    Ok(format!("{{\n{formals}  ...\n}}: {body}"))
}

fn lambda(formals: &[String], list: String) -> String {
    if formals.is_empty() {
        list
    } else {
        let names: Vec<String> = formals.iter().map(|name| spelled(name)).collect();
        format!("({{{}}}: {list})", names.join(", "))
    }
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

struct Scope {
    declared: BTreeSet<String>,
    visible: BTreeSet<String>,
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    scopes: RefCell<Vec<Scope>>,
    globals: RefCell<BTreeSet<String>>,
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

fn float(text: &str) -> bool {
    let Some((whole, fraction)) = text.split_once('.') else {
        return false;
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    digits(whole)
        && digits(fraction)
        && (whole == "0" || !whole.starts_with('0'))
        && !fraction.ends_with('0')
        && whole.len() + fraction.len() <= 15
}

fn prefix_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "function_call"
            | "dot_index_expression"
            | "method_index_expression"
            | "bracket_index_expression"
            | "parenthesized_expression"
    )
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

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!("{}: {what} ({})", self.at(node), node.kind()))
    }

    fn broken(&self, from: usize, to: usize) -> bool {
        self.source[from..to].contains('\n')
    }

    fn name(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if node.kind() != "identifier"
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return self.refuse(node, "a name Nix cannot spell");
        }
        Ok(name.to_owned())
    }

    fn reference(&self, node: Node<'_>) -> Result<Out, String> {
        let name = self.name(node)?;
        let scopes = self.scopes.borrow();
        let luau = scopes.iter().rposition(|s| s.visible.contains(&name));
        let bound = scopes.iter().rposition(|s| s.declared.contains(&name));
        if luau != bound {
            return self.refuse(
                node,
                &format!(
                    "`{name}` is used before the local of its block that Nix would bind it to"
                ),
            );
        }
        if luau.is_none() {
            if !GLOBALS.contains(&name.as_str()) {
                return self.refuse(
                    node,
                    &format!("`{name}` is neither declared nor a global of generators.luau"),
                );
            }
            self.globals.borrow_mut().insert(spelled(&name));
        }
        Ok(Out::new(spelled(&name), name))
    }

    fn reveal(&self, names: &[String]) {
        if let Some(scope) = self.scopes.borrow_mut().last_mut() {
            scope.visible.extend(names.iter().cloned());
        }
    }

    fn declared(&self, node: Node<'_>) -> Result<(Vec<String>, bool), String> {
        match node.kind() {
            "variable_declaration" => {
                let parts = named(node);
                let Some(assignment) = parts.first() else {
                    return Ok((Vec::new(), false));
                };
                let Some(list) = named(*assignment).into_iter().next() else {
                    return Ok((Vec::new(), false));
                };
                let mut out = Vec::new();
                let mut typed = false;
                for child in children(list) {
                    match child.kind() {
                        ":" => typed = true,
                        "," => {}
                        _ if typed => typed = false,
                        _ => out.push(self.name(child)?),
                    }
                }
                Ok((out, false))
            }
            "function_declaration" if self.token(node, "local").is_some() => {
                Ok((vec![self.name(self.field(node, "name")?)?], false))
            }
            "type_definition" => match named(node).first() {
                Some(name) => Ok((vec![self.name(*name)?], true)),
                None => Ok((Vec::new(), false)),
            },
            _ => Ok((Vec::new(), false)),
        }
    }

    fn scoped(
        &self,
        nodes: &[Node<'_>],
        given: Vec<String>,
        depth: usize,
    ) -> Result<(Vec<String>, Vec<Out>), String> {
        let mut order = given.clone();
        let mut visible: BTreeSet<String> = given.into_iter().collect();
        for node in nodes {
            let (declared, hoisted) = self.declared(*node)?;
            for name in declared {
                if hoisted {
                    visible.insert(name.clone());
                }
                if !order.contains(&name) {
                    order.push(name);
                }
            }
        }
        self.scopes.borrow_mut().push(Scope {
            declared: order.iter().cloned().collect(),
            visible,
        });
        let entries = self.entries(nodes, depth);
        self.scopes.borrow_mut().pop();
        Ok((order, entries?))
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

    fn block(&self, nodes: &[Node<'_>], depth: usize, given: Vec<String>) -> Result<Out, String> {
        let (formals, entries) = self.scoped(nodes, given, depth + 1)?;
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
            lambda(&formals, format!("[\n{nix}]")),
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
            "function_call" => self.expression(node, depth),
            "return_statement" => match named(node).as_slice() {
                [] => Ok(Out::new("return".into(), "return".into())),
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
                let body = self.loop_body(node, depth, Vec::new())?;
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
                if children(node).first().map(|c| c.kind()) != Some("type") {
                    return self.refuse(node, "a type definition the printer does not write");
                }
                let value = self.type_(*value, depth)?;
                let name = self.name(*name)?;
                Ok(Out::new(
                    format!("(type {} {})", spelled(&name), value.nix),
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

    fn targets(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let mut outs: Vec<Out> = Vec::new();
        for child in children(node) {
            match child.kind() {
                "," => {}
                _ if child.is_named() => outs.push(self.expression(child, depth)?),
                _ => return self.refuse(child, "an assignment target the printer does not write"),
            }
        }
        Ok(names(outs))
    }

    fn locals(&self, node: Node<'_>, depth: usize) -> Result<(Out, Vec<String>), String> {
        let mut outs: Vec<Out> = Vec::new();
        let mut declared = Vec::new();
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
                _ => {
                    let name = self.name(child)?;
                    outs.push(Out::new(spelled(&name), name.clone()));
                    declared.push(name);
                }
            }
        }
        Ok((names(outs), declared))
    }

    fn assignment(&self, node: Node<'_>, depth: usize) -> Result<(Out, Out), String> {
        let parts = named(node);
        let [targets, values] = parts.as_slice() else {
            return self.refuse(node, "an assignment the printer does not write");
        };
        Ok((self.targets(*targets, depth)?, self.list(*values, depth)?))
    }

    fn local(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let parts = named(node);
        let [assignment] = parts.as_slice() else {
            return self.refuse(node, "a local the printer does not write");
        };
        if assignment.kind() != "assignment_statement" {
            return self.refuse(node, "a local without a value");
        }
        let parts = named(*assignment);
        let [targets, values] = parts.as_slice() else {
            return self.refuse(node, "a local the printer does not write");
        };
        let values = self.list(*values, depth)?;
        let (targets, declared) = self.locals(*targets, depth)?;
        self.reveal(&declared);
        Ok(Out::new(
            format!("(local {} {})", targets.nix, values.nix),
            format!("local {} = {}", targets.text, values.text),
        ))
    }

    fn bound(
        &self,
        node: Node<'_>,
        kind: Option<Node<'_>>,
        depth: usize,
    ) -> Result<(Out, String), String> {
        let name = self.name(node)?;
        let (nix, text) = match kind {
            None => ("null".to_owned(), name.clone()),
            Some(kind) => {
                let kind = self.type_(kind, depth)?;
                (kind.nix, format!("{name}: {}", kind.text))
            }
        };
        Ok((
            Out::new(format!("{{{} = {nix};}}", spelled(&name)), text),
            name,
        ))
    }

    fn signature(&self, node: Node<'_>, depth: usize) -> Result<(Out, Vec<String>), String> {
        if named(node).iter().any(|c| c.kind() == "generic_type_list") {
            return self.refuse(node, "generic parameters the printer does not write");
        }
        let parameters = self.field(node, "parameters")?;
        let mut outs = Vec::new();
        let mut given = Vec::new();
        for parameter in named(parameters) {
            let parts = named(parameter);
            let (out, name) = match parts.as_slice() {
                [name] => self.bound(*name, None, depth)?,
                [name, kind] => self.bound(*name, Some(*kind), depth)?,
                _ => return self.refuse(parameter, "a parameter the printer does not write"),
            };
            outs.push(out);
            given.push(name);
        }
        let joined = join(&outs, " ", ", ");
        let params = Out::new(format!("[{}]", joined.nix), format!("({})", joined.text));
        let returns = children(node)
            .into_iter()
            .skip_while(|c| c.id() != parameters.id())
            .skip(1)
            .find(|c| c.is_named() && c.kind() != "block" && c.kind() != "comment");
        match returns {
            None => Ok((params, given)),
            Some(kind) => {
                let kind = self.type_(kind, depth)?;
                Ok((
                    Out::new(
                        format!("(typed {} {})", params.nix, kind.nix),
                        format!("{}: {}", params.text, kind.text),
                    ),
                    given,
                ))
            }
        }
    }

    fn function(&self, node: Node<'_>, depth: usize) -> Result<(Out, Out), String> {
        let (signature, given) = self.signature(node, depth)?;
        let parameters = self.field(node, "parameters")?;
        let end = self.token(node, "end").ok_or("a function without end")?;
        let nodes = self.between(node, parameters.end_byte(), end.start_byte());
        if nodes.is_empty() {
            let formals: Vec<String> = given;
            return Ok((
                signature,
                Out::new(lambda(&formals, "[]".into()), " ".into()),
            ));
        }
        Ok((signature, self.block(&nodes, depth, given)?))
    }

    fn declaration(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        if self.token(node, "local").is_none() {
            return self.refuse(node, "a global function the printer does not write");
        }
        let name = self.name(self.field(node, "name")?)?;
        self.reveal(std::slice::from_ref(&name));
        let (signature, body) = self.function(node, depth)?;
        Ok(Out::new(
            format!(
                "(localFunction {} {} {})",
                spelled(&name),
                signature.nix,
                body.nix
            ),
            format!("local function {name}{}{}end", signature.text, body.text),
        ))
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
        let body = self.block(&self.between(node, then.end_byte(), end), depth, Vec::new())?;
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
                let otherwise =
                    self.block(&self.region(node, &alternatives, 0), depth, Vec::new())?;
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
        let body = self.block(
            &self.region(node, alternatives, then.end_byte()),
            depth,
            Vec::new(),
        )?;
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
                let otherwise = self.block(&self.region(node, rest, 0), depth, Vec::new())?;
                (
                    format!("(ifElse {} {} {})", test.nix, body.nix, otherwise.nix),
                    format!("else{}end", otherwise.text),
                )
            }
            _ => return self.refuse(node, "an if the printer does not write"),
        };
        Ok(Out::new(nix, format!("{head}{tail}")))
    }

    fn loop_body(&self, node: Node<'_>, depth: usize, given: Vec<String>) -> Result<Out, String> {
        let open = self.token(node, "do").ok_or("a loop without do")?;
        let end = self.token(node, "end").ok_or("a loop without end")?;
        self.block(
            &self.between(node, open.end_byte(), end.start_byte()),
            depth,
            given,
        )
    }

    fn for_statement(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let clause = self.field(node, "clause")?;
        if clause.kind() == "for_numeric_clause" {
            let (name, given) = self.bound(self.field(clause, "name")?, None, depth)?;
            let mut bounds = vec![
                self.expression(self.field(clause, "start")?, depth)?,
                self.expression(self.field(clause, "end")?, depth)?,
            ];
            if let Some(step) = clause.child_by_field_name("step") {
                bounds.push(self.expression(step, depth)?);
            }
            let bounds = many(bounds);
            let body = self.loop_body(node, depth, vec![given])?;
            return Ok(Out::new(
                format!("(forRange {} {} {})", name.nix, bounds.nix, body.nix),
                format!("for {} = {} do{}end", name.text, bounds.text, body.text),
            ));
        }
        let parts = named(clause);
        let [targets, values] = parts.as_slice() else {
            return self.refuse(clause, "a for clause the printer does not write");
        };
        let values = self.list(*values, depth)?;
        let mut outs = Vec::new();
        let mut given = Vec::new();
        for target in named(*targets) {
            let (out, name) = self.bound(target, None, depth)?;
            outs.push(out);
            given.push(name);
        }
        let joined = join(&outs, " ", ", ");
        let body = self.loop_body(node, depth, given)?;
        Ok(Out::new(
            format!("(for [{}] {} {})", joined.nix, values.nix, body.nix),
            format!("for {} in {} do{}end", joined.text, values.text, body.text),
        ))
    }

    fn prefix(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        if node.kind() != "parenthesized_expression" {
            return self.expression(node, depth);
        }
        let parts = named(node);
        let [inner] = parts.as_slice() else {
            return self.refuse(node, "parentheses the printer does not write");
        };
        if prefix_kind(inner.kind()) {
            return self.refuse(node, "parentheses StyLua removes");
        }
        let inner = self.expression(*inner, depth)?;
        Ok(Out::new(inner.nix, format!("({})", inner.text)))
    }

    fn index(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let mark = if node.kind() == "method_index_expression" {
            ":"
        } else {
            "."
        };
        let table = self.field(node, "table")?;
        let op = self.token(node, mark).ok_or("an index without its mark")?;
        if mark == ":" && self.broken(table.end_byte(), op.start_byte()) {
            let base = self.prefix(table, depth)?;
            let name = self.text(self.field(node, "method")?);
            return Ok(Out::new(
                format!(
                    "(rows (dot {} {}))",
                    base.nix,
                    self.string(&format!(":{name}"))
                ),
                format!("{}{}:{name}", base.text, newline(depth + 1)),
            ));
        }
        let mut segments = Vec::new();
        let mut base = node;
        loop {
            let (field, mark) = match base.kind() {
                "dot_index_expression" => ("field", "."),
                "method_index_expression" if base.id() == node.id() => ("method", ":"),
                _ => break,
            };
            let name = self.field(base, field)?;
            segments.push(format!("{mark}{}", self.text(name)));
            base = self.field(base, "table")?;
        }
        segments.reverse();
        let path = segments.concat();
        if path.contains('\n') || self.broken(base.end_byte(), node.end_byte()) {
            return self.refuse(node, "a field path broken over lines");
        }
        let base = self.prefix(base, depth)?;
        let spelled_path = path.strip_prefix('.').unwrap_or(&path);
        Ok(Out::new(
            format!("(dot {} {})", base.nix, self.string(spelled_path)),
            format!("{}{path}", base.text),
        ))
    }

    fn expression(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        match node.kind() {
            "identifier" => self.reference(node),
            "number" => self.number(node, false),
            "nil" => Ok(Out::new("nil".into(), "nil".into())),
            "true" | "false" => {
                let word = self.text(node);
                Ok(Out::new(word.to_owned(), word.to_owned()))
            }
            "dot_index_expression" | "method_index_expression" => self.index(node, depth),
            "bracket_index_expression" => {
                let table = self.prefix(self.field(node, "table")?, depth)?;
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
            "unary_expression" => self.unary(node, depth),
            "parenthesized_expression" => {
                let parts = named(node);
                match parts.as_slice() {
                    [inner] if inner.kind() == "function_call" => {
                        let inner = self.call(*inner, depth)?;
                        Ok(Out::new(
                            format!("(paren {})", inner.nix),
                            format!("({})", inner.text),
                        ))
                    }
                    _ => self.refuse(node, "parentheses the printer does not write"),
                }
            }
            "function_definition" => {
                let (signature, body) = self.function(node, depth)?;
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

    fn number(&self, node: Node<'_>, negative: bool) -> Result<Out, String> {
        let text = self.text(node);
        let sign = if negative { "-" } else { "" };
        if let Some(int) = nix::int(&format!("{sign}{text}")) {
            return Ok(Out::new(int, format!("{sign}{text}")));
        }
        if float(text) {
            let nix = if negative {
                format!("(-{text})")
            } else {
                text.to_owned()
            };
            return Ok(Out::new(nix, format!("{sign}{text}")));
        }
        if let Some(digits) = text.strip_prefix("0x") {
            if let Ok(value) = u64::from_str_radix(digits, 16) {
                if format!("{value:X}") == digits && value < 1 << 62 {
                    let hex = format!("(hex {value})");
                    let nix = if negative {
                        format!("(neg {hex})")
                    } else {
                        hex
                    };
                    return Ok(Out::new(nix, format!("{sign}{text}")));
                }
            }
        }
        self.refuse(node, "a number spelling the printer does not write")
    }

    fn unary(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let op = children(node)
            .first()
            .map(|c| self.text(*c).to_owned())
            .unwrap_or_default();
        let operand = self.field(node, "operand")?;
        if op == "-" && operand.kind() == "number" {
            return self.number(operand, true);
        }
        let constructor = match op.as_str() {
            "not" => "not",
            "-" => "neg",
            "#" => "len",
            _ => return self.refuse(node, "a unary operator the printer does not write"),
        };
        let value = if operand.kind() == "parenthesized_expression" {
            let parts = named(operand);
            let inner = match parts.as_slice() {
                [inner]
                    if matches!(inner.kind(), "if_expression" | "cast_expression")
                        || inner.kind() == "binary_expression"
                            && precedence(&self.operator(*inner)).is_some_and(|s| s < UNARY) =>
                {
                    self.expression(*inner, depth)?
                }
                _ => return self.refuse(operand, "parentheses the printer does not write"),
            };
            Out::new(inner.nix, format!("({})", inner.text))
        } else if matches!(operand.kind(), "if_expression" | "cast_expression") {
            return self.refuse(operand, "an if or cast operand without parentheses");
        } else {
            self.expression(operand, depth)?
        };
        let space = if op == "not" { " " } else { "" };
        Ok(Out::new(
            format!("({constructor} {})", value.nix),
            format!("{op}{space}{}", value.text),
        ))
    }

    fn call(&self, node: Node<'_>, depth: usize) -> Result<Out, String> {
        let callee_node = self.field(node, "name")?;
        let callee = self.prefix(callee_node, depth)?;
        let arguments = self.field(node, "arguments")?;
        let open = children(arguments).into_iter().next();
        if open.map(|o| o.kind()) != Some("(") {
            return self.refuse(node, "a call without parentheses");
        }
        let inner = if callee.nix.starts_with("(rows (dot") {
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
            "identifier" => self.reference(node),
            "builtin_type" if self.text(node) == "nil" => Ok(Out::new("nil".into(), "nil".into())),
            "builtin_type" => {
                let name = self.text(node).to_owned();
                if !GLOBALS.contains(&name.as_str()) {
                    return self.refuse(
                        node,
                        &format!("`{name}` is not a global of generators.luau"),
                    );
                }
                self.globals.borrow_mut().insert(spelled(&name));
                Ok(Out::new(spelled(&name), name))
            }
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
        let mut leaves = Vec::new();
        let mut operators = Vec::new();
        self.flatten(node, &mut leaves, &mut operators);
        let broken = leaves
            .windows(2)
            .any(|pair| self.broken(pair[0].0.end_byte(), pair[1].0.start_byte()));
        let inner = if broken { depth + 1 } else { depth };
        let outs = leaves
            .iter()
            .map(|(leaf, operator, left)| self.operand(*leaf, operator, *left, inner))
            .collect::<Result<Vec<_>, _>>()?;
        let mut nix = outs[0].nix.clone();
        for (operator, out) in operators.iter().zip(&outs[1..]) {
            nix.push_str(&format!(" {} {}", self.string(operator), out.nix));
        }
        if broken {
            if operators.iter().any(|operator| operator != &operators[0]) {
                return self.refuse(
                    node,
                    "an infix expression of several operators broken over lines",
                );
            }
            let rest: String = operators
                .iter()
                .zip(&outs[1..])
                .map(|(operator, out)| format!("{}{operator} {}", newline(inner), out.text))
                .collect();
            return Ok(Out::new(
                format!("(rows (infix [{nix}]))"),
                format!("{}{rest}", outs[0].text),
            ));
        }
        let mut text = outs[0].text.clone();
        for (operator, out) in operators.iter().zip(&outs[1..]) {
            text.push_str(&format!(" {operator} {}", out.text));
        }
        Ok(Out::new(format!("(infix [{nix}])"), text))
    }

    fn operator(&self, node: Node<'_>) -> String {
        children(node)
            .into_iter()
            .find(|c| !c.is_named() || c.kind() == "and" || c.kind() == "or")
            .map(|c| self.text(c).to_owned())
            .unwrap_or_default()
    }

    fn flatten<'t>(
        &self,
        node: Node<'t>,
        leaves: &mut Vec<(Node<'t>, String, bool)>,
        operators: &mut Vec<String>,
    ) {
        let operator = self.operator(node);
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            leaves.push((node, operator, true));
            return;
        };
        for (child, is_left) in [(left, true), (right, false)] {
            if child.kind() == "binary_expression" {
                self.flatten(child, leaves, operators);
            } else {
                leaves.push((child, operator.clone(), is_left));
            }
            if is_left {
                operators.push(operator.clone());
            }
        }
    }

    fn operand(
        &self,
        node: Node<'_>,
        operator: &str,
        left: bool,
        depth: usize,
    ) -> Result<Out, String> {
        if precedence(operator).is_none() {
            return self.refuse(node, &format!("`{operator}` is not a binary operator"));
        }
        if matches!(node.kind(), "if_expression" | "cast_expression") {
            return self.refuse(node, "an if or cast operand without parentheses");
        }
        if node.kind() != "parenthesized_expression" {
            return self.expression(node, depth);
        }
        let parts = named(node);
        let [inner] = parts.as_slice() else {
            return self.refuse(node, "parentheses the printer does not write");
        };
        match inner.kind() {
            "binary_expression" => {
                let level = precedence(&self.operator(*inner)).unwrap_or(0);
                let tight = precedence(operator).unwrap_or(0);
                let needed = level < tight || level == tight && left == right_associative(operator);
                let group = self.binary(*inner, depth)?;
                let nix = if needed {
                    group.nix
                } else {
                    format!("(paren {})", group.nix)
                };
                Ok(Out::new(nix, format!("({})", group.text)))
            }
            "if_expression" | "cast_expression" => {
                let inner = self.expression(*inner, depth)?;
                Ok(Out::new(inner.nix, format!("({})", inner.text)))
            }
            _ => self.expression(node, depth),
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
