use std::collections::BTreeMap;

use crate::{line_of, nix};

type Node<'t> = tree_sitter::Node<'t>;

const MASKED: [&str; 5] = ["bool", "double", "float", "int", "uint"];

const SCALARS: [&str; 4] = ["bool", "int", "uint", "float"];

const KEYWORDS: [&str; 9] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then",
];

const ASSIGNMENTS: [&str; 10] = ["+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|="];

fn precedence(operator: &str) -> Option<u8> {
    Some(match operator {
        "*" | "/" | "%" => 12,
        "+" | "-" => 11,
        "<<" | ">>" => 10,
        "<" | ">" | "<=" | ">=" => 9,
        "==" | "!=" => 8,
        "&" => 7,
        "^" => 6,
        "|" => 5,
        "&&" => 4,
        "^^" => 3,
        "||" => 2,
        _ => return None,
    })
}

fn is_type(name: &str) -> bool {
    if SCALARS.contains(&name) {
        return true;
    }
    let shape = |rest: &str| matches!(rest, "2" | "3" | "4");
    ["vec", "ivec", "uvec", "bvec"]
        .iter()
        .any(|prefix| name.strip_prefix(prefix).is_some_and(shape))
        || name.strip_prefix("mat").is_some_and(|rest| {
            shape(rest)
                || rest
                    .split_once('x')
                    .is_some_and(|(n, m)| shape(n) && shape(m))
        })
}

struct Emitter<'s> {
    source: &'s str,
    holes: &'s [String],
}

#[derive(Default)]
struct Shader {
    defines: BTreeMap<String, String>,
    consts: BTreeMap<String, String>,
    fns: BTreeMap<String, String>,
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_glsl::LANGUAGE_GLSL.into(), &mask(source))?;
    let emitter = Emitter { source, holes };
    let mut shader = Shader::default();
    let mut note: Option<String> = None;
    for node in named(tree.root_node()) {
        if node.kind() == "comment" {
            if note.is_some() {
                return Err(format!(
                    "line {}: two comments before one declaration",
                    emitter.line(node)
                ));
            }
            note = Some(emitter.comment(node)?);
            continue;
        }
        let (section, name, value) = match node.kind() {
            "preproc_def" | "preproc_function_def" => {
                let (name, value) = emitter.define(node)?;
                (&mut shader.defines, name, value)
            }
            "declaration" => {
                let (name, value) = emitter.constant(node)?;
                (&mut shader.consts, name, value)
            }
            "function_definition" => {
                let (name, value) = emitter.function(node)?;
                (&mut shader.fns, name, value)
            }
            kind => {
                return Err(format!(
                    "line {}: no GLSL declaration for {kind}",
                    emitter.line(node)
                ))
            }
        };
        let value = match note.take() {
            Some(text) => format!("(commented {} {value})", emitter.string(&text)),
            None => value,
        };
        if section.insert(name.clone(), value).is_some() {
            return Err(format!(
                "line {}: `{name}` is declared twice; a shader's names are keys",
                emitter.line(node)
            ));
        }
    }
    if note.is_some() {
        return Err("a comment after the last declaration".into());
    }
    let mut out = String::from("{\n");
    for (key, section) in [
        ("defines", &shader.defines),
        ("consts", &shader.consts),
        ("fns", &shader.fns),
    ] {
        if section.is_empty() {
            continue;
        }
        out.push_str(&format!("  {key} = {{\n"));
        for (name, value) in section {
            out.push_str(&format!("    {} = {value};\n", attribute(name)));
        }
        out.push_str("  };\n");
    }
    out.push('}');
    Ok(out)
}

fn attribute(name: &str) -> String {
    if KEYWORDS.contains(&name) {
        format!("\"{name}\"")
    } else {
        name.to_owned()
    }
}

fn mask(source: &str) -> String {
    let mut out = source.as_bytes().to_vec();
    for name in MASKED {
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

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn unparen(mut node: Node<'_>) -> Node<'_> {
    while node.kind() == "parenthesized_expression" {
        match named(node).as_slice() {
            [inner] => node = *inner,
            _ => break,
        }
    }
    node
}

fn binary(node: Node<'_>, emitter: &Emitter<'_>) -> Option<u8> {
    let node = unparen(node);
    if node.kind() != "binary_expression" {
        return None;
    }
    precedence(emitter.text(node.child_by_field_name("operator")?))
}

fn float(value: f64) -> String {
    let size = value.abs();
    if size != 0.0 && !(1e-4..1e15).contains(&size) {
        let text = format!("{value:e}");
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        if mantissa.contains('.') {
            format!("{mantissa}e{exponent}")
        } else {
            format!("{mantissa}.0e{exponent}")
        }
    } else {
        let text = format!("{value}");
        if text.contains('.') {
            text
        } else {
            format!("{text}.0")
        }
    }
}

impl Emitter<'_> {
    fn text(&self, node: Node<'_>) -> &str {
        &self.source[node.byte_range()]
    }

    fn line(&self, node: Node<'_>) -> usize {
        line_of(self.source, node.start_byte())
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn field<'t>(&self, node: Node<'t>, name: &str) -> Result<Node<'t>, String> {
        node.child_by_field_name(name).ok_or_else(|| {
            format!(
                "line {}: a {} without its {name}",
                self.line(node),
                node.kind()
            )
        })
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!(
            "line {}: {what}: `{}`",
            self.line(node),
            self.text(node)
        ))
    }

    fn comment(&self, node: Node<'_>) -> Result<String, String> {
        let text = self.text(node).trim_end_matches('\n');
        match text.strip_prefix("//") {
            Some(rest) if !rest.contains('\n') => {
                Ok(rest.strip_prefix(' ').unwrap_or(rest).to_owned())
            }
            _ => self.refuse(node, "only a // comment on one line is kept"),
        }
    }

    fn number(&self, node: Node<'_>) -> Result<(String, &'static str), String> {
        let text = self.text(node);
        if let Some(positive) = text.strip_prefix('-') {
            let (value, kind) = self.literal(node, positive)?;
            if kind == "uint" {
                return self.refuse(node, "a negated unsigned literal");
            }
            return Ok((format!("(-{value})"), kind));
        }
        self.literal(node, text)
    }

    fn literal(&self, node: Node<'_>, text: &str) -> Result<(String, &'static str), String> {
        let (digits, unsigned) = match text.strip_suffix(['u', 'U']) {
            Some(digits) => (digits, true),
            None => (text, false),
        };
        let hex = digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"));
        let value = if let Some(hex) = hex {
            u64::from_str_radix(hex, 16).ok()
        } else if digits.contains(['.', 'e', 'E']) {
            if unsigned || digits.ends_with(['f', 'F']) {
                return self.refuse(node, "a float literal with a suffix");
            }
            let value: f64 = digits
                .parse()
                .or_else(|_| self.refuse(node, "an unreadable float literal"))?;
            return Ok((float(value), "float"));
        } else if digits.len() > 1 && digits.starts_with('0') {
            return self.refuse(node, "an octal literal");
        } else {
            digits.parse().ok()
        };
        let Some(value) = value.filter(|&value| value <= u64::from(u32::MAX)) else {
            return self.refuse(node, "an integer literal out of range");
        };
        if unsigned {
            Ok((format!("(unsigned {value})"), "uint"))
        } else {
            Ok((value.to_string(), "int"))
        }
    }

    fn inferred(&self, node: Node<'_>) -> Option<String> {
        let node = unparen(node);
        match node.kind() {
            "number_literal" => self.number(node).ok().map(|(_, kind)| kind.to_owned()),
            "true" | "false" => Some("bool".into()),
            "unary_expression" => {
                let argument = unparen(node.child_by_field_name("argument")?);
                let negative = self.text(node.child_by_field_name("operator")?) == "-";
                let (_, kind) = self.number(argument).ok()?;
                (negative
                    && argument.kind() == "number_literal"
                    && !self.text(argument).starts_with('-')
                    && kind != "uint")
                    .then(|| kind.to_owned())
            }
            "call_expression" => {
                let function = node.child_by_field_name("function")?;
                if function.kind() == "identifier" && is_type(self.text(function)) {
                    return Some(self.text(function).to_owned());
                }
                let (base, size) = self.array_type(function)?;
                let count = named(node.child_by_field_name("arguments")?).len();
                (size == count).then(|| format!("{base}[{size}]"))
            }
            _ => None,
        }
    }

    fn array_type(&self, function: Node<'_>) -> Option<(String, usize)> {
        if function.kind() != "subscript_expression" {
            return None;
        }
        let base = function.child_by_field_name("argument")?;
        let index = function.child_by_field_name("index")?;
        let size = self.text(index).parse().ok()?;
        (base.kind() == "identifier"
            && is_type(self.text(base))
            && index.kind() == "number_literal")
            .then(|| (self.text(base).to_owned(), size))
    }

    fn expression(&self, node: Node<'_>) -> Result<String, String> {
        let node = unparen(node);
        match node.kind() {
            "identifier" => Ok(self.string(self.text(node))),
            "true" | "false" => Ok(self.text(node).to_owned()),
            "number_literal" => Ok(self.number(node)?.0),
            "field_expression" => {
                let base = unparen(self.field(node, "argument")?);
                let name = self.text(self.field(node, "field")?);
                if let Some(path) = self.path(base) {
                    Ok(self.string(&format!("{path}.{name}")))
                } else {
                    Ok(format!(
                        "(field {} {})",
                        self.expression(base)?,
                        self.string(name)
                    ))
                }
            }
            "subscript_expression" => Ok(format!(
                "(index {} {})",
                self.expression(self.field(node, "argument")?)?,
                self.expression(self.field(node, "index")?)?
            )),
            "binary_expression" => Ok(format!("[{}]", self.infix(node)?.join(" "))),
            "unary_expression" => {
                let operator = self.text(self.field(node, "operator")?);
                let argument = unparen(self.field(node, "argument")?);
                if operator == "-"
                    && argument.kind() == "number_literal"
                    && !self.text(argument).starts_with('-')
                {
                    let (value, kind) = self.literal(argument, self.text(argument))?;
                    if kind == "uint" {
                        return self.refuse(node, "a negated unsigned literal");
                    }
                    return Ok(format!("(-{value})"));
                }
                let constructor = match operator {
                    "-" => "neg",
                    "!" => "not",
                    "~" => "bitNot",
                    _ => return self.refuse(node, "no constructor for this unary operator"),
                };
                Ok(format!("({constructor} {})", self.expression(argument)?))
            }
            "update_expression" => {
                let operator = self.field(node, "operator")?;
                let argument = self.field(node, "argument")?;
                let prefix = operator.start_byte() < argument.start_byte();
                let constructor = match (self.text(operator), prefix) {
                    ("++", false) => "inc",
                    ("--", false) => "dec",
                    ("++", true) => "preInc",
                    ("--", true) => "preDec",
                    _ => return self.refuse(node, "an unexpected update"),
                };
                Ok(format!("({constructor} {})", self.expression(argument)?))
            }
            "conditional_expression" => Ok(format!(
                "(cond {} {} {})",
                self.expression(self.field(node, "condition")?)?,
                self.expression(self.field(node, "consequence")?)?,
                self.expression(self.field(node, "alternative")?)?
            )),
            "call_expression" => self.call(node),
            _ => self.refuse(node, "no GLSL expression for this"),
        }
    }

    fn path(&self, node: Node<'_>) -> Option<String> {
        match node.kind() {
            "identifier" => Some(self.text(node).to_owned()),
            "field_expression" => {
                let base = unparen(node.child_by_field_name("argument")?);
                let name = self.text(node.child_by_field_name("field")?);
                Some(format!("{}.{name}", self.path(base)?))
            }
            _ => None,
        }
    }

    fn infix(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let operator = self.text(self.field(node, "operator")?);
        let Some(level) = precedence(operator) else {
            return self.refuse(node, "no binary operator for this");
        };
        let left = self.field(node, "left")?;
        let right = self.field(node, "right")?;
        let mut items = match binary(left, self) {
            Some(inner) if inner >= level => self.infix(unparen(left))?,
            _ => vec![self.expression(left)?],
        };
        items.push(self.string(operator));
        match binary(right, self) {
            Some(inner) if inner > level => items.extend(self.infix(unparen(right))?),
            _ => items.push(self.expression(right)?),
        }
        Ok(items)
    }

    fn call(&self, node: Node<'_>) -> Result<String, String> {
        let function = self.field(node, "function")?;
        let mut arguments = Vec::new();
        for argument in named(self.field(node, "arguments")?) {
            arguments.push(self.expression(argument)?);
        }
        if let Some((base, size)) = self.array_type(function) {
            if size != arguments.len() {
                return self.refuse(node, "an array constructor whose size is not its length");
            }
            return Ok(format!(
                "(array {} [{}])",
                self.string(&base),
                arguments.join(" ")
            ));
        }
        if function.kind() != "identifier" {
            return self.refuse(node, "a call of something other than a name");
        }
        let name = self.text(function);
        let head = if is_type(name) {
            name.to_owned()
        } else {
            format!("call {}", self.string(name))
        };
        if arguments.is_empty() {
            Ok(format!("({head})"))
        } else {
            Ok(format!("({head} {})", arguments.join(" ")))
        }
    }

    fn type_words(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let ty = self.field(node, "type")?;
        let mut words = Vec::new();
        let mut cursor = node.walk();
        for (index, child) in node.children(&mut cursor).enumerate() {
            if child.start_byte() >= ty.start_byte() {
                break;
            }
            match node.field_name_for_child(index as u32) {
                None if child.kind() != "comment" => words.push(self.text(child).to_owned()),
                _ => return self.refuse(child, "an unexpected part before a type"),
            }
        }
        words.push(self.text(ty).to_owned());
        Ok(words)
    }

    fn words(&self, words: &[String]) -> String {
        if let [one] = words {
            self.string(one)
        } else {
            let quoted: Vec<String> = words.iter().map(|word| self.string(word)).collect();
            format!("[{}]", quoted.join(" "))
        }
    }

    fn declarator<'t>(
        &self,
        node: Node<'t>,
    ) -> Result<(Node<'t>, Option<Node<'t>>, Option<usize>), String> {
        let (target, value) = if node.kind() == "init_declarator" {
            (
                self.field(node, "declarator")?,
                Some(self.field(node, "value")?),
            )
        } else {
            (node, None)
        };
        match target.kind() {
            "identifier" => Ok((target, value, None)),
            "array_declarator" => {
                let size = self.field(target, "size")?;
                let name = self.field(target, "declarator")?;
                let Ok(count) = self.text(size).parse() else {
                    return self.refuse(size, "an array size that is not a number");
                };
                if name.kind() != "identifier" || size.kind() != "number_literal" {
                    return self.refuse(target, "an unexpected array declarator");
                }
                Ok((name, value, Some(count)))
            }
            _ => self.refuse(target, "an unexpected declarator"),
        }
    }

    fn sized(&self, words: &[String], size: Option<usize>) -> String {
        match size {
            Some(size) => format!("(sized {} {size})", self.words(words)),
            None => self.words(words),
        }
    }

    fn constant(&self, node: Node<'_>) -> Result<(String, String), String> {
        let words = self.type_words(node)?;
        let declarators: Vec<Node<'_>> = {
            let mut cursor = node.walk();
            node.children_by_field_name("declarator", &mut cursor)
                .collect()
        };
        let ([declarator], [qualifier, ty]) = (declarators.as_slice(), words.as_slice()) else {
            return self.refuse(node, "a global is one const declaration with one name");
        };
        if qualifier != "const" {
            return self.refuse(node, "a global is one const declaration with one name");
        }
        let (name, value, size) = self.declarator(*declarator)?;
        let Some(value) = value else {
            return self.refuse(node, "a const without a value");
        };
        let declared = match size {
            Some(size) => format!("{ty}[{size}]"),
            None => ty.clone(),
        };
        let expression = self.expression(value)?;
        let written = if self.inferred(value).as_deref() == Some(declared.as_str()) {
            expression
        } else if size.is_some() {
            return self.refuse(node, "an array const whose value is not its constructor");
        } else {
            format!("(typed {} {expression})", self.string(ty))
        };
        Ok((self.text(name).to_owned(), written))
    }

    fn function(&self, node: Node<'_>) -> Result<(String, String), String> {
        let returns = self.type_words(node)?;
        let declarator = self.field(node, "declarator")?;
        if declarator.kind() != "function_declarator" {
            return self.refuse(declarator, "an unexpected function declarator");
        }
        let name = self.field(declarator, "declarator")?;
        let mut params = Vec::new();
        for param in named(self.field(declarator, "parameters")?) {
            if param.kind() != "parameter_declaration" {
                return self.refuse(param, "an unexpected parameter");
            }
            let mut words = self.type_words(param)?;
            match param.child_by_field_name("declarator") {
                Some(target) if target.kind() == "identifier" => {
                    words.push(self.text(target).to_owned());
                }
                None if words == ["void"] => continue,
                _ => return self.refuse(param, "a parameter that is not `type name`"),
            }
            let quoted: Vec<String> = words.iter().map(|word| self.string(word)).collect();
            params.push(format!("[{}]", quoted.join(" ")));
        }
        Ok((
            self.text(name).to_owned(),
            format!(
                "(fn {} [{}] {})",
                self.words(&returns),
                params.join(" "),
                self.body(self.field(node, "body")?)?
            ),
        ))
    }

    fn define(&self, node: Node<'_>) -> Result<(String, String), String> {
        let name = self.text(self.field(node, "name")?).to_owned();
        let Some(value) = node.child_by_field_name("value") else {
            return self.refuse(node, "a #define without a value");
        };
        let text = self.text(value).trim_end_matches('\n').trim();
        if text.contains('\n') {
            return self.refuse(node, "a macro over several lines");
        }
        let wrapper = format!("int _ = {text};\n");
        let tree = crate::parse(&tree_sitter_glsl::LANGUAGE_GLSL.into(), &mask(&wrapper))
            .or_else(|_| self.refuse(value, "a macro body that is not an expression"))?;
        let init = tree
            .root_node()
            .child(0)
            .and_then(|declaration| declaration.child_by_field_name("declarator"))
            .and_then(|declarator| declarator.child_by_field_name("value"));
        let Some(init) = init else {
            return self.refuse(value, "a macro body that is not an expression");
        };
        let inner = Emitter {
            source: &wrapper,
            holes: self.holes,
        };
        let body = inner.expression(init)?;
        match node.child_by_field_name("parameters") {
            None => Ok((name, body)),
            Some(params) => {
                let names: Vec<String> = named(params)
                    .into_iter()
                    .map(|param| self.string(self.text(param)))
                    .collect();
                Ok((name, format!("(macro [{}] {body})", names.join(" "))))
            }
        }
    }

    fn body(&self, node: Node<'_>) -> Result<String, String> {
        let statements = if node.kind() == "compound_statement" {
            let mut out = Vec::new();
            for statement in named(node) {
                out.extend(self.statement(statement)?);
            }
            out
        } else {
            self.statement(node)?
        };
        Ok(format!("[\n{}\n]", statements.join("\n")))
    }

    fn local(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let words = self.type_words(node)?;
        let mut cursor = node.walk();
        let declarators: Vec<Node<'_>> = node
            .children_by_field_name("declarator", &mut cursor)
            .collect();
        let mut out = Vec::new();
        for declarator in declarators {
            let (name, value, size) = self.declarator(declarator)?;
            let value = match value {
                Some(value) => self.expression(value)?,
                None => "null".into(),
            };
            out.push(format!(
                "(var {} {} {value})",
                self.sized(&words, size),
                self.string(self.text(name))
            ));
        }
        Ok(out)
    }

    fn statement(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        Ok(vec![match node.kind() {
            "comment" => format!("(comment {})", self.string(&self.comment(node)?)),
            "declaration" => return self.local(node),
            "expression_statement" => match named(node).as_slice() {
                [inner] => self.effect(*inner)?,
                _ => return self.refuse(node, "an empty statement"),
            },
            "return_statement" => match named(node).as_slice() {
                [] => "(return null)".into(),
                [value] => format!("(return {})", self.expression(*value)?),
                _ => return self.refuse(node, "an unexpected return"),
            },
            "break_statement" => "break".into(),
            "continue_statement" => "continue".into(),
            "if_statement" => self.conditional(node)?,
            "for_statement" => {
                let init = match node.child_by_field_name("initializer") {
                    None => "null".into(),
                    Some(init) if init.kind() == "declaration" => {
                        match self.local(init)?.as_slice() {
                            [one] => one.clone(),
                            _ => return self.refuse(init, "a for header declaring several names"),
                        }
                    }
                    Some(init) => self.effect(init)?,
                };
                format!(
                    "(for {init} {} {} {})",
                    self.expression(self.field(node, "condition")?)?,
                    self.effect(self.field(node, "update")?)?,
                    self.body(self.field(node, "body")?)?
                )
            }
            "while_statement" => format!(
                "(while {} {})",
                self.expression(self.field(node, "condition")?)?,
                self.body(self.field(node, "body")?)?
            ),
            _ => return self.refuse(node, "no GLSL statement for this"),
        }])
    }

    fn effect(&self, node: Node<'_>) -> Result<String, String> {
        if node.kind() != "assignment_expression" {
            return self.expression(node);
        }
        let operator = self.text(self.field(node, "operator")?);
        let target = self.expression(self.field(node, "left")?)?;
        let value = self.expression(self.field(node, "right")?)?;
        if operator == "=" {
            Ok(format!("(assign {target} {value})"))
        } else if ASSIGNMENTS.contains(&operator) {
            Ok(format!(
                "(update {} {target} {value})",
                self.string(operator)
            ))
        } else {
            self.refuse(node, "an unexpected assignment")
        }
    }

    fn conditional(&self, node: Node<'_>) -> Result<String, String> {
        let test = self.expression(self.field(node, "condition")?)?;
        let then = self.body(self.field(node, "consequence")?)?;
        let Some(clause) = node.child_by_field_name("alternative") else {
            return Ok(format!("(if' {test} {then})"));
        };
        let other = match named(clause).as_slice() {
            [other] => *other,
            _ => return self.refuse(clause, "an unexpected else"),
        };
        let otherwise = if other.kind() == "if_statement" {
            self.conditional(other)?
        } else {
            self.body(other)?
        };
        Ok(format!("(ifElse {test} {then} {otherwise})"))
    }
}
