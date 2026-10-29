//! Procedural macros for the `xui` framework.
//!
//! - `#[component]` — a component: run once at mount, its sources and
//!   effects declared by statements, its view (`xui! { .. }` or `view! { .. }`)
//!   as its tail. What each dynamic part of the view reads is worked out here;
//!   see [`reactive`], [`analyze`], and [`element`] / [`view`] for the grammar.
//! - `style!(padding: 16.0, background: if hovered { .. })` — a `Style` with
//!   state-conditioned rules lowered to `WidgetStateMatcher` at compile time.
//! - `#[widget]` — empty marker type, props struct, state struct and typed
//!   props builder, from one struct declaration.
//! - `#[main]` — installs the asset bootstrap before the user's `main` runs.
//! - `#[derive(Animatable)]` — field-wise `Animatable` impl.
//!
//! Every runtime path is resolved through [`krate`], so these macros work
//! whether the consumer is `xui` itself or a downstream crate that renamed its
//! dependency.
//!
//! # Design note
//!
//! The macros own the grammar; the runtime owns the vocabulary. No list of
//! attribute names or style properties lives in this crate — an attribute is
//! looked up in the node's property table. The one list that does live here
//! is the host widgets' tag names ([`hosts`]).

mod analyze;
mod animatable;
mod element;
mod krate;
mod main_fn;
mod reactive;
mod style;
mod hosts;
mod view;
mod widget;

use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput};

use crate::main_fn::MainFunction;
use crate::style::StyleInput;

/// `xui! { <column><text>{label}</text></column> }` — a `#[component]`'s
/// view, written as its tail. Read by `#[component]`; anywhere else it is an
/// error.
#[proc_macro]
pub fn xui(_input: TokenStream) -> TokenStream {
    tail_only("xui")
}

/// `view! { column(gap: 8.0) { text("hi").color(Color::BLUE_500) } }` — the
/// SwiftUI-style spelling of [`xui!`]: `tag(content, name: value) { children }`
/// followed by `.name(value)` modifiers, with `if`, `for` and `match` among
/// children. A `#[component]`'s view, written as its tail.
#[proc_macro]
pub fn view(_input: TokenStream) -> TokenStream {
    tail_only("view")
}

fn tail_only(name: &str) -> TokenStream {
    let message = format!("`{name}!` is a `#[component]`'s view: the tail of its body");
    quote::quote!(::core::compile_error!(#message)).into()
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

/// A component: a function run once, at mount, whose parameters are props,
/// whose statements declare its sources (`state!`, `memo!`, `context!`,
/// `size!`, `ticker!`), effects (`effect!`) and context (`provide!`), and
/// whose tail is its view, `xui! { .. }` or `view! { .. }`. What each part
/// of the view reads is worked out here, at expansion; see `docs/reactive.md`.
#[proc_macro_attribute]
pub fn component(attrs: TokenStream, item: TokenStream) -> TokenStream {
    if !attrs.is_empty() {
        let attrs = proc_macro2::TokenStream::from(attrs);
        return syn::Error::new_spanned(attrs, "`#[component]` takes no options; defaults go in `#[defaults(..)]`")
            .into_compile_error()
            .into();
    }
    let function = parse_macro_input!(item as syn::ItemFn);
    reactive::expand(function).unwrap_or_else(syn::Error::into_compile_error).into()
}

/// Consumed by `#[component]`: `#[defaults(prop = value, ..)]` makes those
/// props optional. Inert on its own.
#[proc_macro_attribute]
pub fn defaults(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Declares a widget from one struct: plain fields are props, `#[state]`
/// fields are state.
///
/// ```ignore
/// #[widget(style(box_size, background))]
/// #[props(derive(Clone, PartialEq))]
/// pub struct Slider {
///     pub value: f32,               // required
///     #[default(1.0)]
///     #[dirty(PAINT_OUTPUT)]
///     pub max: f32,                 // optional, defaults to 1.0
///     #[state]
///     shown: f32,
/// }
/// ```
///
/// generates the `Slider` marker, `SliderProps`, `SliderState`,
/// `SliderProps::builder()`, the node definition (`SliderStyle`,
/// `SliderBehavior`, `NodeDef`, `Selects`) and the `slider` property module;
/// `impl WidgetT for Slider` is written by hand. The `widget` module of this
/// crate documents every option.
#[proc_macro_attribute]
pub fn widget(attrs: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attrs as widget::WidgetArgs);
    let item = parse_macro_input!(item as syn::ItemStruct);
    match widget::expand_widget(args, item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
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
