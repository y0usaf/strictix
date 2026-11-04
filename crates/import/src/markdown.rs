use tree_sitter::Node;

use crate::{line_of, nix};

const WIDTH: usize = 78;

pub fn import(source: &str, holes: &[String]) -> Result<String, String> {
    let tree = crate::parse(&tree_sitter_md::LANGUAGE.into(), source)?;
    let mut blocks = Vec::new();
    flatten(tree.root_node(), &mut blocks);
    let importer = Importer { source, holes };
    let first = blocks.first().ok_or("the document is empty")?;
    if first.start_byte() != 0 {
        return Err("the document must start with its first block".into());
    }
    let mut out = Vec::new();
    let mut previous: Option<usize> = None;
    for block in &blocks {
        if let Some(end) = previous {
            let gap = &source[end..block.start_byte()];
            if gap.is_empty() || gap.bytes().any(|b| b != b'\n') {
                return Err(format!(
                    "{}: blocks separated by {gap:?}",
                    importer.at(block.start_byte())
                ));
            }
            for _ in 1..gap.len() {
                out.push("blank".to_owned());
            }
        }
        let end = importer.end(*block);
        out.push(importer.block(*block, end)?);
        previous = Some(end);
    }
    if previous.is_none_or(|end| end + 1 != source.len() || !source.ends_with('\n')) {
        return Err("the document must end with one newline after its last block".into());
    }
    Ok(format!(
        "[\n{}]",
        out.iter()
            .map(|line| format!("  {line}\n"))
            .collect::<String>()
    ))
}

fn flatten<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "section" | "document" => flatten(child, out),
            _ => out.push(child),
        }
    }
}

fn named(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| child.kind() != "block_continuation")
        .collect()
}

enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Link(Vec<Inline>, String),
}

struct Importer<'s> {
    source: &'s str,
    holes: &'s [String],
}

impl Importer<'_> {
    fn at(&self, offset: usize) -> String {
        format!("line {}", line_of(self.source, offset))
    }

    fn string(&self, text: &str) -> String {
        nix::string(text, self.holes, 0)
    }

    fn end(&self, node: Node<'_>) -> usize {
        node.start_byte()
            + self.source[node.start_byte()..node.end_byte()]
                .trim_end_matches('\n')
                .len()
    }

    fn block(&self, node: Node<'_>, end: usize) -> Result<String, String> {
        let start = node.start_byte();
        match node.kind() {
            "atx_heading" => self.heading(node, end),
            "paragraph" => self.paragraph(node, end, 0, 0),
            "list" => self.list(node),
            "html_block" => {
                let text = &self.source[start..end];
                let body = text
                    .strip_prefix("<!--")
                    .and_then(|rest| rest.strip_suffix("-->"))
                    .filter(|body| !body.contains("-->"))
                    .ok_or_else(|| format!("{}: HTML other than one comment", self.at(start)))?;
                Ok(format!("(comment {})", self.string(body)))
            }
            "pipe_table" => self.table(node, end),
            kind => Err(format!("{}: {kind} is not supported", self.at(start))),
        }
    }

    fn heading(&self, node: Node<'_>, end: usize) -> Result<String, String> {
        let parts = named(node);
        let [marker, content] = parts.as_slice() else {
            return Err(format!(
                "{}: a heading without text",
                self.at(node.start_byte())
            ));
        };
        let level = marker.end_byte() - marker.start_byte();
        if !marker.kind().starts_with("atx_h")
            || content.start_byte() != marker.end_byte() + 1
            || &self.source[marker.end_byte()..content.start_byte()] != " "
            || content.end_byte() != end
        {
            return Err(format!(
                "{}: a heading is not `# text`",
                self.at(node.start_byte())
            ));
        }
        let inlines = self.inlines(content.start_byte(), end)?;
        Ok(format!("(heading {level} {})", self.nix(&inlines)))
    }

    fn paragraph(
        &self,
        node: Node<'_>,
        end: usize,
        indent: usize,
        column: usize,
    ) -> Result<String, String> {
        let start = node.start_byte();
        let parts = named(node);
        let [content] = parts.as_slice() else {
            return Err(format!(
                "{}: a paragraph holds more than text",
                self.at(start)
            ));
        };
        if content.kind() != "inline" || content.start_byte() != start || content.end_byte() != end
        {
            return Err(format!(
                "{}: a paragraph holds more than text",
                self.at(start)
            ));
        }
        let text = &self.source[start..end];
        let inlines = self.inlines(start, end)?;
        if !text.contains('\n') {
            return Ok(self.nix(&inlines));
        }
        let flat = unwrap(inlines, &format!("\n{}", " ".repeat(indent)))
            .ok_or_else(|| format!("{}: a line break inside code or a link", self.at(start)))?;
        let mut pieces = Vec::new();
        pieces_of(&flat, &mut pieces);
        if fill(&pieces, indent, column) != text {
            return Err(format!(
                "{}: the paragraph's line breaks are not a fill to {WIDTH} columns",
                self.at(start)
            ));
        }
        Ok(format!("(wrap {})", self.nix(&flat)))
    }

    fn list(&self, node: Node<'_>) -> Result<String, String> {
        let items = named(node);
        let mut out = Vec::new();
        let mut previous: Option<usize> = None;
        for item in items {
            let start = item.start_byte();
            if item.kind() != "list_item" {
                return Err(format!("{}: {} in a list", self.at(start), item.kind()));
            }
            if let Some(end) = previous {
                if end + 1 != start {
                    return Err(format!("{}: a loose list", self.at(start)));
                }
            }
            let parts = named(item);
            let [marker, paragraph] = parts.as_slice() else {
                return Err(format!(
                    "{}: a list item holds more than one paragraph",
                    self.at(start)
                ));
            };
            if marker.kind() != "list_marker_minus"
                || &self.source[marker.start_byte()..marker.end_byte()] != "- "
                || paragraph.kind() != "paragraph"
            {
                return Err(format!("{}: a list item is not `- text`", self.at(start)));
            }
            let end = self.end(item);
            out.push(self.paragraph(*paragraph, end, 2, 2)?);
            previous = Some(end);
        }
        Ok(format!(
            "(bullets [\n{}  ])",
            out.iter()
                .map(|item| format!("    {item}\n"))
                .collect::<String>()
        ))
    }

    fn table(&self, node: Node<'_>, end: usize) -> Result<String, String> {
        let start = node.start_byte();
        let rows = named(node);
        let mut out = Vec::new();
        let mut previous = start;
        let mut columns = 0;
        for (index, row) in rows.iter().enumerate() {
            if index > 0 && &self.source[previous..row.start_byte()] != "\n" {
                return Err(format!(
                    "{}: table rows separated by more than a newline",
                    self.at(row.start_byte())
                ));
            }
            previous = row.end_byte();
            let line = &self.source[row.start_byte()..row.end_byte()];
            if row.kind() == "pipe_table_delimiter_row" {
                if line != format!("|{}", "---|".repeat(columns)) {
                    return Err(format!(
                        "{}: a delimiter row other than |---|",
                        self.at(row.start_byte())
                    ));
                }
                continue;
            }
            let cells: Vec<_> = named(*row)
                .into_iter()
                .map(|cell| {
                    let text = &self.source[cell.start_byte()..cell.end_byte()];
                    (
                        cell.start_byte(),
                        cell.start_byte() + text.trim_end_matches(' ').len(),
                    )
                })
                .collect();
            columns = cells.len();
            let texts: Vec<_> = cells
                .iter()
                .map(|&(from, to)| &self.source[from..to])
                .collect();
            if line != format!("| {} |", texts.join(" | ")) {
                return Err(format!(
                    "{}: a table row is not `| a | b |`",
                    self.at(row.start_byte())
                ));
            }
            let cells = cells
                .iter()
                .map(|&(from, to)| Ok(self.nix(&self.inlines(from, to)?)))
                .collect::<Result<Vec<_>, String>>()?;
            out.push(format!("[{}]", cells.join(" ")));
        }
        if previous != end {
            return Err(format!("{}: text after the table", self.at(previous)));
        }
        let Some((header, rows)) = out.split_first() else {
            return Err(format!("{}: a table without a header", self.at(start)));
        };
        Ok(format!(
            "(table {header} [\n{}  ])",
            rows.iter()
                .map(|row| format!("    {row}\n"))
                .collect::<String>()
        ))
    }

    fn inlines(&self, from: usize, to: usize) -> Result<Vec<Inline>, String> {
        let text = &self.source[from..to];
        let tree = crate::parse(&tree_sitter_md::INLINE_LANGUAGE.into(), text)?;
        let inlines = Inlines {
            importer: self,
            text,
            base: from,
        }
        .within(tree.root_node(), 0, text.len())?;
        match literal(&inlines) {
            Some(markup) => Err(format!(
                "{}: the text {markup:?} would read as markup the printer does not write",
                self.at(from)
            )),
            None => Ok(inlines),
        }
    }

    fn nix(&self, inlines: &[Inline]) -> String {
        match inlines {
            [Inline::Text(text)] => self.string(text),
            _ => self.list_of(inlines),
        }
    }

    fn list_of(&self, inlines: &[Inline]) -> String {
        let items: Vec<_> = inlines.iter().map(|inline| self.inline(inline)).collect();
        format!("[{}]", items.join(" "))
    }

    fn inline(&self, inline: &Inline) -> String {
        match inline {
            Inline::Text(text) => self.string(text),
            Inline::Code(code) => format!("(code {})", self.string(code)),
            Inline::Strong(inner) => format!("(strong {})", self.nix(inner)),
            Inline::Em(inner) => format!("(em {})", self.nix(inner)),
            Inline::Link(inner, url) => format!("(link {} {})", self.nix(inner), self.string(url)),
        }
    }
}

struct Inlines<'a, 's> {
    importer: &'a Importer<'s>,
    text: &'a str,
    base: usize,
}

impl Inlines<'_, '_> {
    fn fail(&self, offset: usize, what: &str) -> String {
        format!("{}: {what}", self.importer.at(self.base + offset))
    }

    fn within(&self, node: Node<'_>, from: usize, to: usize) -> Result<Vec<Inline>, String> {
        let mut out = Vec::new();
        let mut at = from;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.start_byte() < from || child.end_byte() > to {
                continue;
            }
            if child.start_byte() > at {
                text(&mut out, &self.text[at..child.start_byte()]);
            }
            if child.kind() == "shortcut_link" {
                text(&mut out, "[");
                for inline in self.bracketed(child)? {
                    match inline {
                        Inline::Text(more) => text(&mut out, &more),
                        other => out.push(other),
                    }
                }
                text(&mut out, "]");
            } else {
                out.push(self.inline(child)?);
            }
            at = child.end_byte();
        }
        if at < to {
            text(&mut out, &self.text[at..to]);
        }
        Ok(out)
    }

    fn bracketed(&self, node: Node<'_>) -> Result<Vec<Inline>, String> {
        let (start, end) = (node.start_byte(), node.end_byte());
        let parts = named(node);
        match parts[..] {
            [] if &self.text[start..end] == "[]" => Ok(Vec::new()),
            [label]
                if label.kind() == "link_text"
                    && label.start_byte() == start + 1
                    && label.end_byte() + 1 == end =>
            {
                self.within(label, label.start_byte(), label.end_byte())
            }
            _ => Err(self.fail(start, "brackets other than literal `[text]`")),
        }
    }

    fn delimited(&self, node: Node<'_>, open: &str) -> Result<(usize, usize), String> {
        let (start, end) = (node.start_byte(), node.end_byte());
        let slice = &self.text[start..end];
        if slice.len() < 2 * open.len() || !slice.starts_with(open) || !slice.ends_with(open) {
            return Err(self.fail(
                start,
                &format!("{} is not delimited by {open}", node.kind()),
            ));
        }
        Ok((start + open.len(), end - open.len()))
    }

    fn inline(&self, node: Node<'_>) -> Result<Inline, String> {
        let start = node.start_byte();
        match node.kind() {
            "code_span" => {
                let (from, to) = self.delimited(node, "`")?;
                let code = &self.text[from..to];
                if code.contains('`') || code.starts_with(' ') || code.ends_with(' ') {
                    return Err(self.fail(start, "a code span with backticks or edge spaces"));
                }
                Ok(Inline::Code(code.to_owned()))
            }
            "strong_emphasis" => {
                let (from, to) = self.delimited(node, "**")?;
                Ok(Inline::Strong(self.within(node, from, to)?))
            }
            "emphasis" => {
                let (from, to) = self.delimited(node, "*")?;
                Ok(Inline::Em(self.within(node, from, to)?))
            }
            "inline_link" => {
                let parts: Vec<_> = {
                    let mut cursor = node.walk();
                    node.named_children(&mut cursor).collect()
                };
                let [label, destination] = parts.as_slice() else {
                    return Err(self.fail(start, "a link other than [text](url)"));
                };
                if label.kind() != "link_text"
                    || destination.kind() != "link_destination"
                    || label.start_byte() != start + 1
                    || &self.text[label.end_byte()..destination.start_byte()] != "]("
                    || destination.end_byte() + 1 != node.end_byte()
                {
                    return Err(self.fail(start, "a link other than [text](url)"));
                }
                Ok(Inline::Link(
                    self.within(*label, label.start_byte(), label.end_byte())?,
                    self.text[destination.start_byte()..destination.end_byte()].to_owned(),
                ))
            }
            kind => Err(self.fail(start, &format!("{kind} is not supported"))),
        }
    }
}

fn literal(inlines: &[Inline]) -> Option<&str> {
    inlines.iter().find_map(|inline| match inline {
        Inline::Text(text) => markup(text).then_some(text.as_str()),
        Inline::Code(_) => None,
        Inline::Strong(inner) | Inline::Em(inner) | Inline::Link(inner, _) => literal(inner),
    })
}

fn markup(text: &str) -> bool {
    let bytes = text.as_bytes();
    let word = |at: Option<&u8>| at.is_some_and(u8::is_ascii_alphanumeric);
    text.contains(['`', '*', '\\', '<', '&'])
        || ["~~", "](", "][", "!["]
            .iter()
            .any(|pair| text.contains(pair))
        || bytes.iter().enumerate().any(|(at, &byte)| {
            byte == b'_' && !(at > 0 && word(bytes.get(at - 1)) && word(bytes.get(at + 1)))
        })
}

fn text(out: &mut Vec<Inline>, more: &str) {
    if let Some(Inline::Text(last)) = out.last_mut() {
        last.push_str(more);
    } else {
        out.push(Inline::Text(more.to_owned()));
    }
}

fn unwrap(inlines: Vec<Inline>, newline: &str) -> Option<Vec<Inline>> {
    inlines
        .into_iter()
        .map(|inline| match inline {
            Inline::Text(text) => {
                let flat = text.replace(newline, " ");
                (!flat.contains('\n')).then_some(Inline::Text(flat))
            }
            Inline::Code(code) => (!code.contains('\n')).then_some(Inline::Code(code)),
            Inline::Strong(inner) => unwrap(inner, newline).map(Inline::Strong),
            Inline::Em(inner) => unwrap(inner, newline).map(Inline::Em),
            Inline::Link(inner, url) => {
                let inner = unwrap(inner, newline)?;
                (!url.contains('\n')).then_some(Inline::Link(inner, url))
            }
        })
        .collect()
}

enum Piece {
    Text(String),
    Space,
}

fn pieces_of(inlines: &[Inline], out: &mut Vec<Piece>) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => {
                for (index, word) in text.split(' ').enumerate() {
                    if index > 0 {
                        out.push(Piece::Space);
                    }
                    out.push(Piece::Text(word.to_owned()));
                }
            }
            Inline::Code(code) => out.push(Piece::Text(format!("`{code}`"))),
            Inline::Strong(inner) => {
                out.push(Piece::Text("**".into()));
                pieces_of(inner, out);
                out.push(Piece::Text("**".into()));
            }
            Inline::Em(inner) => {
                out.push(Piece::Text("*".into()));
                pieces_of(inner, out);
                out.push(Piece::Text("*".into()));
            }
            Inline::Link(inner, url) => {
                out.push(Piece::Text("[".into()));
                pieces_of(inner, out);
                out.push(Piece::Text(format!("]({url})")));
            }
        }
    }
}

fn fill(pieces: &[Piece], indent: usize, column: usize) -> String {
    let mut out = String::new();
    let mut column = column;
    for (index, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(text) => {
                out.push_str(text);
                column += text.len();
            }
            Piece::Space => {
                let next: usize = pieces[index + 1..]
                    .iter()
                    .take_while(|piece| matches!(piece, Piece::Text(_)))
                    .map(|piece| match piece {
                        Piece::Text(text) => text.len(),
                        Piece::Space => 0,
                    })
                    .sum();
                if column + 1 + next <= WIDTH {
                    out.push(' ');
                    column += 1;
                } else {
                    out.push('\n');
                    out.push_str(&" ".repeat(indent));
                    column = indent;
                }
            }
        }
    }
    out
}
