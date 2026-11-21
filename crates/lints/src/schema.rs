use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::json::JsonValue;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, Reference, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Attrpath, AttrsetExpr, Binding, Expr, LambdaParam, Root,
    SelectExpr, StringPart, SyntaxKind, TextRange,
};

pub(crate) struct OptionsSchema {
        types: HashMap<String, String>,
        segmented: Vec<Vec<String>>,
}

impl OptionsSchema {
        fn new(raw: HashMap<String, String>) -> Self {
        let mut types = HashMap::with_capacity(raw.len());
        let mut segmented = Vec::with_capacity(raw.len());
        for (key, ty) in raw {
            let segments = split_option_path(&key);
            types.insert(segments.join("."), ty);
            segmented.push(segments);
        }
        Self { types, segmented }
    }
}

fn split_option_path(key: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in key.chars() {
        match c {
            '"' => quoted = !quoted,
            '.' if !quoted => segments.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    segments.push(current);
    segments
}

type Schema = Result<OptionsSchema, String>;

static SCHEMAS: OnceLock<Mutex<HashMap<PathBuf, &'static Schema>>> = OnceLock::new();

fn schema_for(config: &LintConfig, file: Option<&Path>) -> Option<&'static Schema> {
    let path = config.schema_for(file)?;
    let mut schemas = SCHEMAS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Some(
        schemas
            .entry(path.to_path_buf())
            .or_insert_with(|| Box::leak(Box::new(load_schema(path)))),
    )
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn cache_file_for(schema: &Path) -> PathBuf {
    let hash = std::fs::read(schema)
        .map(|bytes| fnv1a(&bytes))
        .unwrap_or(0);
    std::env::temp_dir()
        .join("strictix-cache")
        .join(format!("{hash:016x}.paths"))
}

fn write_cache(cache: &Path, hash: u64, options: &HashMap<String, String>) {
    let Some(parent) = cache.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let mut out = String::with_capacity(options.len() * 32);
    out.push_str(&format!("{hash:016x}:v2\n"));
    for (key, ty) in options {
        out.push_str(key);
        out.push('\t');
        for ch in ty.chars() {
            out.push(if matches!(ch, '\n' | '\r' | '\t') {
                ' '
            } else {
                ch
            });
        }
        out.push('\n');
    }
    let _ = std::fs::write(cache, out);
}

fn read_cache(cache: &Path, expected: u64) -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(cache).ok()?;
    let mut lines = text.lines();
    let header = lines.next()?;
    if header != format!("{expected:016x}:v2") {
        return None;
    }
    lines
        .map(|line| {
            line.split_once('\t')
                .map(|(path, ty)| (path.to_owned(), ty.to_owned()))
        })
        .collect()
}

fn parse_schema(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let parsed = JsonValue::parse(&text).map_err(|e| e.message)?;
    let options = parsed
        .get("options")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "expected a top-level \"options\" object".to_string())?;
    Ok(options
        .iter()
        .map(|(key, value)| {
            let ty = value
                .get("type")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .to_owned();
            (key.clone(), ty)
        })
        .collect())
}

fn load_schema(path: &Path) -> Schema {
    let cache = cache_file_for(path);
    let hash = std::fs::read(path).map(|b| fnv1a(&b)).unwrap_or(0);
    if let Some(options) = read_cache(&cache, hash) {
        return Ok(OptionsSchema::new(options));
    }
    let options = parse_schema(path)?;
    write_cache(&cache, hash, &options);
    Ok(OptionsSchema::new(options))
}

fn seg_match(schema_seg: &str, written_seg: &str) -> bool {
    (schema_seg.starts_with('<') && schema_seg.ends_with('>')) || schema_seg == written_seg
}

fn prefix_match(schema_path: &[String], written: &[String]) -> bool {
    schema_path
        .iter()
        .zip(written)
        .all(|(s, w)| seg_match(s, w))
}

fn path_declared(
    schema: &OptionsSchema,
    locals: &[Vec<String>],
    written: &[String],
    joined: &str,
) -> bool {
    schema.types.contains_key(joined)
        || schema.segmented.iter().any(|s| prefix_match(s, written))
        || locals.iter().any(|l| prefix_match(l, written))
}

fn declared_type<'s>(
    schema: &'s OptionsSchema,
    written: &[String],
    joined: &str,
) -> Option<&'s str> {
    if let Some(ty) = schema.types.get(joined) {
        return Some(ty);
    }
    let hit = schema
        .segmented
        .iter()
        .find(|s| s.len() == written.len() && prefix_match(s, written))?;
    schema.types.get(&hit.join(".")).map(String::as_str)
}

fn is_config_reference(model: &SemanticModel, reference: &Reference) -> bool {
    reference.name.text(model.source()) == "config"
        && model
            .resolve(reference.name)
            .is_some_and(|b| b.kind == BindingKind::LambdaParam)
}

fn select_chain<'a>(
    model: &SemanticModel<'a>,
    ref_range: strictix_syntax::TextRange,
) -> Vec<SelectExpr<'a>> {
    let root = model.root();
    let mut chain = Vec::new();
    let mut path_selects = Vec::new();
    root.walk_containing(ref_range.start(), |node| {
        if let Some(select) = SelectExpr::cast(node) {
            path_selects.push(select);
        }
        false
    });
    let Some(start_pos) = path_selects
        .iter()
        .rposition(|s| matches!(s.base(), Some(Expr::Ident(t)) if t.range() == ref_range))
    else {
        return chain;
    };
    chain.push(path_selects[start_pos]);
    let mut current_range = path_selects[start_pos].range();
    let mut pos = start_pos;
    while pos > 0 {
        pos -= 1;
        let select = path_selects[pos];
        if select.base().is_some_and(|b| b.range() == current_range) {
            current_range = select.range();
            chain.push(select);
        }
    }
    chain
}

const MODULE_FORMALS: [&str; 4] = ["config", "lib", "pkgs", "options"];

const RESERVED_HEADS: [&str; 5] = ["imports", "options", "_module", "disabledModules", "key"];

pub(crate) fn unwrap_parens(expr: Expr<'_>) -> Expr<'_> {
    let mut current = expr;
    while let Expr::Paren(paren) = current {
        match paren.expr() {
            Some(inner) => current = inner,
            None => break,
        }
    }
    current
}

pub(crate) fn static_segments(path: Attrpath<'_>, source: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    for element in path.elements() {
        match element {
            AttrName::Ident(token) => segments.push(token.text(source).to_owned()),
            AttrName::Str(string) => {
                let mut text = String::new();
                for part in string.parts() {
                    match part {
                        StringPart::Content(token) => text.push_str(token.text(source)),
                        StringPart::Interp(_) => return None,
                    }
                }
                if text.contains('\\') {
                    return None;
                }
                segments.push(text);
            }
            AttrName::Interp(_) => return None,
        }
    }
    Some(segments)
}

pub(crate) fn callee_name<'s>(expr: Expr<'_>, source: &'s str) -> Option<&'s str> {
    match expr {
        Expr::Ident(token) => Some(token.text(source)),
        Expr::Select(select) => {
            let mut last = None;
            for element in select.attrpath()?.elements() {
                match element {
                    AttrName::Ident(token) => last = Some(token.text(source)),
                    _ => return None,
                }
            }
            last
        }
        _ => None,
    }
}

pub(crate) fn module_attrset<'a>(model: &SemanticModel<'a>) -> Option<AttrsetExpr<'a>> {
    let source = model.source();
    let mut expr = Root::cast(model.root())?.expr()?;
    let mut is_module = false;
    loop {
        match expr {
            Expr::Paren(paren) => expr = paren.expr()?,
            Expr::Let(let_expr) => expr = let_expr.body()?,
            Expr::With(with) => expr = with.body()?,
            Expr::Lambda(lambda) => {
                if let LambdaParam::Formals(formals, _) = lambda.param() {
                    if formals.has_ellipsis()
                        || formals
                            .params()
                            .any(|p| MODULE_FORMALS.contains(&p.name.text(source)))
                    {
                        is_module = true;
                    }
                }
                expr = lambda.body()?;
            }
            _ => break,
        }
    }
    let Expr::Attrset(set) = expr else {
        return None;
    };
    if !is_module {
        is_module = set.items().any(|item| match item {
            AttrItem::Binding(binding) => binding
                .attrpath()
                .and_then(|p| p.elements().next())
                .is_some_and(|element| match element {
                    AttrName::Ident(token) => {
                        matches!(token.text(source), "imports" | "options" | "config")
                    }
                    _ => false,
                }),
            AttrItem::Inherit(_) => false,
        });
    }
    is_module.then_some(set)
}

struct OptionWrite<'a> {
    segments: Vec<String>,
    path: String,
    name_range: TextRange,
    value: Expr<'a>,
}

fn collect_writes<'a>(model: &SemanticModel<'a>) -> Vec<OptionWrite<'a>> {
    let Some(set) = module_attrset(model) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_attrset(model.source(), set, &[], true, &mut out);
    out
}

fn collect_attrset<'a>(
    source: &str,
    set: AttrsetExpr<'a>,
    prefix: &[String],
    at_root: bool,
    out: &mut Vec<OptionWrite<'a>>,
) {
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let Some(segments) = static_segments(path, source) else {
            continue;
        };
        let Some(value) = binding.value() else {
            continue;
        };
        let full;
        let mut root_ctx = false;
        if at_root {
            let Some(head) = segments.first() else {
                continue;
            };
            if RESERVED_HEADS.contains(&head.as_str()) {
                continue;
            }
            if head == "config" {
                full = segments[1..].to_vec();
                root_ctx = full.is_empty();
            } else {
                full = segments;
            }
        } else {
            let mut joined = prefix.to_vec();
            joined.extend(segments);
            full = joined;
        }
        process_value(
            source,
            value,
            &full,
            root_ctx,
            path.syntax().content_range(),
            out,
        );
    }
}

fn process_value<'a>(
    source: &str,
    value: Expr<'a>,
    path: &[String],
    at_root: bool,
    name_range: TextRange,
    out: &mut Vec<OptionWrite<'a>>,
) {
    let value = unwrap_parens(value);
    if let Expr::Apply(apply) = value {
        if let (Some(func), Some(arg)) = (apply.func(), apply.arg()) {
            match callee_name(func, source) {
                Some("mkForce" | "mkDefault") => {
                    return process_value(source, arg, path, at_root, name_range, out);
                }
                Some("mkMerge") => {
                    if let Expr::List(list) = unwrap_parens(arg) {
                        for item in list.items() {
                            process_value(source, item, path, at_root, name_range, out);
                        }
                    }
                    return;
                }
                _ => {
                    if let Expr::Apply(inner) = func {
                        if inner
                            .func()
                            .and_then(|f| callee_name(f, source))
                            .is_some_and(|n| matches!(n, "mkIf" | "mkOverride"))
                        {
                            return process_value(source, arg, path, at_root, name_range, out);
                        }
                    }
                }
            }
        }
    }
    match value {
        Expr::Attrset(set) if at_root => collect_attrset(source, set, &[], true, out),
        Expr::Attrset(set) if !path.is_empty() => {
            if set.items().next().is_none() {
                out.push(OptionWrite {
                    path: path.join("."),
                    segments: path.to_vec(),
                    name_range,
                    value,
                });
            } else {
                collect_attrset(source, set, path, false, out);
            }
        }
        Expr::Attrset(_) => {}
        _ if !path.is_empty() => out.push(OptionWrite {
            path: path.join("."),
            segments: path.to_vec(),
            name_range,
            value,
        }),
        _ => {}
    }
}

fn local_declarations(model: &SemanticModel<'_>) -> Vec<Vec<String>> {
    let source = model.source();
    let mut out = Vec::new();
    for node in model.root().descendants() {
        let Some(binding) = Binding::cast(node) else {
            continue;
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let Some(segments) = static_segments(path, source) else {
            continue;
        };
        if segments.first().map(String::as_str) != Some("options") {
            continue;
        }
        let rest = &segments[1..];
        if !rest.is_empty() {
            out.push(rest.to_vec());
        }
        if let Some(value) = binding.value() {
            declared_leaves(source, value, rest, &mut out);
        }
    }
    out
}

fn declared_leaves(source: &str, value: Expr<'_>, prefix: &[String], out: &mut Vec<Vec<String>>) {
    let Expr::Attrset(set) = unwrap_parens(value) else {
        return;
    };
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let Some(segments) = static_segments(path, source) else {
            continue;
        };
        let mut full = prefix.to_vec();
        full.extend(segments);
        match binding.value() {
            Some(value) if matches!(unwrap_parens(value), Expr::Attrset(_)) => {
                declared_leaves(source, value, &full, out);
            }
            _ => out.push(full),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LitKind {
    Bool,
    Int,
    Float,
    Str,
    List,
    Attrs,
    Path,
    Null,
}

impl LitKind {
        fn name(self) -> &'static str {
        match self {
            Self::Bool => "boolean",
            Self::Int => "integer",
            Self::Float => "float",
            Self::Str => "string",
            Self::List => "list",
            Self::Attrs => "attribute set",
            Self::Path => "path",
            Self::Null => "null",
        }
    }
}

fn trimmed_range(expr: Expr<'_>) -> TextRange {
    match expr {
        Expr::Ident(t)
        | Expr::Int(t)
        | Expr::Float(t)
        | Expr::Path(t)
        | Expr::SearchPath(t)
        | Expr::Uri(t) => t.range(),
        Expr::Let(e) => e.syntax().content_range(),
        Expr::With(e) => e.syntax().content_range(),
        Expr::Assert(e) => e.syntax().content_range(),
        Expr::If(e) => e.syntax().content_range(),
        Expr::Attrset(e) => e.syntax().content_range(),
        Expr::RecAttrset(e) => e.syntax().content_range(),
        Expr::List(e) => e.syntax().content_range(),
        Expr::Lambda(e) => e.syntax().content_range(),
        Expr::Apply(e) => e.syntax().content_range(),
        Expr::Unary(e) => e.syntax().content_range(),
        Expr::Bin(e) => e.syntax().content_range(),
        Expr::Select(e) => e.syntax().content_range(),
        Expr::HasAttr(e) => e.syntax().content_range(),
        Expr::String(e) => e.syntax().content_range(),
        Expr::IndString(e) => e.syntax().content_range(),
        Expr::Paren(e) => e.syntax().content_range(),
    }
}

fn literal_kind(model: &SemanticModel<'_>, value: Expr<'_>) -> Option<LitKind> {
    match value {
        Expr::Int(_) => Some(LitKind::Int),
        Expr::Float(_) => Some(LitKind::Float),
        Expr::String(_) | Expr::IndString(_) => Some(LitKind::Str),
        Expr::Path(_) => Some(LitKind::Path),
        Expr::List(_) => Some(LitKind::List),
        Expr::Attrset(_) | Expr::RecAttrset(_) => Some(LitKind::Attrs),
        Expr::Unary(unary) => {
            let minus = unary
                .syntax()
                .child_tokens()
                .any(|t| t.kind() == SyntaxKind::Minus);
            match (minus, unary.operand()) {
                (true, Some(Expr::Int(_))) => Some(LitKind::Int),
                (true, Some(Expr::Float(_))) => Some(LitKind::Float),
                _ => None,
            }
        }
        Expr::Ident(token) => {
            let name = token.text(model.source());
            let kind = match name {
                "true" | "false" => LitKind::Bool,
                "null" => LitKind::Null,
                _ => return None,
            };
            if model.is_bound(name, token.range().start()) {
                return None;
            }
            Some(kind)
        }
        _ => None,
    }
}

fn literal_matches_type(ty: &str, kind: LitKind) -> Option<bool> {
    let ty = ty.trim();
    if let Some(rest) = ty.strip_prefix("null or ") {
        if kind == LitKind::Null {
            return Some(true);
        }
        return literal_matches_type(rest, kind);
    }
    let expected: &[LitKind] = if ty.starts_with("list of") {
        &[LitKind::List]
    } else if ty.starts_with("attribute set") {
        &[LitKind::Attrs]
    } else if ty.starts_with("string") {
        &[LitKind::Str]
    } else if ty.starts_with("path") {
        &[LitKind::Path, LitKind::Str]
    } else if ["package", "lambda", "function", "submodule", "float"]
        .iter()
        .any(|word| ty.contains(word))
    {
        return None;
    } else if ty.contains("boolean") {
        if ty.contains("integer") {
            return None;
        }
        &[LitKind::Bool]
    } else if ty.contains("integer") {
        &[LitKind::Int]
    } else {
        return None;
    };
    Some(expected.contains(&kind))
}

fn integer_range(ty: &str) -> Option<(i128, i128)> {
    let phrase = ty.split_once("; ")?.1;
    let rest = phrase.strip_prefix("between ")?;
    let rest = rest.strip_suffix(" (both inclusive)")?;
    let (min, max) = rest.split_once(" and ")?;
    if min.is_empty()
        || max.is_empty()
        || min.chars().any(char::is_whitespace)
        || max.chars().any(char::is_whitespace)
    {
        return None;
    }
    Some((min.parse().ok()?, max.parse().ok()?))
}

fn integer_literal(model: &SemanticModel<'_>, value: Expr<'_>) -> Option<i128> {
    match value {
        Expr::Int(token) => token.text(model.source()).parse().ok(),
        Expr::Paren(paren) => integer_literal(model, paren.expr()?),
        Expr::Unary(unary) => {
            let minus = unary
                .syntax()
                .child_tokens()
                .any(|token| token.kind() == SyntaxKind::Minus);
            minus.then(|| integer_literal(model, unary.operand()?)?.checked_neg())?
        }
        _ => None,
    }
}

pub struct UnknownOption;

impl Rule for UnknownOption {
    fn code(&self) -> &'static str {
        "unknown-option"
    }

    fn name(&self) -> &'static str {
        "Unknown option"
    }

    fn description(&self) -> &'static str {
        "Flags option paths read off the config module argument or defined in a module body that are not declared in options.json. Hallucinated option names are a classic failure mode of AI-generated NixOS modules."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(schema) = schema_for(config, model.path()) else {
            return;
        };
        let options = match schema {
            Ok(options) => options,
            Err(err) => {
                if let Some(first) = model
                    .references()
                    .iter()
                    .find(|r| is_config_reference(model, r))
                {
                    diags.push(Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!("could not load options schema: {err}"),
                        first.name.range(),
                    ));
                }
                return;
            }
        };
        let locals = local_declarations(model);
        for reference in model.references() {
            if !is_config_reference(model, reference) {
                continue;
            }
            let chain = select_chain(model, reference.name.range());
            if chain.is_empty() || chain.iter().any(|s| s.default().is_some()) {
                continue;
            }
            let mut segments = Vec::new();
            let mut valid = true;
            for select in &chain {
                let Some(attrpath) = select.attrpath() else {
                    valid = false;
                    break;
                };
                for element in attrpath.elements() {
                    match element {
                        AttrName::Ident(token) => {
                            segments.push(token.text(model.source()).to_string())
                        }
                        AttrName::Str(_) | AttrName::Interp(_) => valid = false,
                    }
                }
            }
            if !valid {
                continue;
            }
            let full_path = segments.join(".");
            if path_declared(options, &locals, &segments, &full_path) {
                continue;
            }
            let range = chain
                .last()
                .and_then(|s| s.attrpath())
                .map(|a| a.syntax().content_range())
                .unwrap_or_else(|| chain.last().expect("chain is non-empty").range());
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("option '{full_path}' is not declared in options.json"),
                range,
            ));
        }
        for write in collect_writes(model) {
            if path_declared(options, &locals, &write.segments, &write.path) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!("option '{}' is not declared in options.json", write.path),
                write.name_range,
            ));
        }
    }
}

pub struct OptionTypeMismatch;

impl Rule for OptionTypeMismatch {
    fn code(&self) -> &'static str {
        "option-type-mismatch"
    }

    fn name(&self) -> &'static str {
        "Option type mismatch"
    }

    fn description(&self) -> &'static str {
        "Flags an option set to a literal that contradicts its declared type in options.json — `enable = \"true\"` for a boolean, `port = \"8080\"` for an integer. Non-literal values are never judged."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let Some(schema) = schema_for(config, model.path()) else {
            return;
        };
        let Ok(options) = schema else {
            return;
        };
        for write in collect_writes(model) {
            let Some(ty) = declared_type(options, &write.segments, &write.path) else {
                continue;
            };
            let Some(kind) = literal_kind(model, write.value) else {
                continue;
            };
            if literal_matches_type(ty, kind) == Some(false) {
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!(
                            "option '{}' expects {}, got {}",
                            write.path,
                            ty,
                            kind.name()
                        ),
                        trimmed_range(write.value),
                    )
                    .with_help(format!(
                        "options.json declares '{}' with type '{}'",
                        write.path, ty
                    )),
                );
                continue;
            }
            let Some((min, max)) = integer_range(ty) else {
                continue;
            };
            let Some(value) = integer_literal(model, write.value) else {
                continue;
            };
            if !(min..=max).contains(&value) {
                diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        format!(
                            "option '{}' literal {} is out of range; expected between {} and {} (both inclusive)",
                            write.path, value, min, max
                        ),
                        trimmed_range(write.value),
                    )
                    .with_help(format!(
                        "options.json declares '{}' with type '{}'",
                        write.path, ty
                    )),
                );
            }
        }
    }
}
