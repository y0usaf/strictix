use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use sha2::{Digest, Sha256};
use syn::{
    Error, Expr, ExprArray, Ident, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
    token::Comma,
};

struct MacroInvocation {
    rule: Ident,
    expressions: Punctuated<Expr, Comma>,
}

impl Parse for MacroInvocation {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        const RULE_VALUE: &str = "rule";
        const EXPRESSSIONS_VALUE: &str = "expressions";
        let rule_attribute = input.parse::<Ident>()?;

        if rule_attribute != RULE_VALUE {
            return Err(Error::new(
                rule_attribute.span(),
                "expected `{RULE_VALUE:?}`",
            ));
        }

        input.parse::<Token![:]>()?;
        let rule = input.parse::<Ident>()?;
        input.parse::<Token![,]>()?;
        let expressions = input.parse::<Ident>()?;

        if expressions != EXPRESSSIONS_VALUE {
            return Err(Error::new(
                expressions.span(),
                "expected `{EXPRESSSIONS_VALUE:?}`",
            ));
        }

        input.parse::<Token![:]>()?;
        let ExprArray {
            elems: expressions, ..
        } = input.parse::<ExprArray>()?;

        input.parse::<Token![,]>()?;
        Ok(MacroInvocation { rule, expressions })
    }
}

pub fn generate_tests(input: TokenStream) -> TokenStream {
    let MacroInvocation { rule, expressions } = parse_macro_input!(input as MacroInvocation);
    let property_test = make_property_test(&rule, &expressions);
    let generated_tests = expressions
        .into_iter()
        .map(|nix_expression| {
            let lint_test = make_test(&rule, TestKind::Lint, &nix_expression);
            let fix_test = make_test(&rule, TestKind::Fix, &nix_expression);
            let fix_roundtrip_test = make_test(&rule, TestKind::FixRoundtrip, &nix_expression);

            quote! {
                #lint_test

                #fix_test

                #fix_roundtrip_test
            }
        })
        .collect::<proc_macro2::TokenStream>();

    quote! {
        #generated_tests

        #property_test
    }
    .into()
}

#[derive(Clone, Copy, Debug)]
enum TestKind {
    Lint,
    Fix,
    FixRoundtrip,
}

fn make_test(rule: &Ident, kind: TestKind, nix_expression: &Expr) -> proc_macro2::TokenStream {
    let expression_hash = Sha256::digest(nix_expression.to_token_stream().to_string());
    let expression_hash = hex::encode(expression_hash);

    let kind_str = match kind {
        TestKind::Lint => "lint",
        TestKind::Fix => "fix",
        TestKind::FixRoundtrip => "fix_roundtrip",
    };

    let test_name = format!("{rule}_{kind_str}_{expression_hash}");
    let test_ident = Ident::new(&test_name, nix_expression.span());
    let snap_name = format!("{kind_str}_{expression_hash}");

    match kind {
        TestKind::Lint | TestKind::Fix => {
            let args = match kind {
                TestKind::Lint => quote! {&["check"]},
                TestKind::Fix => quote! {&["fix", "--dry-run"]},
                TestKind::FixRoundtrip => unreachable!(),
            };

            quote! {
                #[test]
                fn #test_ident() {
                    let expression = #nix_expression;
                    let stdout = _utils::test_cli(expression, #args).unwrap();
                    insta::assert_snapshot!(#snap_name, stdout, &format!("{expression:?}"));
                }
            }
        }
        TestKind::FixRoundtrip => {
            quote! {
                #[test]
                fn #test_ident() {
                    let expression = #nix_expression;
                    _utils::assert_fix_roundtrip(expression).unwrap();
                }
            }
        }
    }
}

fn make_property_test(
    rule: &Ident,
    expressions: &Punctuated<Expr, Comma>,
) -> proc_macro2::TokenStream {
    let test_name = format!("{rule}_fix_properties");
    let test_ident = Ident::new(&test_name, rule.span());
    let expression_cases = expressions.iter().map(|expression| {
        quote! { proptest::strategy::Just(#expression) }
    });
    let rule_name = rule.to_string();

    quote! {
        proptest::proptest! {
            #![proptest_config(proptest::test_runner::Config {
                failure_persistence: None,
                .. proptest::test_runner::Config::default()
            })]

            #[test]
            fn #test_ident(
                expression in proptest::prop_oneof![#(#expression_cases),*],
                prefix in _utils::trivia_strategy(),
                suffix in _utils::trivia_strategy(),
            ) {
                _utils::assert_rewrite_invariants(#rule_name, expression, &prefix, &suffix).unwrap();
            }
        }
    }
}
