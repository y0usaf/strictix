use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use crate::{line_of, nix};

const NIX_NAMES: [&str; 13] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with", "true", "false",
    "null",
];

const CONSTRUCTORS: [&str; 28] = [
    "blank", "comment", "shebang", "inline", "rows", "var", "elements", "dq", "sq", "ansic", "sub",
    "word", "array", "assign", "redir", "heredoc", "test", "pipe", "chain", "fn", "braces", "if'",
    "ifElse", "for", "while", "case", "arm", "render",
];
const FREE: [&str; 15] = [
    "HOME",
    "IFS",
    "LANG",
    "OLDPWD",
    "PATH",
    "PWD",
    "SHELL",
    "TERM",
    "TMPDIR",
    "USER",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "XDG_STATE_HOME",
];

const SPECIALS: [(&str, &str); 4] = [
    ("@", "args"),
    ("*", "argsJoined"),
    ("#", "argCount"),
    ("?", "exitStatus"),
];

const EXPANSIONS: [(&str, &str); 6] = [
    (":-", "default"),
    (":?", "required"),
    ("#", "dropPrefix"),
    ("##", "dropLongestPrefix"),
    ("%", "dropSuffix"),
    ("%%", "dropLongestSuffix"),
];

const UNARY_TESTS: [(&str, &str); 5] = [
    ("-n", "nonEmpty"),
    ("-z", "isEmpty"),
    ("-e", "exists"),
    ("-d", "isDir"),
    ("-f", "isFile"),
];

const BINARY_TESTS: [&str; 8] = ["=", "!=", "-eq", "-ne", "-lt", "-le", "-gt", "-ge"];

const REDIRECTIONS: [&str; 8] = ["<", ">", ">>", "2>", "2>>", ">&", "2>&", "&>"];

const BARE_UNSAFE: &str = "$`\"'\\;&|<>() \t\n";

const OPERAND_UNSAFE: &str = "$`\"'\\}";

fn reserved(name: &str) -> bool {
    NIX_NAMES.contains(&name)
        || CONSTRUCTORS.contains(&name)
        || SPECIALS.iter().any(|(_, nix)| *nix == name)
        || EXPANSIONS.iter().any(|(_, nix)| *nix == name)
        || UNARY_TESTS.iter().any(|(_, nix)| *nix == name)
}

fn spell(name: &str) -> String {
    if reserved(name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn function_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn word_char(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn bare_safe(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('#') && !text.contains(|c| BARE_UNSAFE.contains(c))
}

fn push(list: &mut Vec<String>, name: &str) {
    if !FREE.contains(&name) && !list.iter().any(|one| one == name) {
        list.push(name.to_owned());
    }
}

fn read_option(text: &str) -> bool {
    text.strip_prefix('-')
        .is_some_and(|flags| !flags.is_empty() && flags.chars().all(|c| "rse".contains(c)))
}

#[derive(Default)]
struct Frame {
    function: bool,
    assigned: Vec<String>,
    locals: Vec<String>,
}

enum Part<'t> {
    Literal(String),
    Node(Node<'t>),
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_bash::LANGUAGE.into(), source)?;
    let root = tree.root_node();
    let statements = named(root);
    let first = statements.first().ok_or("the script is empty")?;
    if first.start_byte() != 0 {
        return Err("the script must start with its first statement".into());
    }
    let last = statements.last().ok_or("the script is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the script must end with one newline after its last statement".into());
    }
    let mut importer = Importer {
        source,
        holes,
        used: RefCell::new(BTreeSet::new()),
        functions: BTreeSet::new(),
        globals: Vec::new(),
        locals: BTreeMap::new(),
        scope: RefCell::new(Vec::new()),
        subshell: Cell::new(0),
    };
    let mut top = Frame::default();
    for statement in &statements {
        importer.collect(*statement, &mut top)?;
    }
    importer.globals = top.assigned;
    let lines = importer.lines(&statements, 0)?;
    let list = format!(
        "[\n{}]",
        lines
            .iter()
            .map(|line| format!("  {line}\n"))
            .collect::<String>()
    );
    let block = if importer.globals.is_empty() {
        list
    } else {
        let names: Vec<String> = importer.globals.iter().map(|name| spell(name)).collect();
        format!("({{{}}}: {list})", names.join(", "))
    };
    let used = importer.used.borrow();
    let formals = if used.is_empty() {
        "_".to_owned()
    } else {
        let names: String = used.iter().map(|name| format!("  {name},\n")).collect();
        format!("{{\n{names}  ...\n}}")
    };
    Ok(format!("{formals}: {block}"))
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    used: RefCell<BTreeSet<String>>,
    functions: BTreeSet<String>,
    globals: Vec<String>,
    locals: BTreeMap<usize, Vec<String>>,
    scope: RefCell<Vec<String>>,
    subshell: Cell<usize>,
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

fn quoted_kind(node: Node<'_>) -> bool {
    match node.kind() {
        "string" | "raw_string" | "ansi_c_string" => true,
        "concatenation" => children(node)
            .first()
            .is_some_and(|first| quoted_kind(*first)),
        _ => false,
    }
}

impl<'s> Importer<'s> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn text(&self, node: Node<'_>) -> &'s str {
        &self.source[node.byte_range()]
    }

    fn gap(&self, from: usize, to: usize) -> &'s str {
        &self.source[from..to]
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!(
            "{}: {what}: `{}`",
            self.at(node.start_byte()),
            self.text(node)
        ))
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

    fn call(&self, constructor: &str, arguments: &[String]) -> String {
        format!("({} {})", self.uses(constructor), arguments.join(" "))
    }

    fn collect(&mut self, node: Node<'_>, frame: &mut Frame) -> Result<(), String> {
        match node.kind() {
            "command_substitution" => return Ok(()),
            "function_definition" => {
                if frame.function {
                    return self.refuse(node, "a function defined inside a function");
                }
                let name = node
                    .child_by_field_name("name")
                    .ok_or("a function without a name")?;
                let name = self.text(name);
                if !function_name(name) || FREE.contains(&name) {
                    return self.refuse(node, "a function name Nix cannot spell as a variable");
                }
                self.functions.insert(name.to_owned());
                push(&mut frame.assigned, name);
                let mut inner = Frame {
                    function: true,
                    ..Frame::default()
                };
                for child in children(node) {
                    self.collect(child, &mut inner)?;
                }
                for name in &inner.assigned {
                    if !inner.locals.contains(name) {
                        push(&mut frame.assigned, name);
                    }
                }
                self.locals.insert(node.start_byte(), inner.locals);
                return Ok(());
            }
            "declaration_command" => {
                if token(node, "local").is_none() {
                    return self.refuse(node, "a declaration other than local");
                }
                if !frame.function {
                    return self.refuse(node, "local outside a function");
                }
                for child in named(node) {
                    match child.kind() {
                        "variable_assignment" => {
                            let target = children(child)[0];
                            push(&mut frame.locals, self.text(target));
                        }
                        "variable_name" => push(&mut frame.locals, self.text(child)),
                        _ => return self.refuse(child, "local declares names"),
                    }
                }
            }
            "variable_assignment" => {
                let target = children(node)[0];
                if target.kind() != "variable_name" {
                    return self.refuse(node, "an assignment to something other than a name");
                }
                push(&mut frame.assigned, self.text(target));
            }
            "for_statement" => {
                if let Some(variable) = named(node).first() {
                    push(&mut frame.assigned, self.text(*variable));
                }
            }
            "command" => {
                let parts = children(node);
                let reads = parts
                    .iter()
                    .any(|part| part.kind() == "command_name" && self.text(*part) == "read");
                if reads {
                    for part in &parts {
                        let text = self.text(*part);
                        if part.kind() == "word" && identifier(text) {
                            push(&mut frame.assigned, text);
                        }
                    }
                }
            }
            _ => {}
        }
        for child in children(node) {
            self.collect(child, frame)?;
        }
        Ok(())
    }

    fn local(&self, name: &str) -> Result<String, String> {
        if !self.globals.iter().any(|one| one == name) {
            Ok(spell(name))
        } else if reserved(name) {
            Err(format!(
                "`{name}` is both a script-level name and a local, and a Nix keyword or generator attribute: it has no spelling"
            ))
        } else {
            Ok(format!("{name}'"))
        }
    }

    fn resolve(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if self.scope.borrow().iter().any(|one| one == name) {
            return self.local(name);
        }
        if self.globals.iter().any(|one| one == name) {
            return Ok(spell(name));
        }
        if FREE.contains(&name) {
            return Ok(self.uses(name));
        }
        self.refuse(
            node,
            "a name the script does not declare and generators.shell does not know",
        )
    }

    fn assigning(&self, node: Node<'_>) -> Result<(), String> {
        if self.subshell.get() > 0 {
            self.refuse(node, "a name set inside $(...), which a subshell forgets")
        } else {
            Ok(())
        }
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
                out.push(self.flush(&mut group));
                for _ in 1..lines {
                    out.push(self.uses("blank"));
                }
            }
            group.push(self.statement(*statement, indent)?);
        }
        out.push(self.flush(&mut group));
        Ok(out)
    }

    fn flush(&self, group: &mut Vec<String>) -> String {
        let items = std::mem::take(group);
        if items.len() == 1 {
            items.into_iter().next().unwrap_or_default()
        } else {
            self.call("inline", &[format!("[{}]", items.join(" "))])
        }
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
            return Ok(self.call("inline", &[format!("[{}]", items.join(" "))]));
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
        match node.kind() {
            "comment" => self.comment(node),
            "command" => Ok(format!("[{}]", self.command(node)?.join(" "))),
            "declaration_command" => self.declaration(node),
            "test_command" => self.test(node),
            "variable_assignment" => self.assign(node),
            "redirected_statement" => self.redirected(node, indent),
            "pipeline" => self.joined(node, &["|"], "pipe", indent, Vec::new()),
            "list" => self.joined(node, &["&&", "||"], "chain", indent, Vec::new()),
            "function_definition" => self.function(node, indent),
            "compound_statement" => Ok(self.call("braces", &[self.braces(node, indent)?])),
            "if_statement" => self.conditional(node, indent),
            "for_statement" => self.for_loop(node, indent),
            "while_statement" => self.while_loop(node, indent),
            "case_statement" => self.case(node, indent),
            _ => self.refuse(node, "no shell statement for this"),
        }
    }

    fn comment(&self, node: Node<'_>) -> Result<String, String> {
        let text = self.text(node);
        if node.start_byte() == 0 {
            if let Some(interpreter) = text.strip_prefix("#!") {
                return Ok(self.call("shebang", &[self.string(interpreter)]));
            }
        }
        match text.strip_prefix('#') {
            Some("") => Ok(self.call("comment", &[self.string("")])),
            Some(rest) if rest.starts_with(' ') => {
                Ok(self.call("comment", &[self.string(&rest[1..])]))
            }
            _ => self.refuse(node, "a comment is `#` or `# text`"),
        }
    }

    fn command(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        let mut previous: Option<usize> = None;
        let mut reading = false;
        for child in children(node) {
            match previous {
                Some(end) => self.expect(end, child.start_byte(), " ")?,
                None if child.start_byte() != node.start_byte() => {
                    return self.refuse(node, "a command that starts with a gap")
                }
                None => {}
            }
            previous = Some(child.end_byte());
            out.push(match child.kind() {
                "command_name" => {
                    let [head] = named(child)[..] else {
                        return self.refuse(child, "a command name of several parts");
                    };
                    let text = self.text(head);
                    reading = head.kind() == "word" && text == "read";
                    if head.kind() == "word" && self.functions.contains(text) {
                        self.resolve(head)?
                    } else {
                        self.word(head, None)?
                    }
                }
                "variable_assignment" => self.assign(child)?,
                "file_redirect" => self.redirect(child)?,
                "heredoc_redirect" => self.heredoc(child)?,
                "word" if reading && read_option(self.text(child)) => self.string(self.text(child)),
                "word" if reading && identifier(self.text(child)) => {
                    self.assigning(child)?;
                    self.resolve(child)?
                }
                _ if reading => return self.refuse(child, "read takes -r, -s or -e and names"),
                _ => self.word(child, None)?,
            });
        }
        Ok(out)
    }

    fn declaration(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        if parts.first().map(|part| part.kind()) != Some("local") {
            return self.refuse(node, "a declaration other than local");
        }
        self.assigning(node)?;
        let mut out = vec![self.string("local")];
        for pair in parts.windows(2) {
            self.expect(pair[0].end_byte(), pair[1].start_byte(), " ")?;
            out.push(match pair[1].kind() {
                "variable_assignment" => self.assign(pair[1])?,
                "variable_name" => self.resolve(pair[1])?,
                _ => return self.refuse(pair[1], "local declares names"),
            });
        }
        Ok(format!("[{}]", out.join(" ")))
    }

    fn test(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let [open, expression, close] = parts[..] else {
            return self.refuse(node, "a test is `[ expression ]`");
        };
        if open.kind() != "[" || close.kind() != "]" {
            return self.refuse(node, "a test is `[ expression ]`, not `[[ ... ]]`");
        }
        self.expect(open.end_byte(), expression.start_byte(), " ")?;
        self.expect(expression.end_byte(), close.start_byte(), " ")?;
        let condition = match (expression.kind(), &children(expression)[..]) {
            ("unary_expression", [operator, operand]) => {
                let Some((_, constructor)) = UNARY_TESTS
                    .iter()
                    .find(|(symbol, _)| *symbol == self.text(*operator))
                else {
                    return self.refuse(*operator, "a unary test generators.shell does not know");
                };
                self.expect(operator.end_byte(), operand.start_byte(), " ")?;
                self.call(constructor, &[self.operand_word(*operand)?])
            }
            ("binary_expression", [left, operator, right]) => {
                let symbol = self.text(*operator);
                if !BINARY_TESTS.contains(&symbol) {
                    return self.refuse(*operator, "a binary test generators.shell does not know");
                }
                self.expect(left.end_byte(), operator.start_byte(), " ")?;
                self.expect(operator.end_byte(), right.start_byte(), " ")?;
                format!(
                    "[{} {} {}]",
                    self.operand_word(*left)?,
                    self.string(symbol),
                    self.operand_word(*right)?
                )
            }
            _ => return self.refuse(expression, "a test is one unary or binary comparison"),
        };
        Ok(self.call("test", &[condition]))
    }

    fn operand_word(&self, node: Node<'_>) -> Result<String, String> {
        match (node.kind(), &children(node)[..]) {
            ("unary_expression", [dashes, word])
                if !dashes.is_named()
                    && dashes.end_byte() == word.start_byte()
                    && word.kind() == "word" =>
            {
                self.literal(node)
            }
            _ => self.word(node, None),
        }
    }

    fn literal(&self, node: Node<'_>) -> Result<String, String> {
        let text = self.text(node);
        if let Some(int) = nix::int(text) {
            return Ok(int);
        }
        if bare_safe(text) {
            Ok(self.string(text))
        } else {
            self.refuse(node, "a bare word the printer would not write as it is")
        }
    }

    fn word(&self, node: Node<'_>, next: Option<char>) -> Result<String, String> {
        let text = self.text(node);
        match node.kind() {
            "word" | "number" | "extglob_pattern" => self.literal(node),
            "string" => self.double_quoted(node),
            "raw_string" => Ok(self.call("sq", &[self.string(&text[1..text.len() - 1])])),
            "ansi_c_string" => self.ansi_c(node),
            "simple_expansion" | "expansion" => self.expansion(node, next),
            "command_substitution" => self.substitution(node),
            "concatenation" => {
                let parts = children(node);
                for pair in parts.windows(2) {
                    self.expect(pair[0].end_byte(), pair[1].start_byte(), "")?;
                }
                let mut out = Vec::new();
                for (index, part) in parts.iter().enumerate() {
                    let next = parts
                        .get(index + 1)
                        .filter(|one| matches!(one.kind(), "word" | "number"))
                        .and_then(|one| self.text(*one).chars().next());
                    out.push(match part.kind() {
                        "word" | "number" | "extglob_pattern" => {
                            let text = self.text(*part);
                            if !bare_safe(text) {
                                return self.refuse(
                                    *part,
                                    "a bare word the printer would not write as it is",
                                );
                            }
                            self.string(text)
                        }
                        _ => self.word(*part, next)?,
                    });
                }
                Ok(self.call("word", &[format!("[{}]", out.join(" "))]))
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
                    .map(|i| self.word(*i, None))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(self.call("array", &[format!("[{}]", items.join(" "))]))
            }
            _ => self.refuse(node, "no shell word for this"),
        }
    }

    fn parts(&self, parts: &[Part<'_>]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            out.push(match part {
                Part::Literal(text) => self.string(text),
                Part::Node(node) => {
                    let next = match parts.get(index + 1) {
                        Some(Part::Literal(text)) => text.chars().next(),
                        _ => None,
                    };
                    match node.kind() {
                        "command_substitution" => self.substitution(*node)?,
                        _ => self.expansion(*node, next)?,
                    }
                }
            });
        }
        Ok(out)
    }

    fn double_quoted(&self, node: Node<'_>) -> Result<String, String> {
        let inner = children(node);
        let (open, close) = (inner[0].end_byte(), inner[inner.len() - 1].start_byte());
        let mut parts = Vec::new();
        let mut from = open;
        for part in &inner[1..inner.len() - 1] {
            match part.kind() {
                "string_content" => continue,
                "simple_expansion" | "expansion" | "command_substitution" => {
                    if from < part.start_byte() {
                        parts.push(Part::Literal(self.unquote(from, part.start_byte())?));
                    }
                    parts.push(Part::Node(*part));
                    from = part.end_byte();
                }
                _ => return self.refuse(*part, "no double-quoted part for this"),
            }
        }
        if from < close || parts.is_empty() {
            parts.push(Part::Literal(self.unquote(from, close)?));
        }
        let out = self.parts(&parts)?;
        Ok(match &out[..] {
            [one] => self.call("dq", std::slice::from_ref(one)),
            _ => self.call("dq", &[format!("[{}]", out.join(" "))]),
        })
    }

    fn unquote(&self, from: usize, to: usize) -> Result<String, String> {
        let mut out = String::new();
        let mut chars = self.gap(from, to).chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(escaped @ ('"' | '$' | '\\' | '`')) => out.push(escaped),
                    _ => {
                        return Err(format!(
                            "{}: a backslash in double quotes that escapes nothing",
                            self.at(from)
                        ))
                    }
                },
                '$' | '`' => {
                    return Err(format!(
                        "{}: an unescaped {c} in double quotes that expands nothing",
                        self.at(from)
                    ))
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }

    fn ansi_c(&self, node: Node<'_>) -> Result<String, String> {
        let text = self.text(node);
        let mut out = String::new();
        let mut chars = text[2..text.len() - 1].chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            out.push(match chars.next() {
                Some('t') => '\t',
                Some('n') => '\n',
                Some('\\') => '\\',
                Some('\'') => '\'',
                _ => {
                    return self.refuse(node, "an ANSI-C escape other than \\t, \\n, \\\\ and \\'")
                }
            });
        }
        Ok(self.call("ansic", &[self.string(&out)]))
    }

    fn parameter(&self, node: Node<'_>) -> Result<(String, bool), String> {
        let text = self.text(node);
        match node.kind() {
            "special_variable_name" => match SPECIALS.iter().find(|(symbol, _)| *symbol == text) {
                Some((_, name)) => Ok((self.uses(name), false)),
                None => self.refuse(node, "a special parameter generators.shell does not know"),
            },
            "variable_name" if text.bytes().all(|b| b.is_ascii_digit()) => match nix::int(text) {
                Some(int) => Ok((int, true)),
                None => self.refuse(node, "a positional parameter written with a leading zero"),
            },
            "variable_name" => Ok((self.resolve(node)?, false)),
            _ => self.refuse(node, "no parameter for this"),
        }
    }

    fn expansion(&self, node: Node<'_>, next: Option<char>) -> Result<String, String> {
        let parts = children(node);
        if node.kind() == "simple_expansion" {
            let [_, name] = parts[..] else {
                return self.refuse(node, "an unexpected $ expansion");
            };
            let (parameter, positional) = self.parameter(name)?;
            if positional && next.is_some_and(|c| c.is_ascii_digit()) {
                return self.refuse(node, "a positional parameter the printer braces");
            }
            return Ok(self.call("var", &[parameter]));
        }
        let [open, target, ..] = parts[..] else {
            return self.refuse(node, "an empty ${...}");
        };
        let close = parts[parts.len() - 1];
        if open.kind() != "${" || close.kind() != "}" {
            return self.refuse(node, "an unexpected ${...}");
        }
        if target.kind() == "subscript" {
            let inner = children(target);
            if parts.len() != 3
                || inner.len() != 4
                || inner[0].kind() != "variable_name"
                || self.text(inner[2]) != "@"
            {
                return self.refuse(node, "a subscript other than ${name[@]}");
            }
            return Ok(self.call("elements", &[self.resolve(inner[0])?]));
        }
        let (parameter, positional) = self.parameter(target)?;
        if parts.len() == 3 {
            let name = self.text(target);
            let needed = if positional {
                name.len() > 1 || next.is_some_and(|c| c.is_ascii_digit())
            } else {
                word_char(next) && target.kind() == "variable_name"
            };
            if !needed {
                return self.refuse(node, "braces the printer would not write");
            }
            return Ok(self.call("var", &[parameter]));
        }
        let operator = parts[2];
        let Some((_, constructor)) = EXPANSIONS
            .iter()
            .find(|(symbol, _)| *symbol == self.text(operator))
        else {
            return self.refuse(
                operator,
                "a parameter operation generators.shell does not know",
            );
        };
        let operand = self.operand(
            &parts[3..parts.len() - 1],
            operator.end_byte(),
            close.start_byte(),
        )?;
        Ok(self.call(constructor, &[parameter, operand]))
    }

    fn operand(&self, nodes: &[Node<'_>], from: usize, to: usize) -> Result<String, String> {
        let mut found = Vec::new();
        for node in nodes {
            self.expansions_in(*node, &mut found)?;
        }
        let mut parts = Vec::new();
        let mut at = from;
        for node in found {
            if at < node.start_byte() {
                parts.push(Part::Literal(self.raw(at, node.start_byte())?));
            }
            at = node.end_byte();
            parts.push(Part::Node(node));
        }
        if at < to {
            parts.push(Part::Literal(self.raw(at, to)?));
        }
        let out = self.parts(&parts)?;
        Ok(match (&parts[..], &out[..]) {
            ([], _) => self.string(""),
            ([Part::Literal(text)], _) => nix::int(text).unwrap_or_else(|| self.string(text)),
            (_, [one]) => one.clone(),
            _ => self.call("word", &[format!("[{}]", out.join(" "))]),
        })
    }

    fn raw(&self, from: usize, to: usize) -> Result<String, String> {
        let text = self.gap(from, to);
        if text.contains(|c| OPERAND_UNSAFE.contains(c)) {
            Err(format!(
                "{}: an operand the printer would not write as it is: {text:?}",
                self.at(from)
            ))
        } else {
            Ok(text.to_owned())
        }
    }

    fn expansions_in<'t>(&self, node: Node<'t>, out: &mut Vec<Node<'t>>) -> Result<(), String> {
        match node.kind() {
            "simple_expansion" | "expansion" | "command_substitution" => out.push(node),
            "string" | "raw_string" | "ansi_c_string" | "arithmetic_expansion" => {
                return self.refuse(node, "a quoted or arithmetic operand")
            }
            _ => {
                for child in children(node) {
                    self.expansions_in(child, out)?;
                }
            }
        }
        Ok(())
    }

    fn substitution(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        if parts[0].kind() != "$(" {
            return self.refuse(node, "only $(...) substitution is supported");
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
        self.subshell.set(self.subshell.get() + 1);
        let items = statements
            .iter()
            .map(|s| self.statement(*s, 0))
            .collect::<Result<Vec<_>, _>>();
        self.subshell.set(self.subshell.get() - 1);
        Ok(self.call("sub", &[format!("[{}]", items?.join(" "))]))
    }

    fn assign(&self, node: Node<'_>) -> Result<String, String> {
        self.assigning(node)?;
        let parts = children(node);
        let target = parts[0];
        let equals = parts.get(1).filter(|p| p.kind() == "=");
        let Some(equals) = equals else {
            return self.refuse(node, "only = assignments are supported");
        };
        self.expect(target.end_byte(), equals.start_byte(), "")?;
        let value = match parts.get(2) {
            Some(value) => {
                self.expect(equals.end_byte(), value.start_byte(), "")?;
                self.word(*value, None)?
            }
            None => self.string(""),
        };
        Ok(self.call("assign", &[self.resolve(target)?, value]))
    }

    fn redirect(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let target = parts[parts.len() - 1];
        let operator = self.gap(node.start_byte(), parts[parts.len() - 2].end_byte());
        if !REDIRECTIONS.contains(&operator) {
            return self.refuse(node, "a redirection generators.shell does not know");
        }
        let gap = self.gap(parts[parts.len() - 2].end_byte(), target.start_byte());
        let spaced = if quoted_kind(target) { " " } else { "" };
        if gap != spaced {
            return Err(format!(
                "{}: a redirect spaced {gap:?} where the printer writes {spaced:?}",
                self.at(node.start_byte())
            ));
        }
        Ok(self.call("redir", &[self.string(operator), self.word(target, None)?]))
    }

    fn heredoc(&self, node: Node<'_>) -> Result<String, String> {
        let parts = children(node);
        let kinds: Vec<&str> = parts.iter().map(|p| p.kind()).collect();
        if kinds.first() != Some(&"<<")
            || kinds.get(1) != Some(&"heredoc_start")
            || kinds.last() != Some(&"heredoc_end")
            || kinds.len() != 4
        {
            return self.refuse(node, "only a plain <<'DELIMITER' heredoc is supported");
        }
        self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
        let start = self.text(parts[1]);
        let Some(delimiter) = start
            .strip_prefix('\'')
            .and_then(|rest| rest.strip_suffix('\''))
        else {
            return self.refuse(
                parts[1],
                "a heredoc whose delimiter is not single-quoted expands its body",
            );
        };
        let end = parts[parts.len() - 1];
        if self.text(end) != delimiter {
            return self.refuse(end, "the heredoc ends with a different delimiter");
        }
        if self.gap(parts[1].end_byte(), parts[1].end_byte() + 1) != "\n" {
            return self.refuse(parts[1], "a heredoc must end its line");
        }
        let body = self.gap(parts[1].end_byte() + 1, end.start_byte());
        if !body.is_empty() && !body.ends_with('\n') {
            return self.refuse(end, "a heredoc body must end with a newline");
        }
        Ok(self.call("heredoc", &[self.string(delimiter), self.string(body)]))
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
                "file_redirect" => self.redirect(*redirect)?,
                _ => return self.refuse(*redirect, "a redirection generators.shell does not know"),
            });
            previous = redirect.end_byte();
        }
        match command.kind() {
            "command" => {
                let mut words = self.command(command)?;
                words.extend(redirects);
                Ok(format!("[{}]", words.join(" ")))
            }
            "pipeline" => self.joined(command, &["|"], "pipe", indent, redirects),
            _ => self.refuse(
                node,
                "a redirection of something other than a command or pipeline",
            ),
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
                return self.refuse(*item, "an operator generators.shell does not join with");
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
            self.call(name, &[self.call("rows", &[list])])
        } else {
            self.call(name, &[list])
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
            return self.refuse(node, "only name() { ... } functions are supported");
        }
        self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
        self.expect(parts[2].end_byte(), parts[3].start_byte(), " ")?;
        let name = self.resolve(parts[0])?;
        let Some(locals) = self.locals.get(&node.start_byte()) else {
            return self.refuse(
                node,
                "a function the script does not define at its top level",
            );
        };
        let outer = self.scope.replace(locals.clone());
        let body = self.braces(parts[3], indent);
        self.scope.replace(outer);
        let body = body?;
        if locals.is_empty() {
            return Ok(self.call("fn", &[name, body]));
        }
        let names = locals
            .iter()
            .map(|local| self.local(local))
            .collect::<Result<Vec<_>, _>>()?;
        let body = body
            .strip_prefix('(')
            .and_then(|inner| inner.strip_suffix(')'))
            .filter(|inner| inner.starts_with("inline "))
            .unwrap_or(&body);
        Ok(self.call("fn", &[name, format!("({{{}}}: {body})", names.join(", "))]))
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
        let [test] = condition[..] else {
            return self.refuse(node, "an if needs exactly one condition statement");
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
        let test = self.statement(test, indent)?;
        let body = self.body(&body, parts[then].end_byte(), close, indent, "; ", indent)?;
        match alternative {
            None => Ok(self.call("if'", &[test, body])),
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
                Ok(self.call("ifElse", &[test, body, otherwise]))
            }
            Some(index) => self.refuse(parts[index], "elif is not supported"),
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
        if kinds.get(2) != Some(&"in") || kinds.get(1) != Some(&"variable_name") {
            return self.refuse(node, "only for NAME in WORDS loops are supported");
        }
        self.assigning(node)?;
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
        let name = self.resolve(parts[1])?;
        let words = words
            .iter()
            .map(|w| self.word(*w, None))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.call(
            "for",
            &[
                name,
                format!("[{}]", words.join(" ")),
                self.do_group(*body, indent)?,
            ],
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
        let [test] = condition[..] else {
            return self.refuse(node, "a while needs exactly one condition statement");
        };
        self.expect(parts[0].end_byte(), test.start_byte(), " ")?;
        self.expect(test.end_byte(), body.start_byte(), "; ")?;
        Ok(self.call(
            "while",
            &[self.statement(test, indent)?, self.do_group(*body, indent)?],
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
            return self.refuse(node, "unexpected case content");
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
        let subject = self.word(*subject, None)?;
        Ok(if inline {
            self.call("case", &[subject, self.call("inline", &[arms])])
        } else {
            self.call("case", &[subject, arms])
        })
    }

    fn arm(&self, node: Node<'_>, indent: usize) -> Result<String, String> {
        let parts = children(node);
        let close = parts
            .iter()
            .position(|p| p.kind() == ")")
            .ok_or("a case arm without )")?;
        let Some(end) = parts.last().filter(|p| p.kind() == ";;") else {
            return self.refuse(node, "a case arm must end with ;;");
        };
        let patterns: Vec<_> = parts[..close]
            .iter()
            .filter(|p| p.kind() != "|")
            .copied()
            .collect();
        for pair in parts[..close].windows(2) {
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
            .map(|p| self.word(*p, None))
            .collect::<Result<Vec<_>, _>>()?;
        let patterns = if patterns.len() == 1 {
            patterns[0].clone()
        } else {
            format!("[{}]", patterns.join(" "))
        };
        Ok(self.call("arm", &[patterns, body]))
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
