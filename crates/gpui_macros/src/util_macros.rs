#![allow(clippy::test_attr_in_doctest)]

use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{ItemFn, LitStr, parse_macro_input, parse_quote};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Importance {
    Critical,
    Important,
    #[default]
    Average,
    Iffy,
    Fluff,
}

impl std::fmt::Display for Importance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Importance::Critical => write!(f, "Critical"),
            Importance::Important => write!(f, "Important"),
            Importance::Average => write!(f, "Average"),
            Importance::Iffy => write!(f, "Iffy"),
            Importance::Fluff => write!(f, "Fluff"),
        }
    }
}

pub(crate) mod consts {
    pub const SUF_NORMAL: &str = ".perf";
    pub const SUF_MDATA: &str = ".perfm";
    pub const ITER_ENV_VAR: &str = "PERF_ITERATIONS";
    pub const MDATA_LINE_PREF: &str = "PERF_MDATA";
    pub const ITER_COUNT_LINE_NAME: &str = "iterations";
    pub const WEIGHT_LINE_NAME: &str = "weight";
    pub const IMPORTANCE_LINE_NAME: &str = "importance";
    pub const VERSION_LINE_NAME: &str = "version";
    pub const MDATA_VER: u32 = 1;
    pub const WEIGHT_DEFAULT: u32 = 1;
}

pub(crate) fn path_impl(input: TokenStream) -> TokenStream {
    let path = parse_macro_input!(input as LitStr);
    let path = path.value();

    #[cfg(target_os = "windows")]
    {
        path = path.replace("/", "\\");
        if path.starts_with("\\") {
            path = format!("C:{}", path);
        }
    }

    TokenStream::from(quote! {
        #path
    })
}

pub(crate) fn uri_impl(input: TokenStream) -> TokenStream {
    let uri = parse_macro_input!(input as LitStr);
    let uri = uri.value();

    #[cfg(target_os = "windows")]
    let uri = uri.replace("file:///", "file:///C:/");

    TokenStream::from(quote! {
        #uri
    })
}

pub(crate) fn line_endings_impl(input: TokenStream) -> TokenStream {
    let text = parse_macro_input!(input as LitStr);
    let text = text.value();

    #[cfg(target_os = "windows")]
    let text = text.replace("\n", "\r\n");

    TokenStream::from(quote! {
        #text
    })
}

#[derive(Default)]
struct PerfArgs {
    iterations: Option<syn::Expr>,
    weight: Option<syn::Expr>,
    importance: Importance,
}

#[warn(clippy::all, clippy::pedantic)]
impl PerfArgs {
    fn parse_into(&mut self, meta: syn::meta::ParseNestedMeta) -> syn::Result<()> {
        if meta.path.is_ident("iterations") {
            self.iterations = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("weight") {
            self.weight = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("critical") {
            self.importance = Importance::Critical;
        } else if meta.path.is_ident("important") {
            self.importance = Importance::Important;
        } else if meta.path.is_ident("average") {
            self.importance = Importance::Average;
        } else if meta.path.is_ident("iffy") {
            self.importance = Importance::Iffy;
        } else if meta.path.is_ident("fluff") {
            self.importance = Importance::Fluff;
        } else {
            return Err(syn::Error::new_spanned(meta.path, "unexpected identifier"));
        }
        Ok(())
    }
}

#[warn(clippy::all, clippy::pedantic)]
pub(crate) fn perf_impl(our_attr: TokenStream, input: TokenStream) -> TokenStream {
    let mut args = PerfArgs::default();
    let parser = syn::meta::parser(|meta| PerfArgs::parse_into(&mut args, meta));
    parse_macro_input!(our_attr with parser);

    let ItemFn {
        attrs: mut attrs_main,
        vis,
        sig: mut sig_main,
        block,
    } = parse_macro_input!(input as ItemFn);
    if !attrs_main
        .iter()
        .any(|a| Some(&parse_quote!(test)) == a.path().segments.last())
    {
        attrs_main.push(parse_quote!(#[test]));
    }
    attrs_main.push(parse_quote!(#[allow(non_snake_case)]));

    let fns = if cfg!(perf_enabled) {
        #[allow(clippy::wildcard_imports, reason = "We control the other side")]
        use consts::*;

        let mut new_ident_main = sig_main.ident.to_string();
        let mut new_ident_meta = new_ident_main.clone();
        new_ident_main.push_str(SUF_NORMAL);
        new_ident_meta.push_str(SUF_MDATA);

        let new_ident_main = syn::Ident::new(&new_ident_main, sig_main.ident.span());
        sig_main.ident = new_ident_main;

        let new_ident_meta = syn::Ident::new(&new_ident_meta, sig_main.ident.span());
        let sig_meta = parse_quote!(fn #new_ident_meta());
        let attrs_meta = parse_quote!(#[test] #[allow(non_snake_case)]);

        let block_main = {
            parse_quote!({
                let iter_count = std::env::var(#ITER_ENV_VAR).unwrap().parse::<usize>().unwrap();
                for _ in 0..iter_count {
                    #block
                }
            })
        };
        let importance = format!("{}", args.importance);
        let block_meta = {
            let q_iter = if let Some(iter) = args.iterations {
                quote! {
                    println!("{} {} {}", #MDATA_LINE_PREF, #ITER_COUNT_LINE_NAME, #iter);
                }
            } else {
                quote! {}
            };
            let weight = args
                .weight
                .unwrap_or_else(|| parse_quote! { #WEIGHT_DEFAULT });
            parse_quote!({
                #q_iter
                println!("{} {} {}", #MDATA_LINE_PREF, #WEIGHT_LINE_NAME, #weight);
                println!("{} {} {}", #MDATA_LINE_PREF, #IMPORTANCE_LINE_NAME, #importance);
                println!("{} {} {}", #MDATA_LINE_PREF, #VERSION_LINE_NAME, #MDATA_VER);
            })
        };

        vec![
            ItemFn {
                attrs: attrs_main,
                vis: vis.clone(),
                sig: sig_main,
                block: block_main,
            },
            ItemFn {
                attrs: attrs_meta,
                vis,
                sig: sig_meta,
                block: block_meta,
            },
        ]
    } else {
        vec![ItemFn {
            attrs: attrs_main,
            vis,
            sig: sig_main,
            block,
        }]
    };

    fns.into_iter()
        .flat_map(|f| TokenStream::from(f.into_token_stream()))
        .collect()
}
