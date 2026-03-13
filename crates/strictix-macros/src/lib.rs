mod explain;
mod metadata;
mod test;

use explain::generate_explain_impl;
use metadata::{RawLintMeta, generate_meta_impl};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Ident, ItemStruct, parse_macro_input};

fn generate_self_impl(struct_name: &Ident) -> TokenStream2 {
    quote! {
        impl #struct_name {
            pub fn new() -> Self {
                Self
            }
        }
    }
}

/// Derives `Lint`, `Metadata`, `Explain`, and `Rule` impls for a lint struct.
///
/// The generated code references `crate::Lint`, `crate::Metadata`, `crate::Report`,
/// and other items from `strictix-core`. This macro is intended for use within the
/// `strictix-core` crate only.
#[proc_macro_attribute]
pub fn lint(attr: TokenStream, item: TokenStream) -> TokenStream {
    let struct_item = parse_macro_input!(item as ItemStruct);
    let meta = parse_macro_input!(attr as RawLintMeta);

    let struct_name = &struct_item.ident;
    let self_impl = generate_self_impl(struct_name);
    let meta_impl = generate_meta_impl(struct_name, &meta);
    let explain_impl = generate_explain_impl(&struct_item);

    (quote! {
        #struct_item

        pub static LINT: std::sync::LazyLock<Box<dyn crate::Lint>> =
            std::sync::LazyLock::new(|| Box::new(#struct_name::new()));

        #self_impl
        #meta_impl
        #explain_impl

        impl crate::Lint for #struct_name {}
    })
    .into()
}

#[proc_macro]
pub fn generate_tests(input: TokenStream) -> TokenStream {
    crate::test::generate_tests(input)
}
