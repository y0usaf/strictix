pub const HOLE_BASE: u32 = 0xF0000;

pub fn string(text: &str, holes: &[String], indent: usize) -> String {
    if text.contains('\n') && indented_safe(text) {
        indented(text, holes, indent)
    } else {
        quoted(text, holes)
    }
}

fn hole(c: char, holes: &[String]) -> Option<&str> {
    let index = (c as u32).checked_sub(HOLE_BASE)?;
    holes.get(index as usize).map(String::as_str)
}

fn quoted(text: &str, holes: &[String]) -> String {
    let mut out = String::from("\"");
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => match hole(c, holes) {
                Some(expr) => {
                    out.push_str("${");
                    out.push_str(expr);
                    out.push('}');
                }
                None => out.push(c),
            },
        }
    }
    out.push('"');
    out
}

fn indented_safe(text: &str) -> bool {
    let unterminated = !text.ends_with('\n');
    !text.contains(['\t', '\r'])
        && !text.starts_with('\n')
        && !(unterminated && text.ends_with('\''))
        && !text.lines().any(|line| line.ends_with(' '))
        && text
            .lines()
            .any(|line| !line.is_empty() && !line.starts_with(' '))
}

fn indented(text: &str, holes: &[String], indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::from("''\n");
    let mut chars = text.chars().peekable();
    let mut fresh = true;
    while let Some(c) = chars.next() {
        if fresh && c != '\n' {
            out.push_str(&pad);
        }
        fresh = c == '\n';
        match c {
            '\'' if chars.peek() == Some(&'\'') => {
                chars.next();
                out.push_str("'''");
            }
            '$' if chars.peek() == Some(&'{') => out.push_str("''$"),
            c => match hole(c, holes) {
                Some(expr) => {
                    out.push_str("${");
                    out.push_str(expr);
                    out.push('}');
                }
                None => out.push(c),
            },
        }
    }
    if text.ends_with('\n') {
        out.push_str(&pad);
    }
    out.push_str("''");
    out
}

pub fn int(text: &str) -> Option<String> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let plain = !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'))
        && digits.len() < 18
        && text != "-0";
    plain.then(|| {
        if text.starts_with('-') {
            format!("({text})")
        } else {
            text.to_owned()
        }
    })
}
