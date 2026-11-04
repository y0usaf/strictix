use std::cell::RefCell;
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::nix;

const NIX_KEYWORDS: [&str; 10] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
];

const CONSTRUCTORS: [&str; 20] = [
    "assign",
    "blank",
    "byRef",
    "byVal",
    "call",
    "comment",
    "dim",
    "dot",
    "function",
    "if'",
    "ifElse",
    "neg",
    "not",
    "onErrorGoTo0",
    "onErrorResumeNext",
    "optionExplicit",
    "render",
    "set",
    "str",
    "sub",
];

const GLOBALS: [&str; 29] = [
    "CBool",
    "CInt",
    "CLng",
    "CStr",
    "Chr",
    "CreateObject",
    "Empty",
    "Err",
    "GetObject",
    "InStr",
    "IsEmpty",
    "IsNull",
    "IsObject",
    "Join",
    "LCase",
    "Left",
    "Len",
    "Mid",
    "MsgBox",
    "Nothing",
    "Now",
    "Null",
    "Replace",
    "Right",
    "Split",
    "Trim",
    "UBound",
    "UCase",
    "WScript",
];

const NOT: u8 = 6;

const NEG: u8 = 13;

const ATOM: u8 = 15;

fn precedence(operator: &str) -> Option<u8> {
    Some(match operator {
        "Imp" => 1,
        "Eqv" => 2,
        "Xor" => 3,
        "Or" => 4,
        "And" => 5,
        "=" | "<>" | "<" | ">" | "<=" | ">=" | "Is" => 7,
        "&" => 8,
        "+" | "-" => 9,
        "Mod" => 10,
        "\\" => 11,
        "*" | "/" => 12,
        "^" => 14,
        _ => return None,
    })
}

fn spelled(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) || GLOBALS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let importer = Importer {
        source: "",
        holes,
        scopes: RefCell::new(Vec::new()),
        used: RefCell::new(BTreeSet::new()),
    };
    let mut statements = Vec::new();
    let mut offset = 0;
    let mut header = 0;
    for line in source.split_inclusive('\n') {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if !line.ends_with('\n') || !(bare.is_empty() || bare.starts_with('\'')) {
            break;
        }
        header += 1;
        statements.push(if bare.is_empty() {
            Statement::Blank
        } else {
            Statement::Comment(
                comment(bare)
                    .ok_or_else(|| format!("line {header}: a comment other than `' text`"))?,
            )
        });
        offset += line.len();
    }
    let body = &source[offset..];
    let mut bound = Vec::new();
    if !body.is_empty() {
        let tree =
            crate::parse(&tree_sitter_vbscript::language(), body).map_err(|e| shift(&e, header))?;
        let importer = Importer {
            source: body,
            ..importer
        };
        let nodes = named(tree.root_node());
        importer
            .declared(&nodes, &mut bound, true)
            .map_err(|e| shift(&e, header))?;
        importer.scopes.borrow_mut().push(bound.clone());
        statements.extend(
            importer
                .sequence(&nodes, None)
                .map_err(|e| shift(&e, header))?,
        );
        return finish(&importer, source, &statements, &bound);
    }
    finish(&importer, source, &statements, &bound)
}

fn finish(
    importer: &Importer<'_>,
    source: &str,
    statements: &[Statement],
    bound: &[String],
) -> Result<String, String> {
    if statements.is_empty() {
        return Err("the script is empty".into());
    }
    let mut text = String::new();
    for statement in statements {
        for line in lines(statement, 0) {
            text.push_str(&line);
            text.push('\n');
        }
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
    let script = importer.scoped(bound, &importer.block(statements, "  "));
    let used = importer.used.borrow();
    let formals = if used.is_empty() {
        "_".to_owned()
    } else {
        format!(
            "{{\n{}  ...\n}}",
            used.iter()
                .map(|name| format!("  {name},\n"))
                .collect::<String>()
        )
    };
    Ok(format!("{formals}: {script}"))
}

fn comment(text: &str) -> Option<String> {
    if text == "'" {
        Some(String::new())
    } else {
        text.strip_prefix("' ").map(str::to_owned)
    }
}

fn shift(error: &str, by: usize) -> String {
    error
        .strip_prefix("line ")
        .and_then(|rest| rest.split_once(':'))
        .and_then(|(line, rest)| Some(format!("line {}:{rest}", line.parse::<usize>().ok()? + by)))
        .unwrap_or_else(|| error.to_owned())
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn grouped(node: Node<'_>) -> bool {
    node.prev_sibling().is_some_and(|open| open.kind() == "(")
        && node.next_sibling().is_some_and(|close| close.kind() == ")")
}

struct Name {
    vbs: String,
    nix: String,
}

enum Expression {
    Int(i64),
    Bool(bool),
    Name(Name),
    Str(String),
    Dot(Box<Expression>, String),
    Call(Box<Expression>, Vec<Expression>),
    Not(Box<Expression>),
    Neg(Box<Expression>),
    Binary(String, Box<Expression>, Box<Expression>),
}

enum Mode {
    Plain,
    ByVal,
    ByRef,
}

enum Statement {
    Blank,
    Comment(String),
    OptionExplicit,
    OnErrorResumeNext,
    OnErrorGoTo0,
    Dim(Vec<Name>),
    Assign(Expression, Expression),
    Set(Expression, Expression),
    Call(Expression, Vec<Expression>),
    If(Expression, Vec<Statement>, Option<Vec<Statement>>),
    Procedure {
        keyword: &'static str,
        name: Name,
        parameters: Vec<(Mode, Name)>,
        bound: Vec<String>,
        body: Vec<Statement>,
    },
}

enum Item {
    Operand(Expression),
    Prefix(&'static str),
    Operator(String),
}

fn written(least: u8, expression: &Expression) -> String {
    let (precedence, text) = printed(expression);
    if precedence < least {
        format!("({text})")
    } else {
        text
    }
}

fn listed(arguments: &[Expression]) -> String {
    arguments
        .iter()
        .map(|argument| written(0, argument))
        .collect::<Vec<_>>()
        .join(", ")
}

fn printed(expression: &Expression) -> (u8, String) {
    match expression {
        Expression::Int(value) => (ATOM, value.to_string()),
        Expression::Bool(true) => (ATOM, "True".into()),
        Expression::Bool(false) => (ATOM, "False".into()),
        Expression::Name(name) => (ATOM, name.vbs.clone()),
        Expression::Str(value) => (ATOM, format!("\"{}\"", value.replace('"', "\"\""))),
        Expression::Dot(base, path) => (ATOM, format!("{}.{path}", written(ATOM, base))),
        Expression::Call(function, arguments) => (
            ATOM,
            format!("{}({})", written(ATOM, function), listed(arguments)),
        ),
        Expression::Not(operand) => (NOT, format!("Not {}", written(NOT, operand))),
        Expression::Neg(operand) => (NEG, format!("-{}", written(NEG, operand))),
        Expression::Binary(operator, left, right) => {
            let level = precedence(operator).unwrap_or(0);
            (
                level,
                format!(
                    "{} {operator} {}",
                    written(level, left),
                    written(level + 1, right)
                ),
            )
        }
    }
}

fn lines(statement: &Statement, depth: usize) -> Vec<String> {
    let tabs = "\t".repeat(depth);
    let line = |text: String| vec![format!("{tabs}{text}")];
    let body = |statements: &[Statement]| {
        statements
            .iter()
            .flat_map(|statement| lines(statement, depth + 1))
            .collect::<Vec<_>>()
    };
    match statement {
        Statement::Blank => vec![String::new()],
        Statement::Comment(text) if text.is_empty() => line("'".into()),
        Statement::Comment(text) => line(format!("' {text}")),
        Statement::OptionExplicit => line("Option Explicit".into()),
        Statement::OnErrorResumeNext => line("On Error Resume Next".into()),
        Statement::OnErrorGoTo0 => line("On Error GoTo 0".into()),
        Statement::Dim(names) => line(format!(
            "Dim {}",
            names
                .iter()
                .map(|name| name.vbs.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Statement::Assign(target, value) => {
            line(format!("{} = {}", written(0, target), written(0, value)))
        }
        Statement::Set(target, value) => line(format!(
            "Set {} = {}",
            written(0, target),
            written(0, value)
        )),
        Statement::Call(function, arguments) if arguments.is_empty() => {
            line(written(ATOM, function))
        }
        Statement::Call(function, arguments) => {
            line(format!("{} {}", written(ATOM, function), listed(arguments)))
        }
        Statement::If(test, then, otherwise) => {
            let mut out = line(format!("If {} Then", written(0, test)));
            out.extend(body(then));
            if let Some(otherwise) = otherwise {
                out.push(format!("{tabs}Else"));
                out.extend(body(otherwise));
            }
            out.push(format!("{tabs}End If"));
            out
        }
        Statement::Procedure {
            keyword,
            name,
            parameters,
            body: statements,
            ..
        } => {
            let parameters: Vec<_> = parameters
                .iter()
                .map(|(mode, name)| {
                    let prefix = match mode {
                        Mode::Plain => "",
                        Mode::ByVal => "ByVal ",
                        Mode::ByRef => "ByRef ",
                    };
                    format!("{prefix}{}", name.vbs)
                })
                .collect();
            let mut out = line(format!("{keyword} {}({})", name.vbs, parameters.join(", ")));
            out.extend(body(statements));
            out.push(format!("{tabs}End {keyword}"));
            out
        }
    }
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    scopes: RefCell<Vec<Vec<String>>>,
    used: RefCell<BTreeSet<String>>,
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

    fn uses(&self, name: &str) -> &str {
        self.used.borrow_mut().insert(name.to_owned());
        CONSTRUCTORS
            .iter()
            .chain(GLOBALS.iter())
            .find(|known| **known == name)
            .copied()
            .unwrap_or("")
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!("{}: {what}: `{}`", self.at(node), self.text(node)))
    }

    fn unsupported<T>(&self, node: Node<'_>) -> Result<T, String> {
        self.refuse(node, &format!("{} is not supported", node.kind()))
    }

    fn identifier(&self, node: Node<'_>) -> Result<String, String> {
        let node = if node.kind() == "new_identifier" {
            match named(node)[..] {
                [inner] => inner,
                _ => return self.unsupported(node),
            }
        } else {
            node
        };
        let text = self.text(node);
        let mut bytes = text.bytes();
        if node.kind() != "identifier"
            || !bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
            || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return self.refuse(node, "a name Nix cannot spell");
        }
        Ok(text.to_owned())
    }

    fn declare(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        let name = self.identifier(node)?;
        if out.contains(&name) {
            return self.refuse(node, "a name declared twice");
        }
        out.push(name);
        Ok(())
    }

    fn dim_names<'t>(&self, node: Node<'t>) -> Result<Vec<Node<'t>>, String> {
        let [list] = named(node)[..] else {
            return self.unsupported(node);
        };
        if list.kind() != "variable_list" {
            return self.unsupported(list);
        }
        named(list)
            .into_iter()
            .map(|name| match named(name)[..] {
                [inner] if name.kind() == "variable_declaration_identifier" => Ok(inner),
                _ => self.unsupported(name),
            })
            .collect()
    }

    fn declared(
        &self,
        nodes: &[Node<'_>],
        out: &mut Vec<String>,
        script: bool,
    ) -> Result<(), String> {
        for node in nodes {
            match node.kind() {
                "variable_declaration" => {
                    for name in self.dim_names(*node)? {
                        self.declare(name, out)?;
                    }
                }
                "subroutine" | "function" if script => {
                    let Some(name) = named(*node).first().copied() else {
                        return self.unsupported(*node);
                    };
                    self.declare(name, out)?;
                }
                "if_statement" => {
                    self.declared(named(*node).get(1..).unwrap_or_default(), out, script)?
                }
                "else_clause" => self.declared(&named(*node), out, script)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn name(&self, node: Node<'_>) -> Result<Name, String> {
        let vbs = self.identifier(node)?;
        if self
            .scopes
            .borrow()
            .iter()
            .any(|scope| scope.contains(&vbs))
        {
            return Ok(Name {
                nix: spelled(&vbs),
                vbs,
            });
        }
        if GLOBALS.contains(&vbs.as_str()) {
            self.uses(&vbs);
            return Ok(Name {
                nix: vbs.clone(),
                vbs,
            });
        }
        self.refuse(
            node,
            "a name that nothing declares and that is not a VBScript or WScript global the generator knows",
        )
    }

    fn sequence(
        &self,
        nodes: &[Node<'_>],
        mut row: Option<usize>,
    ) -> Result<Vec<Statement>, String> {
        let mut out = Vec::new();
        for node in nodes {
            if let Some(previous) = row {
                for _ in previous + 1..node.start_position().row {
                    out.push(Statement::Blank);
                }
            }
            out.push(self.statement(*node)?);
            row = Some(node.end_position().row);
        }
        Ok(out)
    }

    fn statement(&self, node: Node<'_>) -> Result<Statement, String> {
        let text = self.text(node);
        match node.kind() {
            "comment" => comment(text)
                .map(Statement::Comment)
                .map_or_else(|| self.refuse(node, "a comment other than `' text`"), Ok),
            "option_directive" if text == "Option Explicit" => Ok(Statement::OptionExplicit),
            "on_error_statement" if text == "On Error Resume Next" => {
                Ok(Statement::OnErrorResumeNext)
            }
            "on_error_statement" if text == "On Error GoTo 0" => Ok(Statement::OnErrorGoTo0),
            "variable_declaration" => Ok(Statement::Dim(
                self.dim_names(node)?
                    .into_iter()
                    .map(|name| self.name(name))
                    .collect::<Result<_, _>>()?,
            )),
            "variable_assignment" => {
                let [target, value] = named(node)[..] else {
                    return self.unsupported(node);
                };
                if target.kind() != "identifier" {
                    return self.unsupported(target);
                }
                Ok(Statement::Assign(
                    Expression::Name(self.name(target)?),
                    self.expression(value)?,
                ))
            }
            "invocation_statement" => self.invocation(node),
            "if_statement" => self.conditional(node),
            "subroutine" => self.procedure(node, "Sub"),
            "function" => self.procedure(node, "Function"),
            _ => self.unsupported(node),
        }
    }

    fn invocation(&self, node: Node<'_>) -> Result<Statement, String> {
        let parts = named(node);
        let (head, arguments) = match parts[..] {
            [head] => (head, None),
            [head, arguments] if arguments.kind() == "argument_list" => (head, Some(arguments)),
            _ => return self.unsupported(node),
        };
        if head.kind() == "identifier" && self.text(head) == "Set" {
            let assignment = arguments
                .map(named)
                .and_then(|list| match list[..] {
                    [argument] => named(argument).first().copied(),
                    _ => None,
                })
                .filter(|binary| binary.kind() == "binary_expression");
            let Some(binary) = assignment else {
                return self.refuse(node, "a Set other than `Set target = value`");
            };
            let [target, value] = named(binary)[..] else {
                return self.unsupported(binary);
            };
            if self.source[target.end_byte()..value.start_byte()].trim() != "=" {
                return self.refuse(node, "a Set other than `Set target = value`");
            }
            let target = match target.kind() {
                "identifier" => Expression::Name(self.name(target)?),
                "member_expression" => self.expression(target)?,
                _ => return self.unsupported(target),
            };
            if !matches!(target, Expression::Name(_) | Expression::Dot(..)) {
                return self.refuse(node, "a Set whose target is not a name or a member");
            }
            return Ok(Statement::Set(target, self.expression(value)?));
        }
        let function = self.expression(head)?;
        if !matches!(function, Expression::Name(_) | Expression::Dot(..)) {
            return self.refuse(node, "a statement call whose arguments are in parentheses");
        }
        let arguments = match arguments {
            Some(list) => self.arguments(list)?,
            None => Vec::new(),
        };
        Ok(Statement::Call(function, arguments))
    }

    fn conditional(&self, node: Node<'_>) -> Result<Statement, String> {
        let parts = named(node);
        let Some(test) = parts.first() else {
            return self.unsupported(node);
        };
        let mut body = Vec::new();
        let mut otherwise = None;
        for part in &parts[1..] {
            match part.kind() {
                "else_clause" => otherwise = Some(*part),
                "elseif_clause" => return self.unsupported(*part),
                _ => body.push(*part),
            }
        }
        let then = self.sequence(&body, Some(test.end_position().row))?;
        let otherwise = match otherwise {
            Some(clause) => Some(self.sequence(&named(clause), Some(clause.start_position().row))?),
            None => None,
        };
        Ok(Statement::If(self.expression(*test)?, then, otherwise))
    }

    fn procedure(&self, node: Node<'_>, keyword: &'static str) -> Result<Statement, String> {
        let parts = children(node);
        let Some(close) = parts.iter().position(|part| part.kind() == ")") else {
            return self.unsupported(node);
        };
        let header = &parts[..close];
        let name = match header.iter().find(|part| part.kind() == "new_identifier") {
            Some(name) => self.name(*name)?,
            None => return self.unsupported(node),
        };
        if header.iter().any(|part| {
            part.is_named() && !matches!(part.kind(), "new_identifier" | "parameter_list")
        }) {
            return self.refuse(node, "a procedure with a visibility or a return type");
        }
        let mut bound = Vec::new();
        let mut modes = Vec::new();
        if let Some(list) = header.iter().find(|part| part.kind() == "parameter_list") {
            for parameter in named(*list) {
                let (mode, identifier) =
                    match named(parameter)[..] {
                        [identifier] => (Mode::Plain, identifier),
                        [modifier, identifier] if modifier.kind() == "modifier" => {
                            match self.text(modifier) {
                                "ByVal" => (Mode::ByVal, identifier),
                                "ByRef" => (Mode::ByRef, identifier),
                                _ => return self.refuse(
                                    parameter,
                                    "a parameter other than `name`, `ByVal name` or `ByRef name`",
                                ),
                            }
                        }
                        _ => {
                            return self.refuse(
                                parameter,
                                "a parameter other than `name`, `ByVal name` or `ByRef name`",
                            )
                        }
                    };
                self.declare(identifier, &mut bound)?;
                modes.push((mode, identifier));
            }
        }
        let statements: Vec<_> = parts[close + 1..]
            .iter()
            .copied()
            .filter(|part| part.is_named())
            .collect();
        self.declared(&statements, &mut bound, false)?;
        self.scopes.borrow_mut().push(bound.clone());
        let parameters = modes
            .into_iter()
            .map(|(mode, identifier)| Ok((mode, self.name(identifier)?)))
            .collect::<Result<Vec<_>, String>>();
        let body = self.sequence(&statements, Some(parts[close].end_position().row));
        self.scopes.borrow_mut().pop();
        Ok(Statement::Procedure {
            keyword,
            name,
            parameters: parameters?,
            bound,
            body: body?,
        })
    }

    fn arguments(&self, node: Node<'_>) -> Result<Vec<Expression>, String> {
        named(node)
            .into_iter()
            .map(|argument| match named(argument)[..] {
                [value] if argument.kind() == "argument" && value.kind() != "keyword_argument" => {
                    self.expression(value)
                }
                _ => self.unsupported(argument),
            })
            .collect()
    }

    fn expression(&self, node: Node<'_>) -> Result<Expression, String> {
        let mut items = Vec::new();
        self.flatten(node, &mut items)?;
        let mut items = items.into_iter().peekable();
        let expression = parse(&mut items, 0).ok_or_else(|| {
            format!(
                "{}: an expression VBScript's precedence does not read",
                self.at(node)
            )
        })?;
        if items.next().is_some() {
            return self.refuse(node, "an expression VBScript's precedence does not read");
        }
        Ok(expression)
    }

    fn operand(&self, node: Node<'_>, items: &mut Vec<Item>) -> Result<(), String> {
        if grouped(node) {
            items.push(Item::Operand(self.expression(node)?));
            Ok(())
        } else {
            self.flatten(node, items)
        }
    }

    fn flatten(&self, node: Node<'_>, items: &mut Vec<Item>) -> Result<(), String> {
        match node.kind() {
            "binary_expression" => {
                let [left, right] = named(node)[..] else {
                    return self.unsupported(node);
                };
                let from = left
                    .next_sibling()
                    .filter(|close| close.kind() == ")" && grouped(left))
                    .map_or(left.end_byte(), |close| close.end_byte());
                let to = right
                    .prev_sibling()
                    .filter(|open| open.kind() == "(" && grouped(right))
                    .map_or(right.start_byte(), |open| open.start_byte());
                self.operand(left, items)?;
                items.push(Item::Operator(self.source[from..to].trim().to_owned()));
                self.operand(right, items)
            }
            "unary_expression" => {
                let [operand] = named(node)[..] else {
                    return self.unsupported(node);
                };
                let to = operand
                    .prev_sibling()
                    .filter(|open| open.kind() == "(" && grouped(operand))
                    .map_or(operand.start_byte(), |open| open.start_byte());
                items.push(Item::Prefix(
                    match self.source[node.start_byte()..to].trim() {
                        "Not" => "Not",
                        "-" => "-",
                        _ => return self.unsupported(node),
                    },
                ));
                self.operand(operand, items)
            }
            _ => {
                items.push(Item::Operand(self.atom(node)?));
                Ok(())
            }
        }
    }

    fn atom(&self, node: Node<'_>) -> Result<Expression, String> {
        let text = self.text(node);
        match node.kind() {
            "identifier" => Ok(Expression::Name(self.name(node)?)),
            "literal" => match named(node)[..] {
                [inner] => self.atom(inner),
                _ => self.unsupported(node),
            },
            "number" => match nix::int(text).and_then(|value| value.parse().ok()) {
                Some(value) => Ok(Expression::Int(value)),
                None => self.refuse(node, "a number other than a plain integer"),
            },
            "boolean" => match text {
                "True" => Ok(Expression::Bool(true)),
                "False" => Ok(Expression::Bool(false)),
                _ => self.refuse(node, "a boolean other than `True` or `False`"),
            },
            "string_literal" => Ok(Expression::Str(
                text[1..text.len() - 1].replace("\"\"", "\""),
            )),
            "member_expression" => {
                let parts = children(node);
                let (Some(object), Some(property)) =
                    (named(node).first().copied(), parts.last().copied())
                else {
                    return self.unsupported(node);
                };
                let object = self.expression(object)?;
                let (field, arguments) = match property.kind() {
                    "identifier" => (self.identifier(property)?, None),
                    "function_call" => {
                        let (name, arguments) = self.called(property)?;
                        (self.identifier(name)?, Some(arguments))
                    }
                    _ => return self.unsupported(property),
                };
                let member = match object {
                    Expression::Dot(base, path) => Expression::Dot(base, format!("{path}.{field}")),
                    object => Expression::Dot(Box::new(object), field),
                };
                Ok(match arguments {
                    Some(arguments) => Expression::Call(Box::new(member), arguments),
                    None => member,
                })
            }
            "function_call" => {
                let (name, arguments) = self.called(node)?;
                Ok(Expression::Call(
                    Box::new(Expression::Name(self.name(name)?)),
                    arguments,
                ))
            }
            _ => self.unsupported(node),
        }
    }

    fn called<'t>(&self, node: Node<'t>) -> Result<(Node<'t>, Vec<Expression>), String> {
        let parts = named(node);
        let Some(name) = parts
            .first()
            .copied()
            .filter(|name| name.kind() == "identifier" && name.start_byte() == node.start_byte())
        else {
            return self.refuse(node, "a call other than `name(arguments)`");
        };
        let arguments = match parts.get(1) {
            Some(list) => self.arguments(*list)?,
            None => Vec::new(),
        };
        Ok((name, arguments))
    }

    fn block(&self, statements: &[Statement], indent: &str) -> String {
        format!(
            "[\n{}{}]",
            statements
                .iter()
                .map(|statement| format!("{indent}{}\n", self.statement_nix(statement)))
                .collect::<String>(),
            &indent[2..]
        )
    }

    fn scoped(&self, bound: &[String], list: &str) -> String {
        if bound.is_empty() {
            list.to_owned()
        } else {
            let names: Vec<_> = bound.iter().map(|name| spelled(name)).collect();
            format!("({{{}}}: {list})", names.join(", "))
        }
    }

    fn statement_nix(&self, statement: &Statement) -> String {
        match statement {
            Statement::Blank => self.uses("blank").to_owned(),
            Statement::Comment(text) => format!("({} {})", self.uses("comment"), self.string(text)),
            Statement::OptionExplicit => self.uses("optionExplicit").to_owned(),
            Statement::OnErrorResumeNext => self.uses("onErrorResumeNext").to_owned(),
            Statement::OnErrorGoTo0 => self.uses("onErrorGoTo0").to_owned(),
            Statement::Dim(names) => format!(
                "({} [{}])",
                self.uses("dim"),
                names
                    .iter()
                    .map(|name| name.nix.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            Statement::Assign(target, value) => format!(
                "({} {} {})",
                self.uses("assign"),
                self.nix(target),
                self.nix(value)
            ),
            Statement::Set(target, value) => format!(
                "({} {} {})",
                self.uses("set"),
                self.nix(target),
                self.nix(value)
            ),
            Statement::Call(function, arguments) => format!(
                "({} {} {})",
                self.uses("call"),
                self.nix(function),
                self.arguments_nix(arguments)
            ),
            Statement::If(test, then, None) => format!(
                "({} {} {})",
                self.uses("if'"),
                self.nix(test),
                self.block(then, "    ")
            ),
            Statement::If(test, then, Some(otherwise)) => format!(
                "({} {} {} {})",
                self.uses("ifElse"),
                self.nix(test),
                self.block(then, "    "),
                self.block(otherwise, "    ")
            ),
            Statement::Procedure {
                keyword,
                name,
                parameters,
                bound,
                body,
            } => {
                let constructor = if *keyword == "Sub" { "sub" } else { "function" };
                let parameters: Vec<_> = parameters
                    .iter()
                    .map(|(mode, name)| {
                        let mode = match mode {
                            Mode::Plain => "null",
                            Mode::ByVal => self.uses("byVal"),
                            Mode::ByRef => self.uses("byRef"),
                        };
                        format!("{{{} = {mode};}}", name.nix)
                    })
                    .collect();
                format!(
                    "({} {} [{}] {})",
                    self.uses(constructor),
                    name.nix,
                    parameters.join(" "),
                    self.scoped(bound, &self.block(body, "    "))
                )
            }
        }
    }

    fn arguments_nix(&self, arguments: &[Expression]) -> String {
        format!(
            "[{}]",
            arguments
                .iter()
                .map(|argument| self.nix(argument))
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    fn nix(&self, expression: &Expression) -> String {
        match expression {
            Expression::Int(value) => value.to_string(),
            Expression::Bool(value) => value.to_string(),
            Expression::Name(name) => name.nix.clone(),
            Expression::Str(value) => format!("({} {})", self.uses("str"), self.string(value)),
            Expression::Dot(base, path) => format!(
                "({} {} {})",
                self.uses("dot"),
                self.nix(base),
                self.string(path)
            ),
            Expression::Call(function, arguments) => format!(
                "({} {} {})",
                self.uses("call"),
                self.nix(function),
                self.arguments_nix(arguments)
            ),
            Expression::Not(operand) => format!("({} {})", self.uses("not"), self.nix(operand)),
            Expression::Neg(operand) => format!("({} {})", self.uses("neg"), self.nix(operand)),
            Expression::Binary(..) => {
                let mut items = Vec::new();
                self.infix(expression, &mut items);
                format!("[{}]", items.join(" "))
            }
        }
    }

    fn infix(&self, expression: &Expression, out: &mut Vec<String>) {
        if let Expression::Binary(operator, left, right) = expression {
            let level = precedence(operator).unwrap_or(0);
            self.spliced(left, |other| other >= level, out);
            out.push(self.string(operator));
            self.spliced(right, |other| other > level, out);
        }
    }

    fn spliced(&self, expression: &Expression, splice: impl Fn(u8) -> bool, out: &mut Vec<String>) {
        match expression {
            Expression::Binary(operator, ..) if splice(precedence(operator).unwrap_or(0)) => {
                self.infix(expression, out)
            }
            _ => out.push(self.nix(expression)),
        }
    }
}

fn parse(
    items: &mut std::iter::Peekable<std::vec::IntoIter<Item>>,
    least: u8,
) -> Option<Expression> {
    let mut left = match items.next()? {
        Item::Operand(operand) => operand,
        Item::Prefix("Not") => Expression::Not(Box::new(parse(items, NOT + 1)?)),
        Item::Prefix(_) => Expression::Neg(Box::new(parse(items, NEG + 1)?)),
        Item::Operator(_) => return None,
    };
    while let Some(Item::Operator(operator)) = items.peek() {
        let level = precedence(operator)?;
        if level < least {
            break;
        }
        let Some(Item::Operator(operator)) = items.next() else {
            return None;
        };
        let right = parse(items, level + 1)?;
        left = Expression::Binary(operator, Box::new(left), Box::new(right));
    }
    Some(left)
}
