//! `#[xui::main]` — installs the generated asset bootstrap before `main` runs.

use proc_macro2::Span;
use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute as SynAttribute, Error, Expr, LitStr, Result, Signature, Visibility};

use crate::component::strip_component_param_defaults;
use crate::krate;

pub struct MainFunction {
    attrs: Vec<SynAttribute>,
    vis: Visibility,
    sig: Signature,
    input_defaults: Vec<Option<Expr>>,
    body: TokenStream2,
}

impl Parse for MainFunction {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(SynAttribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let mut sig_tokens = TokenStream2::new();
        let mut body = None;
        while !input.is_empty() {
            let token: TokenTree = input.parse()?;
            if let TokenTree::Group(group) = &token
                && group.delimiter() == Delimiter::Brace
            {
                body = Some(group.stream());
                break;
            }
            sig_tokens.extend(std::iter::once(token));
        }
        let body =
            body.ok_or_else(|| Error::new(input.span(), "component function requires a body"))?;
        let (sig_tokens, input_defaults) = strip_component_param_defaults(sig_tokens)?;
        let sig = syn::parse2::<Signature>(sig_tokens)?;
        Ok(Self {
            attrs,
            vis,
            sig,
            input_defaults,
            body,
        })
    }
}

pub fn expand_main_function(function: MainFunction) -> Result<TokenStream2> {
    let MainFunction {
        attrs,
        vis,
        sig,
        input_defaults,
        body,
    } = function;

    if input_defaults.iter().any(Option::is_some) {
        return Err(Error::new(
            sig.span(),
            "#[main] does not support parameter default values",
        ));
    }

    let xui = krate::xui()?;
    let assets_bootstrap = match std::env::var("XUI_ASSETS_BOOTSTRAP") {
        Ok(path) => {
            let path = LitStr::new(&path, Span::call_site());
            quote! {
                include!(#path);
            }
        }
        Err(_) => {
            return Ok(quote! {
                compile_error!("XUI assets require building through `cargo xui`");

                #(#attrs)*
                #vis #sig {
                    #body
                }
            });
        }
    };

    Ok(quote! {
        #assets_bootstrap

        #(#attrs)*
        #vis #sig {
            match xui_assets::manager() {
                ::std::result::Result::Ok(__xui_asset_manager) => {
                    #xui::assets::install_asset_manager(__xui_asset_manager);
                }
                ::std::result::Result::Err(_) => {
                    #xui::assets::clear_asset_manager();
                }
            }
            #body
        }
    })
}
