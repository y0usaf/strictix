use std::collections::HashSet;

pub(crate) const SENTINEL: char = '\u{1}';

pub(crate) struct Interp<'a> {
    pub(crate) full: &'a str,
    pub(crate) json: Option<&'a str>,
}

enum Node {
    Raw(String),
    List(Vec<Node>),
    Set(Vec<(String, Node)>),
}

const KEYWORDS: [&str; 10] = [
    "if", "then", "else", "assert", "with", "let", "in", "rec", "inherit", "or",
];

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-'))
        && !KEYWORDS.contains(&name)
}

fn nix_string<'a>(
    text: &str,
    interps: &mut impl Iterator<Item = &'a Interp<'a>>,
) -> Option<String> {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            SENTINEL => out.push_str(interps.next()?.full),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => return None,
            c => out.push(c),
        }
    }
    out.push('"');
    Some(out)
}

fn attr_name(name: &str) -> Option<String> {
    if name.contains(SENTINEL) {
        return None;
    }
    if is_ident(name) {
        Some(name.to_owned())
    } else {
        nix_string(name, &mut std::iter::empty())
    }
}

fn render(node: &Node, indent: &str, depth: usize, out: &mut String) {
    let pad = |out: &mut String, depth: usize| {
        out.push_str(indent);
        out.push_str(&"  ".repeat(depth));
    };
    match node {
        Node::Raw(text) => out.push_str(text),
        Node::List(items) if items.is_empty() => out.push_str("[ ]"),
        Node::Set(items) if items.is_empty() => out.push_str("{ }"),
        Node::List(items) => {
            out.push_str("[\n");
            for item in items {
                pad(out, depth + 1);
                render(item, indent, depth + 1, out);
                out.push('\n');
            }
            pad(out, depth);
            out.push(']');
        }
        Node::Set(items) => {
            out.push_str("{\n");
            for (key, value) in items {
                pad(out, depth + 1);
                out.push_str(key);
                out.push_str(" = ");
                render(value, indent, depth + 1, out);
                out.push_str(";\n");
            }
            pad(out, depth);
            out.push('}');
        }
    }
}

pub(crate) fn ini(body: &str, interps: &[Interp<'_>], indent: &str) -> Option<String> {
    let mut interps = interps.iter();
    let mut sections: Vec<(String, Node)> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    for line in body.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let name = header.strip_suffix(']')?;
            names.push(name);
            sections.push((attr_name(name)?, Node::Set(Vec::new())));
            continue;
        }
        let (key, value) = line.split_once('=')?;
        let entry = (attr_name(key)?, Node::Raw(nix_string(value, &mut interps)?));
        match sections.last_mut() {
            Some((_, Node::Set(keys))) => keys.push(entry),
            _ => return None,
        }
    }
    interps.next().is_none().then_some(())?;
    names.windows(2).all(|w| w[0] < w[1]).then_some(())?;
    let mut out = String::from("lib.generators.toINI { } ");
    render(&Node::Set(sections), indent, 0, &mut out);
    Some(out)
}

struct Json<'s, 'i> {
    chars: std::iter::Peekable<std::str::Chars<'s>>,
    interps: std::slice::Iter<'i, Interp<'i>>,
}

impl Json<'_, '_> {
    fn ws(&mut self) {
        while self
            .chars
            .next_if(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
            .is_some()
        {}
    }

    fn eat(&mut self, expected: char) -> Option<()> {
        self.chars.next_if_eq(&expected).map(|_| ())
    }

    fn word(&mut self, word: &str) -> Option<Node> {
        for c in word.chars() {
            self.eat(c)?;
        }
        Some(Node::Raw(word.to_owned()))
    }

    fn value(&mut self) -> Option<Node> {
        self.ws();
        let node = match *self.chars.peek()? {
            SENTINEL => {
                self.chars.next();
                Node::Raw(format!("({})", self.interps.next()?.json?))
            }
            '{' => self.object()?,
            '[' => self.array()?,
            '"' => {
                let text = self.string()?;
                Node::Raw(nix_string(&text, &mut self.interps)?)
            }
            't' => self.word("true")?,
            'f' => self.word("false")?,
            'n' => self.word("null")?,
            _ => self.number()?,
        };
        self.ws();
        Some(node)
    }

    fn number(&mut self) -> Option<Node> {
        let negative = self.eat('-').is_some();
        let mut digits = String::new();
        while let Some(c) = self.chars.next_if(char::is_ascii_digit) {
            digits.push(c);
        }
        if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
            return None;
        }
        if matches!(self.chars.peek(), Some('.' | 'e' | 'E')) {
            return None;
        }
        digits.parse::<i64>().ok()?;
        Some(Node::Raw(if negative {
            format!("(-{digits})")
        } else {
            digits
        }))
    }

    fn hex(&mut self) -> Option<u32> {
        let mut code = 0;
        for _ in 0..4 {
            code = code * 16 + self.chars.next()?.to_digit(16)?;
        }
        Some(code)
    }

    fn string(&mut self) -> Option<String> {
        self.eat('"')?;
        let mut out = String::new();
        loop {
            match self.chars.next()? {
                '"' => return Some(out),
                '\\' => out.push(match self.chars.next()? {
                    '"' => '"',
                    '\\' => '\\',
                    '/' => '/',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'u' => {
                        let code = self.hex()?;
                        let code = if (0xD800..0xDC00).contains(&code) {
                            self.eat('\\')?;
                            self.eat('u')?;
                            let low = self.hex()?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return None;
                            }
                            0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                        } else {
                            code
                        };
                        char::from_u32(code).filter(|&c| c != SENTINEL)?
                    }
                    _ => return None,
                }),
                c if (c as u32) < 0x20 && c != SENTINEL => return None,
                c => out.push(c),
            }
        }
    }

    fn array(&mut self) -> Option<Node> {
        self.eat('[')?;
        self.ws();
        let mut items = Vec::new();
        if self.eat(']').is_some() {
            return Some(Node::List(items));
        }
        loop {
            items.push(self.value()?);
            if self.eat(']').is_some() {
                return Some(Node::List(items));
            }
            self.eat(',')?;
        }
    }

    fn object(&mut self) -> Option<Node> {
        self.eat('{')?;
        self.ws();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        if self.eat('}').is_some() {
            return Some(Node::Set(items));
        }
        loop {
            self.ws();
            let key = self.string()?;
            if !seen.insert(key.clone()) {
                return None;
            }
            self.ws();
            self.eat(':')?;
            items.push((attr_name(&key)?, self.value()?));
            if self.eat('}').is_some() {
                return Some(Node::Set(items));
            }
            self.eat(',')?;
        }
    }
}

pub(crate) fn json(body: &str, interps: &[Interp<'_>], indent: &str) -> Option<String> {
    let mut parser = Json {
        chars: body.chars().peekable(),
        interps: interps.iter(),
    };
    let node = parser.value()?;
    if parser.chars.next().is_some() || parser.interps.next().is_some() {
        return None;
    }
    let Node::Set(_) = node else {
        return None;
    };
    let mut out = String::from("builtins.toJSON ");
    render(&node, indent, 0, &mut out);
    Some(out)
}
