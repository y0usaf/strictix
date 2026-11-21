use std::collections::HashMap;
use std::sync::LazyLock;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{AstNode, AttrName, Expr, SelectExpr, SyntaxKind, TextRange, WithExpr};

use crate::advanced_rules::{with_removal_fix, UnsafeWithShadowing};
use crate::builtin_value_rules::literal_string;
use crate::config_text_generator::is_ident;
use crate::lib_helpers::unparen;

const CODE: &str = "deprecated-package-alias";
const SETS: [&str; 3] = ["pkgs", "final", "prev"];
const DATA: &str = include_str!("../data/nixpkgs-renames.tsv");

struct Renames {
    map: HashMap<&'static str, &'static str>,
    longest: usize,
}

static RENAMES: LazyLock<Renames> = LazyLock::new(|| {
    let map: HashMap<&'static str, &'static str> = DATA
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect();
    let longest = map
        .keys()
        .map(|old| old.split('.').count())
        .max()
        .unwrap_or(0);
    Renames { map, longest }
});

fn renames() -> &'static Renames {
    &RENAMES
}

fn lookup(segments: &[Option<String>]) -> Option<(String, &'static str)> {
    let parts: Option<Vec<&str>> = segments.iter().map(Option::as_deref).collect();
    let old = parts?.join(".");
    let new = renames().map.get(old.as_str())?;
    Some((old, new))
}

fn segment(name: AttrName<'_>, source: &str) -> (Option<String>, TextRange) {
    match name {
        AttrName::Ident(token) => (Some(token.text(source).to_owned()), token.range()),
        AttrName::Str(string) => (
            literal_string(string, source),
            string.syntax().content_range(),
        ),
        AttrName::Interp(interp) => (None, interp.syntax().content_range()),
    }
}

fn attr_segments(new: &str) -> String {
    new.split('.')
        .map(|part| {
            if is_ident(part) {
                part.to_owned()
            } else {
                format!(
                    "\"{}\"",
                    part.replace('\\', "\\\\")
                        .replace('"', "\\\"")
                        .replace('$', "\\$")
                )
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn head(new: &str) -> &str {
    new.split('.').next().unwrap_or(new)
}

fn with_namespace<'a>(model: &SemanticModel<'a>, scope_range: TextRange) -> Option<Expr<'a>> {
    let mut found = None;
    model.root().walk_containing(scope_range.start(), |node| {
        match WithExpr::cast(node).filter(|with| with.range() == scope_range) {
            Some(with) => {
                found = with.scope();
                true
            }
            None => false,
        }
    });
    found.and_then(unparen)
}

fn static_path(expr: Expr<'_>, source: &str) -> Option<(Vec<String>, Vec<TextRange>)> {
    match expr {
        Expr::Ident(token) => Some((vec![token.text(source).to_owned()], vec![token.range()])),
        Expr::Select(select) if select.default().is_none() => {
            let Some(Expr::Ident(base)) = select.base() else {
                return None;
            };
            let mut names = vec![base.text(source).to_owned()];
            let mut ranges = vec![base.range()];
            for element in select.attrpath()?.elements() {
                let (name, range) = segment(element, source);
                names.push(name?);
                ranges.push(range);
            }
            Some((names, ranges))
        }
        _ => None,
    }
}

fn same_binding(model: &SemanticModel<'_>, name: &str, a: u32, b: u32) -> bool {
    match (
        model.resolve_lexical(name, a),
        model.resolve_lexical(name, b),
    ) {
        (Some(x), Some(y)) => x.name.range() == y.name.range(),
        _ => false,
    }
}

fn with_removed_first(model: &SemanticModel<'_>, config: &LintConfig, site: usize) -> bool {
    let rule = UnsafeWithShadowing;
    config.is_enabled(rule.code())
        && (rule.default_enabled() || config.enabled.iter().any(|code| code == rule.code()))
        && with_removal_fix(model, site).is_some()
}

pub struct DeprecatedPackageAlias;

impl DeprecatedPackageAlias {
    fn diagnostic(&self, old: &str, new: &str, range: TextRange) -> Diagnostic {
        Diagnostic::new(
            CODE,
            self.severity(),
            format!("`{old}` is a deprecated nixpkgs alias of `{new}`"),
            range,
        )
    }

    fn fixed(&self, old: &str, new: &str, range: TextRange, replacement: String) -> Diagnostic {
        self.diagnostic(old, new, range)
            .with_fix(Fix::new(format!("use `{new}`")).edit(range, replacement))
    }

    fn after_with_removal(&self, old: &str, new: &str, range: TextRange) -> Diagnostic {
        self.unfixed(
            old,
            new,
            range,
            format!("`unsafe-with-shadowing` removes the `with` this name comes from first; the next fix pass renames it to `{new}`"),
        )
    }

    fn unfixed(&self, old: &str, new: &str, range: TextRange, help: String) -> Diagnostic {
        self.diagnostic(old, new, range).with_help(help)
    }

    fn check_select(
        &self,
        model: &SemanticModel<'_>,
        config: &LintConfig,
        select: SelectExpr<'_>,
        sets: &[&str],
        diags: &mut Vec<Diagnostic>,
    ) {
        let source = model.source();
        let Some(path) = select.attrpath() else {
            return;
        };
        let elements: Vec<(Option<String>, TextRange)> = path
            .elements()
            .map(|element| segment(element, source))
            .collect();
        let names: Vec<Option<String>> = elements.iter().map(|(name, _)| name.clone()).collect();
        let longest = renames().longest;
        let base = select.base().and_then(unparen);
        if let Some(Expr::Ident(token)) = base {
            let base_name = token.text(source).to_owned();
            let start = token.range().start();
            for len in (1..longest.min(names.len() + 1)).rev() {
                let mut key = vec![Some(base_name.clone())];
                key.extend_from_slice(&names[..len]);
                let Some((old, new)) = lookup(&key) else {
                    continue;
                };
                let range = TextRange::new(start, elements[len - 1].1.end());
                diags.push(self.free_head(model, config, &old, new, range, sets));
                return;
            }
        }
        let mut at = 0;
        while at < names.len() {
            let mut matched = None;
            for len in (1..=longest.min(names.len() - at)).rev() {
                if len == 1
                    && !(at == 0
                        && matches!(base, Some(Expr::Ident(token)) if sets.contains(&token.text(source))))
                {
                    continue;
                }
                if let Some((old, new)) = lookup(&names[at..at + len]) {
                    matched = Some((len, old, new));
                    break;
                }
            }
            let Some((len, old, new)) = matched else {
                at += 1;
                continue;
            };
            let range = TextRange::new(elements[at].1.start(), elements[at + len - 1].1.end());
            diags.push(self.fixed(&old, new, range, attr_segments(new)));
            at += len;
        }
    }

    fn free_head(
        &self,
        model: &SemanticModel<'_>,
        config: &LintConfig,
        old: &str,
        new: &str,
        range: TextRange,
        sets: &[&str],
    ) -> Diagnostic {
        let offset = range.start();
        let base = head(old);
        if model.resolve_lexical(base, offset).is_some() {
            return self.unfixed(
                old,
                new,
                range,
                format!("`{base}` is bound here; take `{}` the way `{base}` arrives and use it in place of `{old}`", head(new)),
            );
        }
        let Some(reference) =
            model.reference_at(TextRange::new(offset, offset + base.len() as u32))
        else {
            return self.unfixed(old, new, range, format!("write `{new}`"));
        };
        let Some(innermost) = reference.via_with else {
            return self.unfixed(old, new, range, format!("write `{new}`"));
        };
        let withs = model
            .with_sites()
            .iter()
            .filter(|site| site.body_range.contains(offset))
            .count();
        let site = &model.with_sites()[innermost];
        let named_set = matches!(
            with_namespace(model, site.scope_range),
            Some(Expr::Ident(token)) if sets.contains(&token.text(model.source()))
        );
        if !is_ident(head(new)) || model.resolve_lexical(head(new), offset).is_some() {
            return self.unfixed(
                old,
                new,
                range,
                format!(
                    "`{}` would not reach the package set here; write `{new}` from it",
                    head(new)
                ),
            );
        }
        if with_removed_first(model, config, innermost) {
            self.after_with_removal(old, new, range)
        } else if withs == 1 || named_set {
            self.fixed(old, new, range, new.to_owned())
        } else {
            self.unfixed(
                old,
                new,
                range,
                format!("`{base}` comes from one of {withs} nested `with` scopes; write `{new}` from the package set it names"),
            )
        }
    }

    fn check_reference(
        &self,
        model: &SemanticModel<'_>,
        config: &LintConfig,
        token: &strictix_syntax::SyntaxToken,
        innermost: usize,
        sets: &[&str],
        diags: &mut Vec<Diagnostic>,
    ) {
        let source = model.source();
        let name = token.text(source);
        let offset = token.range().start();
        let site = &model.with_sites()[innermost];
        let Some(namespace) = with_namespace(model, site.scope_range) else {
            return;
        };
        let deferred = with_removed_first(model, config, innermost);
        if let Expr::Ident(set) = namespace {
            if sets.contains(&set.text(source)) {
                let Some(new) = renames().map.get(name) else {
                    return;
                };
                if deferred {
                    diags.push(self.after_with_removal(name, new, token.range()));
                } else if is_ident(head(new)) && model.resolve_lexical(head(new), offset).is_none()
                {
                    diags.push(self.fixed(name, new, token.range(), (*new).to_owned()));
                } else {
                    diags.push(self.unfixed(
                        name,
                        new,
                        token.range(),
                        format!(
                            "`{}` would not reach `{}` here; write `{}.{}`",
                            head(new),
                            set.text(source),
                            set.text(source),
                            attr_segments(new)
                        ),
                    ));
                }
                return;
            }
        }
        let Some((path, ranges)) = static_path(namespace, source) else {
            return;
        };
        for from in 0..path.len() {
            let mut key: Vec<Option<String>> = path[from..].iter().cloned().map(Some).collect();
            key.push(Some(name.to_owned()));
            if key.len() < 2 {
                continue;
            }
            let Some((old, new)) = lookup(&key) else {
                continue;
            };
            let namespace_start = ranges[0].start();
            let prefix_end = if from == 0 {
                None
            } else {
                Some(ranges[from - 1].end())
            };
            match prefix_end {
                Some(_) if deferred => {
                    diags.push(self.after_with_removal(&old, new, token.range()));
                }
                Some(end) if same_binding(model, &path[0], namespace_start, offset) => {
                    let prefix = &source[namespace_start as usize..end as usize];
                    diags.push(self.fixed(
                        &old,
                        new,
                        token.range(),
                        format!("{prefix}.{}", attr_segments(new)),
                    ));
                }
                _ => diags.push(self.unfixed(
                    &old,
                    new,
                    token.range(),
                    format!("`{name}` comes from `with {}`; write `{new}` from the package set that holds it", &source[namespace_start as usize..ranges[ranges.len() - 1].end() as usize]),
                )),
            }
            return;
        }
    }
}

impl Rule for DeprecatedPackageAlias {
    fn code(&self) -> &'static str {
        CODE
    }

    fn name(&self) -> &'static str {
        "Deprecated package alias"
    }

    fn description(&self) -> &'static str {
        "Flags a nixpkgs attribute that nixpkgs keeps only as a warning alias for a renamed package, such as `xorg.libX11` for `libx11`, which makes every evaluation that reaches it print `evaluation warning: The xorg package set has been deprecated`. The renames come from the `warnAlias`/`warnOnInstantiate` definitions in nixpkgs' `pkgs/top-level/aliases.nix`, read by `nix run .#nixpkgs-renames` from the nixpkgs strictix is locked to; an alias whose message is not a plain `'OLD' has been renamed to 'NEW'` is left alone. A path such as `xorg.libX11` is matched inside any selection (`pkgs.xorg.libX11.dev`, `final.xorg.libX11`) and as `with pkgs.xorg; libX11`; a one-segment alias (`swww`) only after a name in the `sets` setting (default `pkgs`, `final`, `prev`) or inside `with` one of them. Fix rewrites the selection, `xorg.libX11` under a single `with` (or under `with` a `sets` name) to `libx11`, and `libX11` under `with pkgs.xorg;` to `pkgs.libx11`; it is withheld when the new name is shadowed by a lexical binding, when nested `with` scopes make the resolution unclear, and when `xorg` is a binding of its own (an argument or `let`). A `with` or argument left unused is for `redundant-with` and `unused-formal`."
    }

    fn severity(&self) -> Severity {
        Severity::Warning
    }

    fn check_file(&self, model: &SemanticModel, config: &LintConfig, diags: &mut Vec<Diagnostic>) {
        let configured: Vec<&str> = config.setting_list(CODE, "sets").collect();
        let sets: &[&str] = if configured.is_empty() {
            &SETS
        } else {
            &configured
        };
        let start = diags.len();
        for node in model.root().descendants() {
            if node.kind() != SyntaxKind::SelectExpr {
                continue;
            }
            if let Some(select) = SelectExpr::cast(node) {
                self.check_select(model, config, select, sets, diags);
            }
        }
        let covered: Vec<TextRange> = diags[start..].iter().map(|diag| diag.range).collect();
        for reference in model.references() {
            if reference.resolved.is_some() {
                continue;
            }
            let Some(innermost) = reference.via_with else {
                continue;
            };
            let range = reference.name.range();
            if covered
                .iter()
                .any(|done| done.start() <= range.start() && range.end() <= done.end())
            {
                continue;
            }
            self.check_reference(model, config, reference.name, innermost, sets, diags);
        }
    }
}
