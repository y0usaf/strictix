//! Options-schema rules: unknown option paths and option/type
//! mismatches, backed by options.json (M8).
//!
//! NixOS modules interact with options in two directions. They READ
//! them through the `config` module argument
//! (`config.services.example.enable`), and they WRITE them by binding
//! paths in the module body (`services.example.enable = true;`).
//! AI-generated modules frequently hallucinate option names in both
//! positions, and set real options to values of the wrong type
//! (`enable = "true"`). These rules load the options schema (shape
//! `{"options": {"a.b.c": {"type": "boolean", ...}}}`) once per
//! process and check both directions against it.
//!
//! Schema paths may contain placeholder segments such as
//! `users.users.<name>.home`; a placeholder matches any single written
//! segment. A written path counts as DECLARED when it aligns with some
//! schema path over their common prefix: an exact match, a write below
//! a declared option (freeform/submodule interiors), or a write of an
//! intermediate attrset (`services.nginx = { ... }` is structural, not
//! a leaf option) are all legitimate.
//!
//! The parsed schema lives in a process-wide [OnceLock] rather than a
//! field: rules are constructed by the registry macro as unit structs
//! (`Type {}`), so the struct itself carries no state. [OnceLock] is
//! Send + Sync, keeping the rule shareable across worker threads. Both
//! schema rules share the one loaded schema.
//!
//! Across runs, the parsed path→type map is cached on disk next to a
//! content fingerprint of options.json (format v2: `{hash:x}:v2`
//! header, then one `path\ttype` line per option), so a second
//! invocation re-reads a few KB instead of re-parsing a multi-MB
//! schema. The cache lives in the OS temp dir and never touches the
//! schema file's directory, so linting a config repo leaves no stray
//! files in it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::json::JsonValue;
use strictix_core::rules::Rule;
use strictix_core::semantic::{BindingKind, Reference, SemanticModel};
use strictix_syntax::{
    AstNode, AttrItem, AttrName, Attrpath, AttrsetExpr, Binding, Expr, LambdaParam, Root,
    SelectExpr, StringPart, SyntaxKind, TextRange,
};

/// The parsed options schema: every declared option path mapped to its
/// declared type string (empty when options.json carries no `type`
/// field), plus a pre-split view of each path so wildcard matching
/// never re-splits the multi-thousand-entry schema per checked path.
pub(crate) struct OptionsSchema {
    /// path → declared type; the canonical store, used for exact
    /// lookups and for serializing the disk cache.
    types: HashMap<String, String>,
    /// Every path split on `.`, for placeholder-aware matching.
    segmented: Vec<Vec<String>>,
}

impl OptionsSchema {
    /// Build the matching view once, at load time, so per-path checks
    /// touch pre-split segments only.
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

/// Split an options.json key into segments. Nix's `showOption` quotes
/// segments that are not plain identifiers (`user.tools."7z".enable`),
/// so dots inside quotes do not split and the quotes are dropped to
/// match the literal content of a written segment.
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

/// The loaded schema, or the reason loading/parsing failed. Loaded once
/// per process and shared by [UnknownOption] and [OptionTypeMismatch].
type Schema = Result<OptionsSchema, String>;

static OPTIONS_SCHEMA: OnceLock<Schema> = OnceLock::new();

/// The process-wide schema for this run: `None` when no schema path is
/// configured (both rules off), otherwise the shared load result.
fn schema_for(config: &LintConfig) -> Option<&'static Schema> {
    let path = config.schema.as_ref()?;
    Some(OPTIONS_SCHEMA.get_or_init(|| load_schema(path)))
}

/// FNV-1a 64-bit hash of `bytes` — a dependency-free content fingerprint
/// so the disk cache can detect that options.json changed.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Cache file name for a schema: `{content-hash:x}.paths`. Two schema
/// files are distinguished by their content, not their path, so moving
/// a config repo never invalidates the cache.
fn cache_file_for(schema: &Path) -> PathBuf {
    let hash = std::fs::read(schema)
        .map(|bytes| fnv1a(&bytes))
        .unwrap_or(0); // unreadable => hash 0, which matches a miss
    std::env::temp_dir()
        .join("strictix-cache")
        .join(format!("{hash:016x}.paths"))
}

/// Serialize the path→type map: the fingerprint plus a `v2` marker on
/// line 1, then one `path\ttype` line per option. The marker makes
/// every pre-v2 cache (which stored bare paths under a `{hash}` header)
/// miss and reparse, instead of being misread as typeless entries.
///
/// Types are single-line human strings; any embedded newline, carriage
/// return, or tab would corrupt the line-per-entry format, so those are
/// replaced with spaces at write time. The coarse type matching below
/// only inspects word prefixes, so the substitution never changes a
/// verdict.
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

/// Read the on-disk cache, returning the path→type map if it is present,
/// v2-formatted, and its stored fingerprint matches `expected` (i.e.
/// options.json is unchanged since the cache was written). Any line
/// without a tab separator marks the cache corrupt: report a miss and
/// reparse rather than guess.
fn read_cache(cache: &Path, expected: u64) -> Option<HashMap<String, String>> {
    let text = std::fs::read_to_string(cache).ok()?;
    let mut lines = text.lines();
    let header = lines.next()?;
    if header != format!("{expected:016x}:v2") {
        return None; // stale or pre-v2: reparse options.json
    }
    lines
        .map(|line| {
            line.split_once('\t')
                .map(|(path, ty)| (path.to_owned(), ty.to_owned()))
        })
        .collect()
}

/// Read and parse options.json, extracting each option's path and its
/// declared type string (empty when the entry has no `type` field).
///
/// This is the authoritative path; callers use it only on a cache miss.
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

/// Load (and cache) options.json, consulting the disk cache first.
///
/// Best-effort: a cache that cannot be read or written falls back to
/// parsing the schema directly, so a lock-free unwritable temp dir never
/// degrades correctness — only speed.
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

// --- path matching ----------------------------------------------------

/// Whether one schema path segment accepts one written segment: a
/// placeholder like `<name>` matches any single segment, anything else
/// matches only itself.
fn seg_match(schema_seg: &str, written_seg: &str) -> bool {
    (schema_seg.starts_with('<') && schema_seg.ends_with('>')) || schema_seg == written_seg
}

/// Whether two paths agree pairwise over their common prefix. Because
/// `zip` stops at the shorter path, this one predicate covers all three
/// DECLARED shapes: exact match (equal length), a write below a
/// declared option (schema shorter: freeform/submodule interior), and a
/// structural intermediate write (written shorter: proper prefix of a
/// schema path).
fn prefix_match(schema_path: &[String], written: &[String]) -> bool {
    schema_path
        .iter()
        .zip(written)
        .all(|(s, w)| seg_match(s, w))
}

/// Whether a written dotted path is declared: by the schema (exact key
/// fast path, then placeholder-aware alignment) or by a local
/// `options.<path>` declaration in the same file. `joined` must be
/// `written` joined with dots — callers already have it for messages.
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

/// The declared type of the schema option that `written` matches at
/// FULL length (placeholders allowed), or `None`. Only exact-length
/// matches carry a judgeable type: a write below or above an option is
/// structural and never type-checked.
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

// --- read side (select chains off `config`) ---------------------------

/// Whether a reference is a use of the `config` module argument: a
/// LambdaParam binding named `config`.
fn is_config_reference(model: &SemanticModel, reference: &Reference) -> bool {
    reference.name.text(model.source()) == "config"
        && model
            .resolve(reference.name)
            .is_some_and(|b| b.kind == BindingKind::LambdaParam)
}

/// The select chain rooted at `ref_range`: the innermost SelectExpr
/// whose base is the referenced ident, then every SelectExpr whose base
/// is the previous one (nested chains such as `Select(Select(config,a),b)`).
/// The parser usually flattens a path into one SelectExpr with a
/// multi-element attrpath, so the chain is typically a single node.
fn select_chain<'a>(
    model: &SemanticModel<'a>,
    ref_range: strictix_syntax::TextRange,
) -> Vec<SelectExpr<'a>> {
    let root = model.root();
    let mut chain = Vec::new();
    let Some(innermost) = root
        .descendants()
        .filter_map(SelectExpr::cast)
        .find(|s| matches!(s.base(), Some(Expr::Ident(t)) if t.range() == ref_range))
    else {
        return chain;
    };
    chain.push(innermost);
    let mut current_range = innermost.range();
    loop {
        let next = root
            .descendants()
            .filter_map(SelectExpr::cast)
            .find(|s| s.base().is_some_and(|b| b.range() == current_range));
        match next {
            Some(select) => {
                current_range = select.range();
                chain.push(select);
            }
            None => break,
        }
    }
    chain
}

// --- write side (option definitions in a module body) -----------------

/// Formal names whose presence marks a NixOS module: these are the
/// arguments the module system passes to every module.
const MODULE_FORMALS: [&str; 4] = ["config", "lib", "pkgs", "options"];

/// Top-level module keys that are not option definitions: `imports` and
/// `options` have their own meaning, `_module`/`disabledModules`/`key`
/// are module-system plumbing. `config` is NOT here — a `config`
/// wrapper is descended into as the definition root instead. `meta` IS
/// a declared option subtree in the real schema, so it stays checkable.
const RESERVED_HEADS: [&str; 5] = ["imports", "options", "_module", "disabledModules", "key"];

/// Peel parentheses off an expression.
fn unwrap_parens(expr: Expr<'_>) -> Expr<'_> {
    let mut current = expr;
    while let Expr::Paren(paren) = current {
        match paren.expr() {
            Some(inner) => current = inner,
            None => break,
        }
    }
    current
}

/// The static dotted segments of an attrpath: idents verbatim, quoted
/// strings by their literal content. `None` when any segment is dynamic
/// (`${...}`) or carries escapes — the written path cannot be proven
/// then, so callers skip that subtree silently.
fn static_segments(path: Attrpath<'_>, source: &str) -> Option<Vec<String>> {
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
                    return None; // escapes: raw text is not the segment
                }
                segments.push(text);
            }
            AttrName::Interp(_) => return None,
        }
    }
    Some(segments)
}

/// The bare name a call goes through: `mkIf` for both `mkIf` and
/// `lib.mkIf`. The base of a select is deliberately ignored — the
/// `mk*` names are unambiguous in module context regardless of how
/// `lib` is reached.
fn callee_name<'s>(expr: Expr<'_>, source: &'s str) -> Option<&'s str> {
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

/// The module body attrset of this file, when the file provably has
/// module shape: the top-level expression (lambdas and parens unwrapped)
/// is an attrset that either carries a module key (`imports`, `options`,
/// `config`) or sits under a lambda whose formals name a module argument
/// (`config`, `lib`, `pkgs`, `options`) or use `...`. Anything else —
/// packages, overlays, plain data — returns `None` and the write side
/// never runs.
fn module_attrset<'a>(model: &SemanticModel<'a>) -> Option<AttrsetExpr<'a>> {
    let source = model.source();
    let mut expr = Root::cast(model.root())?.expr()?;
    let mut is_module = false;
    loop {
        match expr {
            Expr::Paren(paren) => expr = paren.expr()?,
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

/// One leaf option write found in a module body: the full dotted path
/// (any `config` wrapper stripped), the attrpath range to report on,
/// and the value with parens and `mk*` modifiers unwrapped.
struct OptionWrite<'a> {
    segments: Vec<String>,
    path: String,
    name_range: TextRange,
    value: Expr<'a>,
}

/// Collect every leaf option write in this file's module body; empty
/// for non-module files. Shared by [UnknownOption] (unknown paths) and
/// [OptionTypeMismatch] (literal/type contradictions).
fn collect_writes<'a>(model: &SemanticModel<'a>) -> Vec<OptionWrite<'a>> {
    let Some(set) = module_attrset(model) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_attrset(model.source(), set, &[], true, &mut out);
    out
}

/// Walk one binding block. At the module root (`at_root`), reserved
/// heads are skipped and a `config` head is stripped (a `config`
/// wrapper is the definition root, not an option). Below the root,
/// binding paths concatenate onto `prefix`.
fn collect_attrset<'a>(
    source: &str,
    set: AttrsetExpr<'a>,
    prefix: &[String],
    at_root: bool,
    out: &mut Vec<OptionWrite<'a>>,
) {
    for item in set.items() {
        let AttrItem::Binding(binding) = item else {
            continue; // inherits define names, not provable option paths
        };
        let Some(path) = binding.attrpath() else {
            continue;
        };
        let Some(segments) = static_segments(path, source) else {
            continue; // dynamic segment: skip this subtree silently
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

/// Classify one bound value: descend into plain attrset literals
/// (accumulating the path), unwrap the module-system value modifiers
/// (`mkIf c X` → X, `mkMerge [ ... ]` → each element,
/// `mkForce`/`mkDefault`/`mkOverride n` → their payload), and record
/// everything else as a leaf write at `path`.
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
                    return; // mkMerge of a non-list literal: nothing provable
                }
                _ => {
                    // Two-argument modifiers: `mkIf cond X`, `mkOverride n X`
                    // parse as Apply(Apply(mk*, first), X).
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
        // `config = { ... }`: descend as the definition root.
        Expr::Attrset(set) if at_root => collect_attrset(source, set, &[], true, out),
        // A plain attrset literal is structure, not a leaf: descend.
        // An EMPTY one has no leaves to judge, so the accumulated path
        // itself becomes the write — otherwise `services.typo.x = { };`
        // escapes unknown-option entirely.
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
        // A pathless non-attrset (`config = 5;`) proves nothing.
        _ => {}
    }
}

/// Option paths declared locally in this file: every `options.<path>`
/// binding (head stripped) plus the leaves of any plain attrset literal
/// nested below it (`options.foo = { bar = mkOption ...; }` declares
/// `foo.bar`). Locally declared options are legitimate to write even
/// though nixpkgs' options.json has never heard of them.
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

/// Accumulate the leaf paths of nested plain attrset literals under a
/// local `options` declaration.
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

// --- literal/type judgement -------------------------------------------

/// The literal kinds the type rule can prove from syntax alone.
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
    /// The human name used in the diagnostic message.
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

/// The trivia-trimmed range of an expression, for diagnostics: node
/// ranges flush leading trivia into the node, so pointing at a node's
/// raw range would include the whitespace before it. Token atoms carry
/// no trivia and are exact already.
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

/// Classify a (already unwrapped) value as a provable literal, or
/// `None` for anything a static linter cannot judge — variables, calls,
/// selects, arithmetic. `true`/`false`/`null` only count when nothing
/// in scope could shadow them.
fn literal_kind(model: &SemanticModel<'_>, value: Expr<'_>) -> Option<LitKind> {
    match value {
        Expr::Int(_) => Some(LitKind::Int),
        Expr::Float(_) => Some(LitKind::Float),
        Expr::String(_) | Expr::IndString(_) => Some(LitKind::Str),
        Expr::Path(_) => Some(LitKind::Path),
        Expr::List(_) => Some(LitKind::List),
        Expr::Attrset(_) | Expr::RecAttrset(_) => Some(LitKind::Attrs),
        Expr::Unary(unary) => {
            // Negative number literals parse as unary minus over a
            // number token; anything else under a unary op stays silent.
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
                return None; // shadowed: not provably the literal
            }
            Some(kind)
        }
        _ => None,
    }
}

/// Coarse compatibility between a declared type string and a literal
/// kind: `Some(true)` = fits, `Some(false)` = provable mismatch,
/// `None` = this type is never judged. The type strings are nixpkgs'
/// human descriptions, so matching is by leading words, most specific
/// first; anything unrecognized (packages, functions, submodules,
/// enums, floats, ...) stays silent.
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
        // covers both "string" and "strings concatenated with ..."
        &[LitKind::Str]
    } else if ty.starts_with("path") {
        // NixOS path types accept string values
        &[LitKind::Path, LitKind::Str]
    } else if ["package", "lambda", "function", "submodule", "float"]
        .iter()
        .any(|word| ty.contains(word))
    {
        return None;
    } else if ty.contains("boolean") {
        if ty.contains("integer") {
            return None; // mixed description: not judgeable
        }
        &[LitKind::Bool]
    } else if ty.contains("integer") {
        // covers "signed integer", "unsigned integer", and port
        // phrasings like "16 bit unsigned integer; between ..."
        &[LitKind::Int]
    } else {
        return None;
    };
    Some(expected.contains(&kind))
}

/// Parse the exact inclusive range suffix used by integer option types.
/// Returning `None` keeps unfamiliar schema prose and overflowing bounds
/// out of range checking.
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

/// Parse an integer literal, allowing only unary minus and parentheses.
/// This deliberately does not evaluate arithmetic or identifiers.
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

// --- rules --------------------------------------------------------------

/// Flags option paths that options.json does not declare, on both
/// sides: reads off the `config` module argument and writes in a
/// module body.
///
/// Only fires when [LintConfig::schema] is set; the schema is loaded
/// lazily once. A load/parse failure is reported as a single diagnostic
/// on the first `config` reference, so a broken schema cannot crash
/// the run.
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
        let Some(schema) = schema_for(config) else {
            return; // schema rule off for this run
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
        // Read side: select chains rooted at the `config` argument.
        for reference in model.references() {
            if !is_config_reference(model, reference) {
                continue;
            }
            let chain = select_chain(model, reference.name.range());
            // `config.a.b or fallback` tolerates an undeclared path on purpose.
            if chain.is_empty() || chain.iter().any(|s| s.default().is_some()) {
                continue;
            }
            // Collect the path segments innermost-out, skipping chains
            // with any non-ident segment (strings, interpolations).
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
        // Write side: leaf option definitions in the module body.
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

/// Flags option definitions whose literal value contradicts the
/// declared option type: `enable = "true"` for a boolean,
/// `port = "8080"` for an integer.
///
/// Deliberately fixless: rewriting `"true"` to `true` is tempting but
/// changing quoted strings blindly is not provably intended — the
/// string may be exactly what the author meant to fix the other way.
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
        let Some(schema) = schema_for(config) else {
            return; // schema rules off for this run
        };
        let Ok(options) = schema else {
            return; // load failure already reported by unknown-option
        };
        for write in collect_writes(model) {
            let Some(ty) = declared_type(options, &write.segments, &write.path) else {
                continue; // no exact-length declared match: structural
            };
            let Some(kind) = literal_kind(model, write.value) else {
                continue; // not a provable literal: never judged
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A throwaway options.json in a unique temp file (deleted after).
    fn temp_schema(text: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!("strictix-test-schema-{}-{n}", std::process::id()));
        let mut f = std::fs::File::create(&path).expect("create temp schema");
        f.write_all(text.as_bytes()).expect("write temp schema");
        path
    }

    fn segs(path: &str) -> Vec<String> {
        path.split('.').map(str::to_owned).collect()
    }

    #[test]
    fn load_schema_parses_types_and_caches() {
        let path = temp_schema(
            r#"{"options": {"a.b": {"type": "boolean"}, "c.d": {}, "environment.systemPackages": {"type": "list of package"}}}"#,
        );
        let schema = load_schema(&path).expect("loads and parses");
        assert_eq!(schema.types.get("a.b").map(String::as_str), Some("boolean"));
        assert_eq!(schema.types.get("c.d").map(String::as_str), Some(""));
        assert_eq!(
            schema
                .types
                .get("environment.systemPackages")
                .map(String::as_str),
            Some("list of package")
        );
        assert!(!schema.types.contains_key("a.c"));
        // A cache file now exists and read_cache returns the same map.
        let cache = cache_file_for(&path);
        assert!(cache.exists(), "cache file written on successful load");
        let hash = fnv1a(std::fs::read(&path).unwrap().as_slice());
        assert_eq!(
            read_cache(&cache, hash).expect("cache readable"),
            schema.types
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn cache_is_stale_after_schema_change() {
        let path = temp_schema(r#"{"options": {"a.b": {}}}"#);
        let schema = load_schema(&path).expect("first load");
        assert!(schema.types.contains_key("a.b"));
        let cache = cache_file_for(&path);
        let hash = fnv1a(std::fs::read(&path).unwrap().as_slice());
        // Now write a different schema to the same path and bump its
        // content: the stored fingerprint no longer matches, so
        // read_cache must report a miss.
        std::fs::write(&path, r#"{"options": {"new.opt": {}}}"#).expect("rewrite schema");
        let reloaded = load_schema(&path).expect("reloads despite stale cache");
        assert!(reloaded.types.contains_key("new.opt"));
        assert!(!reloaded.types.contains_key("a.b"));
        let hash_after = fnv1a(std::fs::read(&path).unwrap().as_slice());
        assert_ne!(hash, hash_after, "content hash changed");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn missing_schema_is_error_not_cache_hit() {
        let missing = std::env::temp_dir().join("strictix-no-such-schema-anywhere");
        // The cache fingerprint for an unreadable path is 0; this must not
        // accidentally return an empty cache as a hit.
        let result = load_schema(&missing);
        assert!(result.is_err(), "missing schema is an error");
    }

    #[test]
    fn v2_cache_round_trips_and_sanitizes_type_text() {
        let dir = std::env::temp_dir().join("strictix-cache");
        let cache = dir.join(format!("test-roundtrip-{}.paths", std::process::id()));
        let mut map = HashMap::new();
        map.insert("a.b".to_owned(), "boolean".to_owned());
        // Arbitrary type text: embedded newlines/tabs become spaces at
        // write time so the line-per-entry format survives.
        map.insert("weird.opt".to_owned(), "one\nof\t\"x\"\r or y".to_owned());
        map.insert("untyped.opt".to_owned(), String::new());
        write_cache(&cache, 42, &map);
        let back = read_cache(&cache, 42).expect("v2 cache readable");
        assert_eq!(back.get("a.b").map(String::as_str), Some("boolean"));
        assert_eq!(
            back.get("weird.opt").map(String::as_str),
            Some("one of \"x\"  or y")
        );
        assert_eq!(back.get("untyped.opt").map(String::as_str), Some(""));
        // Wrong fingerprint misses.
        assert!(read_cache(&cache, 43).is_none(), "stale fingerprint misses");
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn v1_cache_format_is_rejected() {
        let dir = std::env::temp_dir().join("strictix-cache");
        let _ = std::fs::create_dir_all(&dir);
        let cache = dir.join(format!("test-v1-{}.paths", std::process::id()));
        // The v1 format: `{hash}` header, then bare paths. The v2 reader
        // must miss so the schema is reparsed with types.
        std::fs::write(&cache, format!("{{{:016x}}}\na.b\nc.d\n", 42u64)).unwrap();
        assert!(read_cache(&cache, 42).is_none(), "v1 cache is a miss");
        let _ = std::fs::remove_file(cache);
    }

    #[test]
    fn wildcard_matching_covers_all_declared_shapes() {
        let mut types = HashMap::new();
        types.insert("users.users.<name>.home".to_owned(), "path".to_owned());
        types.insert("services.example.enable".to_owned(), "boolean".to_owned());
        let schema = OptionsSchema::new(types);
        let declared = |path: &str| path_declared(&schema, &[], &segs(path), path);
        // (a) exact, wildcard segment matching any written segment
        assert!(declared("users.users.alice.home"));
        assert!(declared("services.example.enable"));
        // (b) schema path is a prefix: writing below a declared option
        assert!(declared("users.users.alice.home.anything"));
        // (c) written path is a proper prefix: structural intermediate
        assert!(declared("users.users"));
        assert!(declared("users.users.alice"));
        assert!(declared("services"));
        // mismatched segments are not declared
        assert!(!declared("users.uzers.alice.home"));
        assert!(!declared("services.example.enabled"));
        assert!(!declared("services.exmaple.enable"));
    }

    #[test]
    fn local_declarations_extend_the_declared_set() {
        let types = HashMap::new();
        let schema = OptionsSchema::new(types);
        let locals = vec![segs("mine.stuff.enable")];
        assert!(path_declared(
            &schema,
            &locals,
            &segs("mine.stuff.enable"),
            "mine.stuff.enable"
        ));
        // prefix and interior writes flow through the same predicate
        assert!(path_declared(
            &schema,
            &locals,
            &segs("mine.stuff"),
            "mine.stuff"
        ));
        assert!(path_declared(
            &schema,
            &locals,
            &segs("mine.stuff.enable.deep"),
            "mine.stuff.enable.deep"
        ));
        assert!(!path_declared(
            &schema,
            &locals,
            &segs("mine.other"),
            "mine.other"
        ));
    }

    #[test]
    fn declared_type_requires_full_length_match() {
        let mut types = HashMap::new();
        types.insert("users.users.<name>.home".to_owned(), "path".to_owned());
        types.insert("services.example.enable".to_owned(), "boolean".to_owned());
        let schema = OptionsSchema::new(types);
        let ty = |path: &str| declared_type(&schema, &segs(path), path).map(str::to_owned);
        assert_eq!(ty("services.example.enable"), Some("boolean".to_owned()));
        assert_eq!(ty("users.users.alice.home"), Some("path".to_owned()));
        // above/below an option is structural: never type-judged
        assert_eq!(ty("users.users.alice"), None);
        assert_eq!(ty("users.users.alice.home.deep"), None);
        assert_eq!(ty("services.nope"), None);
    }

    #[test]
    fn coarse_type_matching_follows_the_contract() {
        use LitKind as L;
        let m = literal_matches_type;
        // boolean
        assert_eq!(m("boolean", L::Bool), Some(true));
        assert_eq!(m("boolean", L::Str), Some(false));
        // integers, including port phrasing
        assert_eq!(m("signed integer", L::Int), Some(true));
        assert_eq!(m("signed integer", L::Str), Some(false));
        assert_eq!(
            m(
                "16 bit unsigned integer; between 0 and 65535 (both inclusive)",
                L::Int
            ),
            Some(true)
        );
        assert_eq!(
            m(
                "16 bit unsigned integer; between 0 and 65535 (both inclusive)",
                L::Str
            ),
            Some(false)
        );
        // strings, including concatenated phrasing
        assert_eq!(m("string", L::Str), Some(true));
        assert_eq!(m("string", L::Int), Some(false));
        assert_eq!(m("strings concatenated with \"\\n\"", L::Str), Some(true));
        // null or X accepts null and recurses
        assert_eq!(m("null or string", L::Null), Some(true));
        assert_eq!(m("null or string", L::Str), Some(true));
        assert_eq!(m("null or string", L::Int), Some(false));
        assert_eq!(m("string", L::Null), Some(false));
        // lists and attribute sets by prefix, beating word containment
        assert_eq!(m("list of string", L::List), Some(true));
        assert_eq!(m("list of string", L::Str), Some(false));
        assert_eq!(m("list of package", L::List), Some(true));
        assert_eq!(m("attribute set of string", L::Attrs), Some(true));
        assert_eq!(m("attribute set of string", L::List), Some(false));
        // paths accept strings
        assert_eq!(m("path", L::Path), Some(true));
        assert_eq!(m("path", L::Str), Some(true));
        assert_eq!(m("path", L::Int), Some(false));
        // never judged: packages, functions, submodules, floats, enums, empty
        assert_eq!(m("package", L::Str), None);
        assert_eq!(m("lambda", L::Int), None);
        assert_eq!(m("function that evaluates to a(n) string", L::Str), None);
        assert_eq!(m("submodule", L::Attrs), None);
        assert_eq!(m("signed integer or floating point number", L::Str), None);
        assert_eq!(m("one of \"a\", \"b\"", L::Str), None);
        assert_eq!(m("", L::Str), None);
    }
}
