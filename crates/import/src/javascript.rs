use std::cell::RefCell;
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::{line_of, nix};

const NIX_KEYWORDS: [&str; 10] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];

const CONSTRUCTORS: [&str; 31] = [
    "array", "arrow", "async", "await", "blank", "call", "comment", "const", "continue", "dot",
    "function", "if'", "ifElse", "import'", "inline", "let'", "neg", "new", "not", "null'",
    "number", "obj", "optional", "render", "return", "rows", "str", "throw'", "try'", "typeof",
    "while",
];

const GLOBALS: [&str; 32] = [
    "AbortController",
    "Array",
    "Boolean",
    "Buffer",
    "Date",
    "Error",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "TypeError",
    "URL",
    "URLSearchParams",
    "clearInterval",
    "clearTimeout",
    "console",
    "fetch",
    "globalThis",
    "parseFloat",
    "parseInt",
    "process",
    "queueMicrotask",
    "setInterval",
    "setTimeout",
    "structuredClone",
    "undefined",
];

const LEVELS: [&[&str]; 13] = [
    &[
        "=", "+=", "-=", "*=", "/=", "%=", "**=", "<<=", ">>=", ">>>=", "&=", "|=", "^=", "&&=",
        "||=", "??=",
    ],
    &["??"],
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!=", "===", "!=="],
    &["<", ">", "<=", ">=", "in", "instanceof"],
    &["<<", ">>", ">>>"],
    &["+", "-"],
    &["*", "/", "%"],
    &["**"],
];

const LOGICAL: [&str; 3] = ["??", "||", "&&"];

const BITWISE: [&str; 6] = ["|", "^", "&", "<<", ">>", ">>>"];

const OPERATIONS: [&str; 3] = [
    "binary_expression",
    "assignment_expression",
    "augmented_assignment_expression",
];

fn precedence(operator: &str) -> Option<u8> {
    LEVELS
        .iter()
        .position(|level| level.contains(&operator))
        .and_then(|at| u8::try_from(at + 1).ok())
}

fn right_associative(operator: &str) -> bool {
    operator == "**" || precedence(operator) == Some(1)
}

fn flattens(parent: &str, child: &str) -> bool {
    let both = |group: &[&str]| group.contains(&parent) && group.contains(&child);
    parent != "**"
        && !both(&["==", "!=", "===", "!=="])
        && !(both(&["*", "/", "%"]) && parent != child)
        && !both(&["<<", ">>", ">>>"])
}

fn grouped(parent: &str, left: bool, child: &Expr) -> bool {
    let level = precedence(parent).unwrap_or(0);
    if child.precedence < level {
        return true;
    }
    let Some(operator) = child.operator.as_deref() else {
        return parent == "**" && left && child.precedence < 15;
    };
    if LOGICAL.contains(&parent) && LOGICAL.contains(&operator) && parent != operator {
        return true;
    }
    if child.precedence == level {
        return if level == 1 {
            left
        } else {
            !left || !flattens(parent, operator)
        };
    }
    operator == "%" && matches!(parent, "+" | "-") || BITWISE.contains(&parent)
}

fn reserved(name: &str) -> bool {
    NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) || GLOBALS.contains(&name)
}

fn spelled(name: &str) -> String {
    if reserved(name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn float(text: &str) -> Option<String> {
    let (whole, fraction) = text.split_once('.')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(fraction) || whole.len() > 1 && whole.starts_with('0') {
        return None;
    }
    if whole.trim_start_matches('0').len() + fraction.len() > 15 {
        return None;
    }
    let value: f64 = text.parse().ok()?;
    if !(1e-4..1e15).contains(&value) {
        return None;
    }
    let shortest = value.to_string();
    let canonical = if shortest.contains('.') {
        shortest
    } else {
        format!("{shortest}.0")
    };
    (canonical == text).then(|| text.to_owned())
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_javascript::LANGUAGE.into(), source)?;
    let importer = Importer {
        source,
        holes,
        scopes: RefCell::new(Vec::new()),
        used: RefCell::new(BTreeSet::new()),
    };
    let statements = named(tree.root_node());
    if statements.is_empty() {
        return Err("the program is empty".into());
    }
    let names = importer.declared(&statements)?;
    let (items, text) = importer.scoped(names.clone(), || importer.statements(&statements, 0))?;
    importer.compare(&format!("{}\n", &text[1..]))?;
    let body = importer.lambda(&names, &lines(&items));
    let used = importer.used.borrow();
    let formals = if used.is_empty() {
        "_".to_owned()
    } else {
        let names: String = used.iter().map(|name| format!("  {name},\n")).collect();
        format!("{{\n{names}  ...\n}}")
    };
    Ok(format!("{formals}: {body}"))
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    scopes: RefCell<Vec<Vec<String>>>,
    used: RefCell<BTreeSet<String>>,
}

type Out = (String, String);

struct Expr {
    nix: String,
    text: String,
    precedence: u8,
    operator: Option<String>,
}

impl Expr {
    fn new(nix: String, text: String, precedence: u8) -> Self {
        Expr {
            nix,
            text,
            precedence,
            operator: None,
        }
    }

    fn wrapped(&self, least: u8) -> String {
        if self.precedence < least {
            format!("({})", self.text)
        } else {
            self.text.clone()
        }
    }
}

struct Chain {
    items: Vec<String>,
    breaks: Vec<bool>,
    text: String,
    operator: String,
}

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

    fn uses(&self, name: &str) -> String {
        self.used.borrow_mut().insert(name.to_owned());
        name.to_owned()
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

    fn scoped<T>(
        &self,
        names: Vec<String>,
        body: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.scopes.borrow_mut().push(names);
        let result = body();
        self.scopes.borrow_mut().pop();
        result
    }

    fn lambda(&self, names: &[String], body: &str) -> String {
        if names.is_empty() {
            return body.to_owned();
        }
        let formals: Vec<String> = names.iter().map(|name| spelled(name)).collect();
        let body = body
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .filter(|inner| inner.starts_with("inline "))
            .unwrap_or(body);
        format!("({{{}}}: {body})", formals.join(", "))
    }

    fn declare(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        let name = self.text(node);
        let mut bytes = name.bytes();
        let spellable = bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !spellable {
            return Err(format!(
                "{}: Nix cannot spell the name `{name}`",
                self.at(node)
            ));
        }
        if CONSTRUCTORS.contains(&spelled(name).as_str()) {
            return Err(format!(
                "{}: `{name}` would be spelled `{name}'`, a constructor of generators.javascript",
                self.at(node)
            ));
        }
        if out.iter().any(|one| one == name) {
            return Err(format!(
                "{}: `{name}` is declared twice in one scope",
                self.at(node)
            ));
        }
        out.push(name.to_owned());
        Ok(())
    }

    fn pattern_names(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        match node.kind() {
            "identifier" => self.declare(node, out),
            "object_pattern" => {
                for entry in named(node) {
                    match entry.kind() {
                        "shorthand_property_identifier_pattern" => self.declare(entry, out)?,
                        "pair_pattern" => self.pattern_names(self.field(entry, "value")?, out)?,
                        other => return Err(self.unsupported(entry, other)),
                    }
                }
                Ok(())
            }
            "array_pattern" => {
                for item in named(node) {
                    self.pattern_names(item, out)?;
                }
                Ok(())
            }
            other => Err(self.unsupported(node, &format!("the binding {other}"))),
        }
    }

    fn declared(&self, statements: &[Node<'_>]) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for statement in statements {
            match statement.kind() {
                "lexical_declaration" => {
                    for declarator in named(*statement) {
                        self.pattern_names(self.field(declarator, "name")?, &mut names)?;
                    }
                }
                "function_declaration" => {
                    self.declare(self.field(*statement, "name")?, &mut names)?
                }
                "import_statement" => {
                    for clause in named(*statement) {
                        if clause.kind() != "import_clause" {
                            continue;
                        }
                        for specifiers in named(clause) {
                            for specifier in named(specifiers) {
                                self.declare(self.field(specifier, "name")?, &mut names)?;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(names)
    }

    fn resolve(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if self
            .scopes
            .borrow()
            .iter()
            .any(|scope| scope.iter().any(|one| one == name))
        {
            return Ok(spelled(name));
        }
        if GLOBALS.contains(&name) {
            return Ok(self.uses(name));
        }
        Err(format!(
            "{}: `{name}` is declared nowhere and is not a JavaScript global generators.javascript knows",
            self.at(node)
        ))
    }

    fn broken(&self, node: Node<'_>) -> bool {
        let entries = named(node);
        matches!(
            node.kind(),
            "object" | "array" | "object_pattern" | "array_pattern"
        ) && entries
            .first()
            .is_some_and(|first| self.source[node.start_byte()..first.start_byte()].contains('\n'))
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
                    items.push(self.uses("blank"));
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

    fn body(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
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
            return Ok((format!("({} {})", self.uses("inline"), list(&nix)), written));
        }
        let (items, text) = self.statements(&statements, indent + 2)?;
        Ok((lines(&items), format!("{{{text}\n{}}}", pad(indent))))
    }

    fn block(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let names = self.declared(&named(node))?;
        let (nix, text) = self.scoped(names.clone(), || self.body(node, indent))?;
        Ok((self.lambda(&names, &nix), text))
    }

    fn clause(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        if node.kind() == "statement_block" {
            let (nix, text) = self.block(node, indent)?;
            return Ok((nix, format!(" {text}")));
        }
        if node.kind() == "expression_statement"
            && named(node)
                .first()
                .is_some_and(|inner| OPERATIONS.contains(&unparen(*inner).kind()))
        {
            return Err(format!(
                "{}: a body without braces must not be an operation, which the tree reads as a block",
                self.at(node)
            ));
        }
        let (nix, text) = self.statement(node, indent)?;
        Ok((nix, format!(" {text}")))
    }

    fn condition(&self, node: Node<'_>, indent: usize) -> Result<Expr, String> {
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
                Ok((
                    format!("({} {})", self.uses("comment"), self.string(text)),
                    text.to_owned(),
                ))
            }
            "import_statement" => self.import(node),
            "lexical_declaration" => self.declaration(node, indent),
            "function_declaration" => self.function(node, indent),
            "expression_statement" => {
                let inner = named(node);
                let [expression] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this statement"));
                };
                let expression = self.expression(*expression, indent)?;
                Ok((expression.nix, format!("{};", expression.text)))
            }
            "return_statement" => match named(node).as_slice() {
                [] => Ok((
                    format!("({} null)", self.uses("return")),
                    "return;".to_owned(),
                )),
                [value] => {
                    let value = self.expression(*value, indent)?;
                    Ok((
                        format!("({} {})", self.uses("return"), value.nix),
                        format!("return {};", value.wrapped(1)),
                    ))
                }
                _ => Err(self.unsupported(node, "this return")),
            },
            "throw_statement" => {
                let inner = named(node);
                let [value] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this throw"));
                };
                let value = self.expression(*value, indent)?;
                Ok((
                    format!("({} {})", self.uses("throw'"), value.nix),
                    format!("throw {};", value.wrapped(1)),
                ))
            }
            "continue_statement" if named(node).is_empty() => {
                Ok((self.uses("continue"), "continue;".to_owned()))
            }
            "if_statement" => {
                let test = self.condition(self.field(node, "condition")?, indent)?;
                let (body, body_text) = self.clause(self.field(node, "consequence")?, indent)?;
                match node.child_by_field_name("alternative") {
                    None => Ok((
                        format!("({} {} {body})", self.uses("if'"), test.nix),
                        format!("if ({}){body_text}", test.wrapped(1)),
                    )),
                    Some(alternative) => {
                        let inner = named(alternative);
                        let [otherwise] = inner.as_slice() else {
                            return Err(self.unsupported(alternative, "this else"));
                        };
                        let (other, other_text) = self.clause(*otherwise, indent)?;
                        Ok((
                            format!("({} {} {body} {other})", self.uses("ifElse"), test.nix),
                            format!("if ({}){body_text} else{other_text}", test.wrapped(1)),
                        ))
                    }
                }
            }
            "while_statement" => {
                let test = self.condition(self.field(node, "condition")?, indent)?;
                let (body, body_text) = self.clause(self.field(node, "body")?, indent)?;
                Ok((
                    format!("({} {} {body})", self.uses("while"), test.nix),
                    format!("while ({}){body_text}", test.wrapped(1)),
                ))
            }
            "try_statement" => self.attempt(node, indent),
            other => Err(self.unsupported(node, other)),
        }
    }

    fn attempt(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        if node.child_by_field_name("finalizer").is_some() {
            return Err(self.unsupported(node, "finally"));
        }
        let (body, body_text) = self.block(self.field(node, "body")?, indent)?;
        let handler = self.field(node, "handler")?;
        let caught = self.field(handler, "body")?;
        let parameter = handler.child_by_field_name("parameter");
        let mut names = Vec::new();
        if let Some(parameter) = parameter {
            self.pattern_names(parameter, &mut names)?;
        }
        names.extend(self.declared(&named(caught))?);
        let ((nix, text), param) = self.scoped(names.clone(), || {
            let param = parameter.map(|p| self.pattern(p, indent)).transpose()?;
            Ok((self.body(caught, indent)?, param))
        })?;
        let (handler, written) = match param {
            None => (
                self.lambda(&names, &nix),
                format!("try {body_text} catch {text}"),
            ),
            Some((param, param_text)) => (
                self.lambda(&names, &format!("{{param = {param}; body = {nix};}}")),
                format!("try {body_text} catch ({param_text}) {text}"),
            ),
        };
        Ok((format!("({} {body} {handler})", self.uses("try'")), written))
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
        let mut written = Vec::new();
        for specifier in named(*specifiers) {
            if specifier.kind() != "import_specifier" || named(specifier).len() != 1 {
                return Err(self.unsupported(specifier, "a renamed import"));
            }
            names.push(self.resolve(self.field(specifier, "name")?)?);
            written.push(self.text(specifier).to_owned());
        }
        let source = self.field(node, "source")?;
        let quoted = self.text(source);
        if !quoted.starts_with('"') {
            return Err(self.unsupported(source, "a single-quoted string"));
        }
        let path = &quoted[1..quoted.len() - 1];
        Ok((
            format!(
                "({} {} {})",
                self.uses("import'"),
                list(&names),
                self.string(path)
            ),
            format!("import {{ {} }} from \"{path}\";", written.join(", ")),
        ))
    }

    fn declaration(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let kind = self.text(self.field(node, "kind")?).to_owned();
        let declarators = named(node);
        let [declarator] = declarators.as_slice() else {
            return Err(self.unsupported(node, "more than one declarator"));
        };
        let (target, target_text) = self.pattern(self.field(*declarator, "name")?, indent)?;
        let value = declarator
            .child_by_field_name("value")
            .map(|v| self.expression(v, indent))
            .transpose()?;
        match (kind.as_str(), value) {
            ("const", Some(value)) => Ok((
                format!("({} {target} {})", self.uses("const"), value.nix),
                format!("const {target_text} = {};", value.wrapped(1)),
            )),
            ("let", Some(value)) => Ok((
                format!("({} {target} {})", self.uses("let'"), value.nix),
                format!("let {target_text} = {};", value.wrapped(1)),
            )),
            ("let", None) => Ok((
                format!("({} {target} null)", self.uses("let'")),
                format!("let {target_text};"),
            )),
            _ => Err(self.unsupported(node, "this declaration")),
        }
    }

    fn pattern(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        match node.kind() {
            "identifier" => Ok((self.resolve(node)?, self.text(node).to_owned())),
            "object_pattern" | "array_pattern" => {
                let collection = self.collection(node, indent, true)?;
                Ok((collection.nix, collection.text))
            }
            other => Err(self.unsupported(node, &format!("the binding {other}"))),
        }
    }

    fn callable(
        &self,
        params: Node<'_>,
        body: Node<'_>,
        indent: usize,
    ) -> Result<(String, String, String), String> {
        if params.kind() != "formal_parameters" {
            return Err(self.unsupported(params, "a parameter without parentheses"));
        }
        let params = named(params);
        let mut names = Vec::new();
        for param in &params {
            self.pattern_names(*param, &mut names)?;
        }
        let block = body.kind() == "statement_block";
        if block {
            for name in self.declared(&named(body))? {
                if names.contains(&name) {
                    return Err(format!(
                        "{}: `{name}` is declared twice in one scope",
                        self.at(body)
                    ));
                }
                names.push(name);
            }
        }
        let (params, body) = self.scoped(names.clone(), || {
            let params = params
                .iter()
                .map(|p| self.pattern(*p, indent))
                .collect::<Result<Vec<_>, _>>()?;
            let body = if block {
                self.body(body, indent)?
            } else {
                self.concise(body, indent)?
            };
            Ok((params, body))
        })?;
        let nix: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
        let text: Vec<String> = params.iter().map(|(_, t)| t.clone()).collect();
        Ok((
            self.lambda(
                &names,
                &format!("{{params = {}; body = {};}}", list(&nix), body.0),
            ),
            text.join(", "),
            body.1,
        ))
    }

    fn concise(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let inner = unparen(node);
        if OPERATIONS.contains(&inner.kind()) {
            return Err(format!(
                "{}: an arrow whose body is an operation needs braces",
                self.at(node)
            ));
        }
        let value = self.expression(node, indent)?;
        if inner.kind() == "object" {
            return Ok((value.nix, format!("({})", value.text)));
        }
        Ok((value.nix.clone(), value.wrapped(1)))
    }

    fn function(&self, node: Node<'_>, indent: usize) -> Result<Out, String> {
        let name = self.field(node, "name")?;
        let (definition, params, body) = self.callable(
            self.field(node, "parameters")?,
            self.field(node, "body")?,
            indent,
        )?;
        let out = (
            format!(
                "({} {} {definition})",
                self.uses("function"),
                self.resolve(name)?
            ),
            format!("function {}({params}) {body}", self.text(name)),
        );
        Ok(self.asynchronous(node, out))
    }

    fn asynchronous(&self, node: Node<'_>, out: Out) -> Out {
        if has_token(node, "async") {
            (
                format!("({} {})", self.uses("async"), out.0),
                format!("async {}", out.1),
            )
        } else {
            out
        }
    }

    fn arrow(&self, node: Node<'_>, indent: usize) -> Result<Expr, String> {
        let params = match node.child_by_field_name("parameters") {
            Some(params) => params,
            None => self.field(node, "parameter")?,
        };
        let (definition, params, body) =
            self.callable(params, self.field(node, "body")?, indent)?;
        let (nix, text) = self.asynchronous(
            node,
            (
                format!("({} {definition})", self.uses("arrow")),
                format!("({params}) => {body}"),
            ),
        );
        Ok(Expr::new(nix, text, 1))
    }

    fn number(&self, node: Node<'_>, negative: bool) -> Expr {
        let text = self.text(node);
        let sign = if negative { "-" } else { "" };
        let precedence = if negative { 14 } else { 15 };
        let written = format!("{sign}{text}");
        if let Some(int) = nix::int(&written) {
            return Expr::new(int, written, precedence);
        }
        if let Some(float) = float(text) {
            let nix = if negative {
                format!("(-{float})")
            } else {
                float
            };
            return Expr::new(nix, written, precedence);
        }
        let spelling = format!("({} {})", self.uses("number"), self.string(text));
        if negative {
            Expr::new(format!("({} {spelling})", self.uses("neg")), written, 14)
        } else {
            Expr::new(spelling, written, 15)
        }
    }

    fn expression(&self, node: Node<'_>, indent: usize) -> Result<Expr, String> {
        let node = unparen(node);
        let text = self.text(node);
        match node.kind() {
            "identifier" => Ok(Expr::new(self.resolve(node)?, text.to_owned(), 20)),
            "undefined" => Ok(Expr::new(self.uses("undefined"), text.to_owned(), 20)),
            "null" => Ok(Expr::new(self.uses("null'"), text.to_owned(), 20)),
            "true" | "false" => Ok(Expr::new(text.to_owned(), text.to_owned(), 20)),
            "number" => Ok(self.number(node, false)),
            "string" => {
                if !text.starts_with('"') {
                    return Err(self.unsupported(node, "a single-quoted string"));
                }
                let content = &text[1..text.len() - 1];
                Ok(Expr::new(
                    format!("({} {})", self.uses("str"), self.string(content)),
                    text.to_owned(),
                    20,
                ))
            }
            "member_expression" => self.member(node, indent),
            "call_expression" => {
                if has_token(node, "optional_chain") {
                    return Err(self.unsupported(node, "an optional call"));
                }
                let callee = self.expression(self.field(node, "function")?, indent)?;
                let (args, args_text) = self.arguments(self.field(node, "arguments")?, indent)?;
                Ok(Expr::new(
                    format!("({} {} {})", self.uses("call"), callee.nix, list(&args)),
                    format!("{}({args_text})", callee.wrapped(18)),
                    18,
                ))
            }
            "new_expression" => {
                let constructor = self.field(node, "constructor")?;
                let plain = match constructor.kind() {
                    "identifier" => true,
                    "member_expression" => !self.text(constructor).contains("?."),
                    _ => false,
                };
                if !plain {
                    return Err(
                        self.unsupported(constructor, "new of something not a name or a dot path")
                    );
                }
                let callee = self.expression(constructor, indent)?;
                let arguments = node
                    .child_by_field_name("arguments")
                    .ok_or_else(|| self.unsupported(node, "new without arguments"))?;
                let (args, args_text) = self.arguments(arguments, indent)?;
                Ok(Expr::new(
                    format!("({} {} {})", self.uses("new"), callee.nix, list(&args)),
                    format!("new {}({args_text})", callee.text),
                    18,
                ))
            }
            "await_expression" => {
                let inner = named(node);
                let [value] = inner.as_slice() else {
                    return Err(self.unsupported(node, "this await"));
                };
                let value = self.expression(*value, indent)?;
                Ok(Expr::new(
                    format!("({} {})", self.uses("await"), value.nix),
                    format!("await {}", value.wrapped(14)),
                    14,
                ))
            }
            "unary_expression" => {
                let operator = self.text(self.field(node, "operator")?);
                let argument = self.field(node, "argument")?;
                if operator == "-" && argument.kind() == "number" {
                    return Ok(self.number(argument, true));
                }
                let negated = unparen(argument).kind() == "unary_expression"
                    && unparen(argument)
                        .child_by_field_name("operator")
                        .is_some_and(|op| self.text(op) == "-");
                let value = self.expression(argument, indent)?;
                let (name, written) = match operator {
                    "!" => ("not", format!("!{}", value.wrapped(14))),
                    "typeof" => ("typeof", format!("typeof {}", value.wrapped(14))),
                    "-" if negated => ("neg", format!("-({})", value.text)),
                    "-" => ("neg", format!("-{}", value.wrapped(14))),
                    other => return Err(self.unsupported(node, &format!("the {other} operator"))),
                };
                Ok(Expr::new(
                    format!("({} {})", self.uses(name), value.nix),
                    written,
                    14,
                ))
            }
            kind if OPERATIONS.contains(&kind) => {
                let chain = self.operation(node, indent)?;
                let broken = chain.breaks.iter().all(|b| *b);
                if !broken && chain.breaks.iter().any(|b| *b) {
                    return Err(format!(
                        "{}: an operator chain that breaks the line after some operators but not all",
                        self.at(node)
                    ));
                }
                let nix = if broken {
                    format!("({} {})", self.uses("rows"), lines(&chain.items))
                } else {
                    list(&chain.items)
                };
                Ok(Expr {
                    nix,
                    text: chain.text,
                    precedence: precedence(&chain.operator).unwrap_or(0),
                    operator: Some(chain.operator),
                })
            }
            "arrow_function" => self.arrow(node, indent),
            "object" | "array" => self.collection(node, indent, false),
            other => Err(self.unsupported(node, other)),
        }
    }

    fn member(&self, node: Node<'_>, indent: usize) -> Result<Expr, String> {
        let (optional, object, path) = self.path(node, indent)?;
        let (name, link) = if optional {
            ("optional", "?.")
        } else {
            ("dot", ".")
        };
        let path = path.join(".");
        Ok(Expr::new(
            format!(
                "({} {} {})",
                self.uses(name),
                object.nix,
                self.string(&path)
            ),
            format!("{}{link}{path}", object.wrapped(18)),
            18,
        ))
    }

    fn path(&self, node: Node<'_>, indent: usize) -> Result<(bool, Expr, Vec<String>), String> {
        let property = self.field(node, "property")?;
        if property.kind() != "property_identifier" {
            return Err(self.unsupported(property, property.kind()));
        }
        let name = self.text(property).to_owned();
        let object = self.field(node, "object")?;
        let optional = has_token(node, "optional_chain");
        if !optional && object.kind() == "member_expression" {
            let (inner, base, mut path) = self.path(object, indent)?;
            path.push(name);
            return Ok((inner, base, path));
        }
        Ok((optional, self.expression(object, indent)?, vec![name]))
    }

    fn arguments(&self, node: Node<'_>, indent: usize) -> Result<(Vec<String>, String), String> {
        if node.kind() != "arguments" {
            return Err(self.unsupported(node, node.kind()));
        }
        let items = named(node)
            .iter()
            .map(|a| self.expression(*a, indent))
            .collect::<Result<Vec<_>, _>>()?;
        let text: Vec<String> = items.iter().map(|e| e.wrapped(1)).collect();
        Ok((items.into_iter().map(|e| e.nix).collect(), text.join(", ")))
    }

    fn operator<'t>(&self, node: Node<'t>) -> Result<(Node<'t>, Node<'t>, Node<'t>), String> {
        let left = self.field(node, "left")?;
        let right = self.field(node, "right")?;
        let operator = children(node)
            .into_iter()
            .find(|c| c.start_byte() >= left.end_byte() && c.end_byte() <= right.start_byte())
            .ok_or_else(|| self.unsupported(node, "an expression without its operator"))?;
        Ok((left, operator, right))
    }

    fn operand(
        &self,
        parent: &str,
        left: bool,
        node: Node<'_>,
        indent: usize,
    ) -> Result<Chain, String> {
        let inner = unparen(node);
        if OPERATIONS.contains(&inner.kind()) {
            let chain = self.operation(inner, indent)?;
            let child = Expr {
                nix: String::new(),
                text: String::new(),
                precedence: precedence(&chain.operator).unwrap_or(0),
                operator: Some(chain.operator.clone()),
            };
            let level = precedence(parent).unwrap_or(0);
            let flat = child.precedence > level
                || child.precedence == level && left != right_associative(parent);
            let text = if grouped(parent, left, &child) {
                format!("({})", chain.text)
            } else {
                chain.text.clone()
            };
            if flat {
                return Ok(Chain { text, ..chain });
            }
            let broken = chain.breaks.iter().all(|b| *b);
            if !broken && chain.breaks.iter().any(|b| *b) {
                return Err(format!(
                    "{}: an operator chain that breaks the line after some operators but not all",
                    self.at(node)
                ));
            }
            let nix = if broken {
                format!("({} {})", self.uses("rows"), lines(&chain.items))
            } else {
                list(&chain.items)
            };
            return Ok(Chain {
                items: vec![nix],
                breaks: Vec::new(),
                text,
                operator: chain.operator,
            });
        }
        let value = self.expression(inner, indent)?;
        let text = if grouped(parent, left, &value) {
            format!("({})", value.text)
        } else {
            value.text
        };
        Ok(Chain {
            items: vec![value.nix],
            breaks: Vec::new(),
            text,
            operator: String::new(),
        })
    }

    fn operation(&self, node: Node<'_>, indent: usize) -> Result<Chain, String> {
        let (left, operator, right) = self.operator(node)?;
        let symbol = self.text(operator).to_owned();
        if precedence(&symbol).is_none() {
            return Err(self.unsupported(operator, &format!("the {symbol} operator")));
        }
        let broken = self.newlines(operator, right) > 0;
        if self.newlines(left, operator) > 0 {
            return Err(format!(
                "{}: a line break before an operator",
                self.at(operator)
            ));
        }
        let inner = if broken { indent + 2 } else { indent };
        let first = self.operand(&symbol, true, left, indent)?;
        let second = self.operand(&symbol, false, right, inner)?;
        let mut items = first.items;
        items.push(self.string(&symbol));
        items.extend(second.items);
        let mut breaks = first.breaks;
        breaks.push(broken);
        breaks.extend(second.breaks);
        let joint = if broken {
            format!(" {symbol}\n{}", pad(inner))
        } else {
            format!(" {symbol} ")
        };
        Ok(Chain {
            items,
            breaks,
            text: format!("{}{joint}{}", first.text, second.text),
            operator: symbol,
        })
    }

    fn entry(&self, node: Node<'_>, indent: usize, pattern: bool) -> Result<(Out, bool), String> {
        match node.kind() {
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => {
                Ok(((self.resolve(node)?, self.text(node).to_owned()), false))
            }
            "pair" | "pair_pattern" => {
                let key = self.field(node, "key")?;
                let (key, key_text) = match key.kind() {
                    "property_identifier" => {
                        (self.string(self.text(key)), self.text(key).to_owned())
                    }
                    "string" => {
                        let key = self.expression(key, indent)?;
                        (key.nix, key.text)
                    }
                    _ => return Err(self.unsupported(key, "a computed key")),
                };
                let value = self.field(node, "value")?;
                let hug = self.broken(unparen(value));
                let (value, value_text) = if pattern {
                    self.pattern(value, indent)?
                } else {
                    let value = self.expression(value, indent)?;
                    let text = value.wrapped(1);
                    (value.nix, text)
                };
                Ok((
                    (list(&[key, value]), format!("{key_text}: {value_text}")),
                    hug,
                ))
            }
            other => Err(self.unsupported(node, other)),
        }
    }

    fn collection(&self, node: Node<'_>, indent: usize, pattern: bool) -> Result<Expr, String> {
        let object = matches!(node.kind(), "object" | "object_pattern");
        let (name, open, close) = if object {
            ("obj", "{", "}")
        } else {
            ("array", "[", "]")
        };
        let name = self.uses(name);
        let broken = self.broken(node);
        let inner = if broken { indent + 2 } else { indent };
        let mut nix = Vec::new();
        let mut text = Vec::new();
        let mut hug = false;
        for item in named(node) {
            let ((n, t), h) = if object {
                self.entry(item, inner, pattern)?
            } else if pattern {
                (self.pattern(item, inner)?, false)
            } else {
                let value = self.expression(item, inner)?;
                let text = value.wrapped(1);
                ((value.nix, text), false)
            };
            nix.push(n);
            text.push(t);
            hug = h;
        }
        if broken {
            let rows: Vec<String> = text
                .iter()
                .map(|t| format!("\n{}{t}", pad(inner)))
                .collect();
            return Ok(Expr::new(
                format!("({name} ({} {}))", self.uses("rows"), lines(&nix)),
                format!("{open}{}\n{}{close}", rows.join(","), pad(indent)),
                20,
            ));
        }
        let space = if object && !text.is_empty() { " " } else { "" };
        let closing = if hug { "" } else { space };
        Ok(Expr::new(
            format!("({name} {})", list(&nix)),
            format!("{open}{space}{}{closing}{close}", text.join(", ")),
            20,
        ))
    }
}

fn unparen(node: Node<'_>) -> Node<'_> {
    if node.kind() != "parenthesized_expression" {
        return node;
    }
    let inner = named(node);
    match inner.as_slice() {
        [one] if one.kind() != "sequence_expression" => unparen(*one),
        _ => node,
    }
}
