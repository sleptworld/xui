//! `#[widget]` — one struct declaration becomes a widget's empty marker type,
//! its props, its state, a typed props builder, and its node definition: the
//! capabilities it selects and one typed property per prop. Implementing
//! `WidgetT` for the marker is left to the widget's author.
//!
//! ```ignore
//! /// A horizontal slider.
//! #[widget(style(box_size, background), behavior(crate::caps::focus))]
//! #[defaults(max = 1.0)]
//! #[props(derive(Clone, PartialEq))]
//! pub struct Slider {
//!     /// Required: no default.
//!     pub value: f32,
//!     #[default] // `Default::default()`
//!     pub min: f32,
//!     pub max: f32, // defaulted by `#[defaults]` above
//!     #[default("slider".into())]
//!     #[dirty(TEXT_SHAPE | LAYOUT_INPUT)]
//!     pub label: String,
//!
//!     #[state]
//!     dragging: bool,
//!     #[state]
//!     shown: f32,
//! }
//!
//! impl WidgetT for Slider {
//!     type State = SliderState;
//!
//!     fn create_state(props: &SliderProps) -> SliderState {
//!         SliderState { dragging: false, shown: props.value }
//!     }
//! }
//! ```
//!
//! expands to
//!
//! - `pub struct Slider;` — the marker, carrying the struct's docs;
//! - `pub struct SliderProps { pub value, pub min, pub max, pub label }`;
//! - `pub struct SliderState { dragging, shown }`, only when there are
//!   `#[state]` fields (or `#[state(..)]` attributes);
//! - `SliderProps::builder()`, whose `build()` exists only once every required
//!   prop is set, exactly as for `#[component]` props;
//! - `pub struct SliderStyleSet` / `pub struct SliderBehavior`, one field per
//!   selected style / behaviour capability, holding its storage;
//! - `NodeDef for Slider` (a host node) and `Selects<C> for Slider` for each
//!   selected capability `C`;
//! - `pub mod slider`, the node's property namespace: the properties of every
//!   selected capability, re-exported, and one property per prop
//!   (`slider::value`, `slider::label`, ..), each a `Property` and
//!   `HostProperty` over `SliderProps`. `draft.set::<slider::width>(..)` is
//!   how `<slider width={..} />` lowers.
//!
//! Capabilities:
//!
//! - `style(..)` lists style capabilities. A bare name is a module of
//!   `xui_core::caps::style`; a path names any module laid out the same way.
//! - `behavior(..)` lists the other capabilities. A bare name is a module of
//!   `xui_core::caps::behavior`; a path names any module laid out the same
//!   way, as `behavior_capability!` defines them.
//! - A capability module `foo_bar` exports its marker `FooBar`, its `Storage`
//!   and a `props` module; the storage field is named `foo_bar`.
//!
//! Props:
//!
//! - `#[dirty(FLAG | ..)]` names the `WidgetUpdateFlags` a change of the prop
//!   dirties; left out, a change dirties `WidgetUpdateFlags::default()`.
//! - A prop's property accepts anything `Into` its type, except a primitive
//!   one, which accepts only its own type so integer literals infer. Its
//!   change detection is `PartialEq`.
//!
//! Struct-level options:
//!
//! - `#[defaults(name = expr, ..)]` gives props defaults, as on a component;
//!   `#[default]` / `#[default(expr)]` on a field does the same in place.
//! - `#[props(..)]` / `#[state(..)]` attributes go onto the generated props /
//!   state struct (`#[props(derive(Clone))]`). A bare `#[derive]` is rejected,
//!   since it would be ambiguous which struct it meant.
//!
//! Setters take `impl Into<T>`, except for a primitive (`u32`, `f32`, `bool`,
//! ..) prop, whose setter takes the type itself so that integer literals
//! infer. Default expressions are not converted; they are written where the
//! widget is declared and expand there, so they can name anything in that
//! scope. Props are stored as their own type, not as `Prop<T>`: a widget's
//! props are never projected the way a component's are. Generic widgets are
//! not supported yet.

use proc_macro2::{Ident as TokenIdent, Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute, Error, Expr, Fields, Ident, ItemStruct, Meta, Path, Result, Token, Type, Visibility,
};

use crate::component::ident_pascal_case;
use crate::krate;

struct PropField {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    ty: Type,
    default: Option<TokenStream2>,
    /// `#[dirty(..)]`: the flags a change dirties.
    dirty: Option<Vec<Ident>>,
}

struct StateField {
    attrs: Vec<Attribute>,
    vis: Visibility,
    name: Ident,
    ty: Type,
}

pub fn expand_widget(args: WidgetArgs, item: ItemStruct) -> Result<TokenStream2> {
    if let Some(param) = item.generics.params.first() {
        return Err(Error::new(
            param.span(),
            "`#[widget]` does not support generic parameters yet",
        ));
    }
    if let Some(where_clause) = &item.generics.where_clause {
        return Err(Error::new(
            where_clause.span(),
            "`#[widget]` does not support where clauses",
        ));
    }

    let vis = &item.vis;
    let widget_name = &item.ident;
    let props_name = format_ident!("{}Props", widget_name);
    let state_name = format_ident!("{}State", widget_name);
    let builder_name = format_ident!("{}PropsBuilder", widget_name);
    let module_name = format_ident!("__xui_widget_{}", widget_name);

    // Struct-level attributes: docs and anything unrecognized stay on the
    // marker, the rest are sorted onto the struct they name.
    let mut marker_attrs = Vec::new();
    let mut props_attrs = Vec::new();
    let mut state_attrs = Vec::new();
    let mut defaults = Vec::new();
    for attr in &item.attrs {
        let path = attr.path();
        if path.is_ident("defaults") {
            let entries =
                attr.parse_args_with(Punctuated::<DefaultEntry, Token![,]>::parse_terminated)?;
            defaults.extend(entries);
        } else if path.is_ident("props") {
            props_attrs.extend(nested_attrs(attr)?);
        } else if path.is_ident("state") {
            state_attrs.extend(nested_attrs(attr)?);
        } else if path.is_ident("derive") {
            return Err(Error::new(
                attr.span(),
                "`#[widget]` generates a props and a state struct, so say which one a derive \
                 is for: `#[props(derive(..))]` or `#[state(derive(..))]`",
            ));
        } else {
            marker_attrs.push(attr);
        }
    }

    let named = match &item.fields {
        Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
        Fields::Unnamed(fields) => {
            return Err(Error::new(
                fields.span(),
                "`#[widget]` requires named fields: each one is a prop or, with `#[state]`, \
                 a piece of state",
            ));
        }
    };

    let mut props = Vec::new();
    let mut states = Vec::new();
    for field in named {
        let name = field.ident.clone().expect("named field");
        let mut attrs = Vec::new();
        let mut state = None;
        let mut default = None;
        let mut dirty = None;
        for attr in &field.attrs {
            let path = attr.path();
            if path.is_ident("state") {
                if state.is_some() {
                    return Err(Error::new(attr.span(), "duplicate `#[state]`"));
                }
                if !matches!(attr.meta, Meta::Path(_)) {
                    return Err(Error::new(
                        attr.span(),
                        format!(
                            "`#[state]` takes no initial value: build `{name}` in your \
                             `WidgetT::create_state`"
                        ),
                    ));
                }
                state = Some(attr);
            } else if path.is_ident("default") {
                if default.is_some() {
                    return Err(Error::new(attr.span(), "duplicate `#[default]`"));
                }
                default = Some((attr, optional_expr(attr)?));
            } else if path.is_ident("dirty") {
                if dirty.is_some() {
                    return Err(Error::new(attr.span(), "duplicate `#[dirty]`"));
                }
                let flags =
                    attr.parse_args_with(Punctuated::<Ident, Token![|]>::parse_separated_nonempty)?;
                dirty = Some((attr, flags.into_iter().collect::<Vec<_>>()));
            } else {
                attrs.push(attr.clone());
            }
        }
        if let (Some(_), Some((attr, _))) = (&state, &dirty) {
            return Err(Error::new(
                attr.span(),
                format!(
                    "`{name}` is state, not a prop: only a prop has a property whose \
                     change dirties the widget"
                ),
            ));
        }
        let dirty = dirty.map(|(_, flags)| flags);
        match (state, default) {
            (Some(_), Some((attr, _))) => {
                return Err(Error::new(
                    attr.span(),
                    format!(
                        "`{name}` is state, not a prop: it is initialized by \
                         `WidgetT::create_state`, not by a `#[default]`"
                    ),
                ));
            }
            (Some(_), None) => states.push(StateField {
                attrs,
                vis: field.vis.clone(),
                name,
                ty: field.ty.clone(),
            }),
            (None, default) => props.push(PropField {
                attrs,
                vis: field.vis.clone(),
                name,
                ty: field.ty.clone(),
                default: default.map(|(_, expr)| match expr {
                    Some(expr) => quote!(#expr),
                    None => quote!(::core::default::Default::default()),
                }),
                dirty,
            }),
        }
    }

    for DefaultEntry { name, value } in defaults {
        if let Some(prop) = props.iter_mut().find(|prop| prop.name == name) {
            if prop.default.is_some() {
                return Err(Error::new(
                    name.span(),
                    format!("duplicate default value for prop `{name}`"),
                ));
            }
            prop.default = Some(quote!(#value));
        } else if states.iter().any(|state| state.name == name) {
            return Err(Error::new(
                name.span(),
                format!(
                    "`{name}` is state, not a prop: it is initialized by \
                     `WidgetT::create_state`, not by `#[defaults]`"
                ),
            ));
        } else {
            return Err(Error::new(name.span(), format!("unknown prop `{name}`")));
        }
    }

    let builder = generate_builder(
        vis,
        widget_name,
        &props_name,
        &builder_name,
        &module_name,
        &props,
    );

    let prop_fields = props.iter().map(|prop| {
        let PropField {
            attrs,
            vis,
            name,
            ty,
            ..
        } = prop;
        quote!(#(#attrs)* #vis #name: #ty)
    });
    let state_fields = states.iter().map(|state| {
        let StateField {
            attrs,
            vis,
            name,
            ty,
        } = state;
        quote!(#(#attrs)* #vis #name: #ty)
    });

    let node = generate_node(&args, vis, widget_name, &props_name, &props)?;

    let props_doc = format!("Props of the [`{widget_name}`] widget.");
    let state_doc = format!("State of the [`{widget_name}`] widget.");
    // Without `#[state]` fields there is no state struct to generate: the
    // widget's `WidgetT::State` is `()` or a type of its own, which may well
    // be called `FooState`.
    let state = (!states.is_empty() || !state_attrs.is_empty()).then(|| {
        quote! {
            #[doc = #state_doc]
            #(#state_attrs)*
            #vis struct #state_name {
                #(#state_fields,)*
            }
        }
    });
    Ok(quote! {
        #(#marker_attrs)*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        #vis struct #widget_name;

        #[doc = #props_doc]
        #(#props_attrs)*
        #vis struct #props_name {
            #(#prop_fields,)*
        }

        #state

        #builder

        #node
    })
}

/// The props builder: every prop starts unset, a required prop's setter moves
/// its typestate from `Missing` to `Set`, and `build` exists once all of them
/// are `Set` — the same scheme `#[component]` uses for its props.
fn generate_builder(
    vis: &Visibility,
    widget_name: &Ident,
    props_name: &Ident,
    builder_name: &Ident,
    module_name: &Ident,
    props: &[PropField],
) -> TokenStream2 {
    struct Required {
        param: TokenIdent,
        missing: TokenIdent,
        set: TokenIdent,
        is_set: TokenIdent,
    }

    let required = props
        .iter()
        .map(|prop| {
            prop.default.is_none().then(|| {
                let pascal = ident_pascal_case(&prop.name);
                Required {
                    param: TokenIdent::new(&format!("__Xui{pascal}State"), Span::call_site()),
                    missing: format_ident!("{pascal}Missing"),
                    set: format_ident!("{pascal}Set"),
                    is_set: format_ident!("{pascal}IsSet"),
                }
            })
        })
        .collect::<Vec<_>>();
    let required_states = required.iter().flatten().collect::<Vec<_>>();

    let markers = props.iter().zip(&required).filter_map(|(prop, required)| {
        let Required {
            missing,
            set,
            is_set,
            ..
        } = required.as_ref()?;
        let name = &prop.name;
        let message = format!("required prop `{name}` of widget `{widget_name}` was never set");
        let label = format!("`{name}` is missing here");
        let note = format!(
            "set it with `.{name}(..)` before `.build()`, or give it a default with \
             `#[default(..)]` on the field"
        );
        Some(quote! {
            #[diagnostic::on_unimplemented(message = #message, label = #label, note = #note)]
            pub trait #is_set {}
            pub struct #missing;
            pub struct #set;
            impl #is_set for #set {}
        })
    });

    let params = required_states
        .iter()
        .map(|state| &state.param)
        .collect::<Vec<_>>();
    let generics = if params.is_empty() {
        quote!()
    } else {
        quote!(<#(#params),*>)
    };
    let initial_type = if params.is_empty() {
        quote!(#builder_name)
    } else {
        let missing = required_states.iter().map(|state| &state.missing);
        quote!(#builder_name<#(#module_name::#missing),*>)
    };
    let build_where = if params.is_empty() {
        quote!()
    } else {
        let bounds = required_states.iter().map(|state| {
            let param = &state.param;
            let is_set = &state.is_set;
            quote!(#param: #module_name::#is_set)
        });
        quote!(where #(#bounds,)*)
    };

    let fields = props.iter().map(|prop| {
        let name = &prop.name;
        let ty = &prop.ty;
        quote!(#name: ::core::option::Option<#ty>)
    });
    let unset = props.iter().map(|prop| {
        let name = &prop.name;
        quote!(#name: ::core::option::Option::None)
    });
    let names = props.iter().map(|prop| &prop.name).collect::<Vec<_>>();

    let setters = props.iter().zip(&required).map(|(prop, required)| {
        let name = &prop.name;
        let ty = &prop.ty;
        let docs = prop.attrs.iter().filter(|attr| attr.path().is_ident("doc"));
        // `impl Into<u32>` leaves an integer literal ambiguous between every
        // integer type that converts, and rustc falls back to `i32`, so a
        // primitive prop takes its own type.
        let param_type = if is_primitive(ty) {
            quote!(#ty)
        } else {
            quote!(impl ::core::convert::Into<#ty>)
        };
        let Some(required) = required else {
            return quote! {
                #(#docs)*
                pub fn #name(mut self, #name: #param_type) -> Self {
                    self.#name = ::core::option::Option::Some(#name.into());
                    self
                }
            };
        };
        let next_args = required_states.iter().map(|state| {
            if state.param == required.param {
                let set = &state.set;
                quote!(#module_name::#set)
            } else {
                let param = &state.param;
                quote!(#param)
            }
        });
        let others = names.iter().filter(|other| **other != name);
        let others_again = others.clone();
        quote! {
            #(#docs)*
            pub fn #name(
                self,
                #name: #param_type,
            ) -> #builder_name<#(#next_args),*> {
                let Self { #(#others,)* #name: _, _states: _ } = self;
                #builder_name {
                    #(#others_again,)*
                    #name: ::core::option::Option::Some(#name.into()),
                    _states: ::core::marker::PhantomData,
                }
            }
        }
    });

    let build_values = props.iter().map(|prop| {
        let name = &prop.name;
        match &prop.default {
            Some(default) => quote!(#name: self.#name.unwrap_or_else(|| #default)),
            None => quote! {
                #name: self.#name.expect(concat!(
                    "required prop `",
                    stringify!(#name),
                    "` was marked as set by its typed builder state"
                ))
            },
        }
    });

    let state_phantom = match params.as_slice() {
        [] => quote!(()),
        [param] => quote!(#param),
        params => quote!((#(#params),*)),
    };
    let builder_doc = format!(
        "Builds [`{props_name}`]; `build` becomes available once every required prop is set."
    );

    // The typestate markers appear in the builder's type, so their module has
    // to be as visible as the builder; it is hidden from docs instead.
    quote! {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        #vis mod #module_name {
            #(#markers)*
        }

        #[doc = #builder_doc]
        #[must_use = "a props builder does nothing until `build` is called"]
        #vis struct #builder_name #generics {
            #(#fields,)*
            _states: ::core::marker::PhantomData<#state_phantom>,
        }

        impl #props_name {
            pub fn builder() -> #initial_type {
                #builder_name {
                    #(#unset,)*
                    _states: ::core::marker::PhantomData,
                }
            }
        }

        impl #generics #builder_name #generics {
            #(#setters)*

            // The bounds sit on the method rather than the impl so that a
            // missing prop is an unmet bound, which reports the marker's
            // `on_unimplemented` message, not "no method named `build`".
            pub fn build(self) -> #props_name #build_where {
                #props_name { #(#build_values,)* }
            }
        }
    }
}

fn is_primitive(ty: &Type) -> bool {
    const PRIMITIVES: &[&str] = &[
        "bool", "char", "f32", "f64", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
        "u32", "u64", "u128", "usize",
    ];
    matches!(ty, Type::Path(path) if path.qself.is_none()
        && path.path.get_ident().is_some_and(|ident| PRIMITIVES.iter().any(|p| ident == p)))
}

struct DefaultEntry {
    name: Ident,
    value: Expr,
}

impl Parse for DefaultEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        let value = input.parse()?;
        Ok(Self { name, value })
    }
}

/// `#[props(derive(Clone), allow(dead_code))]` → `#[derive(Clone)]`,
/// `#[allow(dead_code)]`.
fn nested_attrs(attr: &Attribute) -> Result<Vec<TokenStream2>> {
    let metas = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
    Ok(metas.into_iter().map(|meta| quote!(#[#meta])).collect())
}

/// `#[name]` → `None`, `#[name(expr)]` → `Some(expr)`.
fn optional_expr(attr: &Attribute) -> Result<Option<Expr>> {
    match &attr.meta {
        Meta::Path(_) => Ok(None),
        Meta::List(_) => attr.parse_args().map(Some),
        Meta::NameValue(meta) => Err(Error::new(
            meta.eq_token.span(),
            format!(
                "write `#[{0}(expr)]` rather than `#[{0} = expr]`",
                attr.path()
                    .get_ident()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            ),
        )),
    }
}

/// `#[widget(style(a, b), behavior(path::to::c))]`.
#[derive(Default)]
pub struct WidgetArgs {
    style: Vec<Path>,
    behavior: Vec<Path>,
    /// Modules of hand-written properties (write-only shorthands) to
    /// re-export into the node module.
    shorthands: Vec<Path>,
}

impl Parse for WidgetArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();
        let mut seen = (false, false, false);
        for meta in Punctuated::<Meta, Token![,]>::parse_terminated(input)? {
            let (list, slot) = if meta.path().is_ident("style") {
                (&mut args.style, &mut seen.0)
            } else if meta.path().is_ident("behavior") {
                (&mut args.behavior, &mut seen.1)
            } else if meta.path().is_ident("shorthands") {
                (&mut args.shorthands, &mut seen.2)
            } else {
                return Err(Error::new(
                    meta.span(),
                    "expected `style(..)` or `behavior(..)`, listing the capabilities the \
                     widget selects, or `shorthands(..)`, listing modules of its own \
                     hand-written properties",
                ));
            };
            if std::mem::replace(slot, true) {
                return Err(Error::new(meta.span(), "duplicate capability list"));
            }
            let Meta::List(meta) = meta else {
                return Err(Error::new(
                    meta.span(),
                    "list the capabilities in parentheses: `style(box_size, padding)`",
                ));
            };
            list.extend(meta.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated)?);
        }
        Ok(args)
    }
}

/// One selected capability, resolved to the paths generated code needs.
struct CapRef {
    /// The capability module, as named from the widget's own scope.
    module: TokenStream2,
    /// The same module, as named from inside the generated node module.
    inner_module: TokenStream2,
    /// The capability's marker type.
    marker: TokenStream2,
    /// The storage field in the style / behaviour struct.
    field: Ident,
    style: bool,
}

fn cap_ref(path: &Path, style: bool, xui: &TokenStream2) -> Result<CapRef> {
    let last = &path.segments.last().expect("a path has a segment").ident;
    let field = last.clone();
    let marker = TokenIdent::new(&ident_pascal_case(last), last.span());

    let (module, inner_module) = if path.get_ident().is_some() {
        let module = if style {
            quote_spanned!(last.span()=> #xui::caps::style::#last)
        } else {
            quote_spanned!(last.span()=> #xui::caps::behavior::#last)
        };
        (module.clone(), module)
    } else {
        (quote!(#path), inner_path(path))
    };
    let marker = quote_spanned!(last.span()=> #module::#marker);
    Ok(CapRef {
        module,
        inner_module,
        marker,
        field,
        style,
    })
}

/// `path`, as named from inside the generated node module: a relative path
/// is one level further out.
fn inner_path(path: &Path) -> TokenStream2 {
    let first = &path.segments[0].ident;
    let absolute = path.leading_colon.is_some() || first == "crate";
    if absolute {
        quote!(#path)
    } else if first == "self" {
        let rest = path.segments.iter().skip(1);
        quote!(super #(:: #rest)*)
    } else {
        quote!(super::#path)
    }
}

/// The node half of the expansion: capability storage, `NodeDef`, `Selects`,
/// and the property namespace module.
fn generate_node(
    args: &WidgetArgs,
    vis: &Visibility,
    widget_name: &Ident,
    props_name: &Ident,
    props: &[PropField],
) -> Result<TokenStream2> {
    let xui = krate::xui()?;
    let caps = args
        .style
        .iter()
        .map(|path| cap_ref(path, true, &xui))
        .chain(args.behavior.iter().map(|path| cap_ref(path, false, &xui)))
        .collect::<Result<Vec<_>>>()?;
    for (index, cap) in caps.iter().enumerate() {
        if let Some(earlier) = caps[..index].iter().find(|other| other.field == cap.field) {
            return Err(Error::new(
                cap.field.span(),
                format!(
                    "capability `{}` is selected twice (or two capability modules share the \
                     name, which their storage fields would too)",
                    earlier.field
                ),
            ));
        }
    }
    let style_caps = caps.iter().filter(|cap| cap.style).collect::<Vec<_>>();
    let behavior_caps = caps.iter().filter(|cap| !cap.style).collect::<Vec<_>>();

    let module_name = TokenIdent::new(&ident_snake_case(widget_name), widget_name.span());
    let style_name = format_ident!("{}StyleSet", widget_name);
    let behavior_name = format_ident!("{}Behavior", widget_name);
    let style_capability = quote!(#xui::capability::StyleCapability);
    let computed = quote!(#xui::__interface::ComputedStyle);
    let theme = quote!(#xui::__interface::Theme);

    // ---- storage structs ----
    let storage_fields = |caps: &[&CapRef]| {
        caps.iter()
            .map(|cap| {
                let CapRef { module, field, .. } = cap;
                quote!(pub #field: #module::Storage)
            })
            .collect::<Vec<_>>()
    };
    let style_fields = storage_fields(&style_caps);
    let behavior_fields = storage_fields(&behavior_caps);
    let style_doc = format!("The style capabilities [`{widget_name}`] selects.");
    let behavior_doc = format!("The behaviour capabilities [`{widget_name}`] selects.");

    let each_style = |body: &dyn Fn(&CapRef) -> TokenStream2| {
        style_caps.iter().map(|cap| body(cap)).collect::<Vec<_>>()
    };
    let merges = each_style(
        &|CapRef { marker, field, .. }| quote!(<#marker as #style_capability>::merge(&mut self.#field, &other.#field);),
    );
    let applies = each_style(
        &|CapRef { marker, field, .. }| quote!(<#marker as #style_capability>::apply(&self.#field, target, parent, initial, theme);),
    );
    let inherits = each_style(
        &|CapRef { marker, field, .. }| quote!(|| <#marker as #style_capability>::inherits_differently(&self.#field, current, parent)),
    );
    let unsets = each_style(
        &|CapRef { marker, field, .. }| quote!(<#marker as #style_capability>::inherit_unset(&self.#field, target, parent);),
    );
    let style_has = each_style(&|CapRef { marker, field, .. }| {
        quote! {
            impl #xui::property::Has<#marker> for #style_name {
                fn get(&self) -> &<#marker as #xui::property::Capability>::Storage {
                    &self.#field
                }
                fn get_mut(&mut self) -> &mut <#marker as #xui::property::Capability>::Storage {
                    &mut self.#field
                }
            }
        }
    });

    // The runtime reads a node's transition; only `motion` holds one.
    let transition = behavior_caps
        .iter()
        .find(|cap| cap.field == "motion")
        .map(|cap| {
            let field = &cap.field;
            quote! {
                fn transition(
                    node: &#xui::node::NodeStorage<Self>,
                ) -> ::core::option::Option<#xui::__interface::Transition> {
                    node.behavior.#field.transition
                }
            }
        });

    let selects = caps.iter().map(|cap| {
        let CapRef { marker, field, .. } = cap;
        let part = if cap.style {
            quote!(style)
        } else {
            quote!(behavior)
        };
        quote! {
            impl #xui::node::Selects<#marker> for #widget_name {
                fn storage(
                    node: &#xui::node::NodeStorage<Self>,
                ) -> &<#marker as #xui::property::Capability>::Storage {
                    &node.#part.#field
                }
                fn storage_mut(
                    node: &mut #xui::node::NodeStorage<Self>,
                ) -> &mut <#marker as #xui::property::Capability>::Storage {
                    &mut node.#part.#field
                }
            }
        }
    });

    // ---- the node as the runtime keeps it ----
    let flags = quote!(#xui::__interface::WidgetUpdateFlags);
    let markers = caps.iter().map(|cap| &cap.marker);
    let layer_vars = style_caps
        .iter()
        .map(|cap| format_ident!("__xui_layers_{}", cap.field))
        .collect::<Vec<_>>();
    let layer_inits = style_caps.iter().zip(&layer_vars).map(|(cap, var)| {
        let marker = &cap.marker;
        quote! {
            let mut #var: #xui::caps::Layers<<#marker as #xui::property::Capability>::Storage> =
                #xui::caps::Layers::new();
        }
    });
    let layer_pushes = style_caps.iter().zip(&layer_vars).map(|(cap, var)| {
        let field = &cap.field;
        quote! {
            if layer.#field != ::core::default::Default::default() {
                #var.push((matcher, layer.#field));
            }
        }
    });
    let style_writes = style_caps.iter().zip(&layer_vars).map(|(cap, var)| {
        let CapRef { marker, field, .. } = cap;
        quote! {
            dirty |= <#marker as #xui::caps::Registered>::write(columns, id, style.#field, cx);
            if <#marker as #xui::caps::RegisteredStyle>::write_layers(columns, id, #var) {
                dirty |= #flags::STYLE_TARGET;
            }
            let frames = keyframes.as_mut().and_then(|keyframes| {
                let from = ::core::mem::take(&mut keyframes.from.0.#field);
                let to = ::core::mem::take(&mut keyframes.to.0.#field);
                let unset = <<#marker as #xui::property::Capability>::Storage as ::core::default::Default>::default();
                (from != unset || to != unset).then_some((from, to))
            });
            if <#marker as #xui::caps::RegisteredStyle>::write_keyframes(columns, id, frames) {
                dirty |= #flags::STYLE_TARGET;
            }
        }
    });
    let behavior_writes = behavior_caps.iter().map(|cap| {
        let CapRef { marker, field, .. } = cap;
        quote! {
            dirty |= <#marker as #xui::caps::Registered>::write(columns, id, behavior.#field, cx);
        }
    });
    let prop_diffs = props.iter().map(|prop| {
        let name = &prop.name;
        let property = quote!(#module_name::#name);
        quote! {
            if !<#property as #xui::property::FieldProperty>::same(&old.#name, &new.#name) {
                dirty |= <#property as #xui::property::HostProperty>::dirty(&old.#name, &new.#name);
            }
        }
    });
    let host_node = quote! {
        impl #xui::node::HostNode for #widget_name {
            const CAPS: #xui::caps::CapMask = #xui::caps::CapMask::EMPTY #(.with::<#markers>())*;

            #[allow(unused_variables, unused_mut)]
            fn scatter(
                draft: #xui::node::Draft<Self>,
                id: #xui::__interface::NodeId,
                columns: &mut #xui::caps::Columns,
                cx: &mut #xui::capability::CommitCx<'_>,
            ) -> (#props_name, #flags) {
                let #xui::node::Draft { storage, rules, mut keyframes, .. } = draft;
                let #xui::node::NodeStorage { props, style, behavior } = storage;
                let mut dirty = <#flags>::empty();
                #(#layer_inits)*
                for rule in rules {
                    let matcher = rule.matcher;
                    let layer = rule.style.0;
                    #(#layer_pushes)*
                }
                #(#style_writes)*
                #(#behavior_writes)*
                (props, dirty)
            }

            #[allow(unused_variables, unused_mut)]
            fn props_dirty(old: &#props_name, new: &#props_name) -> #flags {
                let mut dirty = <#flags>::empty();
                #(#prop_diffs)*
                dirty
            }
        }
    };

    // ---- own props ----
    // Items inside the node module are as visible as the widget. `pub` would
    // be too much for a private widget: its properties' impls name the
    // private marker, which a `pub` enum would leak.
    let inner_vis = match vis {
        Visibility::Inherited => quote!(pub(super)),
        Visibility::Public(_) => quote!(pub),
        // `pub(crate)`, `pub(in ..)`: as visible as the widget, from anywhere.
        restricted => quote!(#restricted),
    };
    let prop_enums = props.iter().map(|prop| {
        let name = &prop.name;
        let docs = prop.attrs.iter().filter(|attr| attr.path().is_ident("doc"));
        quote! {
            #(#docs)*
            #inner_vis enum #name {}
        }
    });
    let prop_impls = props.iter().map(|prop| {
        let PropField { name, ty, .. } = prop;
        let dirty = match &prop.dirty {
            Some(flags) => {
                let flags = flags
                    .iter()
                    .map(|flag| quote_spanned!(flag.span()=> #xui::__interface::WidgetUpdateFlags::#flag));
                quote!(#(#flags)|*)
            }
            None => quote!(<#xui::__interface::WidgetUpdateFlags as ::core::default::Default>::default()),
        };
        let accepts = if is_primitive(ty) {
            quote! {
                impl #xui::property::Accepts<#ty> for #module_name::#name {
                    fn accept(value: #ty) -> #ty {
                        value
                    }
                }
            }
        } else {
            quote! {
                impl<__XuiValue: ::core::convert::Into<#ty>>
                    #xui::property::Accepts<__XuiValue> for #module_name::#name
                {
                    fn accept(value: __XuiValue) -> #ty {
                        ::core::convert::Into::into(value)
                    }
                }
            }
        };
        quote! {
            impl #xui::property::Property for #module_name::#name {
                type Scope = #xui::node::OwnProps<#widget_name>;
                type Value = #ty;

                fn set(props: &mut #props_name, value: #ty) {
                    props.#name = value;
                }
            }

            impl #xui::property::FieldProperty for #module_name::#name {
                fn get(props: &#props_name) -> &#ty {
                    &props.#name
                }

                fn same(a: &#ty, b: &#ty) -> bool {
                    a == b
                }
            }

            impl #xui::property::HostProperty for #module_name::#name {
                fn dirty(_: &#ty, _: &#ty) -> #xui::__interface::WidgetUpdateFlags {
                    #dirty
                }
            }

            #accepts
        }
    });

    let reexports = caps.iter().map(|cap| {
        let inner = &cap.inner_module;
        quote!(#inner_vis use #inner::props::*;)
    });
    let shorthand_reexports = args.shorthands.iter().map(|path| {
        let inner = inner_path(path);
        quote!(#inner_vis use #inner::*;)
    });
    let module_doc = format!(
        "The properties of [`{widget_name}`](super::{widget_name}): its own props and those \
         of every capability it selects."
    );

    Ok(quote! {
        #[doc = #style_doc]
        #[derive(Debug, Clone, PartialEq, Default)]
        #vis struct #style_name {
            #(#style_fields,)*
        }

        #[doc = #behavior_doc]
        #[derive(Clone, Default)]
        #vis struct #behavior_name {
            #(#behavior_fields,)*
        }

        impl #xui::capability::StyleSet for #style_name {
            #[allow(unused_variables)]
            fn merge(&mut self, other: &Self) {
                #(#merges)*
            }

            #[allow(unused_variables)]
            fn apply(
                &self,
                target: &mut #computed,
                parent: &#computed,
                initial: &#computed,
                theme: &#theme,
            ) {
                #(#applies)*
            }

            #[allow(unused_variables)]
            fn inherits_differently(&self, current: &#computed, parent: &#computed) -> bool {
                false #(#inherits)*
            }

            #[allow(unused_variables)]
            fn inherit_unset(&self, target: &mut #computed, parent: &#computed) {
                #(#unsets)*
            }
        }

        #(#style_has)*

        impl #xui::node::NodeDef for #widget_name {
            type Kind = #xui::node::HostKind;
            type Props = #props_name;
            type Style = #style_name;
            type Behavior = #behavior_name;

            #transition
        }

        #(#selects)*

        #host_node

        #[doc = #module_doc]
        #[allow(non_camel_case_types, unused_imports)]
        #vis mod #module_name {
            #(#reexports)*
            #(#shorthand_reexports)*
            #(#prop_enums)*
        }

        #(#prop_impls)*
    })
}

/// `FlexBox` → `flex_box`.
fn ident_snake_case(ident: &Ident) -> String {
    let source = ident.to_string();
    let source = source.strip_prefix("r#").unwrap_or(&source);
    let mut output = String::new();
    for (index, ch) in source.chars().enumerate() {
        if ch.is_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.extend(ch.to_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}
