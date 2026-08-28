//! Resolves the runtime crates by their real names, so the macros keep working
//! when a downstream crate renames its dependency — and when `xui` itself is the
//! consumer.

use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
use proc_macro_crate::{FoundCrate, crate_name};
use quote::quote;
use syn::{Error, Result};

fn resolve(name: &str) -> Result<TokenStream2> {
    match crate_name(name) {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(found)) => {
            let ident = Ident::new(&found, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(Error::new(
            Span::call_site(),
            format!("failed to find `{name}` dependency: {error}"),
        )),
    }
}

pub fn xui() -> Result<TokenStream2> {
    resolve("xui")
}

pub fn animatable() -> Result<TokenStream2> {
    resolve("xui-animation")
}
