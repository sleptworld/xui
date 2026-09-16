//! Resolves the runtime crates by their real names, so the macros keep working
//! when a downstream crate renames its dependency, and falls back to the `xui`
//! facade for crates that depend on it alone.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Ident, Span, TokenStream as TokenStream2};
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

/// `xui-core` when it is a direct dependency, otherwise the `xui` facade,
/// which re-exports all of `xui-core`.
pub fn xui() -> Result<TokenStream2> {
    resolve("xui-core").or_else(|core_error| resolve("xui").map_err(|_| core_error))
}

/// `xui-animation` when it is a direct dependency, otherwise the copy the
/// `xui` facade re-exports.
pub fn animatable() -> Result<TokenStream2> {
    resolve("xui-animation").or_else(|animation_error| {
        resolve("xui")
            .map(|xui| quote!(#xui::xui_animation))
            .map_err(|_| animation_error)
    })
}
