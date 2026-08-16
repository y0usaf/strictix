//! with-fallback, import sites, laziness.

use strictix_core::semantic::{BindingKind, SemanticModel};
use strictix_syntax::{parse, SyntaxNode};

fn model(src: &'static str) -> SemanticModel<'static> {
    let tree: &'static SyntaxNode = Box::leak(Box::new(parse(src)));
    SemanticModel::new(src, tree)
}

/// Bindings matching a name, in declaration order.
fn bindings_named<'m>(
    m: &'m SemanticModel<'static>,
    name: &str,
) -> Vec<&'m strictix_core::semantic::Binding<'static>> {
    let src = m.source();
    m.bindings()
        .iter()
        .filter(|b| b.name.text(src) == name)
        .collect()
}

/// The single binding matching name+kind; panics otherwise.
fn one_binding<'m>(
    m: &'m SemanticModel<'static>,
    name: &str,
    kind: BindingKind,
) -> &'m strictix_core::semantic::Binding<'static> {
    let src = m.source();
    let found: Vec<_> = m
        .bindings()
        .iter()
        .filter(|b| b.name.text(src) == name && b.kind == kind)
        .collect();
    assert_eq!(found.len(), 1, "expected one {kind:?} binding named {name}");
    found[0]
}

fn refs<'a>(m: &'a SemanticModel<'static>) -> Vec<(&'a str, Option<usize>, Option<usize>)> {
    let src = m.source();
    m.references()
        .iter()
        .map(|r| (r.name.text(src), r.resolved, r.via_with))
        .collect()
}

// --- 1. let shadowing -------------------------------------------------------

#[test]
fn let_shadowing_resolves_to_inner() {
    let src = "let a = 1; in let a = 2; in a";
    let m = model(src);
    let inner = bindings_named(&m, "a").pop().expect("inner a");
    assert_eq!(bindings_named(&m, "a").len(), 2);
    let body_refs: Vec<_> = refs(&m);
    // The final a reference resolves to the inner binding (index 1).
    let last = body_refs.last().expect("has body ref");
    assert_eq!(last.0, "a");
    assert_eq!(last.1, Some(1));
    assert_eq!(inner.references.len(), 1);
}

// --- 2. let sequential visibility -------------------------------------------

#[test]
fn let_value_sees_earlier_bindings() {
    let src = "let x = 1; in let y = x; in y";
    let m = model(src);
    let x = one_binding(&m, "x", BindingKind::LetBinding);
    assert_eq!(x.references.len(), 1, "y's value references outer x");
    // The outer x has exactly one use; the inner y has one use (the body).
    let y = one_binding(&m, "y", BindingKind::LetBinding);
    assert_eq!(y.references.len(), 1);
}

#[test]
fn let_binding_visible_in_own_value() {
    // Nix `let` is recursive: the RHS x resolves to the binding itself,
    // which is infinite recursion only when forced at runtime.
    let src = "let x = x; in x";
    let m = model(src);
    let r: Vec<_> = refs(&m);
    // First x (in the value): resolves to the let binding itself.
    // Second x (body): also the let binding.
    assert_eq!(r[0].0, "x");
    assert_eq!(r[0].1, Some(0), "recursive let sees its own binding");
    assert_eq!(r[1].1, Some(0), "body x resolves to the let binding");
}

#[test]
fn let_forward_reference_works() {
    // Recursive let: a binding can forward-reference a later one.
    let src = "let a = b; b = 1; in a";
    let m = model(src);
    let a = one_binding(&m, "a", BindingKind::LetBinding);
    let b = one_binding(&m, "b", BindingKind::LetBinding);
    assert_eq!(a.references.len(), 1, "a's value references b");
    assert_eq!(b.references.len(), 1, "body references b");
    let r = refs(&m);
    assert_eq!(r[0].1, Some(1), "forward reference resolves to b");
}

// --- 3. rec self-ref --------------------------------------------------------

#[test]
fn rec_attrs_mutually_visible() {
    let src = "rec { a = 1; b = a; }";
    let m = model(src);
    let a = one_binding(&m, "a", BindingKind::RecAttr);
    assert_eq!(a.references.len(), 1, "b's value references a");
}

// --- 4. formals defaults ----------------------------------------------------

#[test]
fn formals_default_sees_earlier_param() {
    let src = "{ a, b ? a }: b";
    let m = model(src);
    let a = one_binding(&m, "a", BindingKind::LambdaParam);
    assert_eq!(a.references.len(), 1, "b's default references a");
    let b = one_binding(&m, "b", BindingKind::LambdaParam);
    assert_eq!(b.references.len(), 1, "body references b");
}

// --- 5. at-name -------------------------------------------------------------

#[test]
fn at_name_binds_and_resolves() {
    let src = "args@{ a }: args";
    let m = model(src);
    let args = one_binding(&m, "args", BindingKind::AtName);
    assert_eq!(args.references.len(), 1);
}

// --- 6. inherit in let ------------------------------------------------------

#[test]
fn inherit_in_let_binds_and_resolves_source() {
    let src = "let x = 1; inherit (x) y; in y";
    let m = model(src);
    let x = one_binding(&m, "x", BindingKind::LetBinding);
    assert_eq!(x.references.len(), 1, "inherit source references x");
    let y = one_binding(&m, "y", BindingKind::LetBinding);
    assert_eq!(y.references.len(), 1, "body references y");
}

#[test]
fn inherit_from_source_name_is_not_a_lexical_reference() {
    // `inherit (cfg) bar;` defines `bar` by copying `bar` out of `cfg`;
    // the name is NOT a lexical use, so it must not appear as an
    // (unresolved) reference.
    let src = "let cfg = {}; in { inherit (cfg) bar; }";
    let m = model(src);
    assert!(
        !m.references().iter().any(|r| r.name.text(src) == "bar"),
        "inherit-from name must not be recorded as a reference"
    );
    // The inherit name still binds locally.
    assert!(bindings_named(&m, "bar").len() >= 1);
}

#[test]
fn sourceless_inherit_is_still_a_lexical_reference() {
    // `inherit baz;` reads the outer `baz` binding, so it must still
    // resolve.
    let src = "let baz = 1; in { inherit baz; }";
    let m = model(src);
    let r = refs(&m);
    let baz = r.iter().find(|x| x.0 == "baz").expect("baz reference");
    assert_eq!(baz.1, Some(0), "sourceless inherit still resolves");
}

// --- 7. with fallback -------------------------------------------------------

#[test]
fn with_fallback_recorded() {
    let src = "with { a = 1; }; a";
    let m = model(src);
    assert_eq!(m.with_sites().len(), 1);
    let r = refs(&m);
    let a = r.last().expect("a reference");
    assert_eq!(a.0, "a");
    assert_eq!(a.1, None, "no lexical binding");
    assert_eq!(a.2, Some(0), "innermost with candidate recorded");
}

// --- 8. lexical beats with --------------------------------------------------

#[test]
fn lexical_binding_beats_with() {
    let src = "let a = 1; in with { a = 2; }; a";
    let m = model(src);
    let r = refs(&m);
    let a = r.last().expect("body a");
    assert_eq!(a.1, Some(0), "resolves to the let binding");
    assert_eq!(a.2, None, "not via with");
}

// --- 9. unbound -------------------------------------------------------------

#[test]
fn unbound_reference_recorded() {
    let src = "frobnicate";
    let m = model(src);
    let r = refs(&m);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].0, "frobnicate");
    assert_eq!(r[0].1, None);
    assert_eq!(r[0].2, None);
    assert!(m.bindings().is_empty());
}

// --- 10. import sites -------------------------------------------------------

#[test]
fn import_sites_recorded() {
    let src = "import ./foo.nix";
    let m = model(src);
    assert_eq!(m.import_sites().len(), 1);
    let site = &m.import_sites()[0];
    assert_eq!(site.path_range.start(), 7); // "./foo.nix"

    let src2 = "builtins.import ./foo.nix";
    let m2 = model(src2);
    assert_eq!(m2.import_sites().len(), 1);
    let site2 = &m2.import_sites()[0];
    assert_eq!(site2.path_range.start(), 16); // "./foo.nix" starts at 16
}

// --- 11. laziness / malformed input -----------------------------------------

#[test]
fn construction_is_cheap_and_never_panics() {
    for src in ["let x =", "}", "if true then", "{ a = }"] {
        let tree = parse(src);
        let m = SemanticModel::new(src, &tree); // must not walk or panic
        let _ = m.bindings(); // build must not panic either
        let _ = m.references();
    }
}

// --- 12. attrpath elements are not references -------------------------------

#[test]
fn attrpath_elements_are_not_references() {
    let src = "let a = { b = 1; }; in a.b";
    let m = model(src);
    let r = refs(&m);
    let texts: Vec<&str> = r.iter().map(|x| x.0).collect();
    assert_eq!(texts, ["a"], "only the base is a reference, not b");
    let a = one_binding(&m, "a", BindingKind::LetBinding);
    assert_eq!(a.references.len(), 1);
}

// --- 13. reference/binding sanity -------------------------------------------

#[test]
fn bare_ident_is_one_reference_zero_bindings() {
    let src = "a";
    let m = model(src);
    assert_eq!(m.references().len(), 1);
    assert!(m.bindings().is_empty());
}

// --- extra: select default, string interp, nested with ----------------------

#[test]
fn select_default_and_interp_references() {
    let src = "let x = { y = 1; }; in (x.y or 2) + \"${x}\"";
    let m = model(src);
    let x = one_binding(&m, "x", BindingKind::LetBinding);
    assert_eq!(x.references.len(), 2, "select default + string interp");
}

#[test]
fn nested_with_innermost_candidate() {
    let src = "with a; with b; x";
    let m = model(src);
    assert_eq!(m.with_sites().len(), 2);
    let r = refs(&m);
    let x = r.last().expect("x reference");
    assert_eq!(x.1, None);
    assert_eq!(x.2, Some(1), "innermost with (b) is the candidate");
}

#[test]
fn with_scope_expr_resolves_outside() {
    let src = "let lib = 1; in with lib; body";
    let m = model(src);
    let lib = one_binding(&m, "lib", BindingKind::LetBinding);
    assert_eq!(lib.references.len(), 1, "with scope expr references lib");
    let all_refs = refs(&m);
    let body = all_refs.last().expect("body");
    assert_eq!(body.0, "body");
    assert_eq!(body.1, None, "body unbound lexically");
    assert_eq!(body.2, Some(0));
}

#[test]
fn shadowing_across_scopes_via_outer_shadow() {
    // Recursive let sees itself at its own name position; shadowing is
    // detected from the enclosing scope outward.
    let src = "let a = 1; in let a = 2; in a";
    let m = model(src);
    let inner = bindings_named(&m, "a").pop().expect("inner a");
    let resolved = m
        .resolve_lexical(inner.name.text(src), inner.name.range().start())
        .expect("recursive let sees itself");
    assert_eq!(
        resolved.name.range(),
        inner.name.range(),
        "resolves to itself"
    );
    let outer = m.outer_shadow(inner).expect("outer a shadows the inner");
    assert_eq!(outer.name.range(), m.bindings()[0].name.range());
}

#[test]
fn select_attrpath_interp_is_a_reference() {
    let src = r#"let name = "x"; in cfg.folders.${name}.id"#;
    let m = model(src);
    let name = one_binding(&m, "name", BindingKind::LetBinding);
    assert_eq!(
        name.references.len(),
        1,
        "the attrpath interpolation resolves"
    );
}
