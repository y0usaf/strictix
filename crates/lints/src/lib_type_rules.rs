use strictix_core::{
    config::LintConfig,
    diagnostic::{Diagnostic, Severity},
    fix::Fix,
    rules::Rule,
    semantic::{Binding, BindingKind, SemanticModel},
};
use strictix_syntax::{
    AstNode, AttrName, Expr, InheritStmt, SelectExpr, SyntaxToken, TextRange, WithExpr,
};

const VALID_LIB_TYPES: &[&str] = &[
    "addCheck",
    "anything",
    "attrListOf",
    "attrListWith",
    "attrTag",
    "attrs",
    "attrsOf",
    "attrsWith",
    "bool",
    "boolByOr",
    "coercedTo",
    "commas",
    "defaultFunctor",
    "defaultTypeMerge",
    "deferredModule",
    "deferredModuleWith",
    "either",
    "enum",
    "envVar",
    "externalPath",
    "fileset",
    "float",
    "functionTo",
    "int",
    "ints",
    "isOptionType",
    "isType",
    "json",
    "lazyAttrsOf",
    "lines",
    "listOf",
    "loaOf",
    "luaInline",
    "mergeTypes",
    "mkOptionType",
    "noCheckForDocsModule",
    "nonEmptyListOf",
    "nonEmptyStr",
    "nullOr",
    "number",
    "numbers",
    "oneOf",
    "optionDeclaration",
    "optionDescriptionPhrase",
    "optionType",
    "package",
    "passwdEntry",
    "path",
    "pathInStore",
    "pathWith",
    "pkgs",
    "port",
    "raw",
    "separatedString",
    "serializableValueWith",
    "setType",
    "shellPackage",
    "singleLineStr",
    "str",
    "strMatching",
    "submodule",
    "submoduleWith",
    "toml",
    "types",
    "uniq",
    "unique",
    "unspecified",
];

const VALID_INTS_MEMBERS: &[&str] = &[
    "between", "positive", "s16", "s32", "s8", "u16", "u32", "u8", "unsigned",
];

const VALID_NUMBERS_MEMBERS: &[&str] = &["between", "nonnegative", "positive"];

fn curated_help(name: &str) -> Option<&'static str> {
    match name {
        "string" => Some("types.string was removed; use types.str"),
        "list" => Some("use types.listOf <elem>"),
        "function" => Some("use types.functionTo <return>"),
        _ => None,
    }
}

fn member_segments<'a>(model: &SemanticModel<'a>, ref_range: TextRange) -> Vec<&'a SyntaxToken> {
    let root = model.root();
    let mut segments = Vec::new();
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
        return segments;
    };
    let mut chain = vec![path_selects[start_pos]];
    let mut current_range = path_selects[start_pos].range();
    let mut pos = start_pos;
    while pos > 0 {
        pos -= 1;
        let next = path_selects[pos];
        if next.base().is_some_and(|b| b.range() == current_range) {
            current_range = next.range();
            chain.push(next);
        }
    }
    for select in &chain {
        if select.default().is_some() {
            segments.clear();
            return segments;
        }
    }
    'outer: for select in &chain {
        let Some(attrpath) = select.attrpath() else {
            break;
        };
        for element in attrpath.elements() {
            let AttrName::Ident(token) = element else {
                break 'outer;
            };
            segments.push(token);
        }
    }
    segments
}

fn is_formal(binding: &Binding<'_>) -> bool {
    binding.kind == BindingKind::LambdaParam
}

fn is_nixpkgs_lib_expr(model: &SemanticModel<'_>, expr: &Expr<'_>) -> bool {
    let Expr::Ident(token) = expr else {
        return false;
    };
    if token.text(model.source()) != "lib" {
        return false;
    }
    match model.resolve(token) {
        Some(binding) => is_formal(binding),
        None => true,
    }
}

fn inherit_stmt_of<'a>(
    model: &SemanticModel<'a>,
    binding: &Binding<'a>,
) -> Option<InheritStmt<'a>> {
    let name_range = binding.name.range();
    let mut found = None;
    model.root().walk_containing(name_range.start(), |node| {
        if let Some(stmt) =
            InheritStmt::cast(node).filter(|stmt| stmt.names().any(|t| t.range() == name_range))
        {
            found = Some(stmt);
            true
        } else {
            false
        }
    });
    found
}

pub struct UnknownLibType;

impl UnknownLibType {
        fn check_members(&self, source: &str, segments: &[&SyntaxToken], diags: &mut Vec<Diagnostic>) {
        let Some(first) = segments.first() else {
            return;
        };
        let name = first.text(source);
        if !VALID_LIB_TYPES.contains(&name) {
            let mut diag = Diagnostic::new(
                self.code(),
                self.severity(),
                format!("'{name}' is not a lib.types member"),
                first.range(),
            );
            if let Some(help) = curated_help(name) {
                diag = diag.with_help(help);
            }
            if name == "string" {
                diag = diag.with_fix(
                    Fix::new("replace types.string with types.str").edit(first.range(), "str"),
                );
            }
            diags.push(diag);
            return;
        }
        let nested: &[&str] = match name {
            "ints" => VALID_INTS_MEMBERS,
            "numbers" => VALID_NUMBERS_MEMBERS,
            _ => return,
        };
        let Some(second) = segments.get(1) else {
            return;
        };
        let member = second.text(source);
        if !nested.contains(&member) {
            diags.push(
                Diagnostic::new(
                    self.code(),
                    self.severity(),
                    format!("'{member}' is not a lib.types.{name} member"),
                    second.range(),
                )
                .with_help(format!(
                    "valid lib.types.{name} members: {}",
                    nested.join(", ")
                )),
            );
        }
    }

        fn types_is_lib_types(
        &self,
        model: &SemanticModel<'_>,
        reference: &strictix_core::semantic::Reference<'_>,
    ) -> bool {
        if let Some(idx) = reference.resolved {
            let binding = &model.bindings()[idx];
            let Some(stmt) = inherit_stmt_of(model, binding) else {
                return false;
            };
            let Some(source_expr) = stmt.source() else {
                return false;
            };
            return is_nixpkgs_lib_expr(model, &source_expr);
        }
        let Some(with_idx) = reference.via_with else {
            return false;
        };
        let site = &model.with_sites()[with_idx];
        let mut with_expr = None;
        model.root().walk_containing(site.scope_range.start(), |node| {
            if let Some(w) = WithExpr::cast(node).filter(|w| w.range() == site.scope_range) {
                with_expr = Some(w);
                true
            } else {
                false
            }
        });
        let Some(with_expr) = with_expr else {
            return false;
        };
        with_expr
            .scope()
            .is_some_and(|subject| is_nixpkgs_lib_expr(model, &subject))
    }
}

impl Rule for UnknownLibType {
    fn code(&self) -> &'static str {
        "unknown-lib-type"
    }

    fn name(&self) -> &'static str {
        "Unknown lib.types member"
    }

    fn description(&self) -> &'static str {
        "Flags `lib.types.<name>` where <name> is not a real module-system type: removed ones like types.string, never-existed ones like types.list, and hallucinated members. Fires only when lib/types provably refer to the nixpkgs lib."
    }

    fn severity(&self) -> Severity {
        Severity::Error
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        for (_, members) in lib_type_members(model) {
            self.check_members(model.source(), &members, diags);
        }
    }
}

fn lib_type_members<'a>(model: &SemanticModel<'a>) -> Vec<(&'a SyntaxToken, Vec<&'a SyntaxToken>)> {
    let source = model.source();
    let mut out = Vec::new();
    for reference in model.references() {
        let name = reference.name.text(source);
        if name == "lib" {
            match reference.resolved.map(|i| &model.bindings()[i]) {
                Some(binding) if !is_formal(binding) => continue,
                _ => {}
            }
            let segments = member_segments(model, reference.name.range());
            let Some((first, members)) = segments.split_first() else {
                continue;
            };
            if first.text(source) != "types" {
                continue;
            }
            out.push((reference.name, members.to_vec()));
        } else if name == "types" {
            if !UnknownLibType.types_is_lib_types(model, reference) {
                continue;
            }
            out.push((
                reference.name,
                member_segments(model, reference.name.range()),
            ));
        }
    }
    out
}

pub struct DiscouragedLibType;

impl Rule for DiscouragedLibType {
    fn code(&self) -> &'static str {
        "discouraged-lib-type"
    }

    fn name(&self) -> &'static str {
        "Discouraged lib.types member"
    }

    fn description(&self) -> &'static str {
        "Flags `types.attrs` and `types.unspecified`. `attrs` merges definitions shallowly and does not discharge `mkIf`/`mkDefault` inside them; nixpkgs recommends `types.attrsOf types.anything`. `unspecified` has no real merge semantics; prefer `types.anything` or a precise type."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, _config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let source = model.source();
        for (start, members) in lib_type_members(model) {
            let [member] = members.as_slice() else {
                continue;
            };
            match member.text(source) {
                "attrs" => {
                    let range = TextRange::new(start.range().start(), member.range().end());
                    let text = &source[range.start() as usize..range.end() as usize];
                    let prefix = text.strip_suffix("attrs").unwrap_or(text);
                    diags.push(
                        Diagnostic::new(
                            self.code(),
                            self.severity(),
                            "types.attrs merges definitions shallowly",
                            range,
                        )
                        .with_help("use types.attrsOf types.anything")
                        .with_fix(
                            Fix::new("replace with attrsOf anything")
                                .edit(range, format!("({prefix}attrsOf {prefix}anything)")),
                        ),
                    );
                }
                "unspecified" => diags.push(
                    Diagnostic::new(
                        self.code(),
                        self.severity(),
                        "types.unspecified has no real merge semantics",
                        member.range(),
                    )
                    .with_help("use types.anything or a precise type"),
                ),
                _ => {}
            }
        }
    }
}
