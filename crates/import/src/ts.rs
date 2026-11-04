use std::cell::RefCell;
use std::collections::BTreeSet;

use tree_sitter::Node;

use crate::{line_of, nix};

const NIX_KEYWORDS: [&str; 11] = [
    "assert", "else", "if", "in", "inherit", "let", "null", "or", "rec", "then", "with",
];

const CONSTRUCTORS: [&str; 75] = [
    "array",
    "arrayOf",
    "arrow",
    "async",
    "await",
    "below",
    "blank",
    "braces",
    "break",
    "call",
    "case",
    "catch",
    "chain",
    "class",
    "comment",
    "computed",
    "const",
    "continue",
    "dec",
    "default",
    "delete",
    "dot",
    "export",
    "extends",
    "field",
    "finally",
    "fn",
    "for",
    "forAwait",
    "forIn",
    "forOf",
    "generic",
    "idx",
    "if'",
    "ifElse",
    "import'",
    "inc",
    "infix",
    "interface",
    "intersection",
    "is",
    "let'",
    "method",
    "namespace",
    "neg",
    "new",
    "nonNull",
    "not",
    "num",
    "obj",
    "opt",
    "optCall",
    "optIdx",
    "prop",
    "readonly",
    "regex",
    "render",
    "return",
    "return'",
    "rows",
    "shape",
    "spaces",
    "spread",
    "sq",
    "str",
    "sub",
    "switch",
    "ternary",
    "throw'",
    "tpl",
    "try",
    "type",
    "typeOnly",
    "typed",
    "typeof",
];

const GLOBALS: [&str; 45] = [
    "Array",
    "Awaited",
    "Boolean",
    "Buffer",
    "Bun",
    "Date",
    "Error",
    "Extract",
    "Intl",
    "JSON",
    "Map",
    "Math",
    "NodeJS",
    "Number",
    "Object",
    "Parameters",
    "Partial",
    "Promise",
    "Readonly",
    "ReadonlySet",
    "Record",
    "Response",
    "ReturnType",
    "Set",
    "String",
    "Symbol",
    "WeakMap",
    "WeakSet",
    "any",
    "boolean",
    "console",
    "const",
    "globalThis",
    "never",
    "null",
    "number",
    "object",
    "performance",
    "process",
    "string",
    "super",
    "this",
    "undefined",
    "unknown",
    "void",
];

fn local(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) || GLOBALS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn global(name: &str) -> String {
    if NIX_KEYWORDS.contains(&name) || CONSTRUCTORS.contains(&name) {
        format!("{name}'")
    } else {
        name.to_owned()
    }
}

fn precedence(operator: &str) -> Option<u8> {
    Some(match operator {
        "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "**=" | "<<=" | ">>=" | ">>>=" | "&=" | "|="
        | "^=" | "&&=" | "||=" | "??=" => 1,
        "??" => 2,
        "||" => 3,
        "&&" => 4,
        "|" => 5,
        "^" => 6,
        "&" => 7,
        "==" | "!=" | "===" | "!==" => 8,
        "<" | ">" | "<=" | ">=" | "in" | "instanceof" | "as" | "satisfies" => 9,
        "<<" | ">>" | ">>>" => 10,
        "+" | "-" => 11,
        "*" | "/" | "%" => 12,
        "**" => 13,
        _ => return None,
    })
}

fn level(operator: &str) -> u8 {
    precedence(operator).unwrap_or(0)
}

fn assignment(operator: &str) -> bool {
    level(operator) == 1
}

fn right_associative(operator: &str) -> bool {
    assignment(operator) || operator == "**"
}

fn cast(operator: &str) -> bool {
    operator == "as" || operator == "satisfies"
}

fn multiplicative(operator: &str) -> bool {
    matches!(operator, "*" | "/" | "%")
}

fn bitshift(operator: &str) -> bool {
    matches!(operator, "<<" | ">>" | ">>>")
}

fn bitwise(operator: &str) -> bool {
    bitshift(operator) || matches!(operator, "|" | "^" | "&")
}

fn equality(operator: &str) -> bool {
    matches!(operator, "==" | "!=" | "===" | "!==")
}

fn flatten(parent: &str, child: &str) -> bool {
    !(level(parent) != level(child)
        || parent == "**"
        || equality(parent) && equality(child)
        || child == "%" && multiplicative(parent)
        || parent == "%" && multiplicative(child)
        || child != parent && multiplicative(child) && multiplicative(parent)
        || bitshift(parent) && bitshift(child))
}

fn nested(parent: &str, side: Side, child: &str) -> bool {
    if cast(child) {
        return !cast(parent) || side == Side::Right;
    }
    if assignment(child) || cast(parent) {
        return true;
    }
    let logical = |op: &str| matches!(op, "??" | "||" | "&&");
    if logical(parent) && logical(child) && (parent == "??") != (child == "??") {
        return true;
    }
    if parent == "||" && child == "&&" {
        return true;
    }
    let (outer, inner) = (level(parent), level(child));
    if outer > inner || outer == inner && (side == Side::Right || !flatten(parent, child)) {
        return true;
    }
    if outer < inner && child == "%" {
        return parent == "+" || parent == "-";
    }
    bitwise(parent)
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Slot<'o> {
    Plain,
    Binary(&'o str, Side),
    Unary,
    Spread,
    Member,
    Test,
    Branch,
    ArrowBody,
    Statement,
    ArrayOf,
    Union,
    Intersection,
}

#[derive(Clone, Copy)]
enum Class<'o> {
    Binary(&'o str),
    Ternary,
    Arrow,
    Function,
    Await,
    Unary,
    Negative,
    Object,
    Union,
    Intersection,
    Atom,
}

fn wants(slot: Slot<'_>, class: Class<'_>) -> bool {
    match slot {
        Slot::Plain => false,
        Slot::Binary(parent, side) => {
            if assignment(parent) {
                return side == Side::Left && matches!(class, Class::Binary(op) if cast(op));
            }
            match class {
                Class::Binary(child) => nested(parent, side, child),
                Class::Ternary | Class::Arrow | Class::Await => true,
                Class::Unary | Class::Negative => parent == "**" && side == Side::Left,
                _ => false,
            }
        }
        Slot::Unary => matches!(
            class,
            Class::Binary(_) | Class::Ternary | Class::Arrow | Class::Await
        ),
        Slot::Spread => matches!(class, Class::Binary(_) | Class::Ternary | Class::Await),
        Slot::Member => matches!(
            class,
            Class::Binary(_)
                | Class::Ternary
                | Class::Arrow
                | Class::Function
                | Class::Await
                | Class::Unary
                | Class::Negative
        ),
        Slot::Test => {
            matches!(class, Class::Ternary | Class::Arrow | Class::Await)
                || matches!(class, Class::Binary(op) if cast(op) || assignment(op))
        }
        Slot::Branch => matches!(class, Class::Binary(op) if cast(op) || op == "??"),
        Slot::ArrowBody => matches!(class, Class::Ternary | Class::Object),
        Slot::Statement => matches!(class, Class::Object | Class::Function),
        Slot::ArrayOf => matches!(
            class,
            Class::Union | Class::Intersection | Class::Arrow | Class::Unary
        ),
        Slot::Union => matches!(class, Class::Arrow),
        Slot::Intersection => matches!(class, Class::Union | Class::Arrow),
    }
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), source)?;
    let root = tree.root_node();
    let unit = if source.lines().any(|line| line.starts_with('\t')) {
        "\t"
    } else {
        "  "
    };
    let importer = Importer {
        source,
        holes,
        unit,
        scopes: RefCell::new(Vec::new()),
        bound: RefCell::new(BTreeSet::new()),
    };
    let items = named(root);
    let first = items.first().ok_or("the file is empty")?;
    if first.start_byte() != 0 {
        return Err("the file must start with its first statement".into());
    }
    let last = items.last().ok_or("the file is empty")?;
    if &source[last.end_byte()..] != "\n" {
        return Err("the file must end with one newline after its last statement".into());
    }
    let names = importer.declared(&items)?;
    let body = importer.scope(&names, || {
        Ok(format!(
            "[\n{}]",
            importer
                .statements(&items, 0)?
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        ))
    })?;
    let body = if unit == "\t" {
        body
    } else {
        format!("(spaces 2 {body})")
    };
    let bound = importer.bound.borrow();
    let formals: String = nix::identifiers(&body)
        .into_iter()
        .filter(|name| !bound.contains(name) && !matches!(name.as_str(), "null" | "true" | "false"))
        .map(|name| format!("  {name},\n"))
        .collect();
    Ok(format!("{{\n{formals}  ...\n}}: {body}"))
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
    unit: &'s str,
    scopes: RefCell<Vec<Vec<String>>>,
    bound: RefCell<BTreeSet<String>>,
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor).collect()
}

fn field<'t>(node: Node<'t>, name: &str) -> Option<Node<'t>> {
    node.child_by_field_name(name)
}

fn token<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children(node)
        .into_iter()
        .find(|child| !child.is_named() && child.kind() == kind)
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.join(" "))
}

fn rows(items: &[String]) -> String {
    format!(
        "(rows [\n{}])",
        items
            .iter()
            .map(|item| format!("{item}\n"))
            .collect::<String>()
    )
}

fn lines(items: &[String]) -> String {
    format!(
        "[\n{}]",
        items
            .iter()
            .map(|item| format!("{item}\n"))
            .collect::<String>()
    )
}

fn optional(node: Node<'_>) -> bool {
    children(node)
        .iter()
        .any(|child| child.kind() == "optional_chain" || child.kind() == "?.")
}

const BINARY: [&str; 5] = [
    "binary_expression",
    "assignment_expression",
    "augmented_assignment_expression",
    "as_expression",
    "satisfies_expression",
];

#[derive(Clone, Copy, PartialEq)]
enum Separator {
    Comma,
    Semicolon,
}

impl Separator {
    fn mark(self) -> &'static str {
        match self {
            Separator::Comma => ",",
            Separator::Semicolon => ";",
        }
    }
}

enum Link<'t> {
    Member(bool, Node<'t>, bool),
    Call(Node<'t>, bool, Option<Node<'t>>),
    Index(Node<'t>, bool),
    NonNull,
}

impl<'s> Importer<'s> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn text(&self, node: Node<'_>) -> &'s str {
        &self.source[node.start_byte()..node.end_byte()]
    }

    fn gap(&self, from: usize, to: usize) -> &'s str {
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

    fn indent(&self, depth: usize) -> String {
        format!("\n{}", self.unit.repeat(depth))
    }

    fn broken(&self, from: usize, to: usize, depth: usize) -> Result<bool, String> {
        let gap = self.gap(from, to);
        if !gap.contains('\n') {
            return Ok(false);
        }
        let want = self.indent(depth);
        if gap == want {
            Ok(true)
        } else {
            Err(format!(
                "{}: expected a line break to {want:?}, found {gap:?}",
                self.at(to)
            ))
        }
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn refuse<T>(&self, node: Node<'_>, what: &str) -> Result<T, String> {
        Err(format!("{}: {what}", self.at(node.start_byte())))
    }

    fn add(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        let name = self.text(node);
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return self.refuse(
                node,
                &format!("`{name}` cannot be spelled as a Nix variable"),
            );
        }
        if !out.iter().any(|one| one == name) {
            out.push(name.to_owned());
        }
        Ok(())
    }

    fn binds(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        match node.kind() {
            "identifier" | "shorthand_property_identifier_pattern" => self.add(node, out),
            "object_pattern" | "array_pattern" => {
                for item in named(node) {
                    match item.kind() {
                        "comment" => {}
                        "pair_pattern" => {
                            self.binds(field(item, "value").ok_or("a pair without a value")?, out)?;
                        }
                        _ => self.binds(item, out)?,
                    }
                }
                Ok(())
            }
            "assignment_pattern" | "object_assignment_pattern" => self.binds(
                field(node, "left").ok_or("a default without a target")?,
                out,
            ),
            "rest_pattern" => self.binds(named(node)[0], out),
            "required_parameter" | "optional_parameter" => self.binds(
                field(node, "pattern").ok_or("a parameter without a pattern")?,
                out,
            ),
            kind => self.refuse(node, &format!("{kind} is not a binding pattern")),
        }
    }

    fn declares(&self, node: Node<'_>, out: &mut Vec<String>) -> Result<(), String> {
        match node.kind() {
            "lexical_declaration" => {
                for declarator in named(node) {
                    if let Some(name) = field(declarator, "name") {
                        self.binds(name, out)?;
                    }
                }
                Ok(())
            }
            "function_declaration"
            | "class_declaration"
            | "interface_declaration"
            | "type_alias_declaration" => self.add(
                field(node, "name").ok_or("a declaration without a name")?,
                out,
            ),
            "import_statement" => {
                for clause in named(node)
                    .into_iter()
                    .filter(|child| child.kind() == "import_clause")
                {
                    for part in named(clause) {
                        match part.kind() {
                            "named_imports" => {
                                for specifier in named(part) {
                                    if let Some(name) =
                                        field(specifier, "alias").or(field(specifier, "name"))
                                    {
                                        self.add(name, out)?;
                                    }
                                }
                            }
                            "namespace_import" => self.add(named(part)[0], out)?,
                            "identifier" => self.add(part, out)?,
                            _ => {}
                        }
                    }
                }
                Ok(())
            }
            "export_statement" => match field(node, "declaration") {
                Some(declaration) => self.declares(declaration, out),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn declared(&self, items: &[Node<'_>]) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for item in items {
            self.declares(*item, &mut out)?;
        }
        Ok(out)
    }

    fn block_names(&self, body: Option<Node<'_>>, out: &mut Vec<String>) -> Result<(), String> {
        if let Some(body) = body.filter(|body| body.kind() == "statement_block") {
            for item in named(body) {
                self.declares(item, out)?;
            }
        }
        Ok(())
    }

    fn function_names(&self, node: Node<'_>) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        if let Some(parameter) = field(node, "parameter") {
            self.binds(parameter, &mut out)?;
        }
        if let Some(parameters) = field(node, "parameters") {
            for parameter in named(parameters) {
                if parameter.kind() != "comment" {
                    self.binds(parameter, &mut out)?;
                }
            }
        }
        self.block_names(field(node, "body"), &mut out)?;
        Ok(out)
    }

    fn scope(
        &self,
        names: &[String],
        body: impl FnOnce() -> Result<String, String>,
    ) -> Result<String, String> {
        self.scopes.borrow_mut().push(names.to_vec());
        let result = body();
        self.scopes.borrow_mut().pop();
        let value = result?;
        if names.is_empty() {
            return Ok(value);
        }
        let spelled: Vec<String> = names.iter().map(|name| local(name)).collect();
        self.bound.borrow_mut().extend(spelled.iter().cloned());
        Ok(format!("({{{}}}: {value})", spelled.join(", ")))
    }

    fn name(&self, node: Node<'_>) -> Result<String, String> {
        let name = self.text(node);
        if self
            .scopes
            .borrow()
            .iter()
            .any(|scope| scope.iter().any(|one| one == name))
        {
            return Ok(local(name));
        }
        if GLOBALS.contains(&name) {
            return Ok(global(name));
        }
        self.refuse(
            node,
            &format!("`{name}` is declared nowhere in the file and is not a global the ts generator knows"),
        )
    }

    fn property(&self, node: Node<'_>) -> Result<String, String> {
        match node.kind() {
            "property_identifier" | "private_property_identifier" | "type_identifier" => {
                Ok(self.string(self.text(node)))
            }
            kind => self.refuse(node, &format!("{kind} is not a property name")),
        }
    }

    fn separated(
        &self,
        items: &[Node<'_>],
        depth: usize,
        mut item: impl FnMut(Node<'_>) -> Result<String, String>,
    ) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(items[index - 1].end_byte(), node.start_byte());
                let count = gap.bytes().take_while(|&b| b == b'\n').count();
                if count == 0 || gap[count..] != self.unit.repeat(depth) {
                    return Err(format!(
                        "{}: expected a line break to indentation {depth}, found {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..count {
                    out.push("blank".to_owned());
                }
            }
            out.push(item(*node)?);
        }
        Ok(out)
    }

    fn statements(&self, items: &[Node<'_>], depth: usize) -> Result<Vec<String>, String> {
        self.separated(items, depth, |node| self.statement(node, depth))
    }

    fn comment(&self, node: Node<'_>) -> String {
        format!("(comment {})", self.string(self.text(node)))
    }

    fn block(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut names = Vec::new();
        self.block_names(Some(node), &mut names)?;
        self.scope(&names, || self.block_list(node, depth))
    }

    fn block_list(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if node.kind() != "statement_block" {
            return self.refuse(node, &format!("expected a block, found {}", node.kind()));
        }
        let items = named(node);
        let close = node.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(node.start_byte() + 1, close, "")?;
            return Ok("[]".into());
        };
        let lead = self.gap(node.start_byte() + 1, first.start_byte());
        let blanks = lead
            .bytes()
            .take_while(|&b| b == b'\n')
            .count()
            .saturating_sub(1);
        self.expect(
            node.start_byte() + 1 + blanks,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        let last = items.last().unwrap_or(first);
        self.expect(last.end_byte(), close, &self.indent(depth))?;
        let mut out = vec!["blank".to_owned(); blanks];
        out.extend(self.statements(&items, depth + 1)?);
        Ok(lines(&out))
    }

    fn body(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if node.kind() == "statement_block" {
            self.block(node, depth)
        } else {
            self.statement(node, depth)
        }
    }

    fn semicolon(&self, node: Node<'_>, inner: Node<'_>) -> Result<(), String> {
        let semi = token(node, ";").ok_or_else(|| {
            format!(
                "{}: a statement without its semicolon",
                self.at(node.start_byte())
            )
        })?;
        self.expect(inner.end_byte(), semi.start_byte(), "")?;
        if semi.end_byte() != node.end_byte() {
            return Err(format!(
                "{}: text after a semicolon",
                self.at(semi.end_byte())
            ));
        }
        Ok(())
    }

    fn statement(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        match node.kind() {
            "comment" => Ok(self.comment(node)),
            "expression_statement" => {
                let inner = named(node)[0];
                self.semicolon(node, inner)?;
                self.child(Slot::Statement, inner, depth)
            }
            "lexical_declaration" => {
                let declaration = self.declaration(node, depth)?;
                let last = *named(node).last().ok_or("an empty declaration")?;
                self.semicolon(node, last)?;
                Ok(declaration)
            }
            "return_statement" => match named(node).first() {
                None => {
                    self.expect(start + 6, node.end_byte(), ";")?;
                    Ok("return'".into())
                }
                Some(value) => {
                    self.expect(start + 6, value.start_byte(), " ")?;
                    self.semicolon(node, *value)?;
                    if value.kind() == "parenthesized_expression" {
                        let inner = named(*value)[0];
                        if self.broken(value.start_byte() + 1, inner.start_byte(), depth + 1)? {
                            self.expect(
                                inner.end_byte(),
                                value.end_byte() - 1,
                                &self.indent(depth),
                            )?;
                            return Ok(format!(
                                "(return (below {}))",
                                self.expr(inner, depth + 1)?
                            ));
                        }
                    }
                    Ok(format!("(return {})", self.expr(*value, depth)?))
                }
            },
            "throw_statement" => {
                let value = named(node)[0];
                self.expect(start + 5, value.start_byte(), " ")?;
                self.semicolon(node, value)?;
                Ok(format!("(throw' {})", self.expr(value, depth)?))
            }
            "break_statement" | "continue_statement" => {
                let word = if node.kind() == "break_statement" {
                    "break"
                } else {
                    "continue"
                };
                if self.text(node) != format!("{word};") {
                    return self.refuse(node, &format!("a labelled {word}"));
                }
                Ok(word.to_owned())
            }
            "if_statement" => self.conditional(node, depth),
            "for_statement" => self.for_loop(node, depth),
            "for_in_statement" => self.for_in(node, depth),
            "while_statement" => {
                let condition = field(node, "condition").ok_or("while without a condition")?;
                let body = field(node, "body").ok_or("while without a body")?;
                self.expect(start + 5, condition.start_byte(), " ")?;
                self.expect(condition.end_byte(), body.start_byte(), " ")?;
                Ok(format!(
                    "(while {} {})",
                    self.condition(condition, depth)?,
                    self.body(body, depth)?
                ))
            }
            "switch_statement" => self.switch(node, depth),
            "try_statement" => self.attempt(node, depth),
            "function_declaration" => self.function(node, depth),
            "class_declaration" => self.class(node, depth),
            "interface_declaration" => self.interface(node, depth),
            "type_alias_declaration" => {
                let name = field(node, "name").ok_or("type without a name")?;
                let value = field(node, "value").ok_or("type without a value")?;
                if field(node, "type_parameters").is_some() {
                    return self.refuse(node, "a generic type alias");
                }
                self.expect(start + 4, name.start_byte(), " ")?;
                self.expect(name.end_byte(), value.start_byte(), " = ")?;
                self.semicolon(node, value)?;
                Ok(format!(
                    "(type {} {})",
                    self.name(name)?,
                    self.expr(value, depth)?
                ))
            }
            "export_statement" => self.export(node, depth),
            "import_statement" => self.import(node),
            kind => self.refuse(node, &format!("{kind} is not supported")),
        }
    }

    fn declaration(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let declarators = named(node);
        let [declarator] = declarators.as_slice() else {
            return self.refuse(node, "one declarator per declaration");
        };
        let kind = self.text(node.child(0).ok_or("no kind")?);
        let constructor = match kind {
            "const" => "const",
            "let" => "let'",
            other => return self.refuse(node, &format!("{other} is not supported")),
        };
        self.expect(node.start_byte() + kind.len(), declarator.start_byte(), " ")?;
        let name = field(*declarator, "name").ok_or("a declarator without a name")?;
        let mut target = self.pattern(name, depth)?;
        let mut end = name.end_byte();
        if let Some(annotation) = field(*declarator, "type") {
            self.expect(end, annotation.start_byte(), "")?;
            target = format!("(typed {target} {})", self.annotation(annotation, depth)?);
            end = annotation.end_byte();
        }
        let value = match field(*declarator, "value") {
            None => "null".to_owned(),
            Some(value) => {
                let equals = token(*declarator, "=").ok_or("no =")?;
                self.expect(end, equals.start_byte(), " ")?;
                self.hang(equals.end_byte(), value, depth)?
            }
        };
        Ok(format!("({constructor} {target} {value})"))
    }

    fn hang(&self, after: usize, value: Node<'_>, depth: usize) -> Result<String, String> {
        if self.broken(after, value.start_byte(), depth + 1)? {
            Ok(format!("(below {})", self.expr(value, depth + 1)?))
        } else {
            self.expect(after, value.start_byte(), " ")?;
            self.expr(value, depth)
        }
    }

    fn annotation(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let inner = named(node);
        let [value] = inner.as_slice() else {
            return self.refuse(node, &format!("an annotation of {} parts", inner.len()));
        };
        self.expect(node.start_byte() + 1, value.start_byte(), " ")?;
        self.expr(*value, depth)
    }

    fn condition(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let inner = named(node)[0];
        if self.broken(node.start_byte() + 1, inner.start_byte(), depth + 1)? {
            self.expect(inner.end_byte(), node.end_byte() - 1, &self.indent(depth))?;
            return Ok(format!("(below {})", self.expr(inner, depth + 1)?));
        }
        self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
        self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
        self.expr(inner, depth)
    }

    fn conditional(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let condition = field(node, "condition").ok_or("if without a condition")?;
        let consequence = field(node, "consequence").ok_or("if without a body")?;
        self.expect(node.start_byte() + 2, condition.start_byte(), " ")?;
        self.expect(condition.end_byte(), consequence.start_byte(), " ")?;
        let test = self.condition(condition, depth)?;
        let body = self.body(consequence, depth)?;
        let Some(alternative) = field(node, "alternative") else {
            return Ok(format!("(if' {test} {body})"));
        };
        let lead = if consequence.kind() == "statement_block" {
            " "
        } else {
            &self.indent(depth)
        };
        self.expect(consequence.end_byte(), alternative.start_byte(), lead)?;
        let other = named(alternative)[0];
        self.expect(alternative.start_byte() + 4, other.start_byte(), " ")?;
        Ok(format!(
            "(ifElse {test} {body} {})",
            self.body(other, depth)?
        ))
    }

    fn for_loop(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let body = field(node, "body").ok_or("for without a body")?;
        let open = token(node, "(").ok_or("no (")?;
        let close = token(node, ")").ok_or("no )")?;
        self.expect(node.start_byte() + 3, open.start_byte(), " ")?;
        self.expect(close.end_byte(), body.start_byte(), " ")?;
        let init = field(node, "initializer").filter(|init| init.kind() != "empty_statement");
        let condition = field(node, "condition")
            .filter(|condition| condition.kind() != "empty_statement" && condition.kind() != ";")
            .map(|condition| {
                if condition.kind() == "expression_statement" {
                    named(condition)[0]
                } else {
                    condition
                }
            });
        let increment = field(node, "increment");
        let mut names = Vec::new();
        if let Some(init) = init {
            self.declares(init, &mut names)?;
        }
        self.block_names(Some(body), &mut names)?;
        self.scope(&names, || {
            let mut want = String::new();
            let init = match init {
                None => "null".to_owned(),
                Some(init) if init.kind() == "lexical_declaration" => {
                    let declarators = named(init);
                    want.push_str(self.gap(init.start_byte(), declarators[0].end_byte()));
                    self.declaration(init, depth)?
                }
                Some(init) => {
                    want.push_str(self.text(init).trim_end_matches(';'));
                    let inner = if init.kind() == "expression_statement" {
                        named(init)[0]
                    } else {
                        init
                    };
                    self.expr(inner, depth)?
                }
            };
            want.push(';');
            let condition = match condition {
                None => "null".to_owned(),
                Some(condition) => {
                    want.push(' ');
                    want.push_str(self.text(condition));
                    self.expr(condition, depth)?
                }
            };
            want.push(';');
            let increment = match increment {
                None => "null".to_owned(),
                Some(increment) => {
                    want.push(' ');
                    want.push_str(self.text(increment));
                    self.expr(increment, depth)?
                }
            };
            self.expect(open.end_byte(), close.start_byte(), &want)?;
            Ok(format!(
                "(for {init} {condition} {increment} {})",
                self.loop_body(body, depth)?
            ))
        })
    }

    fn loop_body(&self, body: Node<'_>, depth: usize) -> Result<String, String> {
        if body.kind() == "statement_block" {
            self.block_list(body, depth)
        } else {
            self.statement(body, depth)
        }
    }

    fn for_in(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let left = field(node, "left").ok_or("for without a target")?;
        let right = field(node, "right").ok_or("for without a subject")?;
        let body = field(node, "body").ok_or("for without a body")?;
        let kind = field(node, "kind").ok_or("for without const")?;
        let operator = field(node, "operator").ok_or("for without of or in")?;
        let open = token(node, "(").ok_or("no (")?;
        let waiting = token(node, "await");
        match waiting {
            Some(word) => {
                self.expect(node.start_byte() + 3, word.start_byte(), " ")?;
                self.expect(word.end_byte(), open.start_byte(), " ")?;
            }
            None => self.expect(node.start_byte() + 3, open.start_byte(), " ")?,
        }
        self.expect(open.end_byte(), kind.start_byte(), "")?;
        self.expect(kind.end_byte(), left.start_byte(), " ")?;
        self.expect(left.end_byte(), operator.start_byte(), " ")?;
        self.expect(operator.end_byte(), right.start_byte(), " ")?;
        let close = token(node, ")").ok_or("no )")?;
        self.expect(right.end_byte(), close.start_byte(), "")?;
        self.expect(close.end_byte(), body.start_byte(), " ")?;
        let constructor = match (self.text(kind), self.text(operator)) {
            ("const", "of") if waiting.is_some() => "forAwait",
            ("const", "of") => "forOf",
            ("const", "in") => "forIn",
            (k, o) => return self.refuse(node, &format!("for ({k} .. {o}) is not supported")),
        };
        let subject = self.expr(right, depth)?;
        let mut names = Vec::new();
        self.binds(left, &mut names)?;
        self.block_names(Some(body), &mut names)?;
        self.scope(&names, || {
            Ok(format!(
                "({constructor} {} {subject} {})",
                self.pattern(left, depth)?,
                self.loop_body(body, depth)?
            ))
        })
    }

    fn switch(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let value = field(node, "value").ok_or("switch without a value")?;
        let body = field(node, "body").ok_or("switch without a body")?;
        self.expect(node.start_byte() + 6, value.start_byte(), " ")?;
        self.expect(value.end_byte(), body.start_byte(), " ")?;
        let cases = named(body);
        let first = cases.first().ok_or("an empty switch")?;
        self.expect(
            body.start_byte() + 1,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        self.expect(
            cases.last().unwrap_or(first).end_byte(),
            body.end_byte() - 1,
            &self.indent(depth),
        )?;
        let subject = self.condition(value, depth)?;
        let mut names = Vec::new();
        for case in &cases {
            for item in named(*case) {
                if field(*case, "value") != Some(item) {
                    self.declares(item, &mut names)?;
                }
            }
        }
        self.scope(&names, || {
            let arms = self.separated(&cases, depth + 1, |case| {
                if case.kind() == "comment" {
                    return Ok(self.comment(case));
                }
                let colon = token(case, ":").ok_or("a case without a colon")?;
                let test = match field(case, "value") {
                    Some(test) => {
                        self.expect(case.start_byte() + 4, test.start_byte(), " ")?;
                        self.expect(test.end_byte(), colon.start_byte(), "")?;
                        self.expr(test, depth + 1)?
                    }
                    None => {
                        self.expect(case.start_byte() + 7, colon.start_byte(), "")?;
                        "null".to_owned()
                    }
                };
                let statements: Vec<_> = named(case)
                    .into_iter()
                    .filter(|child| child.start_byte() > colon.start_byte())
                    .collect();
                if let [block] = statements.as_slice() {
                    if block.kind() == "statement_block"
                        && self.gap(colon.end_byte(), block.start_byte()) == " "
                    {
                        return Ok(format!(
                            "(case {test} (braces {}))",
                            self.block(*block, depth + 1)?
                        ));
                    }
                }
                if let Some(first) = statements.first() {
                    self.expect(
                        colon.end_byte(),
                        first.start_byte(),
                        &self.indent(depth + 2),
                    )?;
                }
                let body = self.statements(&statements, depth + 2)?;
                Ok(format!("(case {test} {})", list(&body)))
            })?;
            Ok(format!("(switch {subject} {})", lines(&arms)))
        })
    }

    fn attempt(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let body = field(node, "body").ok_or("try without a body")?;
        self.expect(node.start_byte() + 3, body.start_byte(), " ")?;
        let mut handlers = Vec::new();
        let mut end = body.end_byte();
        if let Some(handler) = field(node, "handler") {
            self.expect(end, handler.start_byte(), " ")?;
            let block = field(handler, "body").ok_or("catch without a body")?;
            let parameter = field(handler, "parameter");
            let mut names = Vec::new();
            if let Some(parameter) = parameter {
                self.binds(parameter, &mut names)?;
            }
            self.block_names(Some(block), &mut names)?;
            handlers.push(self.scope(&names, || {
                let parameter = match parameter {
                    Some(parameter) => {
                        let open = token(handler, "(").ok_or("no (")?;
                        let close = token(handler, ")").ok_or("no )")?;
                        self.expect(handler.start_byte() + 5, open.start_byte(), " ")?;
                        self.expect(open.end_byte(), parameter.start_byte(), "")?;
                        self.expect(parameter.end_byte(), close.start_byte(), "")?;
                        self.expect(close.end_byte(), block.start_byte(), " ")?;
                        self.pattern(parameter, depth)?
                    }
                    None => {
                        self.expect(handler.start_byte() + 5, block.start_byte(), " ")?;
                        "null".to_owned()
                    }
                };
                Ok(format!(
                    "(catch {parameter} {})",
                    self.block_list(block, depth)?
                ))
            })?);
            end = handler.end_byte();
        }
        if let Some(finalizer) = field(node, "finalizer") {
            self.expect(end, finalizer.start_byte(), " ")?;
            let block = field(finalizer, "body").ok_or("finally without a body")?;
            self.expect(finalizer.start_byte() + 7, block.start_byte(), " ")?;
            handlers.push(format!("(finally {})", self.block(block, depth)?));
        }
        Ok(format!(
            "(try {} {})",
            self.block(body, depth)?,
            list(&handlers)
        ))
    }

    fn export(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        if let Some(declaration) = field(node, "declaration") {
            if let Some(default) = token(node, "default") {
                self.expect(start + 6, default.start_byte(), " ")?;
                self.expect(default.end_byte(), declaration.start_byte(), " ")?;
                return Ok(format!(
                    "(export (default {}))",
                    self.statement(declaration, depth)?
                ));
            }
            self.expect(start + 6, declaration.start_byte(), " ")?;
            return Ok(format!("(export {})", self.statement(declaration, depth)?));
        }
        let value = field(node, "value")
            .ok_or_else(|| format!("{}: this export is not supported", self.at(start)))?;
        let default = token(node, "default").ok_or("an export without default")?;
        self.expect(start + 6, default.start_byte(), " ")?;
        self.expect(default.end_byte(), value.start_byte(), " ")?;
        if value.kind() == "function_expression" || value.kind() == "class" {
            return self.refuse(node, "a default export of a function or class expression");
        }
        self.semicolon(node, value)?;
        Ok(format!("(export (default {}))", self.expr(value, depth)?))
    }

    fn import(&self, node: Node<'_>) -> Result<String, String> {
        let start = node.start_byte();
        let source = field(node, "source").ok_or("import without a source")?;
        let clause = named(node)
            .into_iter()
            .find(|child| child.kind() == "import_clause")
            .ok_or_else(|| format!("{}: an import without a clause", self.at(start)))?;
        let typed = token(node, "type");
        let lead = typed.map_or(start + 6, |word| word.end_byte());
        if let Some(word) = typed {
            self.expect(start + 6, word.start_byte(), " ")?;
        }
        self.expect(lead, clause.start_byte(), " ")?;
        let from = token(node, "from").ok_or("no from")?;
        self.expect(clause.end_byte(), from.start_byte(), " ")?;
        self.expect(from.end_byte(), source.start_byte(), " ")?;
        self.semicolon(node, source)?;
        let parts = named(clause);
        let [part] = parts.as_slice() else {
            return self.refuse(node, &format!("an import clause of {} parts", parts.len()));
        };
        let names = match part.kind() {
            "named_imports" => {
                let specifiers = named(*part);
                self.delimited(*part, &specifiers, 0, Separator::Comma, true, |specifier| {
                    let name = field(specifier, "name").ok_or("no name")?;
                    if field(specifier, "alias").is_some() {
                        return self.refuse(specifier, "an import renamed with as");
                    }
                    let typed = token(specifier, "type");
                    match typed {
                        Some(word) => self.expect(word.end_byte(), name.start_byte(), " ")?,
                        None => self.expect(specifier.start_byte(), name.start_byte(), "")?,
                    }
                    if name.end_byte() != specifier.end_byte() {
                        return self.refuse(specifier, "text after an import specifier");
                    }
                    let value = self.name(name)?;
                    Ok(if typed.is_some() {
                        format!("(typeOnly {value})")
                    } else {
                        value
                    })
                })?
            }
            "namespace_import" => {
                let name = named(*part)[0];
                self.expect(part.start_byte(), name.start_byte(), "* as ")?;
                format!("(namespace {})", self.name(name)?)
            }
            "identifier" => self.name(*part)?,
            kind => return self.refuse(node, &format!("{kind} in an import")),
        };
        let names = if typed.is_some() {
            format!("(typeOnly {names})")
        } else {
            names
        };
        Ok(format!("(import' {names} {})", self.bare(source, 0)?))
    }

    fn delimited(
        &self,
        node: Node<'_>,
        items: &[Node<'_>],
        depth: usize,
        separator: Separator,
        padded: bool,
        mut item: impl FnMut(Node<'_>) -> Result<String, String>,
    ) -> Result<String, String> {
        let open = node.start_byte() + 1;
        let close = node.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(open, close, "")?;
            return Ok("[]".into());
        };
        let last = *items.last().unwrap_or(first);
        let mark = separator.mark();
        if !self.gap(open, first.start_byte()).contains('\n') {
            let pad = if padded { " " } else { "" };
            self.expect(open, first.start_byte(), pad)?;
            for pair in items.windows(2) {
                self.expect(
                    pair[0].end_byte(),
                    pair[1].start_byte(),
                    &format!("{mark} "),
                )?;
            }
            let hugged = separator == Separator::Comma && !padded && self.hangs(last);
            let tail = if hugged {
                format!(",{}", self.indent(depth))
            } else {
                pad.to_owned()
            };
            self.expect(last.end_byte(), close, &tail)?;
            let values = items
                .iter()
                .map(|node| item(*node))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(list(&values));
        }
        self.expect(open, first.start_byte(), &self.indent(depth + 1))?;
        let mut out = Vec::new();
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let previous = items[index - 1];
                let mut gap = self.gap(previous.end_byte(), node.start_byte());
                if previous.kind() != "comment" {
                    gap = gap.strip_prefix(mark).ok_or_else(|| {
                        format!(
                            "{}: expected {mark:?} after an item",
                            self.at(previous.end_byte())
                        )
                    })?;
                }
                let count = gap.bytes().take_while(|&b| b == b'\n').count();
                if count == 0 || gap[count..] != self.unit.repeat(depth + 1) {
                    return Err(format!(
                        "{}: items separated by {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..count {
                    out.push("blank".to_owned());
                }
            }
            out.push(if node.kind() == "comment" {
                self.comment(*node)
            } else {
                item(*node)?
            });
        }
        let rest = last.kind() == "rest_pattern"
            || field(last, "pattern").is_some_and(|pattern| pattern.kind() == "rest_pattern");
        let trailing = if last.kind() == "comment" || (separator == Separator::Comma && rest) {
            ""
        } else {
            mark
        };
        self.expect(
            last.end_byte(),
            close,
            &format!("{trailing}{}", self.indent(depth)),
        )?;
        Ok(rows(&out))
    }

    fn class_of(&self, node: Node<'_>) -> Class<'s> {
        match node.kind() {
            "binary_expression" => {
                Class::Binary(field(node, "operator").map_or("", |operator| self.text(operator)))
            }
            "assignment_expression" => Class::Binary("="),
            "augmented_assignment_expression" => {
                Class::Binary(field(node, "operator").map_or("", |operator| self.text(operator)))
            }
            "as_expression" => Class::Binary("as"),
            "satisfies_expression" => Class::Binary("satisfies"),
            "ternary_expression" => Class::Ternary,
            "arrow_function" | "function_type" => Class::Arrow,
            "function_expression" => Class::Function,
            "await_expression" => Class::Await,
            "unary_expression" => {
                let negative = field(node, "operator").is_some_and(|op| self.text(op) == "-")
                    && field(node, "argument").is_some_and(|value| value.kind() == "number");
                if negative {
                    Class::Negative
                } else {
                    Class::Unary
                }
            }
            "type_query" => Class::Unary,
            "object" => Class::Object,
            "union_type" => Class::Union,
            "intersection_type" => Class::Intersection,
            _ => Class::Atom,
        }
    }

    fn child(&self, slot: Slot<'_>, node: Node<'_>, depth: usize) -> Result<String, String> {
        let parenthesized = matches!(
            node.kind(),
            "parenthesized_expression" | "parenthesized_type"
        );
        let inner = if parenthesized {
            let parts = named(node);
            let [inner] = parts.as_slice() else {
                return self.refuse(node, "parentheses around several expressions");
            };
            self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
            self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
            *inner
        } else {
            node
        };
        let wanted = wants(slot, self.class_of(inner));
        if wanted && !parenthesized {
            return self.refuse(
                node,
                "an expression without the parentheses the printer writes around it here",
            );
        }
        if parenthesized && !wanted {
            return self.refuse(node, "parentheses the printer would not write");
        }
        self.bare(inner, depth)
    }

    fn expr(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        self.child(Slot::Plain, node, depth)
    }

    fn number(&self, node: Node<'_>, negative: bool) -> Result<String, String> {
        let text = self.text(node);
        let sign = if negative { "-" } else { "" };
        let canonical = text.bytes().all(|byte| byte.is_ascii_digit())
            && (text == "0" || !text.starts_with('0'))
            && text.parse::<i64>().is_ok();
        if canonical {
            return Ok(if negative {
                format!("({sign}{text})")
            } else {
                text.to_owned()
            });
        }
        let float = text.parse::<f64>().ok().filter(|value| {
            text.contains('.') && !text.starts_with('.') && format!("{value}") == text
        });
        if float.is_some() {
            return Ok(if negative {
                format!("({sign}{text})")
            } else {
                text.to_owned()
            });
        }
        if negative {
            return self.refuse(node, "a negative number in a spelling Nix does not share");
        }
        Ok(format!("(num {})", self.string(text)))
    }

    fn pattern(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        match node.kind() {
            "identifier" => self.name(node),
            "object_pattern" | "array_pattern" => self.bare(node, depth),
            kind => self.refuse(node, &format!("{kind} is not a binding pattern")),
        }
    }

    fn bare(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let kind = node.kind();
        match kind {
            "identifier"
            | "type_identifier"
            | "shorthand_property_identifier"
            | "predefined_type"
            | "this"
            | "super"
            | "undefined"
            | "const"
            | "null" => self.name(node),
            "true" | "false" => Ok(kind.to_owned()),
            "number" => self.number(node, false),
            "nested_type_identifier" => {
                let parts = named(node);
                let [module, name] = parts.as_slice() else {
                    return self.refuse(node, "a qualified type of more than two names");
                };
                self.expect(module.end_byte(), name.start_byte(), ".")?;
                Ok(format!(
                    "(dot {} {})",
                    self.name(*module)?,
                    self.string(self.text(*name))
                ))
            }
            "member_expression"
            | "call_expression"
            | "subscript_expression"
            | "non_null_expression" => self.postfix(node, depth),
            "string" => {
                let text = self.text(node);
                let raw = &text[1..text.len() - 1];
                let constructor = if text.starts_with('"') { "str" } else { "sq" };
                Ok(format!("({constructor} {})", self.string(raw)))
            }
            "template_string" => self.template(node, depth),
            "regex" => {
                let pattern = field(node, "pattern").ok_or("a regex without a pattern")?;
                let flags = field(node, "flags").map_or("", |flags| self.text(flags));
                Ok(format!(
                    "(regex {} {})",
                    self.string(self.text(pattern)),
                    self.string(flags)
                ))
            }
            "array" | "array_pattern" => {
                let items = named(node);
                let inner = depth + usize::from(self.multiline(node));
                let pattern = kind == "array_pattern";
                Ok(format!(
                    "(array {})",
                    self.delimited(node, &items, depth, Separator::Comma, false, |item| {
                        if pattern {
                            self.element(item, inner)
                        } else {
                            self.expr(item, inner)
                        }
                    })?
                ))
            }
            "object" | "object_pattern" => {
                let items = named(node);
                let inner = depth + usize::from(self.multiline(node));
                Ok(format!(
                    "(obj {})",
                    self.delimited(node, &items, depth, Separator::Comma, true, |item| self
                        .member(item, inner))?
                ))
            }
            "object_type" => {
                let items = named(node);
                let inner = depth + usize::from(self.multiline(node));
                if self.text(node).contains(',')
                    && !self.text(node).contains(';')
                    && !items.is_empty()
                {
                    return self.refuse(node, "an object type separated by commas");
                }
                Ok(format!(
                    "(shape {})",
                    self.delimited(node, &items, depth, Separator::Semicolon, true, |item| self
                        .member(item, inner))?
                ))
            }
            _ if BINARY.contains(&kind) => self.infix(node, depth),
            "unary_expression" => {
                let operator = field(node, "operator").ok_or("no operator")?;
                let argument = field(node, "argument").ok_or("no operand")?;
                let op = self.text(operator);
                let gap = if op.chars().all(|c| c.is_ascii_alphabetic()) {
                    " "
                } else {
                    ""
                };
                self.expect(operator.end_byte(), argument.start_byte(), gap)?;
                if op == "-" && argument.kind() == "number" {
                    return self.number(argument, true);
                }
                let constructor = match op {
                    "!" => "not",
                    "-" => "neg",
                    "typeof" => "typeof",
                    "delete" => "delete",
                    other => {
                        return self.refuse(node, &format!("the unary {other} is not supported"))
                    }
                };
                Ok(format!(
                    "({constructor} {})",
                    self.child(Slot::Unary, argument, depth)?
                ))
            }
            "await_expression" | "type_query" | "readonly_type" => {
                let parts = children(node);
                let keyword = parts[0];
                let argument = *parts.last().ok_or("no operand")?;
                self.expect(keyword.end_byte(), argument.start_byte(), " ")?;
                let constructor = match kind {
                    "await_expression" => "await",
                    "type_query" => "typeof",
                    _ => "readonly",
                };
                Ok(format!(
                    "({constructor} {})",
                    self.child(Slot::Unary, argument, depth)?
                ))
            }
            "spread_element" => {
                let inner = named(node)[0];
                self.expect(start + 3, inner.start_byte(), "")?;
                Ok(format!(
                    "(spread {})",
                    self.child(Slot::Spread, inner, depth)?
                ))
            }
            "rest_pattern" => {
                let inner = named(node)[0];
                self.expect(start + 3, inner.start_byte(), "")?;
                Ok(format!("(spread {})", self.pattern(inner, depth)?))
            }
            "update_expression" => {
                let parts = children(node);
                if !parts[0].is_named() {
                    return self.refuse(node, "a prefix increment");
                }
                self.expect(parts[0].end_byte(), parts[1].start_byte(), "")?;
                let constructor = if self.text(parts[1]) == "++" {
                    "inc"
                } else {
                    "dec"
                };
                Ok(format!(
                    "({constructor} {})",
                    self.child(Slot::Member, parts[0], depth)?
                ))
            }
            "array_type" => {
                let inner = named(node)[0];
                self.expect(inner.end_byte(), node.end_byte(), "[]")?;
                Ok(format!(
                    "(arrayOf {})",
                    self.child(Slot::ArrayOf, inner, depth)?
                ))
            }
            "literal_type" => self.bare(named(node)[0], depth),
            "union_type" | "intersection_type" => {
                let mut members = Vec::new();
                let mut current = node;
                loop {
                    let parts = children(current);
                    let [left, bar, right] = parts.as_slice() else {
                        return self.refuse(current, "a leading union bar");
                    };
                    self.expect(left.end_byte(), bar.start_byte(), " ")?;
                    self.expect(bar.end_byte(), right.start_byte(), " ")?;
                    members.push(*right);
                    if left.kind() == kind {
                        current = *left;
                    } else {
                        members.push(*left);
                        break;
                    }
                }
                members.reverse();
                let (constructor, slot) = if kind == "union_type" {
                    ("union", Slot::Union)
                } else {
                    ("intersection", Slot::Intersection)
                };
                let values = members
                    .iter()
                    .map(|member| self.child(slot, *member, depth))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("({constructor} {})", list(&values)))
            }
            "ternary_expression" => {
                let condition = field(node, "condition").ok_or("no condition")?;
                let consequence = field(node, "consequence").ok_or("no consequence")?;
                let alternative = field(node, "alternative").ok_or("no alternative")?;
                let question = token(node, "?").ok_or("no ?")?;
                let colon = token(node, ":").ok_or("no :")?;
                let branch =
                    |before: usize, mark: Node<'_>, value: Node<'_>| -> Result<String, String> {
                        self.expect(mark.end_byte(), value.start_byte(), " ")?;
                        if self.broken(before, mark.start_byte(), depth + 1)? {
                            Ok(format!(
                                "(below {})",
                                self.child(Slot::Branch, value, depth + 2)?
                            ))
                        } else {
                            self.expect(before, mark.start_byte(), " ")?;
                            self.child(Slot::Branch, value, depth)
                        }
                    };
                Ok(format!(
                    "(ternary {} {} {})",
                    self.child(Slot::Test, condition, depth)?,
                    branch(condition.end_byte(), question, consequence)?,
                    branch(consequence.end_byte(), colon, alternative)?
                ))
            }
            "arrow_function" => self.arrow(node, depth),
            "function_expression" => self.function(node, depth),
            "function_type" => {
                let parameters =
                    field(node, "parameters").ok_or("a function type without parameters")?;
                let result =
                    field(node, "return_type").ok_or("a function type without a result")?;
                self.expect(parameters.end_byte(), result.start_byte(), " => ")?;
                let names = self.function_names(node)?;
                self.scope(&names, || {
                    Ok(format!(
                        "(arrow {} {})",
                        self.parameters(parameters, depth)?,
                        self.expr(result, depth)?
                    ))
                })
            }
            "new_expression" => {
                let constructor = field(node, "constructor").ok_or("new without a constructor")?;
                let arguments = field(node, "arguments")
                    .ok_or_else(|| format!("{}: new without arguments", self.at(start)))?;
                self.expect(start + 3, constructor.start_byte(), " ")?;
                let mut callee = self.child(Slot::Member, constructor, depth)?;
                let mut end = constructor.end_byte();
                if let Some(types) = field(node, "type_arguments") {
                    self.expect(end, types.start_byte(), "")?;
                    callee = format!("(generic {callee} {})", self.type_arguments(types, depth)?);
                    end = types.end_byte();
                }
                self.expect(end, arguments.start_byte(), "")?;
                Ok(format!(
                    "(new {callee} {})",
                    self.arguments(arguments, depth)?
                ))
            }
            "generic_type" => {
                let name = field(node, "name").ok_or("a generic without a name")?;
                let types = field(node, "type_arguments").ok_or("a generic without arguments")?;
                self.expect(name.end_byte(), types.start_byte(), "")?;
                Ok(format!(
                    "(generic {} {})",
                    self.bare(name, depth)?,
                    self.type_arguments(types, depth)?
                ))
            }
            "lookup_type" => {
                let parts = named(node);
                self.expect(parts[0].end_byte(), parts[1].start_byte(), "[")?;
                self.expect(parts[1].end_byte(), node.end_byte(), "]")?;
                Ok(format!(
                    "(idx {} {})",
                    self.child(Slot::Member, parts[0], depth)?,
                    self.expr(parts[1], depth)?
                ))
            }
            "type_predicate" => {
                let name = field(node, "name").ok_or("a predicate without a name")?;
                let value = field(node, "type").ok_or("a predicate without a type")?;
                self.expect(name.end_byte(), value.start_byte(), " is ")?;
                Ok(format!(
                    "(is {} {})",
                    self.name(name)?,
                    self.expr(value, depth)?
                ))
            }
            "class" | "class_declaration" => self.class(node, depth),
            "interface_declaration" => self.interface(node, depth),
            "type_annotation" => self.annotation(node, depth),
            _ => self.refuse(node, &format!("{kind} is not supported")),
        }
    }

    fn element(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        match node.kind() {
            "assignment_pattern" => {
                let left = field(node, "left").ok_or("no left")?;
                let right = field(node, "right").ok_or("no right")?;
                self.expect(left.end_byte(), right.start_byte(), " = ")?;
                Ok(format!(
                    "(infix [{} \"=\" {}])",
                    self.pattern(left, depth)?,
                    self.child(Slot::Binary("=", Side::Right), right, depth)?
                ))
            }
            "rest_pattern" => self.bare(node, depth),
            _ => self.pattern(node, depth),
        }
    }

    fn hangs(&self, node: Node<'_>) -> bool {
        node.kind() == "arrow_function"
            && field(node, "body").is_some_and(|body| {
                body.kind() != "statement_block"
                    && token(node, "=>").is_some_and(|arrow| {
                        self.gap(arrow.end_byte(), body.start_byte()).contains('\n')
                    })
            })
    }

    fn multiline(&self, node: Node<'_>) -> bool {
        let items = named(node);
        items.first().is_some_and(|first| {
            self.gap(node.start_byte() + 1, first.start_byte())
                .contains('\n')
        })
    }

    fn type_arguments(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.expr(item, depth)
        })
    }

    fn arguments(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        let inner = depth + usize::from(self.multiline(node));
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.expr(item, inner)
        })
    }

    fn parameters(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let items = named(node);
        let inner = depth + usize::from(self.multiline(node));
        self.delimited(node, &items, depth, Separator::Comma, false, |item| {
            self.parameter(item, inner)
        })
    }

    fn parameter(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        if !matches!(node.kind(), "required_parameter" | "optional_parameter") {
            return self.refuse(node, &format!("{} is not a parameter", node.kind()));
        }
        let pattern = field(node, "pattern").ok_or("a parameter without a pattern")?;
        let modifiers = self.modifiers(node, pattern)?;
        let mut value = if pattern.kind() == "rest_pattern" {
            self.bare(pattern, depth)?
        } else {
            self.pattern(pattern, depth)?
        };
        let mut end = pattern.end_byte();
        if node.kind() == "optional_parameter" {
            self.expect(end, end + 1, "?")?;
            value = format!("(opt {value})");
            end += 1;
        }
        if let Some(annotation) = field(node, "type") {
            self.expect(end, annotation.start_byte(), "")?;
            value = format!("(typed {value} {})", self.annotation(annotation, depth)?);
            end = annotation.end_byte();
        }
        if let Some(default) = field(node, "value") {
            self.expect(end, default.start_byte(), " = ")?;
            value = format!(
                "(infix [{value} \"=\" {}])",
                self.child(Slot::Binary("=", Side::Right), default, depth)?
            );
        }
        Ok(Self::modified(&modifiers, value))
    }

    fn signature(
        &self,
        parameters: Node<'_>,
        result: Option<Node<'_>>,
        depth: usize,
    ) -> Result<(String, usize), String> {
        let mut value = self.parameters(parameters, depth)?;
        let mut end = parameters.end_byte();
        if let Some(result) = result {
            self.expect(end, result.start_byte(), "")?;
            value = format!("(typed {value} {})", self.annotation(result, depth)?);
            end = result.end_byte();
        }
        Ok((value, end))
    }

    fn function(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let parts = children(node);
        let asynchronous = parts[0].kind() == "async";
        let keyword = parts[usize::from(asynchronous)];
        if keyword.kind() != "function" {
            return self.refuse(
                node,
                &format!("{} functions are not supported", keyword.kind()),
            );
        }
        if asynchronous {
            self.expect(parts[0].end_byte(), keyword.start_byte(), " ")?;
        }
        if field(node, "type_parameters").is_some() || token(node, "*").is_some() {
            return self.refuse(node, "generic and generator functions are not supported");
        }
        let parameters = field(node, "parameters").ok_or("a function without parameters")?;
        let declared = node.kind() == "function_declaration";
        let name = match field(node, "name") {
            Some(_) if !declared => {
                return self.refuse(node, "a named function expression");
            }
            Some(name) => {
                self.expect(keyword.end_byte(), name.start_byte(), " ")?;
                self.expect(name.end_byte(), parameters.start_byte(), "")?;
                self.name(name)?
            }
            None => {
                self.expect(keyword.end_byte(), parameters.start_byte(), " ")?;
                "null".to_owned()
            }
        };
        let names = self.function_names(node)?;
        let value = self.scope(&names, || {
            let (signature, end) = self.signature(parameters, field(node, "return_type"), depth)?;
            let body = field(node, "body").ok_or("a function without a body")?;
            self.expect(end, body.start_byte(), " ")?;
            Ok(format!(
                "(fn {name} {signature} {})",
                self.block_list(body, depth)?
            ))
        })?;
        Ok(if asynchronous {
            format!("(async {value})")
        } else {
            value
        })
    }

    fn arrow(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let parts = children(node);
        let asynchronous = parts[0].kind() == "async";
        if asynchronous {
            self.expect(parts[0].end_byte(), parts[1].start_byte(), " ")?;
        }
        if field(node, "type_parameters").is_some() {
            return self.refuse(node, "generic arrows are not supported");
        }
        let names = self.function_names(node)?;
        let value = self.scope(&names, || {
            let (parameters, mut end) = match field(node, "parameter") {
                Some(parameter) => (self.name(parameter)?, parameter.end_byte()),
                None => {
                    let parameters =
                        field(node, "parameters").ok_or("an arrow without parameters")?;
                    self.signature(parameters, field(node, "return_type"), depth)?
                }
            };
            let arrow = token(node, "=>").ok_or("no =>")?;
            self.expect(end, arrow.start_byte(), " ")?;
            end = arrow.end_byte();
            let body = field(node, "body").ok_or("an arrow without a body")?;
            let value = if body.kind() == "statement_block" {
                self.expect(end, body.start_byte(), " ")?;
                self.block_list(body, depth)?
            } else if self.broken(end, body.start_byte(), depth + 1)? {
                format!("(below {})", self.expr(body, depth + 1)?)
            } else {
                self.expect(end, body.start_byte(), " ")?;
                self.child(Slot::ArrowBody, body, depth)?
            };
            Ok(format!("(arrow {parameters} {value})"))
        })?;
        Ok(if asynchronous {
            format!("(async {value})")
        } else {
            value
        })
    }

    fn template(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut parts = Vec::new();
        let mut at = node.start_byte() + 1;
        for substitution in named(node)
            .into_iter()
            .filter(|child| child.kind() == "template_substitution")
        {
            if substitution.start_byte() > at {
                parts.push(self.string(self.gap(at, substitution.start_byte())));
            }
            let inner = named(substitution);
            let [value] = inner.as_slice() else {
                return self.refuse(
                    substitution,
                    &format!("a substitution of {} parts", inner.len()),
                );
            };
            self.expect(substitution.start_byte() + 2, value.start_byte(), "")?;
            self.expect(value.end_byte(), substitution.end_byte() - 1, "")?;
            parts.push(format!("(sub {})", self.expr(*value, depth)?));
            at = substitution.end_byte();
        }
        let end = node.end_byte() - 1;
        if end > at {
            parts.push(self.string(self.gap(at, end)));
        }
        Ok(format!("(tpl {})", list(&parts)))
    }

    fn binary_parts<'t>(&self, node: Node<'t>) -> Result<(Node<'t>, Node<'t>, Node<'t>), String> {
        let parts = children(node);
        Ok(match node.kind() {
            "as_expression" | "satisfies_expression" => (
                named(node)[0],
                parts[1],
                *parts.last().ok_or("no right operand")?,
            ),
            _ => (
                field(node, "left").ok_or("no left operand")?,
                field(node, "operator")
                    .or_else(|| {
                        parts
                            .iter()
                            .copied()
                            .find(|part| !part.is_named() && part.kind() == "=")
                    })
                    .ok_or("no operator")?,
                field(node, "right").ok_or("no right operand")?,
            ),
        })
    }

    fn infix(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut broken = None;
        let items = self.flat(node, depth, Some(&mut broken))?;
        Ok(if broken == Some(true) {
            format!("(infix {})", rows(&items))
        } else {
            format!("(infix {})", list(&items))
        })
    }

    fn unparen<'t>(&self, node: Node<'t>) -> (Node<'t>, bool) {
        if node.kind() == "parenthesized_expression" {
            if let [inner] = named(node).as_slice() {
                return (*inner, true);
            }
        }
        (node, false)
    }

    fn operand(
        &self,
        slot: (&str, Side),
        node: Node<'_>,
        depth: usize,
        chain: Option<&mut Option<bool>>,
        out: &mut Vec<String>,
    ) -> Result<(), String> {
        let (operator, side) = slot;
        let (inner, parenthesized) = self.unparen(node);
        let class = self.class_of(inner);
        let Class::Binary(child) = class else {
            out.push(self.child(Slot::Binary(operator, side), node, depth)?);
            return Ok(());
        };
        if wants(Slot::Binary(operator, side), class) != parenthesized {
            return self.refuse(
                node,
                "parentheses around an operand that differ from those the printer writes",
            );
        }
        if parenthesized {
            self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
            self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
        }
        let (outer, level_inner) = (level(operator), level(child));
        let flat = level_inner > outer
            || level_inner == outer && (right_associative(operator) == (side == Side::Right));
        if !flat {
            out.push(list(&self.flat(inner, depth, None)?));
            return Ok(());
        }
        let continues = side == Side::Left && !parenthesized && level_inner == outer;
        match chain {
            Some(broken) if continues => out.extend(self.flat(inner, depth, Some(broken))?),
            _ => out.extend(self.flat(inner, depth, None)?),
        }
        Ok(())
    }

    fn flat(
        &self,
        node: Node<'_>,
        depth: usize,
        mut chain: Option<&mut Option<bool>>,
    ) -> Result<Vec<String>, String> {
        let (left, operator, right) = self.binary_parts(node)?;
        let op = self.text(operator);
        if precedence(op).is_none() {
            return self.refuse(
                operator,
                &format!("`{op}` is not a binary operator the printer knows"),
            );
        }
        let typed = matches!(node.kind(), "as_expression" | "satisfies_expression");
        self.expect(left.end_byte(), operator.start_byte(), " ")?;
        let mut out = Vec::new();
        if assignment(op) {
            out.push(self.child(Slot::Binary(op, Side::Left), left, depth)?);
        } else {
            self.operand(
                (op, Side::Left),
                left,
                depth,
                chain.as_deref_mut(),
                &mut out,
            )?;
        }
        out.push(self.string(op));
        let after = self.gap(operator.end_byte(), right.start_byte());
        if assignment(op) && after.contains('\n') {
            self.broken(operator.end_byte(), right.start_byte(), depth + 1)?;
            out.push(format!(
                "(below {})",
                self.child(Slot::Binary(op, Side::Right), right, depth + 1)?
            ));
            return Ok(out);
        }
        let split = after.contains('\n');
        match chain {
            Some(broken) => {
                if broken.is_some_and(|known| known != split) {
                    return self
                        .refuse(right, "an operator chain broken after some operators only");
                }
                *broken = Some(split);
                let gap = if split {
                    self.indent(depth)
                } else {
                    " ".to_owned()
                };
                self.expect(operator.end_byte(), right.start_byte(), &gap)?;
            }
            None => self.expect(operator.end_byte(), right.start_byte(), " ")?,
        }
        if typed {
            out.push(self.expr(right, depth)?);
        } else {
            self.operand((op, Side::Right), right, depth, None, &mut out)?;
        }
        Ok(out)
    }

    fn chain<'t>(&self, node: Node<'t>, links: &mut Vec<Link<'t>>) -> Result<Node<'t>, String> {
        let mut current = node;
        loop {
            match current.kind() {
                "member_expression" => {
                    let object = field(current, "object").ok_or("no object")?;
                    let property = field(current, "property").ok_or("no property")?;
                    let optional = optional(current);
                    let accessor = if optional { "?." } else { "." };
                    let gap = self.gap(object.end_byte(), property.start_byte());
                    let broken = gap.contains('\n');
                    if !broken && gap != accessor {
                        return self.refuse(property, &format!("member access spaced as {gap:?}"));
                    }
                    links.push(Link::Member(optional, property, broken));
                    current = object;
                }
                "call_expression" => {
                    let function = field(current, "function").ok_or("no callee")?;
                    let arguments = field(current, "arguments").ok_or("no arguments")?;
                    if arguments.kind() != "arguments" {
                        return self.refuse(arguments, "tagged templates are not supported");
                    }
                    let mut end = arguments.start_byte();
                    let types = field(current, "type_arguments");
                    if let Some(types) = types {
                        self.expect(types.end_byte(), end, "")?;
                        end = types.start_byte();
                    }
                    let optional = optional(current);
                    self.expect(function.end_byte(), end, if optional { "?." } else { "" })?;
                    links.push(Link::Call(arguments, optional, types));
                    current = function;
                }
                "subscript_expression" => {
                    let object = field(current, "object").ok_or("no object")?;
                    let index = field(current, "index").ok_or("no index")?;
                    let optional = optional(current);
                    self.expect(
                        object.end_byte(),
                        index.start_byte(),
                        if optional { "?.[" } else { "[" },
                    )?;
                    self.expect(index.end_byte(), current.end_byte(), "]")?;
                    links.push(Link::Index(index, optional));
                    current = object;
                }
                "non_null_expression" => {
                    let inner = named(current)[0];
                    self.expect(inner.end_byte(), current.end_byte(), "!")?;
                    links.push(Link::NonNull);
                    current = inner;
                }
                _ => return Ok(current),
            }
        }
    }

    fn postfix(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let mut links = Vec::new();
        let base = self.chain(node, &mut links)?;
        links.reverse();
        let starts = groups(&links);
        let chained = links
            .iter()
            .any(|link| matches!(link, Link::Member(_, _, true)));
        if chained {
            for (index, link) in links.iter().enumerate() {
                let start = starts.contains(&index);
                match link {
                    Link::Member(optional, property, broken) => {
                        if *broken != start {
                            return self.refuse(
                                *property,
                                "a member chain broken other than before each call group",
                            );
                        }
                        if !broken {
                            continue;
                        }
                        if *optional {
                            return self.refuse(*property, "an optional link starting a chain row");
                        }
                        let dot = property.start_byte() - 1;
                        let before = self.source[..dot].trim_end_matches([' ', '\t']);
                        let pad = &self.source[before.len()..dot];
                        if self.gap(dot, property.start_byte()) != "."
                            || !before.ends_with('\n')
                            || pad != self.unit.repeat(depth + 1)
                            || before[..before.len() - 1].ends_with('\n')
                        {
                            return self.refuse(*property, "a chain row not one dot one level in");
                        }
                    }
                    _ if start => {
                        return self.refuse(
                            node,
                            "a member chain group that does not start with a member",
                        );
                    }
                    _ => {}
                }
            }
        }
        let mut value = self.child(Slot::Member, base, depth)?;
        let mut path = String::new();
        for (index, link) in links.iter().enumerate() {
            let depth = if chained && starts.first().is_some_and(|first| index >= *first) {
                depth + 1
            } else {
                depth
            };
            if let Link::Member(optional, property, _) = link {
                if *optional {
                    path.push_str("?.");
                } else if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(self.text(*property));
                continue;
            }
            if !path.is_empty() {
                value = format!("(dot {value} {})", self.string(&path));
                path.clear();
            }
            value = match link {
                Link::Call(arguments, optional, types) => {
                    if let Some(types) = types {
                        value =
                            format!("(generic {value} {})", self.type_arguments(*types, depth)?);
                    }
                    let constructor = if *optional { "optCall" } else { "call" };
                    format!(
                        "({constructor} {value} {})",
                        self.arguments(*arguments, depth)?
                    )
                }
                Link::Index(index, optional) => {
                    let constructor = if *optional { "optIdx" } else { "idx" };
                    format!("({constructor} {value} {})", self.expr(*index, depth)?)
                }
                Link::NonNull => format!("(nonNull {value})"),
                Link::Member(..) => value,
            };
        }
        if !path.is_empty() {
            value = format!("(dot {value} {})", self.string(&path));
        }
        Ok(if chained {
            format!("(chain {value})")
        } else {
            value
        })
    }

    fn key(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        match node.kind() {
            "computed_property_name" => {
                let inner = named(node)[0];
                self.expect(node.start_byte() + 1, inner.start_byte(), "")?;
                self.expect(inner.end_byte(), node.end_byte() - 1, "")?;
                Ok(format!("(computed {})", self.expr(inner, depth)?))
            }
            "string" | "number" => self.bare(node, depth),
            _ => self.property(node),
        }
    }

    fn modifiers(&self, node: Node<'_>, name: Node<'_>) -> Result<Vec<&'s str>, String> {
        let mut modifiers = Vec::new();
        let mut at = node.start_byte();
        for child in children(node) {
            if child.start_byte() >= name.start_byte() {
                break;
            }
            self.expect(at, child.start_byte(), "")?;
            let word = self.text(child);
            if !matches!(word, "async" | "readonly") {
                return self.refuse(child, &format!("the modifier {word} is not supported"));
            }
            modifiers.push(word);
            self.expect(child.end_byte(), child.end_byte() + 1, " ")?;
            at = child.end_byte() + 1;
        }
        self.expect(at, name.start_byte(), "")?;
        Ok(modifiers)
    }

    fn modified(modifiers: &[&str], mut value: String) -> String {
        for modifier in modifiers.iter().rev() {
            value = format!("({modifier} {value})");
        }
        value
    }

    fn member(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        match node.kind() {
            "comment" => Ok(self.comment(node)),
            "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => {
                self.name(node)
            }
            "spread_element" | "rest_pattern" => self.bare(node, depth),
            "pair" => {
                let key = field(node, "key").ok_or("a pair without a key")?;
                let value = field(node, "value").ok_or("a pair without a value")?;
                let colon = token(node, ":").ok_or("no :")?;
                self.expect(key.end_byte(), colon.start_byte(), "")?;
                Ok(format!(
                    "(prop {} {})",
                    self.key(key, depth)?,
                    self.hang(colon.end_byte(), value, depth)?
                ))
            }
            "pair_pattern" => {
                let key = field(node, "key").ok_or("a pair without a key")?;
                let value = field(node, "value").ok_or("a pair without a value")?;
                self.expect(key.end_byte(), value.start_byte(), ": ")?;
                Ok(format!(
                    "(prop {} {})",
                    self.key(key, depth)?,
                    self.element(value, depth)?
                ))
            }
            "object_assignment_pattern" => {
                let left = field(node, "left").ok_or("no left")?;
                let right = field(node, "right").ok_or("no right")?;
                self.expect(left.end_byte(), right.start_byte(), " = ")?;
                Ok(format!(
                    "(infix [{} \"=\" {}])",
                    self.name(left)?,
                    self.child(Slot::Binary("=", Side::Right), right, depth)?
                ))
            }
            "method_definition" | "method_signature" => {
                let name = field(node, "name").ok_or("a method without a name")?;
                let modifiers = self.modifiers(node, name)?;
                let parameters = field(node, "parameters").ok_or("a method without parameters")?;
                let mut name_end = name.end_byte();
                let mut key = self.key(name, depth)?;
                if token(node, "?").is_some() {
                    self.expect(name_end, name_end + 1, "?")?;
                    key = format!("(opt {key})");
                    name_end += 1;
                }
                self.expect(name_end, parameters.start_byte(), "")?;
                let names = self.function_names(node)?;
                let value = self.scope(&names, || {
                    let (signature, end) =
                        self.signature(parameters, field(node, "return_type"), depth)?;
                    Ok(match field(node, "body") {
                        Some(body) => {
                            self.expect(end, body.start_byte(), " ")?;
                            format!(
                                "(method {key} {signature} {})",
                                self.block_list(body, depth)?
                            )
                        }
                        None => format!("(method {key} {signature} null)"),
                    })
                })?;
                Ok(Self::modified(&modifiers, value))
            }
            "property_signature" | "public_field_definition" => {
                let name = field(node, "name").ok_or("a property without a name")?;
                let modifiers = self.modifiers(node, name)?;
                let mut value = self.key(name, depth)?;
                let mut end = name.end_byte();
                if token(node, "?").is_some() {
                    self.expect(end, end + 1, "?")?;
                    value = format!("(opt {value})");
                    end += 1;
                }
                if let Some(annotation) = field(node, "type") {
                    self.expect(end, annotation.start_byte(), "")?;
                    value = format!("(typed {value} {})", self.annotation(annotation, depth)?);
                    end = annotation.end_byte();
                }
                if let Some(initial) = field(node, "value") {
                    let equals = token(node, "=").ok_or("no =")?;
                    self.expect(end, equals.start_byte(), " ")?;
                    value = format!(
                        "(field {value} {})",
                        self.hang(equals.end_byte(), initial, depth)?
                    );
                    end = initial.end_byte();
                }
                if end != node.end_byte() {
                    return Err(format!("{}: text after a property", self.at(end)));
                }
                Ok(Self::modified(&modifiers, value))
            }
            _ => self.expr(node, depth),
        }
    }

    fn members(
        &self,
        body: Node<'_>,
        depth: usize,
        separator: Separator,
    ) -> Result<String, String> {
        let items = named(body);
        let close = body.end_byte() - 1;
        let Some(first) = items.first() else {
            self.expect(body.start_byte() + 1, close, "")?;
            return Ok("[]".into());
        };
        self.expect(
            body.start_byte() + 1,
            first.start_byte(),
            &self.indent(depth + 1),
        )?;
        let mark = separator.mark();
        let mut out = Vec::new();
        let mut end = body.start_byte() + 1;
        for (index, node) in items.iter().enumerate() {
            if index > 0 {
                let gap = self.gap(end, node.start_byte());
                let count = gap.bytes().take_while(|&b| b == b'\n').count();
                if count == 0 || gap[count..] != self.unit.repeat(depth + 1) {
                    return Err(format!(
                        "{}: members separated by {gap:?}",
                        self.at(node.start_byte())
                    ));
                }
                for _ in 1..count {
                    out.push("blank".to_owned());
                }
            }
            end = node.end_byte();
            let terminated = !matches!(node.kind(), "comment" | "method_definition");
            if terminated {
                let after = self.gap(end, end + mark.len());
                if after != mark {
                    return Err(format!("{}: a member without {mark:?}", self.at(end)));
                }
                end += mark.len();
            }
            out.push(self.member(*node, depth + 1)?);
        }
        self.expect(end, close, &self.indent(depth))?;
        Ok(lines(&out))
    }

    fn heritage(
        &self,
        name: Node<'_>,
        clause: Option<Node<'_>>,
        keyword: &str,
    ) -> Result<(String, usize), String> {
        let head = self.name(name)?;
        let Some(clause) = clause else {
            return Ok((head, name.end_byte()));
        };
        self.expect(name.end_byte(), clause.start_byte(), " ")?;
        let mut clauses = vec![clause];
        if clause.kind() == "class_heritage" {
            clauses = named(clause);
        }
        let [inner] = clauses.as_slice() else {
            return self.refuse(clause, "several heritage clauses");
        };
        let types = named(*inner);
        if !inner.kind().starts_with("extends") || types.len() != 1 {
            return self.refuse(clause, &format!("only a single {keyword} is supported"));
        }
        self.expect(inner.start_byte() + 7, types[0].start_byte(), " ")?;
        Ok((
            format!("(extends {head} {})", self.expr(types[0], 0)?),
            clause.end_byte(),
        ))
    }

    fn class(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let name = field(node, "name").ok_or("a class without a name")?;
        let body = field(node, "body").ok_or("a class without a body")?;
        if field(node, "type_parameters").is_some() {
            return self.refuse(node, "a generic class");
        }
        self.expect(start + 5, name.start_byte(), " ")?;
        let heritage = named(node)
            .into_iter()
            .find(|child| child.kind() == "class_heritage");
        let (head, end) = self.heritage(name, heritage, "extends")?;
        self.expect(end, body.start_byte(), " ")?;
        Ok(format!(
            "(class {head} {})",
            self.members(body, depth, Separator::Semicolon)?
        ))
    }

    fn interface(&self, node: Node<'_>, depth: usize) -> Result<String, String> {
        let start = node.start_byte();
        let name = field(node, "name").ok_or("an interface without a name")?;
        let body = field(node, "body").ok_or("an interface without a body")?;
        if field(node, "type_parameters").is_some() {
            return self.refuse(node, "a generic interface");
        }
        self.expect(start + 9, name.start_byte(), " ")?;
        let extends = named(node)
            .into_iter()
            .find(|child| child.kind() == "extends_type_clause");
        let (head, end) = self.heritage(name, extends, "extends")?;
        self.expect(end, body.start_byte(), " ")?;
        Ok(format!(
            "(interface {head} {})",
            self.members(body, depth, Separator::Semicolon)?
        ))
    }
}

fn member_link(link: &Link<'_>) -> bool {
    matches!(link, Link::Member(..) | Link::Index(..))
}

fn literal_index(link: &Link<'_>) -> bool {
    matches!(link, Link::Index(index, _) if matches!(index.kind(), "number" | "string"))
}

fn groups(links: &[Link<'_>]) -> Vec<usize> {
    let mut at = 0;
    while at < links.len()
        && (matches!(links[at], Link::Call(..) | Link::NonNull) || literal_index(&links[at]))
    {
        at += 1;
    }
    if !matches!(links.first(), Some(Link::Call(..))) {
        while at + 1 < links.len() && member_link(&links[at]) && member_link(&links[at + 1]) {
            at += 1;
        }
    }
    let mut starts = Vec::new();
    let mut called = false;
    for (index, link) in links.iter().enumerate().skip(at) {
        if index == at {
            starts.push(index);
        } else if called
            && member_link(link)
            && (matches!(link, Link::Member(..)) || literal_index(link))
        {
            starts.push(index);
            called = false;
        }
        if matches!(link, Link::Call(..)) {
            called = true;
        }
    }
    starts
}
