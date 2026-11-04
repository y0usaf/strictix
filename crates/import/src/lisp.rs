use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::{column_of, line_of, nix};

const PREFIXES: [(&str, &str); 5] = [
    ("quoting_lit", "quote"),
    ("syn_quoting_lit", "quasi"),
    ("unquoting_lit", "unquote"),
    ("unquote_splicing_lit", "splice"),
    ("var_quoting_lit", "function"),
];

const BODIES: [(&str, usize); 38] = [
    ("block", 1),
    ("case", 1),
    ("define-extension", 3),
    ("defconstant", 1),
    ("defgeneric", 2),
    ("defmacro", 2),
    ("defmethod", 2),
    ("defpackage", 1),
    ("defparameter", 1),
    ("defstruct", 1),
    ("defun", 2),
    ("defvar", 1),
    ("destructuring-bind", 2),
    ("dolist", 1),
    ("dotimes", 1),
    ("ecase", 1),
    ("etypecase", 1),
    ("flet", 1),
    ("handler-bind", 1),
    ("handler-case", 1),
    ("ignore-errors", 0),
    ("labels", 1),
    ("lambda", 1),
    ("let", 1),
    ("let*", 1),
    ("macrolet", 1),
    ("multiple-value-bind", 2),
    ("prog1", 1),
    ("progn", 0),
    ("typecase", 1),
    ("unless", 1),
    ("unwind-protect", 1),
    ("when", 1),
    ("with-input-from-string", 1),
    ("with-open-file", 1),
    ("with-output-to-string", 1),
    ("with-slots", 2),
    ("loop", usize::MAX),
];

const LOCALS: [&str; 3] = ["flet", "labels", "macrolet"];

enum Form {
    Atom(String),
    Str(String),
    Prefix(&'static str, Box<Item>),
    Comment(String),
    List(Vec<Item>),
}

struct Item {
    form: Form,
    start: usize,
    end: usize,
}

struct Row<'a> {
    items: &'a [Item],
    first: usize,
    at: Option<usize>,
    blank: bool,
}

const NIX_KEYWORDS: [&str; 13] = [
    "assert", "else", "false", "if", "in", "inherit", "let", "null", "or", "rec", "then", "true",
    "with",
];

const CONSTRUCTORS: [&str; 24] = [
    "and-key",
    "and-optional",
    "and-rest",
    "at",
    "blank",
    "char",
    "comment",
    "constant",
    "dot",
    "double",
    "function",
    "hex",
    "kw",
    "quasi",
    "quote",
    "ratio",
    "render",
    "rows",
    "special",
    "splice",
    "str",
    "uninterned",
    "unquote",
    "wrap",
];

const OPERATORS: [&str; 16] = [
    "*", "+", "-", "/", "/=", "1+", "1-", "<", "<=", "=", ">", ">=", "char=", "let*", "list*",
    "string=",
];

const MARKERS: [(&str, &str); 3] = [
    ("&key", "and-key"),
    ("&optional", "and-optional"),
    ("&rest", "and-rest"),
];

const DEFINERS: [&str; 5] = ["defconstant", "defmacro", "defparameter", "defun", "defvar"];

const LOOP_BINDERS: [&str; 4] = ["as", "for", "into", "with"];

const CL: [&str; 122] = [
    "abs",
    "acons",
    "and",
    "append",
    "apply",
    "assoc",
    "atom",
    "car",
    "case",
    "cddr",
    "cdr",
    "ceiling",
    "char",
    "char-code",
    "code-char",
    "concatenate",
    "cond",
    "cons",
    "copy-list",
    "count-if",
    "declare",
    "defpackage",
    "defparameter",
    "defun",
    "defvar",
    "destructuring-bind",
    "digit-char-p",
    "do",
    "dolist",
    "dotimes",
    "ecase",
    "eq",
    "eql",
    "equal",
    "evenp",
    "every",
    "exp",
    "expt",
    "fifth",
    "find",
    "find-if",
    "first",
    "flet",
    "float",
    "floor",
    "format",
    "fourth",
    "get-internal-real-time",
    "getf",
    "if",
    "ignore",
    "in-package",
    "incf",
    "integerp",
    "intern",
    "internal-time-units-per-second",
    "intersection",
    "labels",
    "lambda",
    "last",
    "ldiff",
    "length",
    "let",
    "list",
    "listp",
    "loop",
    "map",
    "mapcar",
    "max",
    "member",
    "min",
    "mod",
    "most-positive-fixnum",
    "multiple-value-bind",
    "nconc",
    "nil",
    "not",
    "nreverse",
    "nth",
    "nthcdr",
    "null",
    "oddp",
    "or",
    "parse-integer",
    "plusp",
    "position",
    "position-if",
    "progn",
    "push",
    "read-line",
    "realp",
    "reduce",
    "remove",
    "remove-duplicates",
    "remove-if",
    "remove-if-not",
    "rest",
    "return",
    "reverse",
    "rotatef",
    "round",
    "second",
    "setf",
    "sin",
    "sort",
    "sqrt",
    "string",
    "string-downcase",
    "string-right-trim",
    "string-upcase",
    "stringp",
    "subseq",
    "subsetp",
    "substitute",
    "t",
    "third",
    "truncate",
    "unless",
    "values",
    "when",
    "with-input-from-string",
    "zerop",
];

const LOOP: [&str; 15] = [
    "across", "below", "by", "collect", "downto", "for", "from", "in", "on", "repeat", "sum",
    "thereis", "to", "while", "with",
];

const TOMOE: [&str; 88] = [
    "+bar+",
    "+cassette+",
    "+gap+",
    "+glass+",
    "+hud+",
    "+hud-theme+",
    "+layout+",
    "+layout-bindings+",
    "+peek+",
    "+policy-bindings+",
    "+policy-displays+",
    "+policy-hidden-window+",
    "+policy-launcher+",
    "+policy-launches+",
    "+policy-settings+",
    "+policy-terminal+",
    "+shader-wallpaper+",
    "+sounds+",
    "adjust-brightness",
    "bind-key",
    "cassette--c",
    "cassette--degrees",
    "cassette--fit",
    "cassette--gib",
    "cassette--grille",
    "cassette--height",
    "cassette--hot-p",
    "cassette--led",
    "cassette--module",
    "cassette--plate",
    "cassette--px",
    "cassette--reel",
    "cassette--split",
    "cassette--window",
    "cassette--zone",
    "clock-text",
    "close-window",
    "configure-output",
    "context",
    "deck--narrow-p",
    "define-extension",
    "exec-async",
    "focus",
    "fullscreen",
    "hide-window",
    "hud--alpha",
    "hud--c",
    "hud--decay",
    "hud--dim",
    "hud--heat",
    "hud--icon",
    "hud--mix",
    "hud--now",
    "hud--off",
    "hud--panel",
    "hud--text",
    "interval",
    "keyboard-grab",
    "launch",
    "media-control",
    "menu-choice",
    "output-power",
    "place",
    "policy--area",
    "policy--centred",
    "policy--launch",
    "policy--terminal",
    "policy--unmanaged-p",
    "previous-context",
    "probe--gpu-p",
    "probe--sample",
    "publish-state",
    "quit",
    "raise-window",
    "rules-for",
    "screenshot",
    "service-state",
    "setting",
    "settings",
    "shell-surface",
    "show-window",
    "sound",
    "state-value",
    "theme",
    "ui",
    "watch-file",
    "window-properties",
    "window-rule",
];

const EKKO: [&str; 8] = [
    "action",
    "register-command",
    "register-component",
    "register-keymap",
    "register-layout-provider",
    "registry",
    "set-option",
    "value",
];

const GLOBALS: [&[&str]; 4] = [&CL, &LOOP, &TOMOE, &EKKO];

fn global(name: &str) -> bool {
    GLOBALS.iter().any(|group| group.contains(&name))
}

struct Binding {
    lisp: String,
    nix: String,
    function: bool,
    site: usize,
}

struct Frame {
    bindings: Vec<Binding>,
    hidden: Vec<(usize, usize, String)>,
}

struct Emitter<'s> {
    source: &'s str,
    holes: &'s [String],
    frames: RefCell<Vec<Frame>>,
    used: RefCell<BTreeSet<String>>,
    unknown: RefCell<BTreeSet<String>>,
}

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_commonlisp::LANGUAGE_COMMONLISP.into(), source)?;
    let root = tree.root_node();
    let mut items = Vec::new();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect(child, source, &mut items)?;
    }
    let items = merge_atoms(items, source)?;
    let emitter = Emitter {
        source,
        holes,
        frames: RefCell::new(Vec::new()),
        used: RefCell::new(BTreeSet::new()),
        unknown: RefCell::new(BTreeSet::new()),
    };
    let mut sites = Vec::new();
    for item in &items {
        if let Form::List(parts) = &item.form {
            let head = parts.first().and_then(atom);
            if head.is_some_and(|head| DEFINERS.contains(&head)) {
                let function = matches!(head, Some("defun" | "defmacro"));
                sites.extend(parts.get(1).and_then(named).map(|site| (site, function)));
            }
        }
    }
    let formals = emitter.enter(&sites, Vec::new(), 1)?;
    let mut body = String::from("[\n");
    let mut previous = 0;
    for item in &items {
        let gap = &source[previous..item.start];
        if previous == 0 {
            if !gap.is_empty() {
                return Err(format!("line 1: the source starts with {gap:?}"));
            }
        } else {
            let lines = newlines(gap).ok_or_else(|| {
                format!(
                    "line {}: two top-level forms share a line or are separated by {gap:?}",
                    line_of(source, item.start)
                )
            })?;
            for _ in 1..lines {
                emitter.uses("blank");
                body.push_str("  blank\n");
            }
        }
        body.push_str("  ");
        body.push_str(&emitter.form(item, false, false, 2)?);
        body.push('\n');
        previous = item.end;
    }
    emitter.frames.borrow_mut().pop();
    let tail = &source[previous..];
    if tail != "\n" {
        return Err(format!(
            "the source must end with one newline after its last form, not {tail:?}"
        ));
    }
    body.push(']');
    let unknown = emitter.unknown.borrow();
    if !unknown.is_empty() {
        let names: Vec<String> = unknown.iter().map(|name| format!("`{name}`")).collect();
        return Err(format!(
            "{} {} bound nowhere and not a global of generators.lisp",
            names.join(", "),
            if names.len() == 1 { "is" } else { "are" }
        ));
    }
    let used = emitter.used.borrow();
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
    match formals {
        Some(formals) => out.push_str(&format!("({{{formals}}}: {body})")),
        None => out.push_str(&body),
    }
    Ok(out)
}

fn atom(item: &Item) -> Option<&str> {
    match &item.form {
        Form::Atom(text) => Some(text.as_str()),
        _ => None,
    }
}

fn named(item: &Item) -> Option<&Item> {
    atom(item).map(|_| item)
}

fn lambda_list<'a>(item: Option<&'a Item>, out: &mut Vec<&'a Item>) {
    if let Some(Item {
        form: Form::List(parts),
        ..
    }) = item
    {
        for part in parts {
            match &part.form {
                Form::Atom(text) if !text.starts_with('&') => out.push(part),
                Form::List(entry) => out.extend(entry.first().and_then(named)),
                _ => {}
            }
        }
    }
}

fn pattern<'a>(item: Option<&'a Item>, out: &mut Vec<&'a Item>) {
    match item.map(|item| (item, &item.form)) {
        Some((item, Form::Atom(text)))
            if !text.starts_with('&') && text != "." && text != "nil" =>
        {
            out.push(item);
        }
        Some((_, Form::List(parts))) => {
            let mut defaulted = false;
            for part in parts {
                match &part.form {
                    Form::Atom(text) if text.starts_with('&') => {
                        defaulted = text != "&rest";
                    }
                    Form::List(entry) if defaulted => out.extend(entry.first().and_then(named)),
                    _ => pattern(Some(part), out),
                }
            }
        }
        _ => {}
    }
}

fn heads<'a>(item: Option<&'a Item>, out: &mut Vec<&'a Item>) {
    if let Some(Item {
        form: Form::List(parts),
        ..
    }) = item
    {
        for part in parts {
            match &part.form {
                Form::Atom(_) => out.push(part),
                Form::List(entry) => out.extend(entry.first().and_then(named)),
                _ => {}
            }
        }
    }
}

fn first<'a>(item: Option<&'a Item>, out: &mut Vec<&'a Item>) {
    if let Some(Item {
        form: Form::List(parts),
        ..
    }) = item
    {
        out.extend(parts.first().and_then(named));
    }
}

fn declared(items: &[Item], definition: bool) -> (Vec<&Item>, bool) {
    let mut out = Vec::new();
    let head = items.first().and_then(atom);
    let functions = matches!(head, Some("flet" | "labels" | "macrolet"));
    match head {
        Some("defun" | "defmacro") => lambda_list(items.get(2), &mut out),
        Some("lambda") => lambda_list(items.get(1), &mut out),
        Some("define-extension") => lambda_list(items.get(3), &mut out),
        Some("let" | "let*") => heads(items.get(1), &mut out),
        Some("flet" | "labels" | "macrolet") => {
            if let Some(Item {
                form: Form::List(definitions),
                ..
            }) = items.get(1)
            {
                for definition in definitions {
                    first(Some(definition), &mut out);
                }
            }
        }
        Some("destructuring-bind" | "multiple-value-bind") => pattern(items.get(1), &mut out),
        Some(
            "dolist"
            | "dotimes"
            | "with-input-from-string"
            | "with-open-file"
            | "with-output-to-string",
        ) => first(items.get(1), &mut out),
        Some("loop") => {
            for (index, item) in items.iter().enumerate() {
                if atom(item).is_some_and(|word| LOOP_BINDERS.contains(&word)) {
                    pattern(items.get(index + 1), &mut out);
                }
            }
        }
        _ if definition => lambda_list(items.get(1), &mut out),
        _ => {}
    }
    (out, functions)
}

fn mentions(item: &Item, name: &str) -> bool {
    match &item.form {
        Form::Atom(text) => text == name,
        Form::Prefix(_, inner) => mentions(inner, name),
        Form::List(parts) => parts.iter().any(|part| mentions(part, name)),
        Form::Str(_) | Form::Comment(_) => false,
    }
}

fn outside<'a>(items: &'a [Item], names: &[&str]) -> Vec<(&'a Item, Vec<String>)> {
    let head = items.first().and_then(atom);
    let all: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let rest = |index: usize| -> Vec<&'a Item> {
        match items.get(index).map(|item| &item.form) {
            Some(Form::List(parts)) => parts.iter().skip(1).collect(),
            _ => Vec::new(),
        }
    };
    let scoped: Vec<(&'a Item, Vec<String>)> = match head {
        Some("let") => initializers(items.get(1))
            .into_iter()
            .map(|(_, init)| (init, all.clone()))
            .collect(),
        Some("let*") => initializers(items.get(1))
            .into_iter()
            .map(|(index, init)| (init, all[index..].to_vec()))
            .collect(),
        Some("destructuring-bind" | "multiple-value-bind") => items
            .get(2)
            .map(|value| vec![(value, all.clone())])
            .unwrap_or_default(),
        Some(
            "dolist"
            | "dotimes"
            | "with-input-from-string"
            | "with-open-file"
            | "with-output-to-string",
        ) => rest(1)
            .into_iter()
            .map(|value| (value, all.clone()))
            .collect(),
        _ => Vec::new(),
    };
    scoped
        .into_iter()
        .map(|(value, names)| {
            let read = names
                .into_iter()
                .filter(|name| mentions(value, name))
                .collect();
            (value, read)
        })
        .filter(|(_, read): &(&Item, Vec<String>)| !read.is_empty())
        .collect()
}

fn initializers(item: Option<&Item>) -> Vec<(usize, &Item)> {
    let mut out = Vec::new();
    if let Some(Item {
        form: Form::List(bindings),
        ..
    }) = item
    {
        let mut index = 0;
        for binding in bindings {
            match &binding.form {
                Form::Atom(_) => index += 1,
                Form::List(parts) => {
                    out.extend(parts.iter().skip(1).map(|init| (index, init)));
                    index += 1;
                }
                _ => {}
            }
        }
    }
    out
}

fn spellable(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn convention(name: &str) -> (&str, Option<&'static str>) {
    for (mark, constructor) in [('+', "constant"), ('*', "special")] {
        if let Some(base) = name
            .strip_prefix(mark)
            .and_then(|rest| rest.strip_suffix(mark))
        {
            if !base.is_empty() {
                return (base, Some(constructor));
            }
        }
    }
    (name, None)
}

fn escaped(base: &str, bound: bool) -> String {
    let taken = NIX_KEYWORDS.contains(&base)
        || CONSTRUCTORS.contains(&base)
        || (bound
            && GLOBALS
                .iter()
                .any(|group| group.iter().any(|global| convention(global).0 == base)));
    if taken {
        format!("{base}'")
    } else {
        base.to_owned()
    }
}

fn float(text: &str) -> Option<String> {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = body.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let canonical = digits(whole)
        && digits(fraction)
        && (whole == "0" || !whole.starts_with('0'))
        && (fraction == "0" || !fraction.ends_with('0'))
        && !(whole == "0" && fraction.starts_with("000"))
        && whole.len() + fraction.len() <= 15;
    canonical.then(|| {
        if text.starts_with('-') {
            format!("({text})")
        } else {
            text.to_owned()
        }
    })
}

fn hex(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("#x")?;
    let value = u64::from_str_radix(digits, 16).ok()?;
    (format!("{value:X}") == digits && value < 1 << 53).then_some(value)
}

fn newlines(gap: &str) -> Option<usize> {
    let lines = gap.bytes().take_while(|&b| b == b'\n').count();
    (lines > 0 && gap[lines..].bytes().all(|b| b == b' ') && !gap[lines..].contains('\n'))
        .then_some(lines)
}

fn is_list(node: tree_sitter::Node<'_>) -> bool {
    let mut cursor = node.walk();
    let first = node.children(&mut cursor).next();
    first.is_some_and(|child| !child.is_named() && child.kind() == "(")
}

fn structured(node: tree_sitter::Node<'_>) -> bool {
    if is_list(node)
        || matches!(node.kind(), "str_lit" | "comment" | "block_comment")
        || prefix(node.kind()).is_some()
    {
        return true;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.into_iter().any(structured)
}

fn prefix(kind: &str) -> Option<&'static str> {
    PREFIXES
        .iter()
        .find(|(node, _)| *node == kind)
        .map(|(_, name)| *name)
}

fn collect(node: tree_sitter::Node<'_>, source: &str, out: &mut Vec<Item>) -> Result<(), String> {
    let (start, end) = (node.start_byte(), node.end_byte());
    let line = line_of(source, start);
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    if is_list(node) {
        let close = children
            .last()
            .filter(|c| c.kind() == ")")
            .ok_or(format!("line {line}: unclosed list"))?;
        let mut items = Vec::new();
        for child in &children[1..children.len() - 1] {
            collect(*child, source, &mut items)?;
        }
        out.push(Item {
            form: Form::List(merge_atoms(items, source)?),
            start,
            end: close.end_byte(),
        });
        return Ok(());
    }
    let form = match node.kind() {
        "str_lit" => Form::Str(unescape(&source[start + 1..end - 1])),
        "comment" => {
            let text = source[start..end].trim_end_matches('\n');
            out.push(Item {
                form: Form::Comment(text.to_owned()),
                start,
                end: start + text.len(),
            });
            return Ok(());
        }
        "block_comment" => return Err(format!("line {line}: block comments are not supported")),
        kind => match prefix(kind) {
            Some(name) => {
                let mut inner = Vec::new();
                for child in &children[1..] {
                    collect(*child, source, &mut inner)?;
                }
                let mut inner = merge_atoms(inner, source)?;
                if inner.len() != 1 || inner[0].start != children[0].end_byte() {
                    return Err(format!(
                        "line {line}: a reader prefix must be followed directly by one form"
                    ));
                }
                Form::Prefix(name, Box::new(inner.remove(0)))
            }
            None if structured(node) || source[start..end].contains(char::is_whitespace) => {
                for child in children {
                    collect(child, source, out)?;
                }
                return Ok(());
            }
            None => Form::Atom(source[start..end].to_owned()),
        },
    };
    out.push(Item { form, start, end });
    Ok(())
}

fn merge_atoms(items: Vec<Item>, source: &str) -> Result<Vec<Item>, String> {
    let mut out: Vec<Item> = Vec::new();
    for item in items {
        if let Some(last) = out.last_mut() {
            if last.end == item.start {
                match (&mut last.form, &item.form) {
                    (Form::Atom(text), Form::Atom(next)) => {
                        text.push_str(next);
                        last.end = item.end;
                        continue;
                    }
                    _ => {
                        return Err(format!(
                            "line {}: forms without a space between them",
                            line_of(source, item.start)
                        ))
                    }
                }
            }
        }
        out.push(item);
    }
    Ok(out)
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' {
            chars.next().unwrap_or('\\')
        } else {
            c
        });
    }
    out
}

fn number(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let mut parts = body.splitn(2, ['.', '/']);
    let whole = parts.next().unwrap_or("");
    let rest = parts.next();
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    match rest {
        None => !whole.is_empty() && digits(whole),
        Some(fraction) => {
            digits(whole)
                && !(whole.is_empty() && fraction.is_empty())
                && digits(fraction.split(['e', 'E', 'd', 'D']).next().unwrap_or(""))
        }
    }
}

fn symbol(text: &str) -> bool {
    !text.starts_with([':', '#', '"']) && !number(text)
}

pub fn offset(head: Option<&str>, local: bool, index: usize, first_row: usize) -> usize {
    if local {
        return if index > 1 { 2 } else { 4 };
    }
    let Some(head) = head else { return 1 };
    if let Some((_, body)) = BODIES.iter().find(|(name, _)| *name == head) {
        if *body != usize::MAX {
            return if index > *body { 2 } else { 4 };
        }
    }
    if symbol(head) && first_row >= 2 {
        head.len() + 2
    } else if head == "cond" {
        2
    } else {
        1
    }
}

impl Emitter<'_> {
    fn string(&self, text: &str, indent: usize) -> String {
        nix::string(text, self.holes, indent)
    }

    fn uses(&self, name: &str) {
        self.used.borrow_mut().insert(name.to_owned());
    }

    fn enter(
        &self,
        sites: &[(&Item, bool)],
        hidden: Vec<(usize, usize, String)>,
        line: usize,
    ) -> Result<Option<String>, String> {
        if sites.is_empty() {
            return Ok(None);
        }
        let mut bindings: Vec<Binding> = Vec::new();
        for (item, function) in sites {
            let name = atom(item).unwrap_or_default();
            let (base, _) = convention(name);
            if !spellable(base) {
                return Err(format!(
                    "line {line}: `{name}` has characters a Nix name cannot spell"
                ));
            }
            let mut nix = escaped(base, true);
            if hidden.iter().any(|(_, _, shadowing)| shadowing == name) {
                if nix.ends_with('\'') {
                    return Err(format!(
                        "line {line}: `{name}` is read from outside the form that rebinds it, but Nix already spells it `{nix}`"
                    ));
                }
                nix.push('\'');
            }
            if bindings.iter().any(|binding| binding.nix == nix) {
                return Err(format!(
                    "line {line}: `{name}` is declared twice in one form, or beside a name Nix spells the same"
                ));
            }
            bindings.push(Binding {
                lisp: name.to_owned(),
                nix,
                function: *function,
                site: item.start,
            });
        }
        let formals: Vec<&str> = bindings
            .iter()
            .map(|binding| binding.nix.as_str())
            .collect();
        let formals = formals.join(", ");
        self.frames.borrow_mut().push(Frame { bindings, hidden });
        Ok(Some(formals))
    }

    fn resolve(&self, text: &str, at: usize, head: bool) -> Option<(usize, String)> {
        let frames = self.frames.borrow();
        if let Some(found) = frames.iter().enumerate().rev().find_map(|(depth, frame)| {
            frame
                .bindings
                .iter()
                .find(|binding| binding.site == at)
                .map(|binding| (depth, binding.nix.clone()))
        }) {
            return Some(found);
        }
        let nearest = |function: bool| {
            frames.iter().enumerate().rev().find_map(|(depth, frame)| {
                let hidden = frame
                    .hidden
                    .iter()
                    .any(|(start, end, name)| name == text && (*start..*end).contains(&at));
                frame
                    .bindings
                    .iter()
                    .find(|binding| !hidden && binding.lisp == text && binding.function == function)
                    .map(|binding| (depth, binding.nix.clone()))
            })
        };
        if head {
            nearest(true).or_else(|| (!global(text)).then(|| nearest(false)).flatten())
        } else {
            nearest(false).or_else(|| nearest(true))
        }
    }

    fn symbol(&self, text: &str, at: usize, head: bool) -> Result<String, String> {
        let line = line_of(self.source, at);
        if let Some(operator) = OPERATORS.iter().find(|operator| **operator == text) {
            return Ok(format!("\"{operator}\""));
        }
        if let Some((_, name)) = MARKERS.iter().find(|(marker, _)| *marker == text) {
            self.uses(name);
            return Ok((*name).to_owned());
        }
        if text == "." {
            self.uses("dot");
            return Ok("dot".to_owned());
        }
        let (base, wrapper) = convention(text);
        if !spellable(base) {
            return Err(format!(
                "line {line}: `{text}` has characters a Nix name cannot spell"
            ));
        }
        let (nix, inner) = match self.resolve(text, at, head) {
            Some((depth, nix)) => (nix, depth + 1),
            None if global(text) => {
                let nix = escaped(base, false);
                self.uses(&nix);
                (nix, 0)
            }
            None => {
                self.unknown.borrow_mut().insert(text.to_owned());
                (escaped(base, false), 0)
            }
        };
        let frames = self.frames.borrow();
        if frames[inner..]
            .iter()
            .any(|frame| frame.bindings.iter().any(|binding| binding.nix == nix))
        {
            return Err(format!(
                "line {line}: `{text}` is spelled `{nix}` in Nix, which a nearer binding of that Nix name shadows"
            ));
        }
        Ok(match wrapper {
            Some(wrapper) => {
                self.uses(wrapper);
                format!("({wrapper} {nix})")
            }
            None => nix,
        })
    }

    fn atom(&self, item: &Item, text: &str, head: bool, indent: usize) -> Result<String, String> {
        if let Some(int) = nix::int(text) {
            return Ok(int);
        }
        if let Some(float) = float(text) {
            return Ok(float);
        }
        let constructed = |name: &str, argument: String| {
            self.uses(name);
            format!("({name} {argument})")
        };
        if let Some((numerator, denominator)) = text.split_once('/') {
            if let (Some(numerator), Some(denominator)) =
                (nix::int(numerator), nix::int(denominator))
            {
                if !denominator.starts_with('(') && denominator != "0" {
                    return Ok(constructed("ratio", format!("{numerator} {denominator}")));
                }
            }
        }
        if let Some(int) = text.strip_suffix("d0").and_then(nix::int) {
            return Ok(constructed("double", int));
        }
        if let Some(value) = hex(text) {
            return Ok(constructed("hex", value.to_string()));
        }
        if let Some(name) = text.strip_prefix("#\\") {
            return Ok(constructed("char", self.string(name, indent)));
        }
        if let Some(name) = text.strip_prefix("#:") {
            if !name.is_empty() && !name.contains([':', '|', '\\', '#']) {
                return Ok(constructed("uninterned", self.string(name, indent)));
            }
        }
        if let Some(name) = text.strip_prefix(':') {
            if !name.is_empty() && !name.contains([':', '|', '\\', '#']) {
                return Ok(constructed("kw", self.string(name, indent)));
            }
        }
        if text.starts_with('#') || text.contains([':', '|', '\\']) || number(text) {
            return Err(format!(
                "line {}: `{text}` is reader syntax the printer does not write",
                line_of(self.source, item.start)
            ));
        }
        self.symbol(text, item.start, head)
    }

    fn form(&self, item: &Item, local: bool, head: bool, indent: usize) -> Result<String, String> {
        let line = line_of(self.source, item.start);
        Ok(match &item.form {
            Form::Atom(text) => self.atom(item, text, head, indent)?,
            Form::Str(text) => {
                self.uses("str");
                format!("(str {})", self.string(text, indent))
            }
            Form::Comment(text) => {
                self.uses("comment");
                format!("(comment {})", self.string(text, indent))
            }
            Form::Prefix(name, inner) => {
                self.uses(name);
                let inner = self.form(inner, false, *name == "function", indent)?;
                format!("({name} {inner})")
            }
            Form::List(items) => {
                let (sites, functions) = declared(items, local);
                let names: Vec<&str> = sites.iter().filter_map(|site| atom(site)).collect();
                let hidden = outside(items, &names)
                    .into_iter()
                    .flat_map(|(value, read)| {
                        read.into_iter()
                            .map(move |name| (value.start, value.end, name))
                    })
                    .collect();
                let sites: Vec<(&Item, bool)> =
                    sites.into_iter().map(|site| (site, functions)).collect();
                match self.enter(&sites, hidden, line)? {
                    Some(formals) => {
                        let body = self.list(item.start, items, local, false, indent);
                        self.frames.borrow_mut().pop();
                        let body = body?;
                        let body = body
                            .strip_prefix('(')
                            .and_then(|rows| rows.strip_suffix(')'))
                            .unwrap_or(&body);
                        format!("({{{formals}}}: {body})")
                    }
                    None => self.list(item.start, items, local, false, indent)?,
                }
            }
        })
    }

    fn list(
        &self,
        open: usize,
        items: &[Item],
        local: bool,
        children_local: bool,
        indent: usize,
    ) -> Result<String, String> {
        let source = self.source;
        let line = |at: usize| line_of(source, at);
        if let Some(first) = items.first() {
            if first.start != open + 1 {
                return Err(format!(
                    "line {}: space after an opening parenthesis",
                    line(open)
                ));
            }
        }
        let head = match items.first().map(|item| &item.form) {
            Some(Form::Atom(text)) => Some(text.as_str()),
            _ => None,
        };
        let mut rows: Vec<Row<'_>> = Vec::new();
        let mut row_start = 0;
        for index in 1..=items.len() {
            let gap = if index < items.len() {
                &source[items[index - 1].end..items[index].start]
            } else {
                ""
            };
            if index < items.len() && gap == " " {
                if matches!(items[index - 1].form, Form::Comment(_)) {
                    return Err(format!(
                        "line {}: a comment runs to the end of its line",
                        line(items[index].start)
                    ));
                }
                continue;
            }
            rows.push(Row {
                items: &items[row_start..index],
                first: row_start,
                at: None,
                blank: false,
            });
            if index == items.len() {
                break;
            }
            let lines = newlines(gap).ok_or_else(|| {
                format!(
                    "line {}: items separated by {gap:?}",
                    line(items[index].start)
                )
            })?;
            for _ in 1..lines {
                rows.push(Row {
                    items: &[],
                    first: index,
                    at: None,
                    blank: true,
                });
            }
            row_start = index;
        }
        if let Some(last) = items.last() {
            if matches!(last.form, Form::Comment(_)) {
                return Err(format!(
                    "line {}: a comment before a closing parenthesis",
                    line(last.start)
                ));
            }
        }
        let first_row = rows.first().map_or(0, |row| row.items.len());
        let base = column_of(source, open);
        for row in rows.iter_mut().skip(1).filter(|row| !row.blank) {
            let actual = column_of(source, row.items[0].start) - base;
            if actual != offset(head, local, row.first, first_row) {
                row.at = Some(actual);
            }
        }
        let locals = head.is_some_and(|h| LOCALS.contains(&h));
        let item = |index: usize, indent: usize| -> Result<String, String> {
            let item = &items[index];
            if locals && index == 1 {
                if let Form::List(definitions) = &item.form {
                    return self.list(item.start, definitions, false, true, indent);
                }
            }
            self.form(item, children_local, index == 0, indent)
        };
        if rows.len() <= 1 {
            let parts = (0..items.len())
                .map(|i| item(i, indent))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(format!("[{}]", parts.join(" ")));
        }
        let pad = " ".repeat(indent + 2);
        self.uses("rows");
        let mut out = String::from("(rows [\n");
        for row in &rows {
            out.push_str(&pad);
            if row.blank {
                self.uses("blank");
                out.push_str("blank");
            } else {
                let parts = (row.first..row.first + row.items.len())
                    .map(|i| item(i, indent + 2))
                    .collect::<Result<Vec<_>, _>>()?;
                let lone = parts.len() == 1 && !parts[0].starts_with('[');
                let body = if lone {
                    parts[0].clone()
                } else {
                    format!("[{}]", parts.join(" "))
                };
                match row.at {
                    Some(at) => {
                        self.uses("at");
                        out.push_str(&format!("(at {at} {body})"));
                    }
                    None => out.push_str(&body),
                }
            }
            out.push('\n');
        }
        out.push_str(&" ".repeat(indent));
        out.push_str("])");
        Ok(out)
    }
}
