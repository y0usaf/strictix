use std::cell::RefCell;
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::{line_of, nix};

const NIX_KEYWORDS: [&str; 13] = [
    "assert", "else", "false", "if", "in", "inherit", "let", "null", "or", "rec", "then", "true",
    "with",
];

const CONSTRUCTORS: [&str; 34] = [
    "apply",
    "assign",
    "blank",
    "bnot",
    "break",
    "comment",
    "do",
    "dot",
    "field",
    "fn",
    "forIn",
    "forNum",
    "if'",
    "ifElse",
    "index",
    "inline",
    "key",
    "len",
    "local",
    "localFunction",
    "method",
    "neg",
    "nil",
    "not",
    "paren",
    "render",
    "return",
    "rows",
    "semicolon",
    "sq",
    "str",
    "table",
    "vararg",
    "while",
];

const GLOBALS: [&str; 44] = [
    "CS",
    "Vector3",
    "_G",
    "_VERSION",
    "assert",
    "bit",
    "collectgarbage",
    "coroutine",
    "debug",
    "dofile",
    "error",
    "getfenv",
    "getmetatable",
    "hl",
    "io",
    "ipairs",
    "jit",
    "load",
    "loadfile",
    "loadstring",
    "math",
    "next",
    "os",
    "package",
    "pairs",
    "pcall",
    "pg",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "require",
    "select",
    "setfenv",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "unpack",
    "utf8",
    "xpcall",
];

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_lua::LANGUAGE.into(), source)?;
    let root = tree.root_node();
    let importer = Importer {
        source,
        holes,
        scopes: RefCell::new(Vec::new()),
        used: RefCell::new(BTreeSet::new()),
    };
    let statements = named(root);
    let first = statements.first().ok_or("the chunk is empty")?;
    if first.start_byte() != 0 {
        return Err("the chunk must start with its first statement".into());
    }
    let last = statements.last().ok_or("the chunk is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the chunk must end with one newline after its last statement".into());
    }
    let names = importer.declared(&statements)?;
    importer.enter(names.clone(), Vec::new());
    let lines = importer.lines(&statements, 0)?;
    importer.leave();
    let list = format!(
        "[\n{}]",
        lines
            .iter()
            .map(|line| format!("  {line}\n"))
            .collect::<String>()
    );
    let top = bound(&names, list);
    let used = importer.used.borrow();
    if used.is_empty() {
        return Ok(format!("_: {top}"));
    }
    let formals: String = used.iter().map(|name| format!("  {name},\n")).collect();
    Ok(format!("{{\n{formals}  ...\n}}: {top}"))
}

#[derive(Clone, Copy, PartialEq)]
enum Place {
    Free,
    Operand,
    Power,
    Unary,
    Prefix,
}

struct Scope {
    all: Vec<String>,
    seen: Vec<String>,
}

struct Body {
    inline: bool,
    items: Vec<String>,
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    scopes: RefCell<Vec<Scope>>,
    used: RefCell<BTreeSet<String>>,
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

fn pad(indent: usize) -> String {
    format!("\n{}", " ".repeat(indent))
}

fn reserved(name: &str) -> bool {
    NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name)
}

fn global(name: &str) -> String {
    if reserved(name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn local(name: &str) -> String {
    if reserved(name) || GLOBALS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn bound(names: &[String], value: String) -> String {
    if names.is_empty() {
        return value;
    }
    let formals: Vec<String> = names.iter().map(|name| local(name)).collect();
    format!("({{{}}}: {value})", formals.join(", "))
}

fn merge(names: &mut Vec<String>, more: Vec<String>) {
    for name in more {
        if !names.contains(&name) {
            names.push(name);
        }
    }
}

fn prefix_kind(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "dot_index_expression"
            | "bracket_index_expression"
            | "method_index_expression"
            | "function_call"
            | "parenthesized_expression"
    )
}

fn float(text: &str) -> Option<String> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = digits.split_once('.')?;
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !plain(whole) || !plain(fraction) {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    let size = value.abs();
    if size != 0.0 && !(1e-4..1e15).contains(&size) {
        return None;
    }
    let shown = format!("{value}");
    let shown = if shown.contains('.') {
        shown
    } else {
        format!("{shown}.0")
    };
    (shown == text).then(|| {
        if text.starts_with('-') {
            format!("({text})")
        } else {
            text.to_owned()
        }
    })
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

    fn uses(&self, name: &str) -> String {
        self.used.borrow_mut().insert(name.to_owned());
        name.to_owned()
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

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!(
            "{}: {what}: `{}`",
            self.at(node.start_byte()),
            self.text(node)
        ))
    }

    fn field<'t>(&self, node: Node<'t>, name: &str) -> Result<Node<'t>, String> {
        node.child_by_field_name(name)
            .ok_or_else(|| format!("{}: no {name} here", self.at(node.start_byte())))
    }

    fn enter(&self, all: Vec<String>, seen: Vec<String>) {
        self.scopes.borrow_mut().push(Scope { all, seen });
    }

    fn leave(&self) {
        self.scopes.borrow_mut().pop();
    }

    fn see(&self, names: &[String]) {
        if let Some(scope) = self.scopes.borrow_mut().last_mut() {
            scope.seen.extend(names.iter().cloned());
        }
    }

    fn declare(&self, node: Node<'_>) -> Result<String, String> {
        if node.kind() != "identifier" {
            return self.refuse(node, "a declaration of something other than a plain name");
        }
        let name = self.text(node);
        let mut bytes = name.bytes();
        let plain = bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !plain {
            return self.refuse(node, "a name Nix cannot spell");
        }
        if GLOBALS.contains(&name) && reserved(name) {
            return self.refuse(
                node,
                "a local named like a global whose Nix spelling already ends in '",
            );
        }
        Ok(name.to_owned())
    }

    fn resolve(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        let mut captured = false;
        for scope in self.scopes.borrow().iter().rev() {
            if scope.seen.iter().any(|one| one == name) {
                if captured {
                    break;
                }
                return Ok(local(name));
            }
            if scope.all.iter().any(|one| one == name) {
                captured = true;
            }
        }
        if captured && (!GLOBALS.contains(&name) || global(name) == local(name)) {
            return self.refuse(
                node,
                "a name used before a local of its block declares it, which its Nix binding would capture",
            );
        }
        if GLOBALS.contains(&name) {
            return Ok(self.uses(&global(name)));
        }
        self.refuse(
            node,
            "a name nothing declares that is not one of the Lua generator's globals",
        )
    }

    fn local_names<'t>(&self, node: Node<'t>) -> Result<Vec<Node<'t>>, String> {
        let rest = named(node);
        let [rest] = rest.as_slice() else {
            return self.unsupported(node);
        };
        let list = if rest.kind() == "variable_list" {
            *rest
        } else {
            *children(*rest)
                .first()
                .ok_or("an empty local declaration")?
        };
        Ok(named(list))
    }

    fn declared(&self, statements: &[Node<'_>]) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for statement in statements {
            match statement.kind() {
                "variable_declaration" => {
                    let declared = self
                        .local_names(*statement)?
                        .into_iter()
                        .map(|name| self.declare(name))
                        .collect::<Result<Vec<_>, _>>()?;
                    merge(&mut names, declared);
                }
                "function_declaration" if token(*statement, "local").is_some() => {
                    let name = self.declare(self.field(*statement, "name")?)?;
                    merge(&mut names, vec![name]);
                }
                _ => {}
            }
        }
        Ok(names)
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
                    out.push(self.uses("blank"));
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
    ) -> Result<Body, String> {
        let statements = block.map(named).unwrap_or_default();
        let (Some(first), Some(last)) = (statements.first(), statements.last()) else {
            self.expect(open, close, " ")?;
            return Ok(Body {
                inline: true,
                items: Vec::new(),
            });
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
            return Ok(Body {
                inline: true,
                items,
            });
        }
        self.expect(open, first.start_byte(), &pad(indent + 2))?;
        self.expect(last.end_byte(), close, &pad(indent))?;
        Ok(Body {
            inline: false,
            items: self.lines(&statements, indent + 2)?,
        })
    }

    fn listed(body: &Body) -> String {
        if body.inline {
            format!("[{}]", body.items.join(" "))
        } else {
            format!(
                "[\n{}]",
                body.items
                    .iter()
                    .map(|line| format!("{line}\n"))
                    .collect::<String>()
            )
        }
    }

    fn body_text(&self, body: &Body) -> String {
        let list = Self::listed(body);
        if body.inline {
            format!("{} {list}", self.uses("inline"))
        } else {
            list
        }
    }

    fn block_text(&self, body: &Body, names: &[String]) -> String {
        let list = bound(names, Self::listed(body));
        if body.inline {
            format!("({} {list})", self.uses("inline"))
        } else {
            list
        }
    }

    fn block(
        &self,
        block: Option<Node<'_>>,
        open: usize,
        close: usize,
        indent: usize,
    ) -> Result<String, String> {
        let names = self.declared(&block.map(named).unwrap_or_default())?;
        self.enter(names.clone(), Vec::new());
        let body = self.body(block, open, close, indent)?;
        self.leave();
        Ok(self.block_text(&body, &names))
    }

    fn closed(
        &self,
        node: Node<'_>,
        open: usize,
        close: &str,
        indent: usize,
    ) -> Result<Body, String> {
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

    fn closed_block(&self, node: Node<'_>, open: usize, indent: usize) -> Result<String, String> {
        let names = self.declared(&self.statements_of(node))?;
        self.enter(names.clone(), Vec::new());
        let body = self.closed(node, open, "end", indent)?;
        self.leave();
        Ok(self.block_text(&body, &names))
    }

    fn statements_of<'t>(&self, node: Node<'t>) -> Vec<Node<'t>> {
        node.child_by_field_name("body")
            .map(named)
            .unwrap_or_default()
    }

    fn statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        match node.kind() {
            "comment" => Ok(format!(
                "({} {})",
                self.uses("comment"),
                self.string(self.text(node))
            )),
            "empty_statement" => Ok(self.uses("semicolon")),
            "break_statement" => Ok(self.uses("break")),
            "function_call" => self.call(node, indent),
            "assignment_statement" => {
                let (targets, values) = self.assignment(node, indent)?;
                Ok(format!("({} {targets} {values})", self.uses("assign")))
            }
            "variable_declaration" => self.local_statement(node, indent),
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
                let test = self.expression(test, indent, Place::Free)?;
                let body = self.closed_block(node, open.end_byte(), indent)?;
                Ok(format!("({} {test} {body})", self.uses("while")))
            }
            "do_statement" => {
                let start = self.keyword(node, "do")?;
                let body = self.closed_block(node, start.end_byte(), indent)?;
                Ok(format!("({} {body})", self.uses("do")))
            }
            _ => self.unsupported(node),
        }
    }

    fn separated(&self, nodes: &[Node<'_>]) -> Result<(), String> {
        for pair in nodes.windows(2) {
            self.expect(pair[0].end_byte(), pair[1].start_byte(), ", ")?;
        }
        Ok(())
    }

    fn expressions(&self, nodes: &[Node<'_>], indent: usize) -> Result<Vec<String>, String> {
        self.separated(nodes)?;
        nodes
            .iter()
            .map(|n| self.expression(*n, indent, Place::Free))
            .collect()
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
        let values = self.expressions(&named(*values), indent)?;
        let targets = self.expressions(&named(*targets), indent)?;
        Ok((
            Self::one_or_list(targets),
            format!("[{}]", values.join(" ")),
        ))
    }

    fn local_statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let word = self.keyword(node, "local")?;
        let rest = named(node);
        let [rest] = rest.as_slice() else {
            return self.unsupported(node);
        };
        self.expect(word.end_byte(), rest.start_byte(), " ")?;
        let nodes = self.local_names(node)?;
        self.separated(&nodes)?;
        let names = nodes
            .iter()
            .map(|name| self.declare(*name))
            .collect::<Result<Vec<_>, _>>()?;
        let values = if rest.kind() == "variable_list" {
            Vec::new()
        } else {
            let parts = children(*rest);
            let [list, equals, values] = parts.as_slice() else {
                return self.unsupported(*rest);
            };
            if equals.kind() != "=" {
                return self.unsupported(*rest);
            }
            self.expect(list.end_byte(), equals.start_byte(), " ")?;
            self.expect(equals.end_byte(), values.start_byte(), " ")?;
            self.expressions(&named(*values), indent)?
        };
        self.see(&names);
        let spelled = names.iter().map(|name| local(name)).collect();
        Ok(format!(
            "({} {} [{}])",
            self.uses("local"),
            Self::one_or_list(spelled),
            values.join(" ")
        ))
    }

    fn function(&self, node: Node<'_>, after: usize, indent: usize) -> Result<String, String> {
        let parameters = self.field(node, "parameters")?;
        self.expect(after, parameters.start_byte(), "")?;
        let nodes = named(parameters);
        if let (Some(first), Some(last)) = (nodes.first(), nodes.last()) {
            self.expect(parameters.start_byte() + 1, first.start_byte(), "")?;
            self.expect(last.end_byte(), parameters.end_byte() - 1, "")?;
        } else {
            self.expect(parameters.start_byte(), parameters.end_byte(), "()")?;
        }
        self.separated(&nodes)?;
        let mut params = Vec::new();
        let mut items = Vec::new();
        for param in nodes {
            if param.kind() == "vararg_expression" {
                items.push(self.uses("vararg"));
                continue;
            }
            let name = self.declare(param)?;
            if params.contains(&name) {
                return self.refuse(param, "a parameter named twice");
            }
            items.push(local(&name));
            params.push(name);
        }
        let mut names = params.clone();
        merge(&mut names, self.declared(&self.statements_of(node))?);
        self.enter(names.clone(), params);
        let body = self.closed(node, parameters.end_byte(), "end", indent)?;
        self.leave();
        if items.is_empty() {
            return Ok(self.block_text(&body, &names));
        }
        let spec = format!(
            "{{params = [{}]; body = {};}}",
            items.join(" "),
            self.body_text(&body)
        );
        Ok(bound(&names, spec))
    }

    fn declaration(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let function = self.keyword(node, "function")?;
        let Some(word) = token(node, "local") else {
            return self.refuse(
                node,
                "a global function declaration (declare it local or assign the field)",
            );
        };
        self.expect(word.end_byte(), function.start_byte(), " ")?;
        let name = self.field(node, "name")?;
        self.expect(function.end_byte(), name.start_byte(), " ")?;
        let declared = self.declare(name)?;
        self.see(std::slice::from_ref(&declared));
        let spec = self.function(node, name.end_byte(), indent)?;
        Ok(format!(
            "({} {} {spec})",
            self.uses("localFunction"),
            local(&declared)
        ))
    }

    fn return_statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        if token(node, ";").is_some() {
            return Err(format!(
                "{}: a semicolon after return is not supported",
                self.at(node.start_byte())
            ));
        }
        let start = self.keyword(node, "return")?;
        let word = self.uses("return");
        match named(node).as_slice() {
            [] => Ok(format!("({word} [])")),
            [list] => {
                self.expect(start.end_byte(), list.start_byte(), " ")?;
                let values = self.expressions(&named(*list), indent)?;
                Ok(format!("({word} [{}])", values.join(" ")))
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
        Ok((self.expression(test, indent, Place::Free)?, then.end_byte()))
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
                    self.block(
                        alternative.child_by_field_name("body"),
                        word.end_byte(),
                        close,
                        indent,
                    )?
                }
                "elseif_statement" => {
                    let (test, open) = self.test(*alternative, "elseif", indent)?;
                    let body = self.block(
                        alternative.child_by_field_name("consequence"),
                        open,
                        close,
                        indent,
                    )?;
                    self.branch(test, body, otherwise.take())
                }
                _ => return self.unsupported(*alternative),
            });
        }
        let (test, open) = self.test(node, "if", indent)?;
        let body = self.block(
            node.child_by_field_name("consequence"),
            open,
            close_of(0),
            indent,
        )?;
        Ok(self.branch(test, body, otherwise))
    }

    fn branch(&self, test: String, body: String, otherwise: Option<String>) -> String {
        match otherwise {
            Some(otherwise) => format!("({} {test} {body} {otherwise})", self.uses("ifElse")),
            None => format!("({} {test} {body})", self.uses("if'")),
        }
    }

    fn for_loop(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let start = self.keyword(node, "for")?;
        let clause = self.field(node, "clause")?;
        let open = self.keyword(node, "do")?;
        self.expect(start.end_byte(), clause.start_byte(), " ")?;
        self.expect(clause.end_byte(), open.start_byte(), " ")?;
        let locals = self.declared(&self.statements_of(node))?;
        match clause.kind() {
            "for_generic_clause" => {
                let parts = children(clause);
                let [list, word, values] = parts.as_slice() else {
                    return self.unsupported(clause);
                };
                if word.kind() != "in" {
                    return self.unsupported(clause);
                }
                self.expect(list.end_byte(), word.start_byte(), " ")?;
                self.expect(word.end_byte(), values.start_byte(), " ")?;
                let nodes = named(*list);
                self.separated(&nodes)?;
                let vars = nodes
                    .iter()
                    .map(|name| self.declare(*name))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut names = vars.clone();
                merge(&mut names, locals);
                self.enter(names.clone(), Vec::new());
                let values = self.expressions(&named(*values), indent)?;
                self.see(&vars);
                let body = self.closed(node, open.end_byte(), "end", indent)?;
                self.leave();
                let spelled: Vec<String> = vars.iter().map(|name| local(name)).collect();
                let spec = format!(
                    "{{names = [{}]; values = [{}]; body = {};}}",
                    spelled.join(" "),
                    values.join(" "),
                    self.body_text(&body)
                );
                Ok(format!("({} {})", self.uses("forIn"), bound(&names, spec)))
            }
            "for_numeric_clause" => {
                let name = self.field(clause, "name")?;
                let equals = self.keyword(clause, "=")?;
                let first = self.field(clause, "start")?;
                self.expect(name.end_byte(), equals.start_byte(), " ")?;
                self.expect(equals.end_byte(), first.start_byte(), " ")?;
                let var = self.declare(name)?;
                let mut names = vec![var.clone()];
                merge(&mut names, locals);
                self.enter(names.clone(), Vec::new());
                let range: Vec<Node<'_>> = ["start", "end", "step"]
                    .iter()
                    .filter_map(|f| clause.child_by_field_name(f))
                    .collect();
                let range = self.expressions(&range, indent)?;
                self.see(std::slice::from_ref(&var));
                let body = self.closed(node, open.end_byte(), "end", indent)?;
                self.leave();
                let spelled = local(&var);
                let binding = if spelled == "name" {
                    "inherit name;".to_owned()
                } else {
                    format!("name = {spelled};")
                };
                let spec = format!(
                    "{{{binding} range = [{}]; body = {};}}",
                    range.join(" "),
                    self.body_text(&body)
                );
                Ok(format!("({} {})", self.uses("forNum"), bound(&names, spec)))
            }
            _ => self.unsupported(clause),
        }
    }

    fn call(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let name = self.field(node, "name")?;
        let arguments = self.field(node, "arguments")?;
        self.expect(name.end_byte(), arguments.start_byte(), "")?;
        let head = if name.kind() == "method_index_expression" {
            let table = self.field(name, "table")?;
            let method = self.field(name, "method")?;
            self.expect(table.end_byte(), method.start_byte(), ":")?;
            format!(
                "({} {} {})",
                self.uses("method"),
                self.expression(table, indent, Place::Prefix)?,
                self.string(self.text(method))
            )
        } else {
            self.expression(name, indent, Place::Prefix)?
        };
        let values = named(arguments);
        if self.text(arguments).as_bytes().first() != Some(&b'(') {
            let [value] = values.as_slice() else {
                return self.unsupported(arguments);
            };
            return Ok(format!(
                "({} {head} {})",
                self.uses("apply"),
                self.expression(*value, indent, Place::Free)?
            ));
        }
        if let (Some(first), Some(last)) = (values.first(), values.last()) {
            self.expect(arguments.start_byte() + 1, first.start_byte(), "")?;
            self.expect(last.end_byte(), arguments.end_byte() - 1, "")?;
        } else {
            self.expect(arguments.start_byte(), arguments.end_byte(), "()")?;
        }
        let mut items = vec![head];
        items.extend(self.expressions(&values, indent)?);
        Ok(format!("[{}]", items.join(" ")))
    }

    fn number(&self, node: Node<'_>, negative: bool) -> Result<String, String> {
        let text = if negative {
            format!("-{}", self.text(node))
        } else {
            self.text(node).to_owned()
        };
        nix::int(&text).or_else(|| float(&text)).map_or_else(
            || self.refuse(node, "a number Nix does not spell as the file does"),
            Ok,
        )
    }

    fn parenthesized(&self, node: Node<'_>, indent: usize, place: Place) -> Result<String, String> {
        let [inner] = named(node)[..] else {
            return self.unsupported(node);
        };
        self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
        self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
        let power = || {
            inner
                .child_by_field_name("operator")
                .is_some_and(|operator| operator.kind() == "^")
        };
        match (inner.kind(), place) {
            ("binary_expression", Place::Operand | Place::Power) => {
                self.infix(inner, indent, false)
            }
            ("binary_expression", Place::Unary) if !power() => self.infix(inner, indent, false),
            (
                "function_call" | "vararg_expression",
                Place::Free | Place::Operand | Place::Power | Place::Unary,
            ) => Ok(format!(
                "({} {})",
                self.uses("paren"),
                self.expression(inner, indent, Place::Free)?
            )),
            ("unary_expression", Place::Power) => self.expression(inner, indent, Place::Free),
            (kind, Place::Prefix) if !prefix_kind(kind) => {
                self.expression(inner, indent, Place::Free)
            }
            _ => self.refuse(node, "parentheses the printer would not write"),
        }
    }

    fn expression(&self, node: Node<'_>, indent: usize, place: Place) -> Result<String, String> {
        match node.kind() {
            "identifier" => self.resolve(node),
            "number" => self.number(node, false),
            "nil" => Ok(self.uses("nil")),
            "true" | "false" => Ok(self.text(node).to_owned()),
            "vararg_expression" => Ok(self.uses("vararg")),
            "string" => self.literal(node),
            "dot_index_expression" => {
                let mut path = Vec::new();
                let mut base = node;
                while base.kind() == "dot_index_expression" {
                    let table = self.field(base, "table")?;
                    let name = self.field(base, "field")?;
                    self.expect(table.end_byte(), name.start_byte(), ".")?;
                    path.push(self.text(name));
                    base = table;
                }
                path.reverse();
                Ok(format!(
                    "({} {} {})",
                    self.uses("dot"),
                    self.expression(base, indent, Place::Prefix)?,
                    self.string(&path.join("."))
                ))
            }
            "bracket_index_expression" => {
                let table = self.field(node, "table")?;
                let key = self.field(node, "field")?;
                self.expect(table.end_byte(), key.start_byte(), "[")?;
                self.expect(key.end_byte(), node.end_byte(), "]")?;
                Ok(format!(
                    "({} {} {})",
                    self.uses("index"),
                    self.expression(table, indent, Place::Prefix)?,
                    self.expression(key, indent, Place::Free)?
                ))
            }
            "function_call" => self.call(node, indent),
            "parenthesized_expression" => self.parenthesized(node, indent, place),
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
                    return self.number(operand, true);
                }
                let kind = match word {
                    "not" => "not",
                    "-" => "neg",
                    "#" => "len",
                    "~" => "bnot",
                    _ => return self.unsupported(node),
                };
                Ok(format!(
                    "({} {})",
                    self.uses(kind),
                    self.expression(operand, indent, Place::Unary)?
                ))
            }
            "binary_expression" => self.infix(node, indent, place == Place::Free),
            "table_constructor" => self.table(node, indent),
            "function_definition" => {
                let word = self.keyword(node, "function")?;
                Ok(format!(
                    "({} {})",
                    self.uses("fn"),
                    self.function(node, word.end_byte(), indent)?
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
            ("\"", "\"") => Ok(format!("({} {})", self.uses("str"), self.string(content))),
            ("'", "'") => Ok(format!("({} {})", self.uses("sq"), self.string(content))),
            _ => Err(format!(
                "{}: a long bracket string is not supported",
                self.at(node.start_byte())
            )),
        }
    }

    fn infix(&self, node: Node<'_>, indent: usize, rows: bool) -> Result<String, String> {
        let mut items = Vec::new();
        Self::flatten(node, &mut items);
        let mut out: Vec<Vec<String>> = vec![Vec::new()];
        let mut margin = indent;
        for (index, item) in items.iter().enumerate() {
            if index % 2 == 1 {
                let before = self.gap(items[index - 1].end_byte(), item.start_byte());
                if rows && before == pad(indent + 2) {
                    out.push(Vec::new());
                    margin = indent + 2;
                } else {
                    self.expect(items[index - 1].end_byte(), item.start_byte(), " ")?;
                }
                self.expect(item.end_byte(), items[index + 1].start_byte(), " ")?;
                out.last_mut().unwrap().push(self.string(item.kind()));
            } else {
                let place = if items.get(index + 1).is_some_and(|next| next.kind() == "^") {
                    Place::Power
                } else {
                    Place::Operand
                };
                let value = self.expression(*item, margin, place)?;
                out.last_mut().unwrap().push(value);
            }
        }
        if out.len() == 1 {
            return Ok(format!("[{}]", out[0].join(" ")));
        }
        let out: Vec<String> = out
            .iter()
            .map(|row| format!("[{}]", row.join(" ")))
            .collect();
        Ok(format!("({} [{}])", self.uses("rows"), out.join(" ")))
    }

    fn flatten<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
        let (Some(left), Some(operator), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("operator"),
            node.child_by_field_name("right"),
        ) else {
            out.push(node);
            return;
        };
        if node.kind() != "binary_expression" {
            out.push(node);
            return;
        }
        Self::flatten(left, out);
        out.push(operator);
        Self::flatten(right, out);
    }

    fn table(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let word = self.uses("table");
        let fields = named(node);
        let (Some(first), Some(last)) = (fields.first(), fields.last()) else {
            self.expect(node.start_byte(), node.end_byte(), "{}")?;
            return Ok(format!("({word} [])"));
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
            return Ok(format!("({word} [{}])", fields.join(" ")));
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
        Ok(format!(
            "({word} ({} [\n{}\n]))",
            self.uses("rows"),
            rows.join("\n")
        ))
    }

    fn table_field(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        if node.kind() != "field" {
            return self.unsupported(node);
        }
        let value = self.field(node, "value")?;
        let Some(name) = node.child_by_field_name("name") else {
            return self.expression(value, indent, Place::Free);
        };
        let equals = self.keyword(node, "=")?;
        self.expect(equals.end_byte(), value.start_byte(), " ")?;
        if name.start_byte() == node.start_byte() && name.kind() == "identifier" {
            self.expect(name.end_byte(), equals.start_byte(), " ")?;
            let value = self.expression(value, indent, Place::Free)?;
            return Ok(format!(
                "({} {} {value})",
                self.uses("field"),
                self.string(self.text(name))
            ));
        }
        self.expect(node.start_byte(), name.start_byte(), "[")?;
        self.expect(name.end_byte(), equals.start_byte(), "] ")?;
        let key = self.expression(name, indent, Place::Free)?;
        let value = self.expression(value, indent, Place::Free)?;
        Ok(format!("({} {key} {value})", self.uses("key")))
    }
}
