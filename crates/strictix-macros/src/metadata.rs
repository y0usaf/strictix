use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Expr, ExprArray, Ident, Lit, Path, Token,
    parse::{Parse, ParseStream, Result},
    spanned::Spanned,
};

enum MatchWith {
    Path(Path),
    Array(ExprArray),
}

pub struct RawLintMeta {
    name: Lit,
    note: Lit,
    code: Lit,
    match_with: MatchWith,
}

impl Parse for RawLintMeta {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut note = None;
        let mut code = None;
        let mut match_with = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "name" => name = Some(parse_lit(input)?),
                "note" => note = Some(parse_lit(input)?),
                "code" => code = Some(parse_lit(input)?),
                "match_with" => match_with = Some(parse_match_with(input)?),
                _ => return Err(syn::Error::new(key.span(), "unknown lint metadata field")),
            }

            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }

        Ok(Self {
            name: name.ok_or_else(|| input.error("`name` not present"))?,
            note: note.ok_or_else(|| input.error("`note` not present"))?,
            code: code.ok_or_else(|| input.error("`code` not present"))?,
            match_with: match_with.ok_or_else(|| input.error("`match_with` not present"))?,
        })
    }
}

fn parse_lit(input: ParseStream) -> Result<Lit> {
    match input.parse::<Expr>()? {
        Expr::Lit(lit) => Ok(lit.lit),
        expr => Err(syn::Error::new(expr.span(), "expected a literal")),
    }
}

fn parse_match_with(input: ParseStream) -> Result<MatchWith> {
    let expr = input.parse::<Expr>()?;
    match expr {
        Expr::Path(path) => Ok(MatchWith::Path(path.path)),
        Expr::Array(array) => Ok(MatchWith::Array(array)),
        expr => Err(syn::Error::new(
            expr.span(),
            "`match_with` is neither a path nor an array",
        )),
    }
}

impl RawLintMeta {
    fn generate_name_fn(&self) -> TokenStream2 {
        let name_str = &self.name;
        quote! {
            fn name(&self) -> &'static str {
                #name_str
            }
        }
    }

    fn generate_note_fn(&self) -> TokenStream2 {
        let note_str = &self.note;
        quote! {
            fn note(&self) -> &'static str {
                #note_str
            }
        }
    }

    fn generate_code_fn(&self) -> TokenStream2 {
        let code_int = &self.code;
        quote! {
            fn code(&self) -> u32 {
                #code_int
            }
        }
    }

    fn generate_match_kind_fn(&self) -> TokenStream2 {
        match &self.match_with {
            MatchWith::Path(path) => {
                quote! {
                    fn match_kind(&self) -> &'static [SyntaxKind] {
                        &[#path]
                    }
                }
            }
            MatchWith::Array(array) => {
                let elems = &array.elems;
                quote! {
                    fn match_kind(&self) -> &'static [SyntaxKind] {
                        &[#elems]
                    }
                }
            }
        }
    }

    fn generate_report_fn() -> TokenStream2 {
        quote! {
            fn report(&self) -> crate::Report {
                crate::Report::new(self.note(), self.code())
            }
        }
    }
}

pub fn generate_meta_impl(struct_name: &Ident, meta: &RawLintMeta) -> TokenStream2 {
    let name_fn = meta.generate_name_fn();
    let note_fn = meta.generate_note_fn();
    let code_fn = meta.generate_code_fn();
    let match_kind = meta.generate_match_kind_fn();
    let report_fn = RawLintMeta::generate_report_fn();

    quote! {
        impl crate::Metadata for #struct_name {
            #name_fn
            #note_fn
            #code_fn
            #match_kind
            #report_fn
        }
    }
}
