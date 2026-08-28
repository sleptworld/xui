//! Procedural macros for the `xui` framework.
//!
//! - `xui! { <tag attr={expr}>{children}</tag> }` — the element DSL. It is a
//!   purely syntactic transform into a builder chain; see [`element`] for what
//!   that buys and what it costs.
//! - `style!(padding: 16.0, background: if hovered { .. })` — a `Style` with
//!   state-conditioned rules lowered to `WidgetStateMatcher` at compile time.
//! - `#[component]` / `component_fn!` — props struct, typed builder, render
//!   handle, and the tag constructor that makes a component usable in `xui!`
//!   exactly like a host widget.
//! - `#[main]` — installs the asset bootstrap before the user's `main` runs.
//! - `#[derive(Animatable)]` — field-wise `Animatable` impl.
//!
//! Every runtime path is resolved through [`krate`], so these macros work
//! whether the consumer is `xui` itself or a downstream crate that renamed its
//! dependency.
//!
//! # Design note
//!
//! The macros own the grammar; the runtime owns the vocabulary. No list of tag
//! names, attribute names, or style properties lives in this crate — adding any
//! of those is a change to `xui`/`xui-interface` alone. The one deliberate
//! exception is nothing: where a DSL name has to differ from a runtime method
//! (`position` vs `position_type`), the alias is defined in `xui::dsl`, not
//! here, so that this crate keeps zero special cases to drift out of sync.

mod animatable;
mod component;
mod element;
mod errors;
mod krate;
mod main_fn;
mod style;

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

use crate::component::{ComponentFunction, ComponentFunctions};
use crate::element::Element;
use crate::main_fn::MainFunction;
use crate::style::StyleInput;

/// `xui! { <container padding={16.0}><text>{label}</text></container> }`
#[proc_macro]
pub fn xui(input: TokenStream) -> TokenStream {
    let element = parse_macro_input!(input as Element);
    let expanded = krate::xui().and_then(|xui| element.expand(&xui));
    match expanded {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// `style!(padding: 16.0, color: if hovered { A } else { B })`
#[proc_macro]
pub fn style(input: TokenStream) -> TokenStream {
    let style = parse_macro_input!(input as StyleInput);
    match style::expand_style(&style) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn component(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ComponentFunction);
    if let Err(error) = component::reject_signature_defaults(&function) {
        return error.to_compile_error().into();
    }
    if let Err(error) = component::apply_defaults_attr(&mut function) {
        return error.to_compile_error().into();
    }
    match component::expand_component_function(&mut function) {
        Ok(expanded) => expanded.tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn component_fn(input: TokenStream) -> TokenStream {
    let mut functions = parse_macro_input!(input as ComponentFunctions);
    match component::expand_component_functions(&mut functions) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Consumed by `#[component]`; inert on its own.
#[proc_macro_attribute]
pub fn defaults(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn main(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as MainFunction);
    match main_fn::expand_main_function(function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(Animatable)]
pub fn derive_animatable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match animatable::expand_derive_animatable(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}
