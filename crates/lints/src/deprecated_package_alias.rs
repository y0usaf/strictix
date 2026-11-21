use std::collections::HashMap;
use std::sync::LazyLock;

use strictix_core::config::LintConfig;
use strictix_core::diagnostic::{Diagnostic, Severity};
use strictix_core::fix::Fix;
use strictix_core::rules::Rule;
use strictix_core::semantic::SemanticModel;
use strictix_syntax::{
    AstNode, AttrName, Expr, SelectExpr, SyntaxKind, SyntaxToken, TextRange, WithExpr,
};

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

type Receiver<'a> = (&'a SyntaxToken, Vec<(Option<String>, TextRange)>);

fn receiver<'a>(expr: Expr<'a>, source: &str) -> Option<Receiver<'a>> {
    match expr {
        Expr::Ident(token) => Some((token, Vec::new())),
        Expr::Select(select) if select.default().is_none() => {
            let Some(Expr::Ident(base)) = select.base().and_then(unparen) else {
                return None;
            };
            let names = select
                .attrpath()?
                .elements()
                .map(|element| segment(element, source))
                .collect();
            Some((base, names))
        }
        _ => None,
    }
}

fn legacy_packages(before: &[Option<String>]) -> bool {
    before.len() >= 2 && before[before.len() - 2].as_deref() == Some("legacyPackages")
}

fn package_set(source: &str, sets: &[&str], head: &SyntaxToken, before: &[Option<String>]) -> bool {
    legacy_packages(before) || (before.is_empty() && sets.contains(&head.text(source)))
}

fn with_is_package_set(model: &SemanticModel<'_>, sets: &[&str], site: usize) -> bool {
    let source = model.source();
    with_namespace(model, model.with_sites()[site].scope_range)
        .and_then(|namespace| receiver(namespace, source))
        .is_some_and(|(head, names)| {
            let before: Vec<Option<String>> = names.into_iter().map(|(name, _)| name).collect();
            package_set(source, sets, head, &before)
        })
}

fn reaches_package_set(model: &SemanticModel<'_>, sets: &[&str], offset: u32) -> bool {
    model.with_sites().iter().enumerate().any(|(site, with)| {
        with.body_range.contains(offset) && with_is_package_set(model, sets, site)
    })
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

    fn unknown_receiver(
        &self,
        old: &str,
        new: &str,
        range: TextRange,
        receiver: &str,
    ) -> Diagnostic {
        self.unfixed(
            old,
            new,
            range,
            format!("`{receiver}` is not known to be nixpkgs; if it is, write `{new}` from it, or name it in the `sets` setting so fix does"),
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
        let written = select.base();
        let base = written.and_then(unparen);
        if let Some(Expr::Ident(token)) = written {
            let base_name = token.text(source).to_owned();
            let start = token.range().start();
            for len in (1..longest.min(names.len() + 1)).rev() {
                let mut key = vec![Some(base_name.clone())];
                key.extend_from_slice(&names[..len]);
                let Some((old, new)) = lookup(&key) else {
                    continue;
                };
                let range = TextRange::new(start, elements[len - 1].1.end());
                diags.extend(self.free_head(model, config, &old, new, range, sets));
                return;
            }
        }
        let mut at = 0;
        while at < names.len() {
            let known = legacy_packages(&names[..at])
                || (at == 0
                    && matches!(base, Some(Expr::Ident(token)) if sets.contains(&token.text(source))));
            let mut matched = None;
            for len in (1..=longest.min(names.len() - at)).rev() {
                if !known && len == 1 {
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
            if known {
                diags.push(self.fixed(&old, new, range, attr_segments(new)));
            } else {
                let receiver_end = if at == 0 {
                    written.map_or(range.start(), |base| base.content_range().end())
                } else {
                    elements[at - 1].1.end()
                };
                let receiver_start = select.syntax().content_range().start();
                let receiver = &source[receiver_start as usize..receiver_end as usize];
                diags.push(self.unknown_receiver(&old, new, range, receiver));
            }
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
    ) -> Option<Diagnostic> {
        let offset = range.start();
        let base = head(old);
        if model.resolve_lexical(base, offset).is_some() {
            return Some(self.unfixed(
                old,
                new,
                range,
                format!("`{base}` is bound here; take `{}` the way `{base}` arrives and use it in place of `{old}`", head(new)),
            ));
        }
        let reference = model.reference_at(TextRange::new(offset, offset + base.len() as u32))?;
        let innermost = reference.via_with?;
        if !with_is_package_set(model, sets, innermost) {
            if reaches_package_set(model, sets, offset) {
                return Some(self.unfixed(
                    old,
                    new,
                    range,
                    format!("`{base}` reaches a package set through nested `with` scopes; write `{new}` from that set"),
                ));
            }
            let namespace = with_namespace(model, model.with_sites()[innermost].scope_range)?;
            let range_of = namespace.content_range();
            let receiver = &model.source()[range_of.start() as usize..range_of.end() as usize];
            return Some(self.unknown_receiver(old, new, range, receiver));
        }
        if !is_ident(head(new)) || model.resolve_lexical(head(new), offset).is_some() {
            return Some(self.unfixed(
                old,
                new,
                range,
                format!(
                    "`{}` would not reach the package set here; write `{new}` from it",
                    head(new)
                ),
            ));
        }
        Some(if with_removed_first(model, config, innermost) {
            self.after_with_removal(old, new, range)
        } else {
            self.fixed(old, new, range, new.to_owned())
        })
    }

    fn check_reference(
        &self,
        model: &SemanticModel<'_>,
        config: &LintConfig,
        token: &SyntaxToken,
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
        let Some((namespace_head, names)) = receiver(namespace, source) else {
            return;
        };
        let namespace_text = &source
            [namespace.content_range().start() as usize..namespace.content_range().end() as usize];
        if with_is_package_set(model, sets, innermost) {
            let Some(new) = renames().map.get(name) else {
                return;
            };
            if deferred {
                diags.push(self.after_with_removal(name, new, token.range()));
            } else if is_ident(head(new)) && model.resolve_lexical(head(new), offset).is_none() {
                diags.push(self.fixed(name, new, token.range(), (*new).to_owned()));
            } else {
                diags.push(self.unfixed(
                    name,
                    new,
                    token.range(),
                    format!(
                        "`{}` would not reach `{namespace_text}` here; write `{namespace_text}.{}`",
                        head(new),
                        attr_segments(new)
                    ),
                ));
            }
            return;
        }
        let head_name = namespace_head.text(source).to_owned();
        let mut path: Vec<Option<String>> = vec![Some(head_name.clone())];
        path.extend(names.iter().map(|(segment, _)| segment.clone()));
        for from in 0..path.len() {
            let mut key = path[from..].to_vec();
            key.push(Some(name.to_owned()));
            if key.len() < 2 {
                continue;
            }
            let Some((old, new)) = lookup(&key) else {
                continue;
            };
            let help = format!("`{name}` comes from `with {namespace_text}`; write `{new}` from the package set that holds it");
            if from == 0 {
                let head_start = namespace_head.range().start();
                let known = model.resolve_lexical(&head_name, head_start).is_some()
                    || model
                        .reference_at(namespace_head.range())
                        .is_some_and(|reference| {
                            reference.resolved.is_none()
                                && reference
                                    .via_with
                                    .is_some_and(|outer| with_is_package_set(model, sets, outer))
                        });
                if known {
                    diags.push(self.unfixed(&old, new, token.range(), help));
                } else {
                    diags.push(self.unknown_receiver(&old, new, token.range(), &head_name));
                }
                return;
            }
            if !package_set(source, sets, namespace_head, &path[1..from]) {
                let receiver = &source[namespace_head.range().start() as usize..if from == 1 {
                    namespace_head.range().end()
                } else {
                    names[from - 2].1.end()
                }
                    as usize];
                diags.push(self.unknown_receiver(&old, new, token.range(), receiver));
                return;
            }
            let namespace_start = namespace_head.range().start();
            let prefix_end = if from == 1 {
                namespace_head.range().end()
            } else {
                names[from - 2].1.end()
            };
            if deferred {
                diags.push(self.after_with_removal(&old, new, token.range()));
            } else if same_binding(model, &head_name, namespace_start, offset) {
                let prefix = &source[namespace_start as usize..prefix_end as usize];
                diags.push(self.fixed(
                    &old,
                    new,
                    token.range(),
                    format!("{prefix}.{}", attr_segments(new)),
                ));
            } else {
                diags.push(self.unfixed(&old, new, token.range(), help));
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
        "Flags a nixpkgs attribute that nixpkgs keeps only as a warning alias for a renamed package, such as `xorg.libX11` for `libx11`, which makes every evaluation that reaches it print `evaluation warning: The xorg package set has been deprecated`. The renames come from the `warnAlias`/`warnOnInstantiate` definitions in nixpkgs' `pkgs/top-level/aliases.nix`, read by `nix run .#nixpkgs-renames` from the nixpkgs strictix is locked to; an alias whose message is not a plain `'OLD' has been renamed to 'NEW'` is left alone. An alias is matched only on a receiver known to be nixpkgs: a name in the `sets` setting (default `pkgs`, `final`, `prev`) or a path ending in `legacyPackages.SYSTEM`, selected from directly (`pkgs.xorg.libX11.dev`, `pkgs.swww`) or opened with `with` (`with pkgs; xorg.libX11`, `with pkgs.xorg; libX11`); on any other receiver (`custom.xorg.libX11`, `pkgs.pkgsCross.X.xorg.libX11`, `with p; xorg.libX11`) a multi-segment alias is reported without a fix, with help to name the receiver in `sets`, and a one-segment alias is left alone. Fix rewrites the selection, `xorg.libX11` under `with pkgs;` to `libx11`, and `libX11` under `with pkgs.xorg;` to `pkgs.libx11`; it is withheld, with help, when the new name is shadowed by a lexical binding, when the innermost `with` is not the package set but an outer one is, and when `xorg` is a binding of its own (an argument or `let`). A `with` or argument left unused is for `redundant-with` and `unused-formal`."
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
