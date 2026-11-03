use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Codec, CodecKind, CodecLoader, Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::json::JsonValue;
use strictix_core::project::ProjectContext;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    parse, ApplyExpr, AstNode, AttrItem, AttrName, AttrsetExpr, Binding, Expr, Root, StringExpr,
    StringPart, SyntaxKind, SyntaxToken, TextRange,
};

use crate::config_text_generator::{self, Interp, SENTINEL};
use crate::schema::{callee_name, unwrap_parens};

const RESERVED_HEADS: [&str; 5] = ["imports", "options", "_module", "disabledModules", "key"];

#[derive(Clone)]
struct Segment {
    text: String,
    origin: TextRange,
    file: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAttr {
    Text,
    Source,
    Generator,
    Value,
}

impl FileAttr {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "source" => Some(Self::Source),
            "generator" => Some(Self::Generator),
            "value" => Some(Self::Value),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Source => "source",
            Self::Generator => "generator",
            Self::Value => "value",
        }
    }

    fn explicit(self) -> bool {
        matches!(self, Self::Text | Self::Source)
    }
}

struct Write {
    attr: FileAttr,
    file: usize,
    anchor: u32,
    range: TextRange,
}

struct FileWrites(BTreeMap<Vec<String>, Vec<Write>>);

fn is_file_name(name: &str) -> bool {
    name.contains('/')
        || name
            .char_indices()
            .any(|(i, c)| c == '.' && i > 0 && i + 1 < name.len())
}

pub(crate) fn string_text(string: StringExpr<'_>, source: &str) -> String {
    let mut text = String::new();
    for part in string.parts() {
        match part {
            StringPart::Content(token) => text.push_str(token.text(source)),
            StringPart::Interp(interp) => text.push_str(interp.syntax().text(source)),
        }
    }
    text
}

fn segment(element: AttrName<'_>, source: &str, origin: TextRange) -> Segment {
    match element {
        AttrName::Ident(token) => Segment {
            text: token.text(source).to_owned(),
            origin,
            file: false,
        },
        AttrName::Str(string) => {
            let text = string_text(string, source);
            Segment {
                file: is_file_name(&text),
                text,
                origin,
            }
        }
        AttrName::Interp(interp) => Segment {
            text: interp.syntax().text(source).to_owned(),
            origin,
            file: false,
        },
    }
}

pub(crate) fn wrapped<'a>(expr: Expr<'a>, source: &str) -> Option<Vec<Expr<'a>>> {
    let Expr::Apply(apply) = unwrap_parens(expr) else {
        return None;
    };
    let (func, arg) = (apply.func()?, apply.arg()?);
    match callee_name(func, source) {
        Some("mkDefault" | "mkForce" | "mkBefore" | "mkAfter" | "mkOptionDefault") => {
            Some(vec![arg])
        }
        Some("mkMerge") => match unwrap_parens(arg) {
            Expr::List(list) => Some(list.items().collect()),
            _ => None,
        },
        _ => match unwrap_parens(func) {
            Expr::Apply(inner)
                if inner
                    .func()
                    .and_then(|f| callee_name(f, source))
                    .is_some_and(|n| matches!(n, "mkIf" | "mkOverride" | "mkOrder")) =>
            {
                Some(vec![arg])
            }
            _ => None,
        },
    }
}

fn record(path: &[Segment], file: usize, out: &mut BTreeMap<Vec<String>, Vec<Write>>) {
    let [.., entry, last] = path else {
        return;
    };
    if !entry.file {
        return;
    }
    let Some(attr) = FileAttr::parse(&last.text) else {
        return;
    };
    let key = path[..path.len() - 1]
        .iter()
        .map(|s| s.text.clone())
        .collect();
    out.entry(key).or_default().push(Write {
        attr,
        file,
        anchor: entry.origin.start(),
        range: last.origin,
    });
}

struct Walker<'s, 'o> {
    source: &'s str,
    file: usize,
    out: &'o mut BTreeMap<Vec<String>, Vec<Write>>,
}

impl Walker<'_, '_> {
    fn walk(&mut self, value: Expr<'_>, path: &[Segment], root: bool) {
        if let [.., entry, last] = path {
            if entry.file && FileAttr::parse(&last.text).is_some() {
                return record(path, self.file, self.out);
            }
        }
        if let Some(items) = wrapped(value, self.source) {
            for item in items {
                self.walk(item, path, root);
            }
            return;
        }
        match unwrap_parens(value) {
            Expr::If(expr) => {
                for branch in [expr.then_branch(), expr.else_branch()]
                    .into_iter()
                    .flatten()
                {
                    self.walk(branch, path, root);
                }
            }
            Expr::Let(expr) => {
                if let Some(body) = expr.body() {
                    self.walk(body, path, root);
                }
            }
            Expr::With(expr) => {
                if let Some(body) = expr.body() {
                    self.walk(body, path, root);
                }
            }
            Expr::Assert(expr) => {
                if let Some(body) = expr.body() {
                    self.walk(body, path, root);
                }
            }
            Expr::Bin(expr) if expr.op() == Some(SyntaxKind::SlashSlash) => {
                for side in [expr.lhs(), expr.rhs()].into_iter().flatten() {
                    self.walk(side, path, root);
                }
            }
            Expr::Attrset(set) => self.attrset(set, path, root),
            Expr::RecAttrset(rec) => {
                if let Some(set) = rec.attrset() {
                    self.attrset(set, path, root);
                }
            }
            _ => record(path, self.file, self.out),
        }
    }

    fn attrset(&mut self, set: AttrsetExpr<'_>, path: &[Segment], root: bool) {
        for item in set.items() {
            match item {
                AttrItem::Inherit(inherit) => {
                    for name in inherit.names() {
                        let mut full = path.to_vec();
                        full.push(Segment {
                            text: name.text(self.source).to_owned(),
                            origin: name.range(),
                            file: false,
                        });
                        record(&full, self.file, self.out);
                    }
                }
                AttrItem::Binding(binding) => {
                    let (Some(attrpath), Some(value)) = (binding.attrpath(), binding.value())
                    else {
                        continue;
                    };
                    let origin = attrpath.syntax().content_range();
                    let mut written: Vec<Segment> = attrpath
                        .elements()
                        .map(|e| segment(e, self.source, origin))
                        .collect();
                    if root {
                        if let Some(head) = written.first() {
                            if RESERVED_HEADS.contains(&head.text.as_str()) {
                                continue;
                            }
                            if head.text == "config" {
                                written.remove(0);
                            }
                        }
                    }
                    let mut full = path.to_vec();
                    full.extend(written);
                    self.walk(value, &full, root && full.is_empty());
                }
            }
        }
    }

    fn module_root(&mut self, expr: Expr<'_>) {
        let mut expr = expr;
        loop {
            expr = match unwrap_parens(expr) {
                Expr::Lambda(lambda) => match lambda.body() {
                    Some(body) => body,
                    None => return,
                },
                Expr::Let(let_expr) => match let_expr.body() {
                    Some(body) => body,
                    None => return,
                },
                Expr::With(with) => match with.body() {
                    Some(body) => body,
                    None => return,
                },
                Expr::Attrset(_) | Expr::RecAttrset(_) => break,
                _ => return,
            };
        }
        self.walk(expr, &[], true);
    }
}

fn project_writes(project: &ProjectContext) -> FileWrites {
    let mut out = BTreeMap::new();
    for (index, file) in project.sources().iter().enumerate() {
        let source = file.source.as_str();
        let root = parse(source);
        let root = &root;
        let mut walker = Walker {
            source,
            file: index,
            out: &mut out,
        };
        if let Some(expr) = Root::cast(root).and_then(|r| r.expr()) {
            walker.module_root(expr);
        }
        for node in root.descendants() {
            let Some(binding) = Binding::cast(node) else {
                continue;
            };
            let named = binding
                .attrpath()
                .and_then(|p| p.elements().last())
                .is_some_and(|last| {
                    matches!(last, AttrName::Ident(t) if matches!(t.text(source), "imports" | "modules"))
                });
            if !named {
                continue;
            }
            if let Some(Expr::List(list)) = binding.value().map(unwrap_parens) {
                for item in list.items() {
                    walker.module_root(item);
                }
            }
        }
    }
    out.retain(|_, writes| {
        let mut seen = HashSet::new();
        writes.retain(|w| seen.insert((w.file, w.anchor, w.range.start())));
        writes.iter().any(|w| w.attr.explicit()) && writes.iter().any(|w| !w.attr.explicit())
    });
    FileWrites(out)
}

fn line_of(source: &str, offset: u32) -> usize {
    source.as_bytes()[..offset as usize]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn relative(from: &Path, to: &Path) -> PathBuf {
    let base: Vec<Component<'_>> = from
        .parent()
        .map_or_else(Vec::new, |p| p.components().collect());
    let target: Vec<Component<'_>> = to.components().collect();
    let shared = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    if shared == 0 && !base.is_empty() {
        return to.to_path_buf();
    }
    let mut out = PathBuf::new();
    for _ in shared..base.len() {
        out.push("..");
    }
    for component in &target[shared..] {
        out.push(component.as_os_str());
    }
    out
}

pub struct MaskedGenerator;

impl Rule for MaskedGenerator {
    fn code(&self) -> &'static str {
        "masked-generator"
    }

    fn name(&self) -> &'static str {
        "Masked generator"
    }

    fn description(&self) -> &'static str {
        "Flags a file entry (an attribute named like a path, e.g. `files.\".config/app/settings.ini\"` or `environment.etc.\"foo.conf\"`) whose `text` or `source` is set in one definition while another definition anywhere in the project sets its `generator` or `value`. Module systems such as manzil and home-manager derive `text` from `generator`/`value` with `mkDefault`, so the explicit text wins and the generated file is silently replaced. Paths are compared project-wide with `config.` stripped, through `mkIf`/`mkMerge`/`mkDefault`/`mkAfter`/`mkBefore`/`mkForce` and nested attrsets; dynamic segments match by source text."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file_project(
        &self,
        model: &SemanticModel,
        _config: &LintConfig,
        project: Option<&ProjectContext>,
        diags: &mut Vec<Diagnostic>,
    ) {
        let Some(project) = project else {
            return;
        };
        let files = project.sources();
        let Some(current) = files
            .iter()
            .position(|f| model.path() == Some(f.path.as_path()))
            .or_else(|| files.iter().position(|f| f.source == model.source()))
        else {
            return;
        };
        let writes = project.memo(project_writes);
        for entries in writes.0.values() {
            for write in entries
                .iter()
                .filter(|w| w.file == current && w.attr.explicit())
            {
                let others = || {
                    entries.iter().filter(|o| {
                        !o.attr.explicit() && (o.file, o.anchor) != (write.file, write.anchor)
                    })
                };
                let Some(other) = others()
                    .find(|o| o.attr == FileAttr::Value)
                    .or_else(|| others().next())
                else {
                    continue;
                };
                let origin = &files[other.file];
                let shown = if other.file == current {
                    PathBuf::from(origin.path.file_name().unwrap_or_default())
                } else {
                    relative(&files[current].path, &origin.path)
                };
                let how = match other.attr {
                    FileAttr::Value => "from `value`",
                    _ => "with `generator`",
                };
                diags.push(Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "`{}` here replaces the file that {}:{} generates {how}; set `value` instead",
                        write.attr.name(),
                        shown.display(),
                        line_of(&origin.source, other.range.start()),
                    ),
                    write.range,
                ));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Format {
    Ini,
    Json,
}

impl Format {
    fn of(name: &str) -> Option<Self> {
        let lower = name.to_ascii_lowercase();
        if [".ini", ".list", ".conf", ".desktop"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        {
            Some(Self::Ini)
        } else if lower.ends_with(".json") {
            Some(Self::Json)
        } else {
            None
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Ini => "INI",
            Self::Json => "JSON",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Ini => "\u{1}",
            Self::Json => "0",
        }
    }

    fn accepts(self, body: &str) -> bool {
        match self {
            Self::Ini => is_ini(body),
            Self::Json => is_json_object(body),
        }
    }
}

fn is_ini(body: &str) -> bool {
    let mut sections = HashSet::new();
    let mut keys: Option<HashSet<&str>> = None;
    for line in body.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let Some(name) = header.strip_suffix(']') else {
                return false;
            };
            if name.is_empty() || name.contains(['[', ']']) || !sections.insert(name) {
                return false;
            }
            keys = Some(HashSet::new());
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            return false;
        };
        let comment = key.starts_with(['#', ';']);
        if key.is_empty() || comment || key.contains(char::is_whitespace) {
            return false;
        }
        if !keys.as_mut().is_some_and(|keys| keys.insert(key)) {
            return false;
        }
    }
    !sections.is_empty()
}

fn is_json_object(body: &str) -> bool {
    matches!(JsonValue::parse(body), Ok(JsonValue::Object(_)))
}

struct TextBody<'a> {
    detect: String,
    body: String,
    interps: Vec<Interp<'a>>,
    range: TextRange,
    parens: bool,
    escaped: bool,
}

fn text_body<'a>(value: Expr<'a>, source: &'a str, format: Format) -> Option<TextBody<'a>> {
    let mut value = value;
    let mut nested = false;
    while let Some(items) = wrapped(value, source) {
        let [item] = items.as_slice() else {
            return None;
        };
        value = *item;
        nested = true;
    }
    let parens = nested && !matches!(value, Expr::Paren(_));
    let Expr::IndString(string) = unwrap_parens(value) else {
        return None;
    };
    let mut text = TextBody {
        detect: String::new(),
        body: String::new(),
        interps: Vec::new(),
        range: unwrap_parens(value).content_range(),
        parens,
        escaped: false,
    };
    let mut dollar = false;
    for part in string.parts() {
        match part {
            StringPart::Content(token) => {
                let content = token.text(source);
                dollar = content.ends_with('$');
                text.escaped |= content.contains("''") || content.contains(SENTINEL);
                text.detect.push_str(content);
                text.body.push_str(content);
            }
            StringPart::Interp(interp) => {
                text.escaped |= std::mem::take(&mut dollar);
                text.detect.push_str(format.placeholder());
                text.body.push(SENTINEL);
                text.interps.push(Interp {
                    full: interp.syntax().text(source),
                    json: interp
                        .expr()
                        .and_then(|expr| to_json_argument(expr, source)),
                });
            }
        }
    }
    Some(text)
}

fn to_json_argument<'a>(expr: Expr<'a>, source: &'a str) -> Option<&'a str> {
    let Expr::Apply(apply) = unwrap_parens(expr) else {
        return None;
    };
    let text = |expr: Expr<'_>| {
        let range = expr.content_range();
        &source[range.start() as usize..range.end() as usize]
    };
    (text(apply.func()?) == "builtins.toJSON").then_some(text(apply.arg()?))
}

fn text_bindings<'a>(value: Expr<'a>, source: &str, out: &mut Vec<(Expr<'a>, TextRange)>) {
    if let Some(items) = wrapped(value, source) {
        for item in items {
            text_bindings(item, source, out);
        }
        return;
    }
    let set = match unwrap_parens(value) {
        Expr::Attrset(set) => set,
        Expr::RecAttrset(rec) => match rec.attrset() {
            Some(set) => set,
            None => return,
        },
        _ => return,
    };
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue;
        };
        let (Some(path), Some(value)) = (binding.attrpath(), binding.value()) else {
            continue;
        };
        let mut elements = path.elements();
        let single = matches!(
            (elements.next(), elements.next()),
            (Some(AttrName::Ident(t)), None) if t.text(source) == "text"
        );
        if single {
            out.push((value, path.syntax().content_range()));
        }
    }
}

pub(crate) fn file_texts<'a>(
    binding: Binding<'a>,
    source: &str,
) -> Option<(String, Vec<(Expr<'a>, TextRange)>)> {
    let (path, value) = (binding.attrpath()?, binding.value()?);
    let elements: Vec<AttrName<'_>> = path.elements().collect();
    let (name, texts) = match elements.as_slice() {
        [.., AttrName::Str(entry), AttrName::Ident(last)] if last.text(source) == "text" => (
            string_text(*entry, source),
            vec![(value, path.syntax().content_range())],
        ),
        [.., AttrName::Str(entry)] => {
            let mut texts = Vec::new();
            text_bindings(value, source, &mut texts);
            (string_text(*entry, source), texts)
        }
        _ => return None,
    };
    is_file_name(&name).then_some((name, texts))
}

pub struct HandWrittenConfig;

impl Rule for HandWrittenConfig {
    fn code(&self) -> &'static str {
        "hand-written-config"
    }

    fn name(&self) -> &'static str {
        "Hand-written config"
    }

    fn description(&self) -> &'static str {
        "Flags a file entry's `text` written as an indented string that parses entirely as the format its name declares: `.ini` (and `.list`/`.conf`/`.desktop` bodies) made only of `[section]` headers and `key=value` lines with no comments or duplicate keys, or `.json` bodies that parse as a JSON object with each interpolation treated as an opaque scalar. Such text is a Nix value spelled by hand; generate it with `lib.generators.toINI {}`, `builtins.toJSON` or `pkgs.formats.*` instead. The fix replaces the string with the equivalent generator call, withheld when it could change the text's meaning (INI without `lib` in scope, `''` escapes, interpolated section or key names; JSON floats, exponents, duplicate keys, inexpressible control characters, or whole-value interpolations other than `${builtins.toJSON e}`)."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for node in model.root().descendants() {
            let Some(binding) = Binding::cast(node) else {
                continue;
            };
            let Some((name, texts)) = file_texts(binding, source) else {
                continue;
            };
            let Some(format) = Format::of(&name) else {
                continue;
            };
            for (text, range) in texts {
                let Some(body) = text_body(text, source, format) else {
                    continue;
                };
                if !format.accepts(&body.detect) {
                    continue;
                }
                let diagnostic = Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!(
                        "{name} is hand-written {}; generate it from a Nix value (e.g. lib.generators.toINI {{}}, builtins.toJSON, pkgs.formats.*)",
                        format.name()
                    ),
                    range,
                );
                diags.push(match generated(model, &body, format) {
                    Some(replacement) => diagnostic.with_fix(
                        Fix::new(format!("generate the {} text", format.name()))
                            .edit(body.range, replacement),
                    ),
                    None => diagnostic,
                });
            }
        }
    }
}

fn generated(model: &SemanticModel, body: &TextBody<'_>, format: Format) -> Option<String> {
    if body.escaped {
        return None;
    }
    let source = model.source();
    let start = body.range.start() as usize;
    let line = &source[source[..start].rfind('\n').map_or(0, |i| i + 1)..start];
    let indent = &line[..line.len() - line.trim_start().len()];
    let generated = match format {
        Format::Ini if model.is_bound("lib", body.range.start()) => {
            config_text_generator::ini(&body.body, &body.interps, indent)?
        }
        Format::Ini => return None,
        Format::Json => config_text_generator::json(&body.body, &body.interps, indent)?,
    };
    Some(if body.parens {
        format!("({generated})")
    } else {
        generated
    })
}

const DATA_FORMATS: [(&str, &str); 4] = [
    (".json", "JSON"),
    (".toml", "TOML"),
    (".yaml", "YAML"),
    (".yml", "YAML"),
];

fn data_path<'a>(expr: Expr<'a>, source: &'a str) -> Option<(&'a SyntaxToken, &'static str)> {
    let Expr::Path(token) = unwrap_parens(expr) else {
        return None;
    };
    let text = token.text(source);
    if !(text.starts_with("./") || text.starts_with("../")) || text.contains("${") {
        return None;
    }
    DATA_FORMATS
        .iter()
        .find(|(ext, _)| text.ends_with(ext))
        .map(|&(_, format)| (token, format))
}

fn data_argument<'a>(apply: ApplyExpr<'a>, source: &'a str) -> Option<Expr<'a>> {
    let (func, arg) = (apply.func()?, apply.arg()?);
    match callee_name(func, source)? {
        "importJSON" | "importTOML" => Some(arg),
        "fromJSON" | "fromTOML" => match unwrap_parens(arg) {
            Expr::Apply(read)
                if read
                    .func()
                    .and_then(|f| callee_name(f, source))
                    .is_some_and(|n| n == "readFile") =>
            {
                read.arg()
            }
            _ => None,
        },
        _ => None,
    }
}

fn source_value<'a>(binding: Binding<'a>, source: &str) -> Option<Expr<'a>> {
    let path = binding.attrpath()?;
    let last = path.elements().last()?;
    matches!(last, AttrName::Ident(t) if t.text(source) == "source").then_some(())?;
    binding.value()
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

pub struct HandMaintainedDataFile;

impl Rule for HandMaintainedDataFile {
    fn code(&self) -> &'static str {
        "hand-maintained-data-file"
    }

    fn name(&self) -> &'static str {
        "Hand-maintained data file"
    }

    fn description(&self) -> &'static str {
        "Flags a literal relative path to a `.json`, `.toml`, `.yaml` or `.yml` file that is read as data (`lib.importJSON`, `lib.importTOML`, `builtins.fromJSON (builtins.readFile ./x.json)`, `builtins.fromTOML (builtins.readFile ./x.toml)`) or installed as a file entry's `source` (e.g. `environment.etc.\"x.json\".source`). Data kept by hand in a serialisation format is better kept as a `.nix` value and serialised by a generator where a file is needed. Interpolated paths, lock files (names containing `lock`) and paths under the config's `generated` prefixes are skipped. Report only."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        let dir = model.path().and_then(Path::parent).unwrap_or(Path::new(""));
        for node in model.root().descendants() {
            let candidate = match Expr::cast(node) {
                Some(Expr::Apply(apply)) => data_argument(apply, source),
                _ => Binding::cast(node).and_then(|b| source_value(b, source)),
            };
            let Some((token, format)) = candidate.and_then(|e| data_path(e, source)) else {
                continue;
            };
            let text = token.text(source);
            let file = normalize(&dir.join(text));
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or(text);
            if name.contains("lock") || config.is_generated(&file) {
                continue;
            }
            diags.push(Diagnostic::new(
                self.code(),
                self.severity(),
                format!(
                    "{text} is data maintained by hand in {format}; keep it as a .nix value (import ./x.nix) and let a generator write {format} where a file is needed"
                ),
                token.range(),
            ));
        }
    }
}

fn codec_of<'c>(name: &str, config: &'c LintConfig) -> Option<&'c str> {
    let lower = name.to_ascii_lowercase();
    config
        .codecs
        .iter()
        .find(|(_, extensions)| {
            extensions.iter().any(|ext| {
                lower
                    .strip_suffix(&ext.to_ascii_lowercase())
                    .and_then(|stem| stem.strip_suffix('.'))
                    .is_some_and(|stem| !stem.is_empty())
            })
        })
        .map(|(id, _)| id.as_str())
}

fn plain_literal(value: Expr<'_>, source: &str) -> Option<TextRange> {
    let mut value = value;
    while let Some(items) = wrapped(value, source) {
        let [item] = items.as_slice() else {
            return None;
        };
        value = *item;
    }
    let literal = unwrap_parens(value);
    let parts: Vec<StringPart<'_>> = match literal {
        Expr::String(string) => string.parts().collect(),
        Expr::IndString(string) => string.parts().collect(),
        _ => return None,
    };
    (!parts
        .iter()
        .any(|part| matches!(part, StringPart::Interp(_))))
    .then(|| literal.content_range())
}

fn import_call<'a>(apply: ApplyExpr<'a>, source: &'a str) -> Option<Expr<'a>> {
    let func = callee_name(apply.func()?, source)?;
    matches!(func, "importJSON" | "importTOML" | "fromJSON" | "fromTOML")
        .then(|| data_argument(apply, source))?
}

pub struct CodecCandidate;

impl Rule for CodecCandidate {
    fn code(&self) -> &'static str {
        "codec-candidate"
    }

    fn name(&self) -> &'static str {
        "Codec candidate"
    }

    fn description(&self) -> &'static str {
        "Runs only when the config declares `[codecs.<id>]` sections with `extensions`. Flags data a codec can convert into a Nix value: a file entry whose name ends in a codec extension and whose `text` is a string literal without interpolations (through `mkForce`/`mkAfter`/... wrappers), and a literal relative path to a file with a codec extension read by `lib.importJSON`, `lib.importTOML`, `builtins.fromJSON (builtins.readFile ./x)`, `builtins.fromTOML (builtins.readFile ./x)` or installed as a `source`. Lock files (names containing `lock`) and paths under the config's `generated` prefixes are skipped. The JSON output carries a `codec` object (id, kind, target, loader, replace range, whether `lib`/`pkgs` are in scope, entry) for `nix run .#lint -- fix` to perform the conversion."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        if config.codecs.is_empty() {
            return;
        }
        let source = model.source();
        let dir = model.path().and_then(Path::parent).unwrap_or(Path::new(""));
        let scope = |range: TextRange| {
            (
                model.is_bound("lib", range.start()),
                model.is_bound("pkgs", range.start()),
            )
        };
        for node in model.root().descendants() {
            if let Some(binding) = Binding::cast(node) {
                if let Some((name, texts)) = file_texts(binding, source) {
                    if let Some(id) = codec_of(&name, config) {
                        for (text, range) in texts {
                            let Some(replace) = plain_literal(text, source) else {
                                continue;
                            };
                            let (lib, pkgs) = scope(replace);
                            diags.push(
                                Diagnostic::new(
                                    self.code(),
                                    self.severity(),
                                    format!("{name} is {id}; nix run .#lint -- fix converts it"),
                                    range,
                                )
                                .with_codec(Codec {
                                    id: id.to_owned(),
                                    kind: CodecKind::Text,
                                    target: None,
                                    loader: None,
                                    replace,
                                    lib,
                                    pkgs,
                                    entry: Some(name.clone()),
                                }),
                            );
                        }
                    }
                }
            }
            let (candidate, loader, call) = match Expr::cast(node) {
                Some(Expr::Apply(apply)) => (
                    import_call(apply, source),
                    CodecLoader::Import,
                    Some(apply.syntax().content_range()),
                ),
                _ => (
                    Binding::cast(node).and_then(|b| source_value(b, source)),
                    CodecLoader::Source,
                    None,
                ),
            };
            let Some(Expr::Path(token)) = candidate.map(unwrap_parens) else {
                continue;
            };
            let text = token.text(source);
            if !(text.starts_with("./") || text.starts_with("../")) || text.contains("${") {
                continue;
            }
            let file = normalize(&dir.join(text));
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or(text);
            let Some(id) = codec_of(name, config) else {
                continue;
            };
            if name.contains("lock") || config.is_generated(&file) {
                continue;
            }
            let replace = call.unwrap_or_else(|| token.range());
            let (lib, pkgs) = scope(replace);
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("{text} is {id}; nix run .#lint -- fix converts it"),
                    token.range(),
                )
                .with_codec(Codec {
                    id: id.to_owned(),
                    kind: CodecKind::File,
                    target: Some(file.display().to_string()),
                    loader: Some(loader),
                    replace,
                    lib,
                    pkgs,
                    entry: None,
                }),
            );
        }
    }
}
