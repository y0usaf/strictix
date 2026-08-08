//! Typed AST tests: every node kind wraps, accessors return the right
//! children, and the corpus fixtures build correct typed trees.

use strictix_syntax::{
    parse, AstNode, AttrItem, AttrName, Attrpath, BinExpr, Expr, FormalParam, Formals, LambdaExpr,
    LambdaParam, LetBindings, LetExpr, Root, SelectExpr, StringPart, SyntaxKind, SyntaxNode,
};

use SyntaxKind as K;

// --- every node kind has a wrapper ---

/// Cast any node to its wrapper; panics if the kind has none.
fn assert_any_kind_wraps(node: &SyntaxNode) {
    match node.kind() {
        K::Root => assert!(Root::cast(node).is_some()),
        K::ErrorNode => assert!(strictix_syntax::ErrorNode::cast(node).is_some()),
        K::LetExpr => assert!(LetExpr::cast(node).is_some()),
        K::LetBindings => assert!(LetBindings::cast(node).is_some()),
        K::Binding => assert!(strictix_syntax::Binding::cast(node).is_some()),
        K::Attrpath => assert!(Attrpath::cast(node).is_some()),
        K::InheritStmt => assert!(strictix_syntax::InheritStmt::cast(node).is_some()),
        K::WithExpr => assert!(strictix_syntax::WithExpr::cast(node).is_some()),
        K::AssertExpr => assert!(strictix_syntax::AssertExpr::cast(node).is_some()),
        K::IfExpr => assert!(strictix_syntax::IfExpr::cast(node).is_some()),
        K::AttrsetExpr => assert!(strictix_syntax::AttrsetExpr::cast(node).is_some()),
        K::RecAttrsetExpr => assert!(strictix_syntax::RecAttrsetExpr::cast(node).is_some()),
        K::Formals => assert!(Formals::cast(node).is_some()),
        K::ListExpr => assert!(strictix_syntax::ListExpr::cast(node).is_some()),
        K::LambdaExpr => assert!(LambdaExpr::cast(node).is_some()),
        K::ApplyExpr => assert!(strictix_syntax::ApplyExpr::cast(node).is_some()),
        K::UnaryExpr => assert!(strictix_syntax::UnaryExpr::cast(node).is_some()),
        K::BinExpr => assert!(BinExpr::cast(node).is_some()),
        K::SelectExpr => assert!(SelectExpr::cast(node).is_some()),
        K::HasAttrExpr => assert!(strictix_syntax::HasAttrExpr::cast(node).is_some()),
        K::StringExpr => assert!(strictix_syntax::StringExpr::cast(node).is_some()),
        K::IndStringExpr => assert!(strictix_syntax::IndStringExpr::cast(node).is_some()),
        K::InterpExpr => assert!(strictix_syntax::InterpExpr::cast(node).is_some()),
        K::ParenExpr => assert!(strictix_syntax::ParenExpr::cast(node).is_some()),
        k => panic!("node kind {k:?} has no typed wrapper"),
    }
}

#[test]
fn corpus_every_node_kind_wraps() {
    for file in ["tests/corpus/module.nix", "tests/corpus/overlay.nix"] {
        let src =
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file))
                .expect("read corpus file");
        let tree = parse(&src);
        for node in tree.descendants() {
            assert_any_kind_wraps(node);
        }
    }
}

// --- corpus: module.nix ---

#[test]
fn corpus_module_shape() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/corpus/module.nix"
    ))
    .expect("read corpus file");
    let tree = parse(&src);
    let root = Root::cast(&tree).expect("root");

    // Top: { config, lib, pkgs, ... }:
    let Expr::Lambda(top) = root.expr().expect("top-level lambda") else {
        panic!("module.nix is a lambda");
    };
    let LambdaParam::Formals(formals, None) = top.param() else {
        panic!("module.nix lambda takes formals");
    };
    let names: Vec<_> = formals
        .params()
        .map(|p: FormalParam<'_>| p.name.text(&src))
        .collect();
    assert_eq!(names, ["config", "lib", "pkgs"]);
    assert!(formals.has_ellipsis());

    // Body: let ... in { ... }
    let Expr::Let(let_expr) = top.body().expect("lambda body") else {
        panic!("lambda body is a let");
    };
    let bindings = let_expr.bindings().expect("let bindings");
    let items: Vec<_> = bindings.items().collect();
    assert_eq!(items.len(), 2);
    let AttrItem::Inherit(inh) = items[0] else {
        panic!("first binding is inherit")
    };
    let Expr::Ident(from) = inh.source().expect("inherit source") else {
        panic!("inherit (lib) source is an ident");
    };
    assert_eq!(from.text(&src), "lib");
    let names: Vec<_> = inh.names().map(|t| t.text(&src)).collect();
    assert_eq!(names, ["mkIf", "mkOption", "types"]);
    let AttrItem::Binding(port) = items[1] else {
        panic!("second binding is a binding")
    };
    assert_eq!(
        attrpath_text(&port.attrpath().expect("attrpath"), &src),
        "port"
    );
    let Expr::Int(_) = port.value().expect("port value") else {
        panic!("port = int")
    };

    // Body of the let: the module attrset.
    let Expr::Attrset(module) = let_expr.body().expect("let body") else {
        panic!("let body is an attrset");
    };
    let module_items: Vec<_> = module.items().collect();
    assert_eq!(module_items.len(), 2);
    let AttrItem::Binding(options) = module_items[0] else {
        panic!()
    };
    assert_eq!(
        attrpath_text(&options.attrpath().unwrap(), &src),
        "options.services.example"
    );

    let AttrItem::Binding(config) = module_items[1] else {
        panic!()
    };
    assert_eq!(attrpath_text(&config.attrpath().unwrap(), &src), "config");
    // config = mkIf config.services.example.enable { ... }
    // mkIf a b = Apply(Apply(mkIf, a), b)
    let Expr::Apply(app1) = config.value().unwrap() else {
        panic!("config value is apply")
    };
    let Expr::Apply(app2) = app1.func().unwrap() else {
        panic!("mkIf chain")
    };
    let Expr::Ident(mkif) = app2.func().unwrap() else {
        panic!("mkIf")
    };
    assert_eq!(mkif.text(&src), "mkIf");
    let Expr::Select(cond) = app2.arg().unwrap() else {
        panic!("cond is select")
    };
    assert_eq!(cond.base().unwrap().text(&src), "config");
    assert_eq!(
        attrpath_text(&cond.attrpath().unwrap(), &src),
        "services.example.enable"
    );
    let Expr::Attrset(config_body) = app1.arg().unwrap() else {
        panic!()
    };

    // environment.etc."example/config.json".text = builtins.toJSON { ... }
    let items: Vec<_> = config_body.items().collect();
    let AttrItem::Binding(etc) = items[0] else {
        panic!()
    };
    let etc_path = etc.attrpath().unwrap();
    let elements: Vec<_> = etc_path.elements().collect();
    assert_eq!(elements.len(), 4);
    let AttrName::Ident(t) = elements[0] else {
        panic!()
    };
    assert_eq!(t.text(&src), "environment");
    let AttrName::Str(s) = elements[2] else {
        panic!("quoted segment")
    };
    assert!(matches!(s.parts().next(), Some(StringPart::Content(_))));
}

fn attrpath_text(path: &Attrpath<'_>, src: &str) -> String {
    path.elements()
        .map(|e| match e {
            AttrName::Ident(t) => t.text(src).to_string(),
            AttrName::Str(s) => s
                .parts()
                .filter_map(|p| match p {
                    StringPart::Content(t) => Some(t.text(src)),
                    StringPart::Interp(_) => None,
                })
                .collect(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

// --- corpus: overlay.nix ---

#[test]
fn corpus_overlay_shape() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/corpus/overlay.nix"
    ))
    .expect("read corpus file");
    let tree = parse(&src);
    let root = Root::cast(&tree).expect("root");

    // final: prev: { ... }
    let Expr::Lambda(l1) = root.expr().unwrap() else {
        panic!("overlay is lambda")
    };
    let LambdaParam::Ident(fin) = l1.param() else {
        panic!()
    };
    assert_eq!(fin.text(&src), "final");
    let Expr::Lambda(l2) = l1.body().unwrap() else {
        panic!()
    };
    let LambdaParam::Ident(prev_tok) = l2.param() else {
        panic!()
    };
    assert_eq!(prev_tok.text(&src), "prev");
    let Expr::Attrset(attrset) = l2.body().unwrap() else {
        panic!()
    };

    // example = prev.callPackage ./pkgs/example { ... }
    let items: Vec<_> = attrset.items().collect();
    assert_eq!(items.len(), 4);
    let AttrItem::Binding(example) = items[0] else {
        panic!()
    };
    assert_eq!(attrpath_text(&example.attrpath().unwrap(), &src), "example");
    let Expr::Apply(app1) = example.value().unwrap() else {
        panic!()
    };
    let Expr::Apply(app2) = app1.func().unwrap() else {
        panic!()
    };
    let Expr::Select(sel) = app2.func().unwrap() else {
        panic!()
    };
    assert_eq!(sel.base().unwrap().text(&src), "prev");
    assert_eq!(attrpath_text(&sel.attrpath().unwrap(), &src), "callPackage");
    let Expr::Path(_) = app2.arg().unwrap() else {
        panic!("callPackage arg is a path")
    };
    let Expr::Attrset(arg) = app1.arg().unwrap() else {
        panic!()
    };

    // inherit (prev) lib stdenv;
    let arg_items: Vec<_> = arg.items().collect();
    let AttrItem::Inherit(inh) = arg_items[0] else {
        panic!()
    };
    let Expr::Ident(from) = inh.source().unwrap() else {
        panic!()
    };
    assert_eq!(from.text(&src), "prev");
    let names: Vec<_> = inh.names().map(|t| t.text(&src)).collect();
    assert_eq!(names, ["lib", "stdenv"]);

    // patched = prev.hello.overrideAttrs (old: { patches = (old.patches or [ ]) ++ [...]; });
    let AttrItem::Binding(patched) = items[1] else {
        panic!()
    };
    assert_eq!(attrpath_text(&patched.attrpath().unwrap(), &src), "patched");
    let Expr::Apply(app) = patched.value().unwrap() else {
        panic!()
    };
    let Expr::Select(override_) = app.func().unwrap() else {
        panic!()
    };
    assert_eq!(
        attrpath_text(&override_.attrpath().unwrap(), &src),
        "hello.overrideAttrs"
    );
    let Expr::Paren(paren) = app.arg().unwrap() else {
        panic!()
    };
    let Expr::Lambda(old_lambda) = paren.expr().unwrap() else {
        panic!()
    };
    let LambdaParam::Ident(old_tok) = old_lambda.param() else {
        panic!()
    };
    assert_eq!(old_tok.text(&src), "old");
    let Expr::Attrset(old_body) = old_lambda.body().unwrap() else {
        panic!()
    };
    let old_items: Vec<_> = old_body.items().collect();
    let AttrItem::Binding(patches) = old_items[0] else {
        panic!()
    };
    assert_eq!(attrpath_text(&patches.attrpath().unwrap(), &src), "patches");
    let Expr::Bin(concat) = patches.value().unwrap() else {
        panic!("++ is a binop")
    };
    assert_eq!(concat.op(), Some(K::PlusPlus));
    let Expr::Paren(or_paren) = concat.lhs().unwrap() else {
        panic!()
    };
    let Expr::Select(or_sel) = or_paren.expr().unwrap() else {
        panic!()
    };
    assert_eq!(attrpath_text(&or_sel.attrpath().unwrap(), &src), "patches");
    assert!(matches!(or_sel.default(), Some(Expr::List(_))));
    let Expr::List(rhs) = concat.rhs().unwrap() else {
        panic!()
    };
    assert!(matches!(rhs.items().next(), Some(Expr::Path(_))));

    // uri = "https://example.com/\${final.example.version}/release";
    let AttrItem::Binding(uri) = items[2] else {
        panic!()
    };
    let Expr::String(uri_str) = uri.value().unwrap() else {
        panic!()
    };
    let parts: Vec<_> = uri_str.parts().collect();
    assert_eq!(parts.len(), 3);
    let StringPart::Content(c0) = parts[0] else {
        panic!()
    };
    assert_eq!(c0.text(&src), "https://example.com/");
    let StringPart::Interp(interp) = parts[1] else {
        panic!()
    };
    let Expr::Select(ver) = interp.expr().unwrap() else {
        panic!()
    };
    assert_eq!(
        attrpath_text(&ver.attrpath().unwrap(), &src),
        "example.version"
    );
    let StringPart::Content(c2) = parts[2] else {
        panic!()
    };
    assert_eq!(c2.text(&src), "/release");
}

// --- inline fixtures per wrapper ---

#[test]
fn binop_nests_lhs_inside() {
    // '1 + 2 * 3' is BinExpr(1, +, BinExpr(2, *, 3)): lhs is a child.
    let src = "1 + 2 * 3";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Bin(outer) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(outer.op(), Some(K::Plus));
    let Expr::Int(one) = outer.lhs().unwrap() else {
        panic!()
    };
    assert_eq!(one.text(src), "1");
    let Expr::Bin(inner) = outer.rhs().unwrap() else {
        panic!()
    };
    assert_eq!(inner.op(), Some(K::Star));
    let Expr::Int(three) = inner.rhs().unwrap() else {
        panic!()
    };
    assert_eq!(three.text(src), "3");
}

#[test]
fn apply_nests_func_inside() {
    let src = "f x";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Apply(app) = root.expr().unwrap() else {
        panic!()
    };
    let Expr::Ident(f) = app.func().unwrap() else {
        panic!()
    };
    assert_eq!(f.text(src), "f");
    let Expr::Ident(x) = app.arg().unwrap() else {
        panic!()
    };
    assert_eq!(x.text(src), "x");
}

#[test]
fn apply_chain_left_assoc() {
    // f g x = Apply(Apply(f, g), x)
    let src = "f g x";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Apply(outer) = root.expr().unwrap() else {
        panic!()
    };
    let Expr::Apply(inner) = outer.func().unwrap() else {
        panic!("left-assoc")
    };
    let Expr::Ident(f) = inner.func().unwrap() else {
        panic!()
    };
    assert_eq!(f.text(src), "f");
}

#[test]
fn select_multi_segment() {
    let src = "config.services.example.enable";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Select(sel) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(sel.base().unwrap().text(src), "config");
    assert_eq!(
        attrpath_text(&sel.attrpath().unwrap(), src),
        "services.example.enable"
    );
    assert!(sel.default().is_none());
}

#[test]
fn select_or_default() {
    let src = "old.patches or [ ]";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Select(sel) = root.expr().unwrap() else {
        panic!()
    };
    assert!(matches!(sel.default(), Some(Expr::List(_))));
}

#[test]
fn lambda_ident_param() {
    let src = "x: x + 1";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Lambda(l) = root.expr().unwrap() else {
        panic!()
    };
    let LambdaParam::Ident(x) = l.param() else {
        panic!()
    };
    assert_eq!(x.text(src), "x");
    assert!(matches!(l.body(), Some(Expr::Bin(_))));
}

#[test]
fn lambda_formals_param() {
    let src = "{ a, b ? 1, ... }: b";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Lambda(l) = root.expr().unwrap() else {
        panic!()
    };
    let LambdaParam::Formals(formals, None) = l.param() else {
        panic!()
    };
    assert!(formals.has_ellipsis());
    let params: Vec<_> = formals.params().collect();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name.text(src), "a");
    assert!(params[0].default.is_none());
    assert_eq!(params[1].name.text(src), "b");
    let Some(Expr::Int(one)) = params[1].default else {
        panic!("default is int")
    };
    assert_eq!(one.text(src), "1");
}

#[test]
fn lambda_named_formals_both_orders() {
    for src in ["args@{ a }: a", "{ a }@args: a"] {
        let tree = parse(src);
        let root = Root::cast(&tree).unwrap();
        let Expr::Lambda(l) = root.expr().unwrap() else {
            panic!("{src}")
        };
        let LambdaParam::Formals(_, Some(name)) = l.param() else {
            panic!("{src} binds an at-name");
        };
        assert_eq!(name.text(src), "args", "src: {src}");
    }
}

#[test]
fn if_with_assert_accessors() {
    let src = "if c then t else e";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::If(if_e) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(if_e.cond().unwrap().text(src), "c");
    assert_eq!(if_e.then_branch().unwrap().text(src), "t");
    assert_eq!(if_e.else_branch().unwrap().text(src), "e");

    let src = "with lib; body";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::With(w) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(w.scope().unwrap().text(src), "lib");
    assert_eq!(w.body().unwrap().text(src), "body");

    let src = "assert cond; body";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Assert(a) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(a.cond().unwrap().text(src), "cond");
    assert_eq!(a.body().unwrap().text(src), "body");
}

#[test]
fn rec_attrset_and_list() {
    let src = "rec { a = 1; }";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::RecAttrset(rec) = root.expr().unwrap() else {
        panic!()
    };
    let inner = rec.attrset().expect("rec has an attrset");
    assert_eq!(inner.items().count(), 1);

    let src = "[ 1 true ./x ]";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::List(list) = root.expr().unwrap() else {
        panic!()
    };
    let items: Vec<_> = list.items().collect();
    assert_eq!(items.len(), 3);
    assert!(matches!(items[0], Expr::Int(_)));
    assert!(matches!(items[1], Expr::Ident(_)));
    assert!(matches!(items[2], Expr::Path(_)));
}

#[test]
fn string_interp_and_unary() {
    let src = "\"a${x}b\"";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::String(s) = root.expr().unwrap() else {
        panic!()
    };
    let parts: Vec<_> = s.parts().collect();
    assert_eq!(parts.len(), 3);
    let StringPart::Content(c0) = parts[0] else {
        panic!()
    };
    assert_eq!(c0.text(src), "a");
    let StringPart::Interp(interp) = parts[1] else {
        panic!()
    };
    assert_eq!(interp.expr().unwrap().text(src), "x");
    let StringPart::Content(c2) = parts[2] else {
        panic!()
    };
    assert_eq!(c2.text(src), "b");

    let src = "!x";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Unary(u) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(u.operand().unwrap().text(src), "x");

    let src = "(x)";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::Paren(p) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(p.expr().unwrap().text(src), "x");
}

#[test]
fn has_attr() {
    let src = "config ? services";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let Expr::HasAttr(h) = root.expr().unwrap() else {
        panic!()
    };
    assert_eq!(h.base().unwrap().text(src), "config");
    assert_eq!(h.attrpath().unwrap().text(src), "services");
}

#[test]
fn expr_range_and_text() {
    let src = "let x = 1; in x";
    let tree = parse(src);
    let root = Root::cast(&tree).unwrap();
    let expr = root.expr().unwrap();
    assert_eq!(expr.text(src), src);
    assert_eq!(expr.range().start(), 0);
    assert_eq!(expr.range().end(), src.len() as u32);
}
