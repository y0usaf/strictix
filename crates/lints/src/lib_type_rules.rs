//! `lib.types.*` lints: hallucinated or removed module option types.
//!
//! AI-generated NixOS modules routinely invent option types:
//! `types.string` (removed years ago), `types.list` (never existed;
//! `listOf` did), `types.function` (it is `functionTo`). The evaluator
//! only reports these deep inside module evaluation, so catching them
//! at lint time is a large quality-of-life win. The rule fires only
//! when the `lib`/`types` ident provably refers to the nixpkgs lib —
//! a `lib` bound by `let` or `rec` could be anything, so it is skipped.

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

/// Every attribute of nixpkgs `lib.types`, verbatim from
/// `nix eval nixpkgs#lib.types --apply builtins.attrNames` (2026-08-25).
/// Underscore-free by accident, not by filtering: everything nix
/// reports is a valid member by definition, so the list is unedited.
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

/// Every attribute of `lib.types.ints`, verbatim from
/// `nix eval nixpkgs#lib.types.ints --apply builtins.attrNames`
/// (2026-08-25). AI loves inventing widths (`ints.u42`), so the second
/// hop under `ints` is validated too.
const VALID_INTS_MEMBERS: &[&str] = &[
    "between", "positive", "s16", "s32", "s8", "u16", "u32", "u8", "unsigned",
];

/// Every attribute of `lib.types.numbers`, verbatim from
/// `nix eval nixpkgs#lib.types.numbers --apply builtins.attrNames`
/// (2026-08-25).
const VALID_NUMBERS_MEMBERS: &[&str] = &["between", "nonnegative", "positive"];

/// Curated help for the classic hallucinations — only names ABSENT
/// from the real member list get a special message (e.g. `attrs` DOES
/// exist, so it never reaches here).
fn curated_help(name: &str) -> Option<&'static str> {
    match name {
        "string" => Some("types.string was removed; use types.str"),
        "list" => Some("use types.listOf <elem>"),
        "function" => Some("use types.functionTo <return>"),
        _ => None,
    }
}

/// The ident segments selected off `ref_range`, innermost-out across
/// the (usually single-node) select chain — the same chain walk as the
/// schema rule uses for `config` paths. Collection stops at the first
/// non-ident segment (a string or interpolation makes the rest of the
/// path statically unknowable), and the whole chain is abandoned when
/// any link carries an `or` default: `types.foo or bar` is a probe
/// with a fallback, not a hard member access, so it never fires —
/// mirroring `UnknownBuiltin`'s `builtins ? attr` silence.
fn member_segments<'a>(model: &SemanticModel<'a>, ref_range: TextRange) -> Vec<&'a SyntaxToken> {
    let root = model.root();
    let mut segments = Vec::new();
    let Some(innermost) = root
        .descendants()
        .filter_map(SelectExpr::cast)
        .find(|s| matches!(s.base(), Some(Expr::Ident(t)) if t.range() == ref_range))
    else {
        return segments;
    };
    let mut chain = vec![innermost];
    let mut current_range = innermost.range();
    while let Some(next) = root
        .descendants()
        .filter_map(SelectExpr::cast)
        .find(|s| s.base().is_some_and(|b| b.range() == current_range))
    {
        current_range = next.range();
        chain.push(next);
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

/// Whether a binding could be the nixpkgs `lib`: a module/function
/// formal (`{ lib, ... }:` or `lib:`). A `let`/`rec`/`inherit`/`@`
/// binding could be a custom lib (`let lib = import ./mine.nix;`), so
/// only formals count as provable.
fn is_formal(binding: &Binding<'_>) -> bool {
    binding.kind == BindingKind::LambdaParam
}

/// Whether an expression is the bare ident `lib` referring provably to
/// the nixpkgs lib: a formal binding named `lib`, or nothing lexically
/// (top-level `lib` in a file that will receive it as scope). A
/// `let`-bound `lib` fails this test on purpose.
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

/// The `inherit` statement that introduced `binding` (an InheritName),
/// found by locating the InheritStmt whose name list contains the
/// binding's defining token. The tree has no parent pointers, so this
/// is a range-identity search, same as the chain walks elsewhere.
fn inherit_stmt_of<'a>(
    model: &SemanticModel<'a>,
    binding: &Binding<'a>,
) -> Option<InheritStmt<'a>> {
    let name_range = binding.name.range();
    model
        .root()
        .descendants()
        .filter_map(InheritStmt::cast)
        .find(|stmt| stmt.names().any(|t| t.range() == name_range))
}

/// Flags `lib.types.<name>` members that do not exist.
///
/// Fires on three provable shapes:
/// - `lib.types.X` where `lib` is a formal parameter or unbound;
/// - `types.X` where `types` came from `inherit (lib) types` (which
///   binds the whole `lib.types` set — `inherit (lib.types) str`
///   binds individual members, a different shape, and is ignored);
/// - `types.X` under `with lib;` where `lib` is a formal or unbound.
///
/// Everything else — `let lib = ...`, dynamic segments, `or` defaults,
/// `? attr` probes (whose right side is never a reference) — is silent.
pub struct UnknownLibType;

impl UnknownLibType {
    /// Validate the member hops in `segments` (the path after
    /// `lib.types.` / `types.`): the first hop against the top-level
    /// member list, and — only under `ints`/`numbers` — the second hop
    /// against the nested list. Deeper hops are type internals
    /// (`functor`, `check`, ...) and are not this rule's business.
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
                // The one safe fix: `str` is the direct replacement
                // for the removed `string`, and the edit is a single
                // ident token, so the splice cannot touch trivia.
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

    /// Whether a `types` reference provably means nixpkgs `lib.types`:
    /// either it resolves to a binding introduced by `inherit (lib)
    /// types` whose source `lib` is a formal/unbound, or it resolves
    /// to nothing and the innermost covering `with` has the bare
    /// subject `lib` (again a formal/unbound).
    ///
    /// The binding *kind* is deliberately not inspected: the model
    /// tags an inherit name with the kind of its enclosing construct
    /// (`LetBinding` in a `let`, `RecAttr` in a `rec`, `InheritName`
    /// only in a bare attrset). What proves the shape is the
    /// `InheritStmt` that owns the defining token.
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
                // Plain `inherit types;` re-binds an outer `types`;
                // nothing provable about it.
                return false;
            };
            return is_nixpkgs_lib_expr(model, &source_expr);
        }
        let Some(with_idx) = reference.via_with else {
            return false;
        };
        let site = &model.with_sites()[with_idx];
        let Some(with_expr) = model
            .root()
            .descendants()
            .filter_map(WithExpr::cast)
            .find(|w| w.range() == site.scope_range)
        else {
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

/// Every provable `lib.types` member access in the file: the reference
/// token the access starts at (`lib` or `types`) and the member hops
/// after `types`. Shared by [UnknownLibType] and [DiscouragedLibType].
fn lib_type_members<'a>(model: &SemanticModel<'a>) -> Vec<(&'a SyntaxToken, Vec<&'a SyntaxToken>)> {
    let source = model.source();
    let mut out = Vec::new();
    for reference in model.references() {
        let name = reference.name.text(source);
        if name == "lib" {
            // Only a formal or lexically-unbound `lib` provably
            // means nixpkgs lib; a let/rec/inherit binding could
            // be a custom lib, so it stays silent.
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

/// Flags `lib.types` members that exist but lose definitions:
/// `types.attrs` merges shallowly (later definitions silently replace
/// earlier nested keys, and `mkIf`/`mkDefault` inside are not
/// discharged) and `types.unspecified` has no real merge semantics.
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
                continue; // deeper hops are type internals, not uses of the type
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
