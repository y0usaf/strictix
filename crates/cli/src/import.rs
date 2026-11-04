use std::path::Path;

use strictix_syntax::{AstNode, Expr, ListExpr, Root, StringPart};

pub fn run(language: &str, path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path.extension().is_some_and(|extension| extension == "nix") {
        nix_file(language, &text)
    } else {
        strictix_import::data_file(language, &text, &[])
    }
}

fn unparen(mut expr: Expr<'_>) -> Expr<'_> {
    while let Expr::Paren(paren) = expr {
        match paren.expr() {
            Some(inner) => expr = inner,
            None => break,
        }
    }
    expr
}

fn nix_file(language: &str, source: &str) -> Result<String, String> {
    let tree = strictix_syntax::parse(source);
    if tree.error_nodes().next().is_some() {
        return Err("the file does not parse as Nix".into());
    }
    let root = Root::cast(&tree)
        .and_then(|root| root.expr())
        .ok_or("the file holds no expression")?;
    if let Expr::List(list) = unparen(root) {
        let (text, holes) = lines(list, source)?;
        return strictix_import::data_file(language, &text, &holes);
    }
    let calls: Vec<_> = tree
        .descendants()
        .filter_map(|node| match Expr::cast(node) {
            Some(Expr::Apply(apply)) => Some(apply),
            _ => None,
        })
        .filter(|apply| {
            let generator = apply.func().map_or("", |func| func.content_text(source));
            generator.starts_with("generators.")
                && generator.ends_with("Source")
                && matches!(apply.arg().map(unparen), Some(Expr::List(_)))
        })
        .collect();
    let [call] = calls.as_slice() else {
        return Err(
            "expected a line list, or exactly one `generators.<name>Source [ ... ]` call".into(),
        );
    };
    let Some(Expr::List(list)) = call.arg().map(unparen) else {
        return Err("the call's argument is not a list".into());
    };
    let (text, holes) = lines(list, source)?;
    let expression = strictix_import::expression(language, &text, &holes)?;
    let names = strictix_import::constructors(&expression);
    let tree = if strictix_import::bound(language) {
        format!("(({expression}) generators.{language})")
    } else if names.is_empty() {
        expression
    } else {
        format!(
            "(let inherit (generators.{language}) {}; in {expression})",
            names.join(" ")
        )
    };
    let range = call.syntax().content_range();
    Ok(format!(
        "{}generators.{language}.render {tree}{}",
        &source[..range.start() as usize],
        &source[range.end() as usize..]
    ))
}

fn lines(list: ListExpr<'_>, source: &str) -> Result<(String, Vec<String>), String> {
    let mut text = String::new();
    let mut holes = Vec::new();
    for item in list.items() {
        let Expr::String(string) = unparen(item) else {
            return Err(format!(
                "`{}` is not a \"...\" string line",
                item.content_text(source)
            ));
        };
        for part in string.parts() {
            match part {
                StringPart::Content(token) => text.push_str(&unescape(token.text(source))),
                StringPart::Interp(interp) => {
                    let expr = interp.expr().ok_or("an empty interpolation")?;
                    let index = u32::try_from(holes.len()).map_err(|e| e.to_string())?;
                    let hole = char::from_u32(strictix_import::HOLE_BASE + index)
                        .ok_or("too many interpolations")?;
                    holes.push(expr.content_text(source).to_owned());
                    text.push(hole);
                }
            }
        }
        text.push('\n');
    }
    Ok((text, holes))
}

fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
