mod glsl;
mod lisp;
mod nix;
mod shell;

pub use nix::HOLE_BASE;

pub const LANGUAGES: [&str; 3] = ["glsl", "lisp", "shell"];

pub fn expression(language: &str, source: &str, holes: &[String]) -> Result<String, String> {
    match language {
        "glsl" => glsl::import(source, holes),
        "lisp" => lisp::import(source, holes),
        "shell" => shell::import(source, holes),
        other => Err(format!(
            "no importer for {other}; known: {}",
            LANGUAGES.join(", ")
        )),
    }
}

pub fn data_file(language: &str, source: &str, holes: &[String]) -> Result<String, String> {
    let expression = expression(language, source, holes)?;
    let names = constructors(&expression);
    let formals = if names.is_empty() {
        "_".to_owned()
    } else {
        let names: String = names.iter().map(|name| format!("  {name},\n")).collect();
        format!("{{\n{names}  ...\n}}")
    };
    Ok(format!("{formals}: {expression}\n"))
}

pub fn constructors(expression: &str) -> Vec<String> {
    nix::identifiers(expression).into_iter().collect()
}

fn parse(language: &tree_sitter::Language, source: &str) -> Result<tree_sitter::Tree, String> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(language).map_err(|e| e.to_string())?;
    let tree = parser.parse(source, None).ok_or("the parser gave up")?;
    if let Some(bad) = first_error(tree.root_node()) {
        let at = bad.start_position().row + 1;
        return Err(format!("line {at}: the grammar does not parse this source"));
    }
    Ok(tree)
}

fn first_error(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    if !node.has_error() {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<_> = node.children(&mut cursor).collect();
    children.into_iter().find_map(first_error)
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

fn column_of(source: &str, offset: usize) -> usize {
    offset - source[..offset].rfind('\n').map_or(0, |at| at + 1)
}
