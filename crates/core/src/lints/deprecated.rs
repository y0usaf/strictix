use crate::{Metadata, Report, Rule, Suggestion};

use macros::lint;
use rnix::{
    NodeOrToken, SyntaxElement, SyntaxKind,
    ast::{Apply, Attr, Expr, Select},
};
use rowan::ast::AstNode as _;

/// ## What it does
/// Checks for deprecated Nix and nixpkgs APIs that already emit
/// evaluation-time warnings.
///
/// ## Why is this bad?
/// Deprecated APIs are noisy at evaluation time and will eventually be
/// removed.
///
/// ## Example
///
/// ```nix
/// builtins.toPath "/path"
/// pkgs.system
/// lib.nixpkgsVersion
/// mkAliasOptionModuleMD old new
/// ```
///
/// Prefer the modern equivalents instead:
///
/// ```nix
/// let __strictix_to_path_arg = builtins.toString ("/path");
/// in if builtins.substring 0 1 __strictix_to_path_arg == "/"
///    then /. + __strictix_to_path_arg
///    else ./. + "/${__strictix_to_path_arg}"
/// pkgs.stdenv.hostPlatform.system
/// lib.version
/// mkAliasOptionModule old new
/// ```
#[lint(
    name = "deprecated",
    note = "Found usage of deprecated Nix feature",
    code = 17,
    match_with = [SyntaxKind::NODE_APPLY, SyntaxKind::NODE_SELECT]
)]
struct Deprecated;

impl Rule for Deprecated {
    fn validate(&self, node: &SyntaxElement) -> Option<Report> {
        let NodeOrToken::Node(node) = node else {
            return None;
        };

        if let Some(apply) = Apply::cast(node.clone()) {
            if let Some(report) = validate_apply(self, &apply) {
                return Some(report);
            }
        }

        if let Some(select) = Select::cast(node.clone()) {
            if let Some(report) = validate_select(self, &select) {
                return Some(report);
            }
        }

        None
    }
}

fn validate_select(lint: &Deprecated, select: &Select) -> Option<Report> {
    let path = expr_static_path(&Expr::Select(select.clone()))?;
    let at = select.syntax().text_range();

    let (message, replacement) = match path.as_str() {
        "lib.nixpkgsVersion" => (
            "`lib.nixpkgsVersion` is deprecated, use `lib.version` instead",
            Some("lib.version"),
        ),
        "lib.evalOptionValue" => (
            "External use of `lib.evalOptionValue` is deprecated",
            None,
        ),
        "lib.isInOldestRelease" => (
            "`lib.isInOldestRelease` is deprecated, use `lib.oldestSupportedReleaseIsAtLeast` instead",
            Some("lib.oldestSupportedReleaseIsAtLeast"),
        ),
        "lib.lists.fold" => (
            "`lib.lists.fold` is deprecated, use `lib.lists.foldr` instead",
            Some("lib.lists.foldr"),
        ),
        "lib.cli.toGNUCommandLine" => (
            "`lib.cli.toGNUCommandLine` is deprecated, use `lib.cli.toCommandLine` instead",
            Some("lib.cli.toCommandLine"),
        ),
        "lib.cli.toGNUCommandLineShell" => (
            "`lib.cli.toGNUCommandLineShell` is deprecated, use `lib.cli.toCommandLineShell` instead",
            Some("lib.cli.toCommandLineShell"),
        ),
        "pkgs.buildPlatform" => (
            "`pkgs.buildPlatform` is deprecated, use `pkgs.stdenv.buildPlatform` instead",
            Some("pkgs.stdenv.buildPlatform"),
        ),
        "pkgs.hostPlatform" => (
            "`pkgs.hostPlatform` is deprecated, use `pkgs.stdenv.hostPlatform` instead",
            Some("pkgs.stdenv.hostPlatform"),
        ),
        "pkgs.system" => (
            "`pkgs.system` is deprecated, use `pkgs.stdenv.hostPlatform.system` instead",
            Some("pkgs.stdenv.hostPlatform.system"),
        ),
        "pkgs.targetPlatform" => (
            "`pkgs.targetPlatform` is deprecated, use `pkgs.stdenv.targetPlatform` instead",
            Some("pkgs.stdenv.targetPlatform"),
        ),
        "pkgs.dontRecurseIntoAttrs" => (
            "`pkgs.dontRecurseIntoAttrs` is deprecated, use `pkgs.lib.dontRecurseIntoAttrs` instead",
            Some("pkgs.lib.dontRecurseIntoAttrs"),
        ),
        "pkgs.stringsWithDeps" => (
            "`pkgs.stringsWithDeps` is deprecated, use `pkgs.lib.stringsWithDeps` instead",
            Some("pkgs.lib.stringsWithDeps"),
        ),
        "pkgs.forceSystem" => (
            "`pkgs.forceSystem` is deprecated; import nixpkgs explicitly instead",
            None,
        ),
        _ => return None,
    };

    Some(match replacement {
        Some(replacement) => lint
            .report()
            .suggest(at, message, Suggestion::with_text(at, replacement)),
        None => lint.report().diagnostic(at, message),
    })
}

fn validate_apply(lint: &Deprecated, apply: &Apply) -> Option<Report> {
    let lambda = strip_parens(apply.lambda()?)?;
    let lambda_path = expr_static_path(&lambda)?;

    if matches!(lambda_path.as_str(), "builtins.toPath" | "toPath") {
        let at = apply.syntax().text_range();
        let message = format!(
            "`{lambda_path}` is deprecated, see `:doc builtins.toPath` within the REPL for more"
        );
        return Some(lint.report().suggest(
            at,
            message,
            Suggestion::with_text(at, to_path_replacement(apply)),
        ));
    }

    if lambda_path == "mkAliasOptionModuleMD" {
        let at = lambda.syntax().text_range();
        return Some(lint.report().suggest(
            at,
            "`mkAliasOptionModuleMD` is deprecated, use `mkAliasOptionModule` instead",
            Suggestion::with_text(at, "mkAliasOptionModule"),
        ));
    }

    if lambda_path == "mkAliasIfDef" {
        let at = apply.syntax().text_range();
        let argument = strip_parens(apply.argument()?)?;
        let replacement = format!("mkIf {}.isDefined", fmt_as_select_base(argument.syntax()));
        return Some(lint.report().suggest(
            at,
            "`mkAliasIfDef` is deprecated, use `mkIf option.isDefined` instead",
            Suggestion::with_text(at, replacement),
        ));
    }

    if lambda_path == "lib.mkFixStrictness" {
        let at = apply.syntax().text_range();
        let argument = apply.argument()?;
        return Some(lint.report().suggest(
            at,
            "`lib.mkFixStrictness` has no effect and can be removed",
            Suggestion::with_text(at, argument.syntax().to_string()),
        ));
    }

    None
}

fn strip_parens(expr: Expr) -> Option<Expr> {
    let mut current = expr;
    loop {
        match current {
            Expr::Paren(paren) => current = paren.expr()?,
            _ => return Some(current),
        }
    }
}

fn expr_static_path(expr: &Expr) -> Option<String> {
    match strip_parens(expr.clone())? {
        Expr::Ident(ident) => Some(ident.to_string()),
        Expr::Select(select) => {
            if select.or_token().is_some() {
                return None;
            }

            let mut path = expr_static_path(&select.expr()?)?;
            for attr in select.attrpath()?.attrs() {
                let Attr::Ident(ident) = attr else {
                    return None;
                };
                path.push('.');
                path.push_str(&ident.to_string());
            }
            Some(path)
        }
        _ => None,
    }
}

fn fmt_as_select_base(node: &rnix::SyntaxNode) -> String {
    let text = node.to_string();
    let needs_parens = !matches!(
        Expr::cast(node.clone()),
        Some(Expr::Ident(_) | Expr::Select(_) | Expr::Paren(_))
    );

    if needs_parens {
        format!("({text})")
    } else {
        text
    }
}

fn to_path_replacement(apply: &Apply) -> String {
    let apply_text = apply.syntax().to_string();
    let binding = fresh_name(&apply_text, "__strictix_to_path_arg");
    let argument = apply
        .argument()
        .map(|argument| argument.syntax().to_string())
        .unwrap_or_default();

    format!(
        "let {binding} = builtins.toString ({argument}); in if builtins.substring 0 1 {binding} == \"/\" then /. + {binding} else ./. + \"/${{{binding}}}\""
    )
}

fn fresh_name(haystack: &str, base: &str) -> String {
    let mut candidate = base.to_string();
    while haystack.contains(&candidate) {
        candidate.push('_');
    }
    candidate
}
