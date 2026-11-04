use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::{column_of, line_of, nix};

const NIX_KEYWORDS: [&str; 10] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];

const CONSTRUCTORS: [&str; 68] = [
    "blank",
    "comment",
    "str",
    "sq",
    "triple",
    "raw",
    "bytes",
    "field",
    "spec",
    "strings",
    "octal",
    "placeholder",
    "rows",
    "aligned",
    "hang",
    "call",
    "dot",
    "index",
    "slice",
    "list",
    "tuple",
    "set",
    "dict",
    "pair",
    "kw",
    "default",
    "star",
    "starstar",
    "commas",
    "neg",
    "invert",
    "not",
    "await",
    "lambda",
    "ternary",
    "walrus",
    "listComp",
    "setComp",
    "dictComp",
    "gen",
    "compFor",
    "compIf",
    "as",
    "assign",
    "augAssign",
    "return",
    "raise",
    "pass",
    "break",
    "continue",
    "del",
    "global",
    "nonlocal",
    "assert'",
    "import'",
    "from",
    "def",
    "async",
    "class",
    "if'",
    "ifElse",
    "for",
    "while",
    "with'",
    "try",
    "except",
    "finally",
    "render",
];

const BUILTINS: [&str; 122] = [
    "None",
    "NotImplemented",
    "Ellipsis",
    "__name__",
    "__doc__",
    "__file__",
    "abs",
    "aiter",
    "all",
    "anext",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "BaseException",
    "Exception",
    "ArithmeticError",
    "AssertionError",
    "AttributeError",
    "BlockingIOError",
    "BrokenPipeError",
    "BufferError",
    "ChildProcessError",
    "ConnectionAbortedError",
    "ConnectionError",
    "ConnectionRefusedError",
    "ConnectionResetError",
    "EOFError",
    "FileExistsError",
    "FileNotFoundError",
    "GeneratorExit",
    "ImportError",
    "IndexError",
    "InterruptedError",
    "IsADirectoryError",
    "KeyError",
    "KeyboardInterrupt",
    "LookupError",
    "MemoryError",
    "ModuleNotFoundError",
    "NameError",
    "NotADirectoryError",
    "NotImplementedError",
    "OSError",
    "OverflowError",
    "PermissionError",
    "ProcessLookupError",
    "RecursionError",
    "RuntimeError",
    "StopAsyncIteration",
    "StopIteration",
    "SyntaxError",
    "SystemExit",
    "TimeoutError",
    "TypeError",
    "UnicodeDecodeError",
    "UnicodeEncodeError",
    "UnicodeError",
    "ValueError",
    "ZeroDivisionError",
];

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let masked = mask(source);
    let tree = crate::parse(&tree_sitter_python::LANGUAGE.into(), &masked)?;
    let importer = Importer {
        source,
        holes,
        scopes: RefCell::new(Vec::new()),
        skip: Cell::new(None),
        used: RefCell::new(BTreeSet::new()),
    };
    let nodes = named(tree.root_node());
    let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
        return Err("the source holds no statement".into());
    };
    if first.start_byte() != 0 {
        return Err("line 1: the source starts with blank space".into());
    }
    let tail = &source[last.end_byte()..];
    if tail != "\n" {
        return Err(format!(
            "the source must end with one newline after its last statement, not {tail:?}"
        ));
    }
    let module = importer.scope(Kind::Module, Vec::new(), &nodes, |importer| {
        importer.statements(&nodes, 0)
    })?;
    let used = importer.used.borrow();
    let formals: String = used.iter().map(|name| format!("  {name},\n")).collect();
    Ok(format!("{{\n{formals}  ...\n}}: {module}"))
}

fn mask(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'@' {
            let name = bytes[at + 1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            let close = at + 1 + name;
            if name > 0 && !bytes[at + 1].is_ascii_digit() && bytes.get(close) == Some(&b'@') {
                bytes[at] = b'_';
                bytes[close] = b'_';
                at = close + 1;
                continue;
            }
        }
        at += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| source.to_owned())
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "line_continuation")
        .collect()
}

fn kids(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() != "line_continuation")
        .collect()
}

fn row(items: &[String]) -> String {
    if items.len() == 1 && !items[0].starts_with('[') {
        items[0].clone()
    } else {
        format!("[{}]", items.join(" "))
    }
}

fn bind(formals: &[String], body: String) -> String {
    if formals.is_empty() {
        body
    } else {
        format!("({{{}}}: {body})", formals.join(", "))
    }
}

fn infix(kind: &str) -> bool {
    matches!(
        kind,
        "binary_operator" | "boolean_operator" | "comparison_operator"
    )
}

fn binary_level(operator: &str) -> Option<u8> {
    Some(match operator {
        "or" => 4,
        "and" => 5,
        "|" => 8,
        "^" => 9,
        "&" => 10,
        "<<" | ">>" => 11,
        "+" | "-" => 12,
        "*" | "/" | "//" | "%" | "@" => 13,
        "**" => 15,
        _ => return None,
    })
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn local_spelling(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) || BUILTINS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn builtin_spelling(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

#[derive(Clone, Copy)]
struct Ctx {
    margin: usize,
    bare: bool,
}

enum Expr {
    Nix(String),
    Infix(Vec<Vec<String>>),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Module,
    Function,
    Class,
    Lambda,
    Comprehension,
}

struct Scope {
    kind: Kind,
    names: Vec<String>,
    globals: Vec<String>,
    nonlocals: Vec<String>,
}

#[derive(Default)]
struct Declared {
    names: Vec<String>,
    globals: Vec<String>,
    nonlocals: Vec<String>,
}

struct Run {
    rows: Vec<Vec<String>>,
    end: Option<usize>,
    broke: bool,
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    scopes: RefCell<Vec<Scope>>,
    skip: Cell<Option<usize>>,
    used: RefCell<BTreeSet<String>>,
}

impl Importer<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn line(&self, at: usize) -> usize {
        line_of(self.source, at)
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!("line {}: {what}", self.line(node.start_byte())))
    }

    fn want(&self, from: usize, to: usize, expected: &str, what: &str) -> Result<(), String> {
        let gap = &self.source[from..to];
        if gap == expected {
            Ok(())
        } else {
            Err(format!(
                "line {}: {what}: {gap:?} where the printer writes {expected:?}",
                self.line(to)
            ))
        }
    }

    fn adjacent(&self, nodes: &[Node<'_>], gaps: &[&str], what: &str) -> Result<(), String> {
        for (pair, gap) in nodes.windows(2).zip(gaps) {
            self.want(pair[0].end_byte(), pair[1].start_byte(), gap, what)?;
        }
        Ok(())
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 4)
    }

    fn c(&self, constructor: &str) -> String {
        self.used.borrow_mut().insert(constructor.to_owned());
        constructor.to_owned()
    }

    fn nix(&self, expr: Expr) -> String {
        match expr {
            Expr::Nix(nix) => nix,
            Expr::Infix(rows) if rows.len() == 1 && rows[0].len() == 1 => rows[0][0].clone(),
            Expr::Infix(rows) if rows.len() == 1 => format!("[{}]", rows[0].join(" ")),
            Expr::Infix(rows) => {
                let rows: Vec<String> = rows.iter().map(|r| row(r)).collect();
                format!("({} [{}])", self.c("rows"), rows.join(" "))
            }
        }
    }

    fn declare_name(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        let name = self.text(node);
        if !identifier(name) {
            return self.refuse(node, &format!("`{name}` is not a name Nix can spell"));
        }
        if !out.iter().any(|known| known == name) {
            out.push(name.to_owned());
        }
        Ok(())
    }

    fn targets(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        match node.kind() {
            "identifier" => self.declare_name(node, out),
            "pattern_list" | "tuple_pattern" | "list_pattern" | "expression_list" | "tuple"
            | "list" | "list_splat_pattern" | "list_splat" | "as_pattern_target" => {
                for child in named(node) {
                    self.targets(child, out)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn alias(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        match node.kind() {
            "as_pattern" => match node.child_by_field_name("alias") {
                Some(target) => self.targets(target, out),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn declare(&self, node: Node<'_>, out: &mut Declared) -> Result<(), String> {
        match node.kind() {
            "expression_statement" => {
                for child in named(node) {
                    if matches!(child.kind(), "assignment" | "augmented_assignment") {
                        if let Some(left) = child.child_by_field_name("left") {
                            self.targets(left, &mut out.names)?;
                        }
                    }
                }
            }
            "import_statement" | "import_from_statement" => {
                let mut cursor = node.walk();
                let names: Vec<Node<'_>> =
                    node.children_by_field_name("name", &mut cursor).collect();
                for name in names {
                    self.declare_name(name, &mut out.names)?;
                }
            }
            "function_definition" | "class_definition" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.declare_name(name, &mut out.names)?;
                }
                return Ok(());
            }
            "for_statement" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.targets(left, &mut out.names)?;
                }
            }
            "if_statement" | "elif_clause" | "while_statement" => {
                if let Some(test) = node.child_by_field_name("condition") {
                    if test.kind() == "named_expression" {
                        if let Some(name) = test.child_by_field_name("name") {
                            self.declare_name(name, &mut out.names)?;
                        }
                    }
                }
            }
            "with_statement" => {
                for clause in named(node) {
                    if clause.kind() != "with_clause" {
                        continue;
                    }
                    for item in named(clause) {
                        for value in named(item) {
                            self.alias(value, &mut out.names)?;
                        }
                    }
                }
            }
            "except_clause" => {
                for value in named(node) {
                    self.alias(value, &mut out.names)?;
                }
            }
            "delete_statement" => {
                for target in named(node) {
                    self.targets(target, &mut out.names)?;
                }
            }
            "global_statement" | "nonlocal_statement" => {
                let list = if node.kind() == "global_statement" {
                    &mut out.globals
                } else {
                    &mut out.nonlocals
                };
                for name in named(node) {
                    self.declare_name(name, list)?;
                }
            }
            _ => {}
        }
        for child in named(node) {
            match child.kind() {
                "block" => {
                    for statement in named(child) {
                        self.declare(statement, out)?;
                    }
                }
                "elif_clause" | "else_clause" | "except_clause" | "finally_clause" => {
                    self.declare(child, out)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn scope(
        &self,
        kind: Kind,
        given: Vec<String>,
        body: &[Node<'_>],
        emit: impl FnOnce(&Self) -> Result<String, String>,
    ) -> Result<String, String> {
        let mut declared = Declared {
            names: given,
            ..Declared::default()
        };
        for statement in body {
            self.declare(*statement, &mut declared)?;
        }
        if kind == Kind::Module && !declared.nonlocals.is_empty() {
            return Err("a nonlocal statement at module level".into());
        }
        let names: Vec<String> = declared
            .names
            .into_iter()
            .filter(|name| !declared.globals.contains(name) && !declared.nonlocals.contains(name))
            .collect();
        let formals: Vec<String> = names.iter().map(|name| local_spelling(name)).collect();
        self.scopes.borrow_mut().push(Scope {
            kind,
            names,
            globals: declared.globals,
            nonlocals: declared.nonlocals,
        });
        let result = emit(self);
        self.scopes.borrow_mut().pop();
        Ok(bind(&formals, result?))
    }

    fn outside<T>(&self, emit: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let saved = self.skip.get();
        let innermost = self.scopes.borrow().len().checked_sub(1);
        self.skip.set(innermost);
        let result = emit();
        self.skip.set(saved);
        result
    }

    fn resolve(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if let Some(inner) = name
            .strip_prefix('@')
            .and_then(|rest| rest.strip_suffix('@'))
        {
            return Ok(format!(
                "({} {})",
                self.c("placeholder"),
                self.string(inner)
            ));
        }
        if !identifier(name) {
            return self.refuse(node, &format!("`{name}` is not a name Nix can spell"));
        }
        let scopes = self.scopes.borrow();
        let skip = self.skip.get();
        let innermost = match skip {
            Some(index) if index + 1 == scopes.len() => index.checked_sub(1),
            _ => scopes.len().checked_sub(1),
        };
        let mut global = false;
        for (index, scope) in scopes.iter().enumerate().rev() {
            let declares = scope.names.iter().any(|known| known == name);
            if Some(index) == skip {
                if declares {
                    return self.refuse(
                        node,
                        &format!("`{name}` is evaluated outside the scope that declares it, which Nix would bind"),
                    );
                }
                continue;
            }
            if global && scope.kind != Kind::Module {
                if declares {
                    return self.refuse(
                        node,
                        &format!("`{name}` is global here and declared by an enclosing function"),
                    );
                }
                continue;
            }
            if scope.globals.iter().any(|known| known == name) {
                global = true;
                continue;
            }
            if scope.nonlocals.iter().any(|known| known == name) {
                continue;
            }
            if scope.kind == Kind::Class && Some(index) != innermost {
                if declares {
                    return self.refuse(
                        node,
                        &format!("`{name}` is a class attribute a nested scope names"),
                    );
                }
                continue;
            }
            if declares {
                return Ok(local_spelling(name));
            }
        }
        if BUILTINS.contains(&name) {
            return Ok(self.c(&builtin_spelling(name)));
        }
        self.refuse(
            node,
            &format!("`{name}` is not declared and not a Python builtin the generator knows"),
        )
    }

    fn statements(&self, nodes: &[Node<'_>], indent: usize) -> Result<String, String> {
        let mut out = String::from("[\n");
        for (index, node) in nodes.iter().enumerate() {
            if index > 0 {
                let gap = &self.source[nodes[index - 1].end_byte()..node.start_byte()];
                let lines = gap.bytes().take_while(|&b| b == b'\n').count();
                if lines == 0 || gap[lines..] != " ".repeat(indent) {
                    return Err(format!(
                        "line {}: statements separated by {gap:?}, not newlines and {indent} spaces",
                        self.line(node.start_byte())
                    ));
                }
                for _ in 1..lines {
                    out.push_str(&self.c("blank"));
                    out.push('\n');
                }
            }
            out.push_str(&self.statement(*node, indent)?);
            out.push('\n');
        }
        out.push(']');
        Ok(out)
    }

    fn block(&self, colon: Node<'_>, block: Node<'_>, indent: usize) -> Result<String, String> {
        if block.kind() != "block" {
            return self.refuse(block, "expected a block");
        }
        let nodes = named(block);
        let Some(first) = nodes.first() else {
            return self.refuse(block, "an empty block");
        };
        self.want(
            colon.end_byte(),
            first.start_byte(),
            &format!("\n{}", " ".repeat(indent + 4)),
            "a block starts on the next line, four spaces in",
        )?;
        self.statements(&nodes, indent + 4)
    }

    fn top(&self, node: Node<'_>, ctx: Ctx) -> Result<String, String> {
        if matches!(node.kind(), "expression_list" | "pattern_list") {
            let items = self.commas(&kids(node), ctx, "a comma list")?;
            Ok(format!("({} [{}])", self.c("commas"), items.join(" ")))
        } else {
            self.value(node, ctx, 2)
        }
    }

    fn test(&self, node: Node<'_>, ctx: Ctx) -> Result<String, String> {
        if node.kind() != "named_expression" {
            return self.value(node, ctx, 1);
        }
        let parts = kids(node);
        self.adjacent(&parts, &[" ", " "], "an assignment expression")?;
        Ok(format!(
            "({} {} {})",
            self.c("walrus"),
            self.resolve(parts[0])?,
            self.value(parts[2], ctx, 2)?
        ))
    }

    fn statement(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let ctx = Ctx {
            margin: indent,
            bare: true,
        };
        let parts = kids(node);
        match node.kind() {
            "comment" => Ok(format!(
                "({} {})",
                self.c("comment"),
                self.string(self.text(node))
            )),
            "function_definition" => {
                self.async_or(&parts, |parts| self.function(node, parts, indent))
            }
            "class_definition" => self.class(node, &parts, indent),
            "if_statement" => {
                self.adjacent(&parts[..3], &[" ", ""], "if header")?;
                let test = self.test(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent)?;
                self.branches(test, body, parts[3], &parts[4..], indent)
            }
            "for_statement" => self.async_or(&parts, |parts| {
                if parts.len() != 6 {
                    return self.refuse(node, "a for with an else");
                }
                self.adjacent(&parts[..5], &[" ", " ", " ", ""], "for header")?;
                let target = self.top(parts[1], ctx)?;
                let iter = self.top(parts[3], ctx)?;
                let body = self.block(parts[4], parts[5], indent)?;
                Ok(format!("({} {target} {iter} {body})", self.c("for")))
            }),
            "while_statement" => {
                if parts.len() != 4 {
                    return self.refuse(node, "a while with an else");
                }
                self.adjacent(&parts[..3], &[" ", ""], "while header")?;
                let test = self.test(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent)?;
                Ok(format!("({} {test} {body})", self.c("while")))
            }
            "with_statement" => self.async_or(&parts, |parts| {
                if parts.len() != 4 || parts[1].kind() != "with_clause" {
                    return self.refuse(node, "an unsupported with");
                }
                self.adjacent(&parts[..3], &[" ", ""], "with header")?;
                let clause = kids(parts[1]);
                let items = self.commas(&clause, ctx, "with items")?;
                let body = self.block(parts[2], parts[3], indent)?;
                Ok(format!(
                    "({} [{}] {body})",
                    self.c("with'"),
                    items.join(" ")
                ))
            }),
            "try_statement" => {
                self.adjacent(&parts[..2], &[""], "try header")?;
                let body = self.block(parts[1], parts[2], indent)?;
                let mut clauses = Vec::new();
                let mut previous = parts[2];
                for clause in &parts[3..] {
                    self.want(
                        previous.end_byte(),
                        clause.start_byte(),
                        &format!("\n{}", " ".repeat(indent)),
                        "a clause on the line after its block",
                    )?;
                    clauses.push(self.clause(*clause, indent)?);
                    previous = *clause;
                }
                Ok(format!(
                    "({} {body} [{}])",
                    self.c("try"),
                    clauses.join(" ")
                ))
            }
            "decorated_definition" => self.refuse(node, "decorators are not supported"),
            "expression_statement" if parts.len() == 1 => self.expression_statement(parts[0], ctx),
            "return_statement" => match parts.as_slice() {
                [_] => Ok(format!("({} null)", self.c("return"))),
                [keyword, value] => {
                    self.adjacent(&[*keyword, *value], &[" "], "return")?;
                    Ok(format!("({} {})", self.c("return"), self.top(*value, ctx)?))
                }
                _ => self.refuse(node, "an unsupported return"),
            },
            "raise_statement" => match parts.as_slice() {
                [_] => Ok(format!("({} null)", self.c("raise"))),
                [keyword, value] => {
                    self.adjacent(&[*keyword, *value], &[" "], "raise")?;
                    Ok(format!(
                        "({} {})",
                        self.c("raise"),
                        self.value(*value, ctx, 2)?
                    ))
                }
                _ => self.refuse(node, "a raise with a cause"),
            },
            "pass_statement" => Ok(self.c("pass")),
            "break_statement" => Ok(self.c("break")),
            "continue_statement" => Ok(self.c("continue")),
            "delete_statement" => {
                self.adjacent(&parts, &[" "], "del")?;
                let targets = match parts[1].kind() {
                    "expression_list" => self.commas(&kids(parts[1]), ctx, "del targets")?,
                    _ => vec![self.value(parts[1], ctx, 2)?],
                };
                Ok(format!("({} [{}])", self.c("del"), targets.join(" ")))
            }
            "global_statement" | "nonlocal_statement" => {
                self.want(parts[0].end_byte(), parts[1].start_byte(), " ", "global")?;
                let names = self.commas(&parts[1..], ctx, "names")?;
                let constructor = if node.kind() == "global_statement" {
                    "global"
                } else {
                    "nonlocal"
                };
                Ok(format!("({} [{}])", self.c(constructor), names.join(" ")))
            }
            "assert_statement" => {
                if parts.len() != 2 {
                    return self.refuse(node, "an assert with a message");
                }
                self.adjacent(&parts, &[" "], "assert")?;
                Ok(format!(
                    "({} {})",
                    self.c("assert'"),
                    self.value(parts[1], ctx, 2)?
                ))
            }
            "import_statement" => {
                if parts.len() != 2
                    || parts[1].kind() != "dotted_name"
                    || named(parts[1]).len() != 1
                {
                    return self.refuse(node, "an import that is not of one top-level module");
                }
                self.adjacent(&parts, &[" "], "import")?;
                Ok(format!(
                    "({} {})",
                    self.c("import'"),
                    self.resolve(parts[1])?
                ))
            }
            "import_from_statement" => self.from(node, &parts),
            kind => self.refuse(node, &format!("the statement {kind} is not supported")),
        }
    }

    fn expression_statement(&self, inner: Node<'_>, ctx: Ctx) -> Result<String, String> {
        let sides = kids(inner);
        match inner.kind() {
            "assignment" => {
                if sides.len() != 3 || sides[1].kind() != "=" {
                    return self.refuse(inner, "an annotated assignment");
                }
                if sides[2].kind() == "assignment" {
                    return self.refuse(inner, "a chained assignment");
                }
                self.adjacent(&sides, &[" ", " "], "an assignment")?;
                Ok(format!(
                    "({} {} {})",
                    self.c("assign"),
                    self.top(sides[0], ctx)?,
                    self.top(sides[2], ctx)?
                ))
            }
            "augmented_assignment" => {
                self.adjacent(&sides, &[" ", " "], "an augmented assignment")?;
                Ok(format!(
                    "({} {} {} {})",
                    self.c("augAssign"),
                    self.top(sides[0], ctx)?,
                    self.string(self.text(sides[1])),
                    self.top(sides[2], ctx)?
                ))
            }
            _ => self.top(inner, ctx),
        }
    }

    fn from(&self, node: Node<'_>, parts: &[Node<'_>]) -> Result<String, String> {
        if parts.len() < 4 || parts[1].kind() != "dotted_name" || parts[2].kind() != "import" {
            return self.refuse(node, "an unsupported from import");
        }
        self.adjacent(&parts[..4], &[" ", " ", " "], "from import")?;
        let module = self.text(parts[1]);
        if module.contains(char::is_whitespace) {
            return self.refuse(node, "a module path with spaces");
        }
        let names = parts[3..]
            .iter()
            .enumerate()
            .map(|(index, part)| {
                if index % 2 == 1 {
                    if part.kind() != "," || 3 + index + 1 >= parts.len() {
                        return self.refuse(*part, "imported names without commas");
                    }
                    self.adjacent(&parts[2 + index..5 + index], &["", " "], "imported names")?;
                    Ok(None)
                } else if part.kind() != "dotted_name" || named(*part).len() != 1 {
                    self.refuse(*part, "an imported name that is not a plain name")
                } else {
                    self.resolve(*part).map(Some)
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        let names: Vec<String> = names.into_iter().flatten().collect();
        Ok(format!(
            "({} {} [{}])",
            self.c("from"),
            self.string(module),
            names.join(" ")
        ))
    }

    fn async_or(
        &self,
        parts: &[Node<'_>],
        statement: impl Fn(&[Node<'_>]) -> Result<String, String>,
    ) -> Result<String, String> {
        if parts.first().is_some_and(|part| part.kind() == "async") {
            self.adjacent(&parts[..2], &[" "], "async")?;
            Ok(format!("({} {})", self.c("async"), statement(&parts[1..])?))
        } else {
            statement(parts)
        }
    }

    fn parameters(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        for param in named(node) {
            match param.kind() {
                "identifier" => self.declare_name(param, out)?,
                "default_parameter" => match param.child_by_field_name("name") {
                    Some(name) if name.kind() == "identifier" => self.declare_name(name, out)?,
                    _ => return self.refuse(param, "an unsupported default parameter"),
                },
                "list_splat_pattern" | "dictionary_splat_pattern" => {
                    match named(param).as_slice() {
                        [name] if name.kind() == "identifier" => self.declare_name(*name, out)?,
                        _ => return self.refuse(param, "a bare star parameter"),
                    }
                }
                kind => {
                    return self.refuse(param, &format!("the parameter {kind} is not supported"))
                }
            }
        }
        Ok(())
    }

    fn function(
        &self,
        node: Node<'_>,
        parts: &[Node<'_>],
        indent: usize,
    ) -> Result<String, String> {
        if parts.len() != 5 {
            return self.refuse(node, "a def with a return annotation");
        }
        self.adjacent(&parts[..4], &[" ", "", ""], "def header")?;
        let name = self.resolve(parts[1])?;
        let mut given = Vec::new();
        self.parameters(parts[2], &mut given)?;
        let body = named(parts[4]);
        let function = self.scope(Kind::Function, given, &body, |importer| {
            let params = importer.seq(
                parts[2],
                Ctx {
                    margin: indent,
                    bare: false,
                },
                false,
            )?;
            let body = importer.block(parts[3], parts[4], indent)?;
            Ok(format!("{{\nparams = {params};\nbody = {body};\n}}"))
        })?;
        Ok(format!("({} {name} {function})", self.c("def")))
    }

    fn class(&self, node: Node<'_>, parts: &[Node<'_>], indent: usize) -> Result<String, String> {
        let (bases, colon) = match parts.len() {
            4 => {
                self.adjacent(&parts[..3], &[" ", ""], "class header")?;
                ("null".to_owned(), 2)
            }
            5 => {
                self.adjacent(&parts[..4], &[" ", "", ""], "class header")?;
                let ctx = Ctx {
                    margin: indent,
                    bare: false,
                };
                (self.seq(parts[2], ctx, false)?, 3)
            }
            _ => return self.refuse(node, "an unsupported class"),
        };
        let name = self.resolve(parts[1])?;
        let nodes = named(parts[colon + 1]);
        let body = self.scope(Kind::Class, Vec::new(), &nodes, |importer| {
            importer.block(parts[colon], parts[colon + 1], indent)
        })?;
        Ok(format!("({} {name} {bases} {body})", self.c("class")))
    }

    fn branches(
        &self,
        test: String,
        body: String,
        previous: Node<'_>,
        rest: &[Node<'_>],
        indent: usize,
    ) -> Result<String, String> {
        let Some(clause) = rest.first() else {
            return Ok(format!("({} {test} {body})", self.c("if'")));
        };
        self.want(
            previous.end_byte(),
            clause.start_byte(),
            &format!("\n{}", " ".repeat(indent)),
            "elif or else on the line after its block",
        )?;
        let parts = kids(*clause);
        let otherwise = match clause.kind() {
            "elif_clause" => {
                self.adjacent(&parts[..3], &[" ", ""], "elif header")?;
                let ctx = Ctx {
                    margin: indent,
                    bare: true,
                };
                let test = self.test(parts[1], ctx)?;
                let body = self.block(parts[2], parts[3], indent)?;
                self.branches(test, body, parts[3], &rest[1..], indent)?
            }
            "else_clause" if rest.len() == 1 => {
                self.adjacent(&parts[..2], &[""], "else header")?;
                self.block(parts[1], parts[2], indent)?
            }
            _ => return self.refuse(*clause, "an unsupported if clause"),
        };
        Ok(format!("({} {test} {body} {otherwise})", self.c("ifElse")))
    }

    fn clause(&self, clause: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = kids(clause);
        match clause.kind() {
            "finally_clause" => {
                self.adjacent(&parts[..2], &[""], "finally header")?;
                Ok(format!(
                    "({} {})",
                    self.c("finally"),
                    self.block(parts[1], parts[2], indent)?
                ))
            }
            "except_clause" => {
                let colon = parts.len() - 2;
                let ctx = Ctx {
                    margin: indent,
                    bare: true,
                };
                let caught = match colon {
                    1 => {
                        self.adjacent(&parts[..2], &[""], "except header")?;
                        "null".to_owned()
                    }
                    2 => {
                        self.adjacent(&parts[..3], &[" ", ""], "except header")?;
                        self.value(parts[1], ctx, 2)?
                    }
                    _ => return self.refuse(clause, "an unsupported except"),
                };
                let body = self.block(parts[colon], parts[colon + 1], indent)?;
                Ok(format!("({} {caught} {body})", self.c("except")))
            }
            kind => self.refuse(clause, &format!("{kind} is not supported")),
        }
    }

    fn commas(&self, parts: &[Node<'_>], ctx: Ctx, what: &str) -> Result<Vec<String>, String> {
        let mut items = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            if index % 2 == 1 {
                if part.kind() != "," {
                    return self.refuse(*part, &format!("{what} without commas"));
                }
                self.want(parts[index - 1].end_byte(), part.start_byte(), "", what)?;
                if index + 1 == parts.len() {
                    return self.refuse(*part, &format!("a trailing comma in {what}"));
                }
                self.want(part.end_byte(), parts[index + 1].start_byte(), " ", what)?;
            } else {
                items.push(self.value(*part, ctx, 2)?);
            }
        }
        Ok(items)
    }

    fn value(&self, node: Node<'_>, ctx: Ctx, need: u8) -> Result<String, String> {
        let expr = self.expr(node, ctx, need)?;
        Ok(self.nix(expr))
    }

    fn level(&self, node: Node<'_>) -> u8 {
        match node.kind() {
            "named_expression" => 1,
            "lambda" => 2,
            "conditional_expression" => 3,
            "not_operator" => 6,
            "comparison_operator" => 7,
            "boolean_operator" | "binary_operator" => node
                .child_by_field_name("operator")
                .and_then(|operator| binary_level(self.text(operator)))
                .unwrap_or(17),
            "unary_operator" => 14,
            "await" => 16,
            "expression_list" | "pattern_list" => 0,
            _ => 17,
        }
    }

    fn chain(&self, node: Node<'_>, ctx: Ctx, run: &mut Run) -> Result<(), String> {
        let parts = kids(node);
        if node.kind() == "comparison_operator" {
            let mut index = 0;
            while index < parts.len() {
                let part = parts[index];
                if part.is_named() {
                    self.operand(part, ctx, 8, run)?;
                    index += 1;
                    continue;
                }
                let mut last = index;
                while last + 1 < parts.len() && !parts[last + 1].is_named() {
                    last += 1;
                }
                self.adjacent(&parts[index..=last], &[" "], "a comparison operator")?;
                let words: Vec<&str> = parts[index..=last].iter().map(|p| self.text(*p)).collect();
                let operator = words.join(" ");
                if !matches!(
                    operator.as_str(),
                    "in" | "not in" | "is" | "is not" | "<" | "<=" | ">" | ">=" | "!=" | "=="
                ) {
                    return self.refuse(part, &format!("`{operator}` is not a comparison"));
                }
                self.piece(
                    part.start_byte(),
                    parts[last].end_byte(),
                    self.string(&operator),
                    ctx,
                    run,
                )?;
                index = last + 1;
            }
            return Ok(());
        }
        if parts.len() != 3 {
            return self.refuse(node, "an unsupported operator");
        }
        let operator = self.text(parts[1]);
        let Some(level) = binary_level(operator) else {
            return self.refuse(parts[1], &format!("`{operator}` is not a binary operator"));
        };
        let (left, right) = if operator == "**" {
            (16, 14)
        } else {
            (level, level + 1)
        };
        self.operand(parts[0], ctx, left, run)?;
        self.piece(
            parts[1].start_byte(),
            parts[1].end_byte(),
            self.string(operator),
            ctx,
            run,
        )?;
        self.operand(parts[2], ctx, right, run)
    }

    fn operand(&self, node: Node<'_>, ctx: Ctx, need: u8, run: &mut Run) -> Result<(), String> {
        if infix(node.kind()) {
            return self.chain(node, ctx, run);
        }
        let inner = Ctx {
            margin: if ctx.bare && run.broke {
                ctx.margin + 4
            } else {
                ctx.margin
            },
            bare: ctx.bare,
        };
        let value = self.value(node, inner, need)?;
        self.piece(node.start_byte(), node.end_byte(), value, ctx, run)
    }

    fn continuation(&self, ctx: Ctx) -> String {
        if ctx.bare {
            format!(" \\\n{}", " ".repeat(ctx.margin + 4))
        } else {
            format!("\n{}", " ".repeat(ctx.margin))
        }
    }

    fn piece(
        &self,
        start: usize,
        end: usize,
        text: String,
        ctx: Ctx,
        run: &mut Run,
    ) -> Result<(), String> {
        if let Some(previous) = run.end {
            let gap = &self.source[previous..start];
            if gap.contains('\n') {
                self.want(
                    previous,
                    start,
                    &self.continuation(ctx),
                    "a continuation line",
                )?;
                run.rows.push(Vec::new());
                run.broke = true;
            } else if gap != " " {
                return Err(format!(
                    "line {}: tokens separated by {gap:?}, not one space",
                    self.line(start)
                ));
            }
        }
        run.end = Some(end);
        if let Some(last) = run.rows.last_mut() {
            last.push(text);
        }
        Ok(())
    }

    fn infix(&self, node: Node<'_>, ctx: Ctx) -> Result<Vec<Vec<String>>, String> {
        let mut run = Run {
            rows: vec![Vec::new()],
            end: None,
            broke: false,
        };
        self.chain(node, ctx, &mut run)?;
        Ok(run.rows)
    }

    fn expr(&self, node: Node<'_>, ctx: Ctx, need: u8) -> Result<Expr, String> {
        let parts = kids(node);
        let flat = Ctx { bare: false, ..ctx };
        Ok(Expr::Nix(match node.kind() {
            "comment" => return self.refuse(node, "a comment inside a statement"),
            "identifier" | "none" => self.resolve(node)?,
            "true" => "true".to_owned(),
            "false" => "false".to_owned(),
            "integer" => self.integer(node)?,
            "string" => self.literal(node, &parts)?,
            "concatenated_string" => self.strings(node, &parts, ctx)?,
            "attribute" => {
                let mut path = Vec::new();
                let mut base = node;
                while base.kind() == "attribute" {
                    let pieces = kids(base);
                    self.adjacent(&pieces, &["", ""], "attribute")?;
                    path.push(self.text(pieces[2]).to_owned());
                    base = pieces[0];
                }
                path.reverse();
                format!(
                    "({} {} {})",
                    self.c("dot"),
                    self.value(base, ctx, 17)?,
                    self.string(&path.join("."))
                )
            }
            "call" => {
                self.adjacent(&parts, &[""], "call")?;
                let function = self.value(parts[0], ctx, 17)?;
                let arguments = match parts[1].kind() {
                    "generator_expression" => {
                        format!("[{}]", self.comprehension(parts[1], "gen")?)
                    }
                    _ => self.seq(parts[1], flat, false)?,
                };
                format!("({} {function} {arguments})", self.c("call"))
            }
            "subscript" => {
                if parts.len() != 4 {
                    return self.refuse(node, "a subscript with several indices");
                }
                self.adjacent(&parts, &["", "", ""], "subscript")?;
                let inner = Ctx {
                    margin: column_of(self.source, parts[1].end_byte()),
                    bare: false,
                };
                let key = if parts[2].kind() == "slice" {
                    self.slice(&kids(parts[2]), inner)?
                } else {
                    self.top(parts[2], inner)?
                };
                format!(
                    "({} {} {key})",
                    self.c("index"),
                    self.value(parts[0], ctx, 17)?
                )
            }
            "unary_operator" => {
                self.adjacent(&parts, &[""], "unary operator")?;
                let negative = nix::int(&format!("-{}", self.text(parts[1])));
                match (self.text(parts[0]), negative) {
                    ("-", Some(int)) if parts[1].kind() == "integer" => int,
                    ("-", _) => format!("({} {})", self.c("neg"), self.value(parts[1], ctx, 14)?),
                    ("~", _) => {
                        format!("({} {})", self.c("invert"), self.value(parts[1], ctx, 14)?)
                    }
                    (other, _) => {
                        return self.refuse(node, &format!("unary {other} is not supported"))
                    }
                }
            }
            "not_operator" => {
                self.adjacent(&parts, &[" "], "not")?;
                format!("({} {})", self.c("not"), self.value(parts[1], ctx, 6)?)
            }
            "await" => {
                self.adjacent(&parts, &[" "], "await")?;
                format!("({} {})", self.c("await"), self.value(parts[1], ctx, 17)?)
            }
            "conditional_expression" => {
                if parts.len() != 5 {
                    return self.refuse(node, "an unsupported conditional expression");
                }
                self.adjacent(&parts, &[" ", " ", " ", " "], "a conditional expression")?;
                format!(
                    "({} {} {} {})",
                    self.c("ternary"),
                    self.value(parts[0], ctx, 4)?,
                    self.value(parts[2], ctx, 4)?,
                    self.value(parts[4], ctx, 3)?
                )
            }
            "named_expression" => {
                return self.refuse(
                    node,
                    "an assignment expression that is not an if or while test",
                )
            }
            "keyword_argument" => {
                self.adjacent(&parts, &["", ""], "keyword argument")?;
                format!(
                    "({} {} {})",
                    self.c("kw"),
                    self.string(self.text(parts[0])),
                    self.value(parts[2], flat, 2)?
                )
            }
            "default_parameter" => {
                self.adjacent(&parts, &["", ""], "default parameter")?;
                let name = self.resolve(parts[0])?;
                let value = self.outside(|| self.value(parts[2], flat, 2))?;
                format!("({} {name} {value})", self.c("default"))
            }
            "list_splat"
            | "list_splat_pattern"
            | "dictionary_splat"
            | "dictionary_splat_pattern" => {
                if parts.len() != 2 {
                    return self.refuse(node, "a bare star");
                }
                self.adjacent(&parts, &[""], "star")?;
                let constructor = if self.text(parts[0]) == "**" {
                    "starstar"
                } else {
                    "star"
                };
                format!(
                    "({} {})",
                    self.c(constructor),
                    self.value(parts[1], flat, 8)?
                )
            }
            "pair" => {
                self.adjacent(&parts, &["", " "], "dict pair")?;
                format!(
                    "({} {} {})",
                    self.c("pair"),
                    self.value(parts[0], flat, 2)?,
                    self.value(parts[2], flat, 2)?
                )
            }
            "lambda" => self.lambda(&parts, flat)?,
            "as_pattern" => {
                self.adjacent(&parts, &[" ", " "], "as")?;
                format!(
                    "({} {} {})",
                    self.c("as"),
                    self.value(parts[0], flat, 2)?,
                    self.value(parts[2], flat, 2)?
                )
            }
            "as_pattern_target" | "with_item" if parts.len() == 1 => {
                return self.expr(parts[0], ctx, need)
            }
            "list" | "list_pattern" => {
                format!("({} {})", self.c("list"), self.seq(node, flat, false)?)
            }
            "tuple" | "tuple_pattern" => {
                format!("({} {})", self.c("tuple"), self.seq(node, flat, true)?)
            }
            "set" => format!("({} {})", self.c("set"), self.seq(node, flat, false)?),
            "dictionary" => format!("({} {})", self.c("dict"), self.seq(node, flat, false)?),
            "list_comprehension" => self.comprehension(node, "listComp")?,
            "set_comprehension" => self.comprehension(node, "setComp")?,
            "dictionary_comprehension" => self.comprehension(node, "dictComp")?,
            "generator_expression" => self.comprehension(node, "gen")?,
            "parenthesized_expression" => return self.paren(node, &parts, ctx, need),
            "expression_list" | "pattern_list" => {
                return self.refuse(
                    node,
                    "a bare tuple where the printer writes a tuple in parentheses",
                )
            }
            kind if infix(kind) => return Ok(Expr::Infix(self.infix(node, ctx)?)),
            kind => return self.refuse(node, &format!("{kind} is not supported")),
        }))
    }

    fn integer(&self, node: Node<'_>) -> Result<String, String> {
        let text = self.text(node);
        if let Some(int) = nix::int(text) {
            return Ok(int);
        }
        match text
            .strip_prefix("0o")
            .and_then(|digits| u64::from_str_radix(digits, 8).ok())
        {
            Some(value) if format!("0o{value:o}") == text => {
                Ok(format!("({} {value})", self.c("octal")))
            }
            _ => self.refuse(
                node,
                &format!("the integer {text} is not spelled as the printer writes it"),
            ),
        }
    }

    fn strings(&self, node: Node<'_>, parts: &[Node<'_>], ctx: Ctx) -> Result<String, String> {
        let flat = Ctx { bare: false, ..ctx };
        let mut lines: Vec<String> = Vec::new();
        let mut broke = false;
        let mut spaced = false;
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                let from = parts[index - 1].end_byte();
                if self.source[from..part.start_byte()].contains('\n') {
                    self.want(
                        from,
                        part.start_byte(),
                        &self.continuation(ctx),
                        "a string on the next line",
                    )?;
                    broke = true;
                } else {
                    self.want(from, part.start_byte(), " ", "adjacent strings")?;
                    spaced = true;
                }
            }
            lines.push(self.value(*part, flat, 17)?);
        }
        if broke && spaced {
            return self.refuse(
                node,
                "adjacent strings broken over lines and sharing a line",
            );
        }
        Ok(if broke {
            format!(
                "({} ({} [{}]))",
                self.c("strings"),
                self.c("rows"),
                lines.join(" ")
            )
        } else {
            format!("({} [{}])", self.c("strings"), lines.join(" "))
        })
    }

    fn lambda(&self, parts: &[Node<'_>], flat: Ctx) -> Result<String, String> {
        let (params, colon) = if parts.len() == 4 {
            self.adjacent(parts, &[" ", "", " "], "lambda")?;
            (Some(parts[1]), 2)
        } else {
            self.adjacent(parts, &["", " "], "lambda")?;
            (None, 1)
        };
        let mut given = Vec::new();
        if let Some(params) = params {
            self.parameters(params, &mut given)?;
        }
        let function = self.scope(Kind::Lambda, given, &[], |importer| {
            let params = match params {
                Some(params) => importer.commas(&kids(params), flat, "lambda parameters")?,
                None => Vec::new(),
            };
            let body = importer.value(parts[colon + 1], flat, 2)?;
            Ok(format!(
                "{{\nparams = [{}];\nbody = {body};\n}}",
                params.join(" ")
            ))
        })?;
        Ok(format!("({} {function})", self.c("lambda")))
    }

    fn paren(
        &self,
        node: Node<'_>,
        parts: &[Node<'_>],
        ctx: Ctx,
        need: u8,
    ) -> Result<Expr, String> {
        if parts.len() != 3 {
            return self.refuse(node, "an unsupported parenthesis");
        }
        let inner = parts[1];
        if self.source[parts[0].end_byte()..inner.start_byte()].contains('\n') {
            let margin = ctx.margin + 4;
            self.adjacent(
                parts,
                &[
                    &format!("\n{}", " ".repeat(margin)),
                    &format!("\n{}", " ".repeat(ctx.margin)),
                ],
                "a hanging parenthesis",
            )?;
            let value = self.value(
                inner,
                Ctx {
                    margin,
                    bare: false,
                },
                0,
            )?;
            return Ok(Expr::Nix(format!("({} {value})", self.c("hang"))));
        }
        self.adjacent(parts, &["", ""], "a parenthesis")?;
        let within = Ctx {
            margin: column_of(self.source, parts[0].end_byte()),
            bare: false,
        };
        if self.level(inner) < need {
            return self.expr(inner, within, 0);
        }
        if infix(inner.kind()) && self.text(inner).contains('\n') {
            let rows: Vec<String> = self.infix(inner, within)?.iter().map(|r| row(r)).collect();
            return Ok(Expr::Nix(format!(
                "({} [{}])",
                self.c("aligned"),
                rows.join(" ")
            )));
        }
        self.refuse(
            node,
            "a parenthesis neither the grouping nor a line break needs",
        )
    }

    fn comprehension(&self, node: Node<'_>, constructor: &str) -> Result<String, String> {
        let parts = kids(node);
        let (open, close) = (parts[0], parts[parts.len() - 1]);
        let inner = &parts[1..parts.len() - 1];
        self.want(
            open.end_byte(),
            inner[0].start_byte(),
            "",
            "a comprehension's opening bracket",
        )?;
        self.want(
            inner[inner.len() - 1].end_byte(),
            close.start_byte(),
            "",
            "a comprehension's closing bracket",
        )?;
        let ctx = Ctx {
            margin: column_of(self.source, open.end_byte()),
            bare: false,
        };
        let mut given = Vec::new();
        for clause in inner {
            if clause.kind() == "for_in_clause" {
                if let Some(left) = clause.child_by_field_name("left") {
                    self.targets(left, &mut given)?;
                }
            }
        }
        let body = self.scope(Kind::Comprehension, given, &[], |importer| {
            let mut rows: Vec<Vec<String>> = vec![Vec::new()];
            let mut first = true;
            for (index, part) in inner.iter().enumerate() {
                if index > 0 {
                    let from = inner[index - 1].end_byte();
                    if importer.source[from..part.start_byte()].contains('\n') {
                        importer.want(
                            from,
                            part.start_byte(),
                            &format!("\n{}", " ".repeat(ctx.margin)),
                            "a comprehension clause on the next line",
                        )?;
                        rows.push(Vec::new());
                    } else {
                        importer.want(from, part.start_byte(), " ", "comprehension clauses")?;
                    }
                }
                let text = importer.clause_part(*part, ctx, first)?;
                if part.kind() == "for_in_clause" {
                    first = false;
                }
                if let Some(last) = rows.last_mut() {
                    last.push(text);
                }
            }
            Ok(if rows.len() == 1 {
                format!("[{}]", rows[0].join(" "))
            } else {
                let rows: Vec<String> = rows.iter().map(|r| row(r)).collect();
                format!("({} [{}])", importer.c("rows"), rows.join(" "))
            })
        })?;
        Ok(format!("({} {body})", self.c(constructor)))
    }

    fn clause_part(&self, part: Node<'_>, ctx: Ctx, first: bool) -> Result<String, String> {
        let pieces = kids(part);
        match part.kind() {
            "for_in_clause" => {
                if pieces.len() != 4 {
                    return self.refuse(part, "an unsupported for clause");
                }
                self.adjacent(&pieces, &[" ", " ", " "], "a for clause")?;
                let target = self.top(pieces[1], ctx)?;
                let iter = if first {
                    self.outside(|| self.value(pieces[3], ctx, 4))?
                } else {
                    self.value(pieces[3], ctx, 4)?
                };
                Ok(format!("({} {target} {iter})", self.c("compFor")))
            }
            "if_clause" => {
                self.adjacent(&pieces, &[" "], "an if clause")?;
                Ok(format!(
                    "({} {})",
                    self.c("compIf"),
                    self.value(pieces[1], ctx, 4)?
                ))
            }
            _ => self.value(part, ctx, 2),
        }
    }

    fn seq(&self, node: Node<'_>, ctx: Ctx, tuple: bool) -> Result<String, String> {
        let parts = kids(node);
        let line = self.line(node.start_byte());
        let (open, close) = match (parts.first(), parts.last()) {
            (Some(open), Some(close)) if parts.len() >= 2 => (*open, *close),
            _ => return Err(format!("line {line}: an unbracketed sequence")),
        };
        let middle = &parts[1..parts.len() - 1];
        if middle.is_empty() {
            self.want(open.end_byte(), close.start_byte(), "", "an empty bracket")?;
            return Ok("[]".into());
        }
        let mut items: Vec<Node<'_>> = Vec::new();
        let mut trailing = false;
        for (index, part) in middle.iter().enumerate() {
            if index % 2 == 1 {
                if part.kind() != "," {
                    return self.refuse(*part, "items without a comma");
                }
                self.want(
                    middle[index - 1].end_byte(),
                    part.start_byte(),
                    "",
                    "a comma",
                )?;
                trailing = index + 1 == middle.len();
            } else {
                if part.kind() == "comment" {
                    return self.refuse(*part, "a comment inside brackets");
                }
                items.push(*part);
            }
        }
        let last_end = middle[middle.len() - 1].end_byte();
        let hang = self.source[open.end_byte()..items[0].start_byte()].contains('\n');
        let column = if hang {
            ctx.margin + 4
        } else {
            column_of(self.source, open.end_byte())
        };
        let inner = Ctx {
            margin: column,
            bare: false,
        };
        let first_gap = if hang {
            format!("\n{}", " ".repeat(column))
        } else {
            String::new()
        };
        self.want(
            open.end_byte(),
            items[0].start_byte(),
            &first_gap,
            "an opening bracket",
        )?;
        let mut rows: Vec<Vec<String>> = vec![Vec::new()];
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                let comma = middle[2 * index - 1];
                let gap = &self.source[comma.end_byte()..item.start_byte()];
                if gap.contains('\n') {
                    self.want(
                        comma.end_byte(),
                        item.start_byte(),
                        &format!("\n{}", " ".repeat(column)),
                        "an item on a new line",
                    )?;
                    rows.push(Vec::new());
                } else {
                    self.want(
                        comma.end_byte(),
                        item.start_byte(),
                        " ",
                        "items after a comma",
                    )?;
                }
            }
            let value = self.value(*item, inner, 2)?;
            if let Some(r) = rows.last_mut() {
                r.push(value);
            }
        }
        let closing = if hang {
            format!("\n{}", " ".repeat(ctx.margin))
        } else {
            String::new()
        };
        self.want(last_end, close.start_byte(), &closing, "a closing bracket")?;
        let expected_trailing = if hang {
            rows.len() > 1
        } else {
            tuple && items.len() == 1
        };
        if trailing != expected_trailing {
            return Err(format!(
                "line {line}: a trailing comma the printer {} write",
                if trailing { "would not" } else { "would" }
            ));
        }
        let rendered: Vec<String> = rows.iter().map(|r| row(r)).collect();
        Ok(if hang {
            format!("({} [{}])", self.c("rows"), rendered.join(" "))
        } else if rows.len() > 1 {
            format!("({} [{}])", self.c("aligned"), rendered.join(" "))
        } else {
            format!("[{}]", rows[0].join(" "))
        })
    }

    fn slice(&self, parts: &[Node<'_>], ctx: Ctx) -> Result<String, String> {
        let colons = parts.iter().filter(|p| p.kind() == ":").count();
        let line = parts.first().map_or(0, |p| self.line(p.start_byte()));
        if colons != 1 {
            return Err(format!("line {line}: a slice with a step"));
        }
        let colon = parts.iter().position(|p| p.kind() == ":").unwrap_or(0);
        let simple = |node: &Node<'_>| matches!(node.kind(), "identifier" | "integer");
        let lower = (colon == 1).then(|| parts[0]);
        let upper = parts.get(colon + 1).copied();
        let spaced = !(lower.as_ref().is_none_or(simple) && upper.as_ref().is_none_or(simple));
        let pad = if spaced { " " } else { "" };
        if let Some(lower) = lower {
            self.want(lower.end_byte(), parts[colon].start_byte(), pad, "a slice")?;
        }
        if let Some(upper) = upper {
            self.want(parts[colon].end_byte(), upper.start_byte(), pad, "a slice")?;
        }
        let bound = |node: Option<Node<'_>>| match node {
            None => Ok("null".to_owned()),
            Some(node) => self.value(node, ctx, 2),
        };
        Ok(format!(
            "({} {} {})",
            self.c("slice"),
            bound(lower)?,
            bound(upper)?
        ))
    }

    fn literal(&self, node: Node<'_>, parts: &[Node<'_>]) -> Result<String, String> {
        let line = self.line(node.start_byte());
        let (start, end) = (parts[0], parts[parts.len() - 1]);
        let opening = self.text(start);
        let prefix: String = opening
            .chars()
            .take_while(char::is_ascii_alphabetic)
            .collect();
        let quote = &opening[prefix.len()..];
        if self.text(end) != quote {
            return Err(format!(
                "line {line}: a string closed differently than it opens"
            ));
        }
        let constructor = match quote {
            "\"" => "str",
            "'" => "sq",
            "\"\"\"" => "triple",
            _ => return Err(format!("line {line}: {quote} strings are not supported")),
        };
        let body = match prefix.as_str() {
            "f" => {
                let mut out = Vec::new();
                let mut from = start.end_byte();
                for part in &parts[1..parts.len() - 1] {
                    if part.kind() != "interpolation" {
                        continue;
                    }
                    if from < part.start_byte() {
                        out.push(self.string(&self.source[from..part.start_byte()]));
                    }
                    out.push(self.field(*part)?);
                    from = part.end_byte();
                }
                if from < end.start_byte() || out.is_empty() {
                    out.push(self.string(&self.source[from..end.start_byte()]));
                }
                format!("[{}]", out.join(" "))
            }
            "" | "r" | "b" => self.string(&self.source[start.end_byte()..end.start_byte()]),
            other => {
                return Err(format!(
                    "line {line}: the string prefix {other} is not supported"
                ))
            }
        };
        let literal = format!("({} {body})", self.c(constructor));
        Ok(match prefix.as_str() {
            "r" => format!("({} {literal})", self.c("raw")),
            "b" => format!("({} {literal})", self.c("bytes")),
            _ => literal,
        })
    }

    fn field(&self, node: Node<'_>) -> Result<String, String> {
        let parts = kids(node);
        let ctx = Ctx {
            margin: column_of(self.source, node.start_byte()) + 1,
            bare: false,
        };
        match parts.as_slice() {
            [open, value, close] => {
                self.adjacent(&[*open, *value, *close], &["", ""], "an interpolation")?;
                Ok(format!(
                    "({} {})",
                    self.c("field"),
                    self.value(*value, ctx, 2)?
                ))
            }
            [open, value, spec, close] if spec.kind() == "format_specifier" => {
                self.adjacent(
                    &[*open, *value, *spec, *close],
                    &["", "", ""],
                    "an interpolation",
                )?;
                let text = self.text(*spec);
                let Some(text) = text.strip_prefix(':').filter(|t| !t.contains(['{', '}'])) else {
                    return self.refuse(node, "an unsupported format specifier");
                };
                Ok(format!(
                    "({} {} {})",
                    self.c("spec"),
                    self.value(*value, ctx, 2)?,
                    self.string(text)
                ))
            }
            _ => self.refuse(node, "an unsupported interpolation"),
        }
    }
}
