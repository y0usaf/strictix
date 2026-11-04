use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use crate::{line_of, nix};

type Node<'t> = tree_sitter::Node<'t>;

const MASKED: [&str; 5] = ["bool", "double", "float", "int", "uint"];

const SCALARS: [&str; 4] = ["bool", "int", "uint", "float"];

const NIX_KEYWORDS: [&str; 11] = [
    "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with", "self",
];

const CONSTRUCTORS: [&str; 36] = [
    "fn",
    "local",
    "set",
    "update",
    "return",
    "break",
    "continue",
    "if'",
    "ifElse",
    "for",
    "while",
    "comment",
    "commented",
    "neg",
    "not",
    "bitNot",
    "preInc",
    "preDec",
    "inc",
    "dec",
    "cond",
    "unsigned",
    "sized",
    "out",
    "in'",
    "inout",
    "typed",
    "macro",
    "array",
    "index",
    "math",
    "rgb",
    "render",
    "atan2",
    "step",
    "smoothstep",
];

const EXTERNALS: [&str; 5] = ["iResolution", "iTime", "iFrame", "iMouse", "iChannel0"];

const ASSIGNMENTS: [&str; 10] = ["+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|="];

fn arity(name: &str) -> Option<usize> {
    Some(match name {
        "abs" | "sign" | "floor" | "ceil" | "fract" | "round" | "trunc" | "sin" | "cos" | "tan"
        | "asin" | "acos" | "atan" | "exp" | "log" | "exp2" | "log2" | "sqrt" | "inversesqrt"
        | "normalize" | "radians" | "degrees" | "fwidth" | "dFdx" | "dFdy" | "length" => 1,
        "min" | "max" | "mod" | "pow" | "reflect" | "atan2" | "distance" | "dot" | "step"
        | "cross" | "texture" => 2,
        "clamp" | "mix" | "smoothstep" => 3,
        _ => return None,
    })
}

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
    if SCALARS.contains(&name) || name == "sampler2D" || name == "void" {
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

fn reserved(name: &str) -> bool {
    NIX_KEYWORDS.contains(&name)
        || CONSTRUCTORS.contains(&name)
        || EXTERNALS.contains(&name)
        || is_type(name)
        || arity(name).is_some()
}

const VECTORS: [(&str, &str); 4] = [
    ("vec", "float"),
    ("ivec", "int"),
    ("uvec", "uint"),
    ("bvec", "bool"),
];

const NUMBERS: [&str; 3] = ["int", "uint", "float"];

fn vector(ty: &str) -> Option<(&'static str, usize)> {
    VECTORS.iter().find_map(|(prefix, scalar)| {
        let size = ty.strip_prefix(prefix)?.parse().ok()?;
        (2..=4).contains(&size).then_some((*scalar, size))
    })
}

fn shape(ty: &str) -> Option<(usize, usize)> {
    let rest = ty.strip_prefix("mat")?;
    let (columns, rows) = rest.split_once('x').unwrap_or((rest, rest));
    let size = |text: &str| text.parse().ok().filter(|size| (2..=4).contains(size));
    Some((size(columns)?, size(rows)?))
}

fn scalar_of(ty: &str) -> Option<&'static str> {
    SCALARS
        .into_iter()
        .find(|scalar| *scalar == ty)
        .or_else(|| vector(ty).map(|(scalar, _)| scalar))
        .or_else(|| shape(ty).map(|_| "float"))
}

fn dimension(ty: &str) -> Option<usize> {
    if SCALARS.contains(&ty) {
        Some(1)
    } else {
        vector(ty).map(|(_, size)| size)
    }
}

fn vector_of(scalar: &str, size: usize) -> String {
    if size == 1 {
        return scalar.to_owned();
    }
    let prefix = VECTORS
        .iter()
        .find(|(_, one)| *one == scalar)
        .map_or("vec", |(prefix, _)| prefix);
    format!("{prefix}{size}")
}

fn swizzle(name: &str) -> Option<usize> {
    if !(1..=4).contains(&name.len()) {
        return None;
    }
    ["xyzw", "rgba", "stpq"].iter().find_map(|set| {
        let places: Option<Vec<usize>> = name.chars().map(|letter| set.find(letter)).collect();
        places
            .and_then(|places| places.into_iter().max())
            .map(|reach| reach + 1)
    })
}

fn within(kinds: &[&str], ty: &str) -> bool {
    scalar_of(ty).is_some_and(|scalar| kinds.contains(&scalar))
}

fn integral(ty: &str) -> bool {
    within(&["int", "uint"], ty) && shape(ty).is_none()
}

fn componentwise(left: &str, right: &str) -> Option<String> {
    let with = |scalar: &str, other: &str| {
        dimension(scalar) == Some(1) && scalar_of(other) == Some(scalar)
    };
    if left == right || with(right, left) {
        Some(left.to_owned())
    } else {
        with(left, right).then(|| right.to_owned())
    }
}

fn matrix_of(columns: usize, rows: usize) -> String {
    if columns == rows {
        format!("mat{columns}")
    } else {
        format!("mat{columns}x{rows}")
    }
}

fn linear(left: &str, right: &str) -> Option<String> {
    match (shape(left), shape(right)) {
        (Some((columns, rows)), Some((others, inner))) => {
            (columns == inner).then(|| matrix_of(others, rows))
        }
        (Some((columns, rows)), None) => {
            (right == vector_of("float", columns)).then(|| vector_of("float", rows))
        }
        (None, Some((columns, rows))) => {
            (left == vector_of("float", rows)).then(|| vector_of("float", columns))
        }
        (None, None) => None,
    }
}

fn combine(operator: &str, left: &str, right: &str) -> Option<String> {
    let numeric = within(&NUMBERS, left) && within(&NUMBERS, right);
    let matrix = shape(left).is_some() || shape(right).is_some();
    let scalar = dimension(left) == Some(1) || dimension(right) == Some(1);
    match operator {
        "*" if numeric && matrix && !scalar => linear(left, right),
        "*" | "/" | "+" | "-" if numeric => componentwise(left, right),
        "%" | "&" | "^" | "|" if integral(left) && integral(right) => componentwise(left, right),
        "<<" | ">>"
            if integral(left)
                && integral(right)
                && (dimension(right) == Some(1) || dimension(right) == dimension(left)) =>
        {
            Some(left.to_owned())
        }
        "<" | ">" | "<=" | ">=" if left == right && NUMBERS.contains(&left) => Some("bool".into()),
        "==" | "!=" if left == right => Some("bool".into()),
        "&&" | "||" | "^^" if left == "bool" && right == "bool" => Some("bool".into()),
        _ => None,
    }
}

struct Emitter<'s> {
    source: &'s str,
    holes: &'s [String],
    globals: BTreeSet<String>,
    scopes: RefCell<Vec<Vec<String>>>,
    used: RefCell<BTreeSet<String>>,
    selves: RefCell<BTreeSet<String>>,
    types: RefCell<BTreeMap<String, String>>,
}

#[derive(Default)]
struct Shader {
    defines: BTreeMap<String, String>,
    consts: BTreeMap<String, String>,
    fns: BTreeMap<String, String>,
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_glsl::LANGUAGE_GLSL.into(), &mask(source))?;
    let mut emitter = Emitter {
        source,
        holes,
        globals: BTreeSet::new(),
        scopes: RefCell::new(Vec::new()),
        used: RefCell::new(BTreeSet::new()),
        selves: RefCell::new(BTreeSet::new()),
        types: RefCell::new(BTreeMap::new()),
    };
    let items = named(tree.root_node());
    for node in &items {
        let name = match node.kind() {
            "preproc_def" | "preproc_function_def" => emitter.field(*node, "name")?,
            "declaration" => {
                let declarator = emitter.field(*node, "declarator")?;
                let target = declarator
                    .child_by_field_name("declarator")
                    .unwrap_or(declarator);
                target.child_by_field_name("declarator").unwrap_or(target)
            }
            "function_definition" => {
                let declarator = emitter.field(*node, "declarator")?;
                emitter.field(declarator, "declarator")?
            }
            _ => continue,
        };
        let name = emitter.text(name).to_owned();
        emitter.name(&name, emitter.line(*node))?;
        if !emitter.globals.insert(name.clone()) {
            return Err(format!(
                "line {}: `{name}` is declared twice; a shader's names are keys",
                emitter.line(*node)
            ));
        }
    }
    let mut shader = Shader::default();
    let mut note: Option<String> = None;
    for node in items {
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
            Some(text) => {
                emitter.uses("commented");
                format!("(commented {} {value})", emitter.string(&text))
            }
            None => value,
        };
        section.insert(name, value);
    }
    if note.is_some() {
        return Err("a comment after the last declaration".into());
    }
    let used = emitter.used.borrow();
    let selves = emitter.selves.borrow();
    let mut out = String::new();
    if used.is_empty() {
        out.push_str("_: ");
    } else {
        out.push_str("{\n");
        for name in used.iter() {
            out.push_str(&format!("  {name},\n"));
        }
        out.push_str("  ...\n}: ");
    }
    if selves.is_empty() {
        out.push_str("_: {\n");
    } else {
        let names: Vec<&str> = selves.iter().map(String::as_str).collect();
        out.push_str(&format!(
            "self: let\n  inherit (self) {};\nin {{\n",
            names.join(" ")
        ));
    }
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
            out.push_str(&format!("    {name} = {value};\n"));
        }
        out.push_str("  };\n");
    }
    out.push('}');
    Ok(out)
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

    fn uses(&self, name: &str) -> String {
        self.used.borrow_mut().insert(name.to_owned());
        name.to_owned()
    }

    fn name(&self, name: &str, line: usize) -> Result<(), String> {
        if reserved(name) {
            Err(format!(
                "line {line}: `{name}` is a name the GLSL generator or Nix already gives a meaning"
            ))
        } else {
            Ok(())
        }
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

    fn resolve(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if self
            .scopes
            .borrow()
            .iter()
            .any(|scope| scope.iter().any(|one| one == name))
        {
            return Ok(name.to_owned());
        }
        if self.globals.contains(name) {
            self.selves.borrow_mut().insert(name.to_owned());
            return Ok(name.to_owned());
        }
        if EXTERNALS.contains(&name) {
            return Ok(self.uses(name));
        }
        self.refuse(
            node,
            "a name neither bound here nor declared by the shader or tomoe's prelude",
        )
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
            Ok((format!("({} {value})", self.uses("unsigned")), "uint"))
        } else {
            Ok((value.to_string(), "int"))
        }
    }

    fn type_of(&self, node: Node<'_>) -> Option<String> {
        let node = unparen(node);
        let part = |name: &str| node.child_by_field_name(name);
        match node.kind() {
            "number_literal" => self.number(node).ok().map(|(_, kind)| kind.to_owned()),
            "true" | "false" => Some("bool".into()),
            "identifier" => {
                let name = self.text(node);
                let bound = self
                    .scopes
                    .borrow()
                    .iter()
                    .any(|scope| scope.iter().any(|one| one == name));
                if bound {
                    None
                } else {
                    self.types.borrow().get(name).cloned()
                }
            }
            "unary_expression" => {
                let operand = self.type_of(part("argument")?)?;
                let fits = match self.text(part("operator")?) {
                    "-" => within(&NUMBERS, &operand),
                    "!" => operand == "bool",
                    "~" => integral(&operand),
                    _ => false,
                };
                fits.then_some(operand)
            }
            "binary_expression" => combine(
                self.text(part("operator")?),
                &self.type_of(part("left")?)?,
                &self.type_of(part("right")?)?,
            ),
            "conditional_expression" => {
                let yes = self.type_of(part("consequence")?)?;
                (self.type_of(part("alternative")?)? == yes).then_some(yes)
            }
            "field_expression" => {
                let (scalar, size) = vector(&self.type_of(part("argument")?)?)?;
                let name = self.text(part("field")?);
                (swizzle(name)? <= size).then(|| vector_of(scalar, name.len()))
            }
            "subscript_expression" => {
                let base = self.type_of(part("argument")?)?;
                if let Some((element, _)) = base.split_once('[') {
                    Some(element.to_owned())
                } else if let Some((_, rows)) = shape(&base) {
                    Some(vector_of("float", rows))
                } else {
                    vector(&base).map(|(scalar, _)| scalar.to_owned())
                }
            }
            "call_expression" => {
                let function = part("function")?;
                if let Some((base, size)) = self.array_type(function) {
                    return Some(format!("{base}[{size}]"));
                }
                let name = self.text(function);
                if function.kind() != "identifier" || self.globals.contains(name) {
                    return None;
                }
                if is_type(name) {
                    return Some(name.to_owned());
                }
                let arguments = named(part("arguments")?);
                let at = match name {
                    "length" | "distance" | "dot" => return Some("float".into()),
                    "cross" => return Some("vec3".into()),
                    "texture" => return Some("vec4".into()),
                    "step" => 1,
                    "smoothstep" => 2,
                    _ if arity(name).is_some() => 0,
                    _ => return None,
                };
                self.type_of(*arguments.get(at)?)
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

    fn type_name(&self, name: &str) -> String {
        self.uses(name)
    }

    fn expression(&self, node: Node<'_>) -> Result<String, String> {
        let node = unparen(node);
        match node.kind() {
            "identifier" => self.resolve(node),
            "true" | "false" => Ok(self.text(node).to_owned()),
            "number_literal" => Ok(self.number(node)?.0),
            "field_expression" => {
                let base = unparen(self.field(node, "argument")?);
                let name = self.text(self.field(node, "field")?);
                if swizzle(name).is_none() {
                    return self.refuse(node, "a field that is not a swizzle");
                }
                let selectable = match base.kind() {
                    "identifier"
                    | "subscript_expression"
                    | "field_expression"
                    | "call_expression"
                    | "conditional_expression" => true,
                    "unary_expression" => base
                        .child_by_field_name("argument")
                        .is_some_and(|argument| unparen(argument).kind() != "number_literal"),
                    _ => false,
                };
                if !selectable {
                    return self.refuse(
                        node,
                        "a swizzle of an infix expression, a literal or an increment",
                    );
                }
                Ok(format!("{}.{name}", self.expression(base)?))
            }
            "subscript_expression" => Ok(format!(
                "({} {} {})",
                self.uses("index"),
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
                Ok(format!(
                    "({} {})",
                    self.uses(constructor),
                    self.expression(argument)?
                ))
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
                Ok(format!(
                    "({} {})",
                    self.uses(constructor),
                    self.expression(argument)?
                ))
            }
            "conditional_expression" => Ok(format!(
                "({} {} {} {})",
                self.uses("cond"),
                self.expression(self.field(node, "condition")?)?,
                self.expression(self.field(node, "consequence")?)?,
                self.expression(self.field(node, "alternative")?)?
            )),
            "call_expression" => self.call(node),
            _ => self.refuse(node, "no GLSL expression for this"),
        }
    }

    fn infix(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let operator = self.text(self.field(node, "operator")?);
        let Some(level) = precedence(operator) else {
            return self.refuse(node, "no binary operator for this");
        };
        let left = self.field(node, "left")?;
        let right = self.field(node, "right")?;
        let mut items = match self.binary(left) {
            Some(inner) if inner >= level => self.infix(unparen(left))?,
            _ => vec![self.expression(left)?],
        };
        items.push(self.string(operator));
        match self.binary(right) {
            Some(inner) if inner > level => items.extend(self.infix(unparen(right))?),
            _ => items.push(self.expression(right)?),
        }
        Ok(items)
    }

    fn binary(&self, node: Node<'_>) -> Option<u8> {
        let node = unparen(node);
        if node.kind() != "binary_expression" {
            return None;
        }
        precedence(self.text(node.child_by_field_name("operator")?))
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
                "({} {} [{}])",
                self.uses("array"),
                self.type_name(&base),
                arguments.join(" ")
            ));
        }
        if function.kind() != "identifier" {
            return self.refuse(node, "a call of something other than a name");
        }
        let name = self.text(function);
        let (head, expected) = if is_type(name) {
            (self.type_name(name), None)
        } else if self.globals.contains(name) {
            self.selves.borrow_mut().insert(name.to_owned());
            (name.to_owned(), None)
        } else {
            let name = if name == "atan" && arguments.len() == 2 {
                "atan2"
            } else {
                name
            };
            match arity(name) {
                Some(count) => (self.uses(name), Some(count)),
                None => {
                    return self
                        .refuse(node, "a call of a function neither GLSL's nor the shader's")
                }
            }
        };
        if expected.is_some_and(|count| count != arguments.len()) {
            return self.refuse(node, "a builtin called with another number of arguments");
        }
        if arguments.is_empty() {
            Ok(head)
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

    fn qualified(&self, node: Node<'_>, words: &[String]) -> Result<String, String> {
        let Some((ty, qualifiers)) = words.split_last() else {
            return self.refuse(node, "a declaration without a type");
        };
        if !is_type(ty) {
            return self.refuse(node, "a type GLSL's generator does not know");
        }
        let mut out = self.type_name(ty);
        for qualifier in qualifiers.iter().rev() {
            let constructor = match qualifier.as_str() {
                "in" => "in'",
                "out" => "out",
                "inout" => "inout",
                _ => return self.refuse(node, "a qualifier the generator does not write here"),
            };
            out = format!("({} {out})", self.uses(constructor));
        }
        Ok(out)
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
        let expression = self.expression(value)?;
        let declared = match size {
            Some(size) => format!("{ty}[{size}]"),
            None => ty.clone(),
        };
        let written = match self.type_of(value) {
            Some(found) if found == declared => expression,
            Some(found) => {
                return self.refuse(
                    node,
                    &format!("a {declared} const whose value is a {found}"),
                )
            }
            None if size.is_some() => {
                return self.refuse(node, "an array const whose value is not its constructor")
            }
            None => format!(
                "({} {} {expression})",
                self.uses("typed"),
                self.type_name(ty)
            ),
        };
        let name = self.text(name).to_owned();
        self.types.borrow_mut().insert(name.clone(), declared);
        Ok((name, written))
    }

    fn function(&self, node: Node<'_>) -> Result<(String, String), String> {
        let returns = self.type_words(node)?;
        let returns = self.qualified(node, &returns)?;
        let declarator = self.field(node, "declarator")?;
        if declarator.kind() != "function_declarator" {
            return self.refuse(declarator, "an unexpected function declarator");
        }
        let name = self.field(declarator, "declarator")?;
        let mut params = Vec::new();
        let mut names = Vec::new();
        for param in named(self.field(declarator, "parameters")?) {
            if param.kind() != "parameter_declaration" {
                return self.refuse(param, "an unexpected parameter");
            }
            let words = self.type_words(param)?;
            match param.child_by_field_name("declarator") {
                Some(target) if target.kind() == "identifier" => {
                    let target = self.text(target).to_owned();
                    self.name(&target, self.line(param))?;
                    params.push(format!(
                        "{{{target} = {};}}",
                        self.qualified(param, &words)?
                    ));
                    names.push(target);
                }
                None if words == ["void"] => continue,
                _ => return self.refuse(param, "a parameter that is not `type name`"),
            }
        }
        let body = self.scope(self.field(node, "body")?, names, None)?;
        Ok((
            self.text(name).to_owned(),
            format!(
                "({} {returns} [{}] {body})",
                self.uses("fn"),
                params.join(" ")
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
        let params: Vec<String> = match node.child_by_field_name("parameters") {
            None => Vec::new(),
            Some(params) => named(params)
                .into_iter()
                .map(|param| self.text(param).to_owned())
                .collect(),
        };
        if params.len() > 1 {
            return self.refuse(node, "a macro of more than one parameter");
        }
        for param in &params {
            self.name(param, self.line(node))?;
        }
        let inner = Emitter {
            source: &wrapper,
            holes: self.holes,
            globals: self.globals.clone(),
            scopes: RefCell::new(vec![params.clone()]),
            used: RefCell::new(BTreeSet::new()),
            selves: RefCell::new(BTreeSet::new()),
            types: self.types.clone(),
        };
        let body = inner.expression(init)?;
        if let Some(ty) = inner.type_of(init).filter(|_| params.is_empty()) {
            self.types.borrow_mut().insert(name.clone(), ty);
        }
        self.used.borrow_mut().extend(inner.used.into_inner());
        self.selves.borrow_mut().extend(inner.selves.into_inner());
        if params.is_empty() {
            Ok((name, body))
        } else {
            Ok((
                name,
                format!(
                    "({} ({{{}}}: {body}))",
                    self.uses("macro"),
                    params.join(", ")
                ),
            ))
        }
    }

    fn declared(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        if node.kind() != "declaration" {
            return Ok(());
        }
        let mut cursor = node.walk();
        let declarators: Vec<Node<'_>> = node
            .children_by_field_name("declarator", &mut cursor)
            .collect();
        for declarator in declarators {
            let (name, _, _) = self.declarator(declarator)?;
            let name = self.text(name).to_owned();
            self.name(&name, self.line(node))?;
            if out.contains(&name) {
                return self.refuse(node, "a name declared twice in one block");
            }
            out.push(name);
        }
        Ok(())
    }

    fn items<'t>(&self, node: Node<'t>) -> Vec<Node<'t>> {
        if node.kind() == "compound_statement" {
            named(node)
        } else {
            vec![node]
        }
    }

    fn scope(
        &self,
        node: Node<'_>,
        given: Vec<String>,
        header: Option<Node<'_>>,
    ) -> Result<String, String> {
        let mut names = given;
        if let Some(header) = header {
            self.declared(header, &mut names)?;
        }
        let items = self.items(node);
        for item in &items {
            self.declared(*item, &mut names)?;
        }
        self.scopes.borrow_mut().push(names.clone());
        let result = self.statements(&items);
        self.scopes.borrow_mut().pop();
        let list = result?;
        if names.is_empty() {
            Ok(list)
        } else {
            Ok(format!("({{{}}}: {list})", names.join(", ")))
        }
    }

    fn statements(&self, items: &[Node<'_>]) -> Result<String, String> {
        let mut out = Vec::new();
        for item in items {
            out.extend(self.statement(*item)?);
        }
        Ok(format!("[\n{}\n]", out.join("\n")))
    }

    fn local(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let words = self.type_words(node)?;
        let ty = self.qualified(node, &words)?;
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
            let ty = match size {
                Some(size) => format!("({} {ty} {size})", self.uses("sized")),
                None => ty.clone(),
            };
            out.push(format!(
                "({} {} {ty} {value})",
                self.uses("local"),
                self.text(name)
            ));
        }
        Ok(out)
    }

    fn statement(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        Ok(vec![match node.kind() {
            "comment" => format!(
                "({} {})",
                self.uses("comment"),
                self.string(&self.comment(node)?)
            ),
            "declaration" => return self.local(node),
            "expression_statement" => match named(node).as_slice() {
                [inner] => self.effect(*inner)?,
                _ => return self.refuse(node, "an empty statement"),
            },
            "return_statement" => match named(node).as_slice() {
                [] => format!("({} null)", self.uses("return")),
                [value] => format!("({} {})", self.uses("return"), self.expression(*value)?),
                _ => return self.refuse(node, "an unexpected return"),
            },
            "break_statement" => self.uses("break"),
            "continue_statement" => self.uses("continue"),
            "if_statement" => self.conditional(node)?,
            "for_statement" => self.for_loop(node)?,
            "while_statement" => format!(
                "({} {} {})",
                self.uses("while"),
                self.expression(self.field(node, "condition")?)?,
                self.scope(self.field(node, "body")?, Vec::new(), None)?
            ),
            "compound_statement" => return self.refuse(node, "a block that is not a body"),
            _ => return self.refuse(node, "no GLSL statement for this"),
        }])
    }

    fn for_loop(&self, node: Node<'_>) -> Result<String, String> {
        let body = self.field(node, "body")?;
        let header = node.child_by_field_name("initializer");
        let mut names = Vec::new();
        if let Some(header) = header {
            self.declared(header, &mut names)?;
        }
        let items = self.items(body);
        for item in &items {
            self.declared(*item, &mut names)?;
        }
        self.scopes.borrow_mut().push(names.clone());
        let result = (|| -> Result<String, String> {
            let init = match header {
                None => "null".into(),
                Some(init) if init.kind() == "declaration" => match self.local(init)?.as_slice() {
                    [one] => one.clone(),
                    _ => return self.refuse(init, "a for header declaring several names"),
                },
                Some(init) => self.effect(init)?,
            };
            Ok(format!(
                "{{\n init = {init};\n test = {};\n step = {};\n body = {};\n}}",
                self.expression(self.field(node, "condition")?)?,
                self.effect(self.field(node, "update")?)?,
                self.statements(&items)?
            ))
        })();
        self.scopes.borrow_mut().pop();
        let spec = result?;
        let loop_ = if names.is_empty() {
            spec
        } else {
            format!("({{{}}}: {spec})", names.join(", "))
        };
        Ok(format!("({} {loop_})", self.uses("for")))
    }

    fn effect(&self, node: Node<'_>) -> Result<String, String> {
        if node.kind() != "assignment_expression" {
            return self.expression(node);
        }
        let operator = self.text(self.field(node, "operator")?);
        let target = self.expression(self.field(node, "left")?)?;
        let value = self.expression(self.field(node, "right")?)?;
        if operator == "=" {
            Ok(format!("({} {target} {value})", self.uses("set")))
        } else if ASSIGNMENTS.contains(&operator) {
            Ok(format!(
                "({} {target} {} {value})",
                self.uses("update"),
                self.string(operator)
            ))
        } else {
            self.refuse(node, "an unexpected assignment")
        }
    }

    fn conditional(&self, node: Node<'_>) -> Result<String, String> {
        let test = self.expression(self.field(node, "condition")?)?;
        let then = self.scope(self.field(node, "consequence")?, Vec::new(), None)?;
        let Some(clause) = node.child_by_field_name("alternative") else {
            return Ok(format!("({} {test} {then})", self.uses("if'")));
        };
        let other = match named(clause).as_slice() {
            [other] => *other,
            _ => return self.refuse(clause, "an unexpected else"),
        };
        let otherwise = if other.kind() == "if_statement" {
            self.conditional(other)?
        } else {
            self.scope(other, Vec::new(), None)?
        };
        Ok(format!(
            "({} {test} {then} {otherwise})",
            self.uses("ifElse")
        ))
    }
}
