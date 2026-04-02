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
    /// Whether this lint is enabled by default. Defaults to true if not specified.
    default_enabled: Option<Lit>,
}

impl Parse for RawLintMeta {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut note = None;
        let mut code = None;
        let mut match_with = None;
        let mut default_enabled = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "name" => name = Some(parse_lit(input)?),
                "note" => note = Some(parse_lit(input)?),
                "code" => code = Some(parse_lit(input)?),
                "match_with" => match_with = Some(parse_match_with(input)?),
                "default_enabled" => default_enabled = Some(parse_lit(input)?),
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
            default_enabled,
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

    fn generate_default_enabled_fn(&self) -> Option<TokenStream2> {
        let default_enabled = self.default_enabled.as_ref()?;
        Some(quote! {
            fn default_enabled(&self) -> bool {
                #default_enabled
            }
        })
    }
}

pub fn generate_meta_impl(struct_name: &Ident, meta: &RawLintMeta) -> TokenStream2 {
    let name_fn = meta.generate_name_fn();
    let note_fn = meta.generate_note_fn();
    let code_fn = meta.generate_code_fn();
    let match_kind = meta.generate_match_kind_fn();
    let report_fn = RawLintMeta::generate_report_fn();
    let default_enabled_fn = meta.generate_default_enabled_fn();

    quote! {
        impl crate::Metadata for #struct_name {
            #name_fn
            #note_fn
            #code_fn
            #match_kind
            #report_fn
            #default_enabled_fn
        }
    }
}
