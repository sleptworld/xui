//! `#[component]` / `component_fn!` — a component as a node: its props, its
//! node definition and property table, one property per prop, and the tag
//! constructor returning a `Draft`, so that `xui!` treats a component exactly
//! like a host widget.
//!
//! For `fn badge(label: &String, tone: &String)` the support module
//! `__xui_badge` holds
//!
//! - `BadgeProps`, the props the render function reads, each a `Prop<T>`;
//! - `Badge`, the node marker, whose `NodeDef::Props` is `BadgePropsDraft`
//!   (each prop optional until set) and whose table is `BadgeProperties`;
//! - one property per prop (`__xui_badge::label`), accepting anything
//!   `Into` the prop's type;
//! - for the props without a default, a requirement state
//!   (`BadgeRequirements<..>`, one `Missing`/`Set` parameter per required
//!   prop) that setting each moves along; only a draft with every required
//!   prop set implements `IntoElement`;
//! - the render function, and the handle and type identifying it.
//!
//! `badge()` is re-exported next to the function's own name, with
//! `BadgeProps` and the render handle.
//!
//! # Generic components
//!
//! A component may take type and const parameters. They go on every generated
//! item — `{Name}Props<P>`, the builder (ahead of its typestate parameters),
//! the tag, call, type and handle functions — together with the `where`
//! clause, plus a `P: 'static` bound per type parameter, since props are
//! stored as `Rc<dyn Any>`. A parameter that no prop names is carried as
//! phantom data.
//!
//! - A prop whose type names a parameter is where that parameter is inferred
//!   from, so its setter takes the field type itself rather than
//!   `impl Into<_>`: `<list items={vec![1, 2]} />` infers `T = i32`.
//! - A parameter that no prop names has to be given explicitly, as
//!   `<marker::<T> />` in `xui!` or `marker::<T>()` on the builder.
//! - Each instantiation is its own component: the `ComponentType` carries the
//!   props `TypeId`, so `list::<A>` replacing `list::<B>` at one position
//!   remounts it instead of handing it the other's props and hook slots.
//! - Lifetime parameters are rejected, as is `impl Trait` in a prop's type —
//!   a struct field cannot hold one; name the parameter instead.

use proc_macro2::{
    Delimiter, Group, Ident as TokenIdent, Span, TokenStream as TokenStream2, TokenTree,
};
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute as SynAttribute, Error, Expr, FnArg, GenericParam, Generics, Ident, Pat, Result,
    ReturnType, Signature, Token, Type, TypeReference, Visibility, WherePredicate, parse_quote,
};

use crate::krate;

pub struct ComponentFunctions {
    functions: Vec<ComponentFunction>,
}

impl Parse for ComponentFunctions {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut functions = Vec::new();
        while !input.is_empty() {
            functions.push(input.parse()?);
        }

        if functions.is_empty() {
            return Err(Error::new(
                input.span(),
                "component_fn! requires at least one function",
            ));
        }

        Ok(Self { functions })
    }
}

pub struct ComponentFunction {
    attrs: Vec<SynAttribute>,
    vis: Visibility,
    sig: Signature,
    input_defaults: Vec<Option<Expr>>,
    body: TokenStream2,
    options: ComponentOptions,
}

/// What `#[component(..)]` is given.
#[derive(Default)]
pub struct ComponentOptions {
    /// Render again only when the props change by value.
    memo: bool,
}

impl Parse for ComponentOptions {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut options = Self::default();
        for option in Punctuated::<Ident, Token![,]>::parse_terminated(input)? {
            if option == "memo" {
                options.memo = true;
            } else {
                return Err(Error::new(
                    option.span(),
                    format!("unknown component option `{option}`; the one option is `memo`"),
                ));
            }
        }
        Ok(options)
    }
}

impl ComponentFunction {
    pub fn set_options(&mut self, options: ComponentOptions) {
        self.options = options;
    }
}

pub struct ExpandedComponentFunction {
    pub tokens: TokenStream2,
}

struct ComponentParam {
    arg: FnArg,
    default: Option<Expr>,
}

struct DefaultsAttr {
    defaults: Vec<(Ident, Expr)>,
}

impl Parse for DefaultsAttr {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut defaults = Vec::new();
        let entries = Punctuated::<ComponentDefaultEntry, Token![,]>::parse_terminated(input)?;
        for entry in entries {
            defaults.push((entry.name, entry.value));
        }
        Ok(Self { defaults })
    }
}

struct ComponentDefaultEntry {
    name: Ident,
    value: Expr,
}

impl Parse for ComponentDefaultEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value = input.parse()?;
        Ok(Self { name, value })
    }
}

impl Parse for ComponentFunction {
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
            options: ComponentOptions::default(),
        })
    }
}

impl Parse for ComponentParam {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let arg = input.parse()?;
        let default = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self { arg, default })
    }
}

pub fn strip_component_param_defaults(
    sig_tokens: TokenStream2,
) -> Result<(TokenStream2, Vec<Option<Expr>>)> {
    let mut output = TokenStream2::new();
    let mut defaults = None;
    // The parameter list is the first parenthesized group outside the generics:
    // `fn f<F: Fn(i32) -> R>(..)` has one inside them. The `>` of a `->` does
    // not close an angle bracket.
    let mut angle_depth = 0usize;
    let mut after_minus = false;
    for token in sig_tokens {
        if let TokenTree::Punct(punct) = &token {
            match punct.as_char() {
                '<' => angle_depth += 1,
                '>' if !after_minus => angle_depth = angle_depth.saturating_sub(1),
                _ => {}
            }
            after_minus = punct.as_char() == '-' && punct.spacing() == proc_macro2::Spacing::Joint;
        } else {
            after_minus = false;
        }
        match token {
            TokenTree::Group(group)
                if group.delimiter() == Delimiter::Parenthesis
                    && angle_depth == 0
                    && defaults.is_none() =>
            {
                let params = Punctuated::<ComponentParam, Token![,]>::parse_terminated
                    .parse2(group.stream())?;
                let mut clean_args = TokenStream2::new();
                for param in params.iter() {
                    let arg = &param.arg;
                    clean_args.extend(quote!(#arg,));
                }
                defaults = Some(
                    params
                        .into_iter()
                        .map(|param| param.default)
                        .collect::<Vec<_>>(),
                );
                let mut clean_group = Group::new(Delimiter::Parenthesis, clean_args);
                clean_group.set_span(group.span());
                output.extend(std::iter::once(TokenTree::Group(clean_group)));
            }
            other => output.extend(std::iter::once(other)),
        }
    }

    let defaults = defaults.ok_or_else(|| {
        Error::new(
            Span::call_site(),
            "component function signature requires an argument list",
        )
    })?;
    Ok((output, defaults))
}

pub fn reject_signature_defaults(function: &ComponentFunction) -> Result<()> {
    if let Some(default) = function
        .input_defaults
        .iter()
        .find_map(|default| default.as_ref())
    {
        return Err(Error::new(
            default.span(),
            "default component parameters in `#[component]` must be declared with `#[defaults(name = expr)]`",
        ));
    }
    Ok(())
}

pub fn apply_defaults_attr(function: &mut ComponentFunction) -> Result<()> {
    let mut defaults = Vec::new();
    let mut attrs = Vec::new();
    let old_attrs = std::mem::take(&mut function.attrs);
    for attr in old_attrs {
        if attr.path().is_ident("defaults") {
            let parsed = attr.parse_args::<DefaultsAttr>()?;
            defaults.extend(parsed.defaults);
        } else {
            attrs.push(attr);
        }
    }
    function.attrs = attrs;

    for (name, value) in defaults {
        let Some(index) = component_param_index(function, &name) else {
            return Err(Error::new(
                name.span(),
                format!("unknown component default parameter `{name}`"),
            ));
        };
        if function.input_defaults[index].is_some() {
            return Err(Error::new(
                name.span(),
                format!("duplicate default value for component parameter `{name}`"),
            ));
        }
        function.input_defaults[index] = Some(value);
    }
    Ok(())
}

fn component_param_index(function: &ComponentFunction, name: &Ident) -> Option<usize> {
    function
        .sig
        .inputs
        .iter()
        .enumerate()
        .find_map(|(index, input)| {
            let FnArg::Typed(arg) = input else {
                return None;
            };
            let Pat::Ident(pat) = arg.pat.as_ref() else {
                return None;
            };
            (pat.ident == *name).then_some(index)
        })
}

pub fn expand_component_functions(functions: &mut ComponentFunctions) -> Result<TokenStream2> {
    let mut output = TokenStream2::new();
    for function in &mut functions.functions {
        let expanded = expand_component_function(function)?;
        output.extend(expanded.tokens);
    }
    Ok(output)
}

pub fn expand_component_function(
    function: &mut ComponentFunction,
) -> Result<ExpandedComponentFunction> {
    let original_name = function.sig.ident.clone();
    let component_name = component_render_name(&original_name);
    let component_type_name = component_type_name(&original_name);
    let component_call_name = component_call_name(&original_name);
    let component_handle_name = component_handle_name(&original_name);
    let module_name = component_module_name(&original_name);
    let props_name = component_props_name(&original_name);
    let xui_core = krate::xui()?;
    function.sig.ident = component_name.clone();
    function.sig.output = ReturnType::Type(
        Default::default(),
        Box::new(parse_quote!(#xui_core::ElementDesc)),
    );
    let generics = ComponentGenerics::new(&mut function.sig.generics)?;

    for input in &function.sig.inputs {
        if let FnArg::Receiver(receiver) = input {
            return Err(Error::new(
                receiver.span(),
                "component functions cannot take self",
            ));
        }
    }

    let has_explicit_cx = function.sig.inputs.first().is_some_and(is_hook_context_arg);

    if has_explicit_cx {
        if function.input_defaults.first().is_some_and(Option::is_some) {
            return Err(Error::new(
                function.sig.inputs.first().span(),
                "cx parameters cannot have default values",
            ));
        }
        let Some(first) = function.sig.inputs.first_mut() else {
            unreachable!("checked first argument");
        };
        if let FnArg::Typed(arg) = first {
            if let Pat::Ident(pat) = arg.pat.as_mut() {
                pat.ident = TokenIdent::new("cx", pat.ident.span());
            }
            *arg.ty = parse_quote!(&mut #xui_core::HookContext<'_>);
        }
    } else {
        function
            .sig
            .inputs
            .insert(0, parse_quote!(cx: &mut #xui_core::HookContext<'_>));
        function.input_defaults.insert(0, None);
    }

    // Items inside the support module are as visible as the component: `pub`
    // would leak a private prop type through a property's `Value`.
    let inner_vis = match &function.vis {
        Visibility::Inherited => quote!(pub(super)),
        Visibility::Public(_) => quote!(pub),
        restricted => quote!(#restricted),
    };
    let props_arg_count = function.sig.inputs.len().saturating_sub(1);
    // Every parameter after `cx` becomes a field of the generated props
    // struct, whatever the count. A generic component always has one, even
    // without parameters, since the props type is what carries its type
    // arguments.
    let props_ty = generics.ty(&props_name, &[]);
    let node = if props_arg_count >= 1 || !generics.is_empty() {
        let generated = generate_component_node(
            function,
            &inner_vis,
            &generics,
            &props_name,
            &xui_core,
            &original_name,
        )?;
        let cx_arg = function
            .sig
            .inputs
            .first()
            .cloned()
            .expect("cx argument is inserted above");
        let mut inputs = Punctuated::new();
        inputs.push(cx_arg);
        inputs.push(parse_quote!(__xui_props: &#props_ty));
        function.sig.inputs = inputs;
        generated
    } else {
        generate_propless_node(&xui_core, &inner_vis, &original_name, function)
    };

    let fn_params = generics.params(&[]);
    let fn_where = generics.where_clause(&[]);
    let turbofish = generics.turbofish();
    let props_type = component_props_type(&function.sig)?;
    // What tells monomorphizations apart in the fiber tree; see `ComponentType`.
    let marker_name = component_marker_name(&original_name);
    let identity_type = if props_type.is_some() {
        props_ty.clone()
    } else {
        quote!(#marker_name)
    };
    let component_call = if let Some(props_type) = props_type {
        quote! {
            fn #component_call_name #fn_params (
                cx: &mut #xui_core::HookContext<'_>,
                props: ::std::option::Option<#xui_core::ErasedPropsRef<'_>>,
            ) -> #xui_core::ElementDesc #fn_where {
                let owner = props.expect("component props missing");
                let props = owner
                    .downcast_ref::<#props_type>()
                    .unwrap_or_else(|| panic!("component props type mismatch"));
                #xui_core::prop::render_with_props::<#props_type, _>(
                    owner,
                    || #component_name(cx, props),
                )
            }
        }
    } else {
        quote! {
            fn #component_call_name(
                cx: &mut #xui_core::HookContext<'_>,
                props: ::std::option::Option<#xui_core::ErasedPropsRef<'_>>,
            ) -> #xui_core::ElementDesc {
                let _ = props;
                #component_name(cx)
            }
        }
    };
    let attrs: Vec<_> = function
        .attrs
        .iter()
        .filter(|attr| !attr.path().is_ident("doc"))
        .collect();
    let vis = &function.vis;
    let sig = &function.sig;
    let body = &function.body;
    let node_tokens = &node.tokens;
    let prop_bindings = &node.bindings;
    // Without props there is nothing to compare: a component without any
    // renders again only on its own updates already.
    let memo = (function.options.memo && node.has_props)
        .then(|| quote!(.memo(__xui_same_props #turbofish)))
        .unwrap_or_default();
    // Without props there is nothing for the fiber to hold.
    let erase = if node.has_props {
        quote! {
            let props: #xui_core::fiber::ErasedProps = ::std::rc::Rc::new(self);
            ::core::option::Option::Some(props)
        }
    } else {
        quote!(::core::option::Option::None)
    };
    // A component with no props has no props struct to surface.
    let props_reexport = node
        .has_props
        .then(|| quote!(#[allow(unused_imports)] #vis use #module_name::#props_name;))
        .unwrap_or_default();

    // The support module holds everything the component needs but nobody
    // names directly: its node definition, property table and properties,
    // the requirement markers, and the erased call adapter. Its items are
    // public, since the tag's signature names them; it is hidden from docs.
    Ok(ExpandedComponentFunction {
        tokens: quote! {
            #[doc(hidden)]
            #[allow(non_snake_case, non_camel_case_types)]
            #vis mod #module_name {
                #[allow(unused_imports)]
                use super::*;

                #node_tokens

                #(#attrs)*
                #[doc(hidden)]
                #inner_vis #sig {
                    #(#prop_bindings)*
                    #body
                }

                #[doc(hidden)]
                #inner_vis const fn #component_type_name #fn_params () -> #xui_core::ComponentType #fn_where {
                    #xui_core::ComponentType::of::<#identity_type>(
                        concat!(module_path!(), "::", stringify!(#original_name)),
                    )
                }

                #component_call

                #[doc(hidden)]
                #inner_vis const fn #component_handle_name #fn_params () -> #xui_core::ComponentRender #fn_where {
                    #xui_core::ComponentRender::new(
                        #component_type_name #turbofish (),
                        #component_call_name #turbofish,
                    )
                    #memo
                }

                impl #fn_params #xui_core::element::ComponentProps for #identity_type #fn_where {
                    const RENDER: #xui_core::ComponentRender = #component_handle_name #turbofish ();

                    fn erase(self) -> ::core::option::Option<#xui_core::fiber::ErasedProps> {
                        #erase
                    }
                }
            }

            #[allow(unused_imports)]
            #[doc(hidden)]
            #vis use #module_name::{#component_handle_name, #component_name};
            #[doc(inline)]
            #[allow(unused_imports)]
            #vis use #module_name::#original_name;
            #props_reexport
        },
    })
}

/// Preserves user-written doc attributes on the DSL entry point. Non-doc
/// attributes remain on the render implementation as before.
fn component_documentation(function: &ComponentFunction, tag: &Ident) -> TokenStream2 {
    let attrs: Vec<_> = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("doc"))
        .collect();
    if attrs.is_empty() {
        let doc = format!("Constructs the `{tag}` component for use with `view!` or `xui!`.");
        quote!(#[doc = #doc])
    } else {
        quote!(#(#attrs)*)
    }
}

struct GeneratedComponentNode {
    tokens: TokenStream2,
    /// `let label = &*__xui_props.label;`, one per prop, at the top of the
    /// render function.
    bindings: Vec<TokenStream2>,
    has_props: bool,
}

/// A component without props or type parameters: a node with `()` props and
/// an empty property table.
fn generate_propless_node(
    xui_core: &TokenStream2,
    vis: &TokenStream2,
    tag_name: &Ident,
    function: &ComponentFunction,
) -> GeneratedComponentNode {
    let marker = component_marker_name(tag_name);
    let table = component_table_name(tag_name);
    let tag_docs = component_documentation(function, tag_name);
    let marker_doc = format!("The `{tag_name}` component, as a node.");
    GeneratedComponentNode {
        tokens: quote! {
            #[doc = #marker_doc]
            #vis struct #marker;

            #[doc(hidden)]
            #[derive(Clone, Copy, Debug, Default)]
            #vis struct #table;

            impl #xui_core::node::NodeDef for #marker {
                type Kind = #xui_core::node::ComponentKind;
                type Props = ();
                type Style = ();
                type Behavior = ();
                type Properties = #table;
            }

            impl #xui_core::dsl::NodeElement<#xui_core::property::Complete, #xui_core::dsl::NoChildren>
                for #marker
            {
                type Output = #xui_core::element::ComponentElement<#marker>;

                fn into_element(
                    draft: #xui_core::node::Draft<#marker>,
                    _: #xui_core::dsl::NoChildren,
                ) -> Self::Output {
                    #xui_core::element::ComponentElement { props: #marker, key: draft.key }
                }
            }

            #tag_docs
            #vis fn #tag_name() -> #xui_core::node::Draft<#marker> {
                #xui_core::node::Draft::new(())
            }
        },
        bindings: Vec::new(),
        has_props: false,
    }
}

fn generate_component_node(
    function: &ComponentFunction,
    vis: &TokenStream2,
    generics: &ComponentGenerics,
    props_name: &TokenIdent,
    xui_core: &TokenStream2,
    tag_name: &Ident,
) -> Result<GeneratedComponentNode> {
    let tag_docs = component_documentation(function, tag_name);
    let marker = component_marker_name(tag_name);
    let table = component_table_name(tag_name);
    let draft_props = TokenIdent::new(&format!("{props_name}Draft"), Span::call_site());
    let requirements = TokenIdent::new(&format!("{marker}Requirements"), Span::call_site());
    let mut props = Vec::new();
    let mut has_children = false;

    for (arg, default) in function
        .sig
        .inputs
        .iter()
        .skip(1)
        .zip(function.input_defaults.iter().skip(1))
    {
        let FnArg::Typed(arg) = arg else {
            return Err(Error::new(arg.span(), "component props cannot be self"));
        };
        let Pat::Ident(pat) = arg.pat.as_ref() else {
            return Err(Error::new(
                arg.pat.span(),
                "named component props require identifier parameters",
            ));
        };
        if pat.by_ref.is_some() || pat.mutability.is_some() {
            return Err(Error::new(
                pat.span(),
                "named component props cannot use ref or mut patterns",
            ));
        }
        let field = &pat.ident;
        if field == "key" {
            return Err(Error::new(
                field.span(),
                "`key` is every element's identity among its siblings, not a prop",
            ));
        }
        // A props field cannot be `impl Trait`, and an anonymous parameter
        // could not be named in the props type anyway.
        let ty = &arg.ty;
        if let Some(impl_token) = find_ident(quote!(#ty), &|ident| ident == "impl") {
            return Err(Error::new(
                impl_token.span(),
                format!(
                    "component prop `{field}` cannot use `impl Trait`: props are stored in a \
                     struct, so declare a named generic parameter instead, as in \
                     `fn {tag_name}<F: Trait>({field}: &F)`"
                ),
            ));
        }
        let Some((field_type, binding)) = component_prop_field_type_and_binding(field, &arg.ty)?
        else {
            return Err(Error::new(
                arg.ty.span(),
                "named component props must be shared references like `name: &Type`",
            ));
        };
        let is_children = field == "children";
        has_children |= is_children;
        let default_value = default
            .as_ref()
            .map(|expr| quote!(#expr))
            .or_else(|| is_children.then(|| quote!(::std::vec::Vec::new())));
        let required = default_value.is_none().then(|| RequiredPropState {
            param: TokenIdent::new(
                &format!("__Xui{}State", ident_pascal_case(field)),
                Span::call_site(),
            ),
            missing: component_prop_state_name(props_name, field, "Missing"),
            set: component_prop_state_name(props_name, field, "Set"),
            required_trait: component_prop_state_name(props_name, field, "RequiredPropIsSet"),
        });

        props.push(ComponentProp {
            field: field.clone(),
            generic: generics.mentioned_by(&field_type),
            field_type,
            default_value,
            required,
            binding,
        });
    }

    // Every item is generic over the component's parameters: a property's
    // scope is the props of one instantiation. A type parameter that a struct
    // does not otherwise name is carried as phantom data; `fn() -> T` keeps it
    // `Send`, `Sync` and covariant whatever `T` is.
    let type_params = &generics.type_params;
    let phantom = quote!(::core::marker::PhantomData<fn() -> (#(#type_params,)*)>);
    let params = generics.params(&[]);
    let args = &generics.args;
    let args = if args.is_empty() {
        quote!()
    } else {
        quote!(<#(#args),*>)
    };
    let user_where = generics.where_clause(&[]);
    let marker_ty = quote!(#marker #args);
    let props_ty = quote!(#props_name #args);
    // The same type in expression position: a parameter that only the
    // phantom field names cannot be inferred from a struct literal.
    let props_expr = if generics.args.is_empty() {
        quote!(#props_name)
    } else {
        let args = &generics.args;
        quote!(#props_name::<#(#args),*>)
    };
    let draft_props_ty = quote!(#draft_props #args);
    let table_ty = quote!(#table #args);
    let property_ty = |field: &Ident| quote!(#field #args);

    // ---- the props, as the component renders them, and as a draft holds them ----
    let phantom_params = type_params
        .iter()
        .filter(|param| {
            !props.iter().any(|prop| {
                let field_type = &prop.field_type;
                tokens_mention(quote!(#field_type), &|ident| ident == *param)
            })
        })
        .collect::<Vec<_>>();
    let (props_phantom_field, props_phantom_value) = if phantom_params.is_empty() {
        (quote!(), quote!())
    } else {
        (
            quote!(__xui_phantom: ::core::marker::PhantomData<fn() -> (#(#phantom_params,)*)>,),
            quote!(__xui_phantom: ::core::marker::PhantomData,),
        )
    };
    let fields = props.iter().map(|prop| {
        let ComponentProp { field, field_type, .. } = prop;
        let doc = match &prop.default_value {
            Some(default) => format!("The `{field}` prop. Defaults to `{default}` when omitted."),
            None => format!("The required `{field}` prop."),
        };
        quote!(#[doc = #doc] pub #field: #xui_core::prop::Prop<#field_type>)
    });
    let draft_fields = props.iter().map(|prop| {
        let ComponentProp { field, field_type, .. } = prop;
        quote!(pub #field: ::core::option::Option<#xui_core::prop::Prop<#field_type>>)
    });
    let draft_defaults = props.iter().map(|prop| {
        let field = &prop.field;
        quote!(#field: ::core::option::Option::None)
    });
    let build_values = props.iter().map(|prop| {
        let field = &prop.field;
        match &prop.default_value {
            Some(default) => quote! {
                #field: draft.#field.unwrap_or_else(|| #xui_core::prop::Prop::new(#default))
            },
            None => quote! {
                #field: draft.#field.expect(concat!(
                    "required component prop `",
                    stringify!(#field),
                    "` was marked as set by its draft's requirement state"
                ))
            },
        }
    });
    let field_names = props.iter().map(|prop| &prop.field).collect::<Vec<_>>();
    // `#[component(memo)]`: props compare field by field, by value. A prop
    // type without `PartialEq` fails to compile at its own parameter.
    let same_props = function.options.memo.then(|| {
        let comparisons = props.iter().map(|prop| {
            let field = &prop.field;
            let field_type = &prop.field_type;
            quote_spanned! {field_type.span()=>
                && ::core::cmp::PartialEq::eq(&*a.#field, &*b.#field)
            }
        });
        quote! {
            #[doc(hidden)]
            fn __xui_same_props #params (
                a: &dyn ::core::any::Any,
                b: &dyn ::core::any::Any,
            ) -> bool #user_where {
                match (a.downcast_ref::<#props_ty>(), b.downcast_ref::<#props_ty>()) {
                    (::core::option::Option::Some(a), ::core::option::Option::Some(b)) => {
                        true #(#comparisons)*
                    }
                    _ => false,
                }
            }
        }
    });
    let prop_fields_impl = quote! {
        impl #params #xui_core::prop::PropFields for #props_ty #user_where {
            const FIELDS: &'static [#xui_core::prop::PropGetter] = &[
                #(
                    |props| &props
                        .downcast_ref::<Self>()
                        .expect("component props type mismatch")
                        .#field_names,
                )*
            ];
        }
    };

    // ---- requirements ----
    let required = props
        .iter()
        .filter_map(|prop| prop.required.as_ref().map(|state| (prop, state)))
        .collect::<Vec<_>>();
    let required_markers = required.iter().map(|(prop, state)| {
        let field = &prop.field;
        let RequiredPropState {
            missing,
            set,
            required_trait,
            ..
        } = state;
        let message = format!("required prop `{field}` of `<{tag_name}>` was never set");
        let label = format!("`{field}` is missing here");
        let note = format!(
            "set it with `{field}={{..}}` on the tag, or give the parameter a default value \
             with `#[defaults({field} = ..)]`"
        );
        quote! {
            #[doc(hidden)]
            #[diagnostic::on_unimplemented(message = #message, label = #label, note = #note)]
            #vis trait #required_trait {}
            #[doc(hidden)]
            #vis struct #missing;
            #[doc(hidden)]
            #vis struct #set;
            impl #required_trait for #set {}
        }
    });
    let state_params = required
        .iter()
        .map(|(_, state)| state.param.clone())
        .collect::<Vec<_>>();
    let (requirements_def, initial_state, complete_bounds, state_ty) = if required.is_empty() {
        (
            quote!(),
            quote!(#xui_core::property::Complete),
            Vec::new(),
            quote!(#xui_core::property::Complete),
        )
    } else {
        let missing = required.iter().map(|(_, state)| &state.missing);
        let doc = format!(
            "Which required props of `<{tag_name}>` a draft has been given, one parameter each."
        );
        let fulfils = required.iter().map(|(prop, state)| {
            let property = property_ty(&prop.field);
            let output = required.iter().map(|(_, other)| {
                if other.param == state.param {
                    other.set.clone()
                } else {
                    other.param.clone()
                }
            });
            let impl_params = generics.params(&state_params);
            quote! {
                impl #impl_params #xui_core::property::Fulfil<#property>
                    for #requirements<#(#state_params),*>
                #user_where
                {
                    type Output = #requirements<#(#output),*>;
                }
            }
        });
        let bounds = required
            .iter()
            .map(|(_, state)| {
                let RequiredPropState {
                    param,
                    required_trait,
                    ..
                } = state;
                quote!(#param: #required_trait)
            })
            .collect::<Vec<_>>();
        (
            quote! {
                #[doc = #doc]
                #vis struct #requirements<#(#state_params),*>(
                    ::core::marker::PhantomData<fn() -> (#(#state_params,)*)>,
                );

                #(#fulfils)*
            },
            quote!(#requirements<#(#missing),*>),
            bounds,
            quote!(#requirements<#(#state_params),*>),
        )
    };
    let impl_params = generics.params(&state_params);
    let complete_where = generics.where_clause(&complete_bounds);

    // ---- properties ----
    let property_defs = props.iter().map(|prop| {
        let field = &prop.field;
        let doc = format!("The `{field}` prop of `<{tag_name}>`.");
        if generics.is_empty() {
            quote!(#[doc = #doc] #vis enum #field {})
        } else {
            // An enum, like every property: a `let` binding of the prop's
            // name in the render function must not read as a pattern.
            quote! {
                #[doc = #doc]
                #vis enum #field #params #user_where {
                    #[doc(hidden)]
                    __Never(::core::convert::Infallible, #phantom),
                }
            }
        }
    });
    let property_impls = props.iter().map(|prop| {
        let ComponentProp {
            field, field_type, ..
        } = prop;
        let property = property_ty(field);
        let requirement = if prop.required.is_some() {
            quote!(Self)
        } else {
            quote!(#xui_core::property::NotRequired)
        };
        // A field whose type names a component parameter is where that
        // parameter is inferred from, so it takes the field type itself: an
        // `Into` blanket would leave the parameter ambiguous as soon as a
        // second `From` impl exists. A primitive takes only itself, so that
        // literals infer.
        let accepts = if prop.generic || crate::widget::is_primitive(field_type) {
            quote! {
                impl #params #xui_core::property::Accepts<#field_type> for #property #user_where {
                    fn accept(value: #field_type) -> #xui_core::prop::Prop<#field_type> {
                        #xui_core::prop::Prop::new(value)
                    }
                }
            }
        } else {
            let value_params = generics.params(&[TokenIdent::new("__XuiValue", Span::call_site())]);
            let value_where =
                generics.where_clause(&[quote!(__XuiValue: ::core::convert::Into<#field_type>)]);
            quote! {
                impl #value_params #xui_core::property::Accepts<__XuiValue> for #property #value_where {
                    fn accept(value: __XuiValue) -> #xui_core::prop::Prop<#field_type> {
                        #xui_core::prop::Prop::new(::core::convert::Into::into(value))
                    }
                }
            }
        };
        quote! {
            impl #params #xui_core::property::Property for #property #user_where {
                type Scope = #xui_core::node::OwnProps<#marker_ty>;
                type Value = #xui_core::prop::Prop<#field_type>;
                type Requirement = #requirement;

                fn set(props: &mut #draft_props_ty, value: #xui_core::prop::Prop<#field_type>) {
                    props.#field = ::core::option::Option::Some(value);
                }
            }

            #accepts
        }
    });
    let table_methods = props.iter().map(|prop| {
        let field = &prop.field;
        let property = property_ty(field);
        quote! {
            #[doc(hidden)]
            #[inline(always)]
            #vis fn #field(&self) -> #xui_core::property::PropertyMarker<#property> {
                #xui_core::property::PropertyMarker::new()
            }
        }
    });

    // ---- elements ----
    // Only a component that declares a `children` prop accepts a body, and
    // that is expressed by *not* implementing the child-bearing
    // `IntoElement`s: `<no_children_component>{x}</no_children_component>` is
    // a type error.
    let take_children = if has_children {
        quote! {
            if let ::core::option::Option::Some(children) = children {
                draft.children = ::core::option::Option::Some(#xui_core::prop::Prop::new(children));
            }
        }
    } else {
        quote!(let _ = children;)
    };
    let children_impls = has_children.then(|| {
        let mut content_params = vec![TokenIdent::new("__XuiChildren", Span::call_site())];
        content_params.extend(state_params.iter().cloned());
        let content_generics = generics.params(&content_params);
        let mut content_bounds = vec![quote!(__XuiChildren: #xui_core::IntoChildren)];
        content_bounds.extend(complete_bounds.iter().cloned());
        let content_where = generics.where_clause(&content_bounds);
        quote! {
            impl #impl_params #xui_core::dsl::NodeElement<#state_ty, #xui_core::dsl::Children>
                for #marker_ty
            #complete_where
            {
                type Output = #xui_core::element::ComponentElement<#props_ty>;

                fn into_element(
                    draft: #xui_core::node::Draft<Self, #state_ty>,
                    children: #xui_core::dsl::Children,
                ) -> Self::Output {
                    __xui_into_element(draft, ::core::option::Option::Some(children.collect()))
                }
            }

            impl #content_generics
                #xui_core::dsl::NodeElement<#state_ty, #xui_core::dsl::Content<__XuiChildren>>
                for #marker_ty
            #content_where
            {
                type Output = #xui_core::element::ComponentElement<#props_ty>;

                fn into_element(
                    draft: #xui_core::node::Draft<Self, #state_ty>,
                    content: #xui_core::dsl::Content<__XuiChildren>,
                ) -> Self::Output {
                    __xui_into_element(draft, ::core::option::Option::Some(content.collect()))
                }
            }
        }
    });
    let into_params = generics.params(&[TokenIdent::new("__XuiState", Span::call_site())]);
    let marker_doc = format!("The `{tag_name}` component, as a node.");
    let props_doc = format!(
        "Owned props for [`{tag_name}()`]. See the component documentation for behavior, \
         defaults, and examples."
    );

    Ok(GeneratedComponentNode {
        tokens: quote! {
            #[doc = #props_doc]
            #vis struct #props_name #params #user_where {
                #(#fields,)*
                #props_phantom_field
            }

            #prop_fields_impl

            #same_props

            #[doc = #marker_doc]
            #vis struct #marker #params (#phantom) #user_where;

            #[doc(hidden)]
            #vis struct #draft_props #params #user_where {
                #(#draft_fields,)*
                __xui_phantom: #phantom,
            }

            impl #params ::core::default::Default for #draft_props_ty #user_where {
                fn default() -> Self {
                    Self {
                        #(#draft_defaults,)*
                        __xui_phantom: ::core::marker::PhantomData,
                    }
                }
            }

            #[doc(hidden)]
            #vis struct #table #params (#phantom) #user_where;

            impl #params ::core::default::Default for #table_ty #user_where {
                fn default() -> Self {
                    Self(::core::marker::PhantomData)
                }
            }

            impl #params #table_ty #user_where {
                #(#table_methods)*
            }

            impl #params #xui_core::node::NodeDef for #marker_ty #user_where {
                type Kind = #xui_core::node::ComponentKind;
                type Props = #draft_props_ty;
                type Style = ();
                type Behavior = ();
                type Properties = #table_ty;
            }

            #(#property_defs)*
            #(#property_impls)*

            #(#required_markers)*
            #requirements_def

            #tag_docs
            #vis fn #tag_name #params () -> #xui_core::node::Draft<#marker_ty, #initial_state> #user_where {
                #xui_core::node::Draft::with_requirements(::core::default::Default::default())
            }

            fn __xui_into_element #into_params (
                draft: #xui_core::node::Draft<#marker_ty, __XuiState>,
                children: ::core::option::Option<::std::vec::Vec<#xui_core::ElementDesc>>,
            ) -> #xui_core::element::ComponentElement<#props_ty> #user_where {
                let #xui_core::node::Draft { key, storage, .. } = draft;
                let mut draft = storage.props;
                #take_children
                let props = #props_expr {
                    #(#build_values,)*
                    #props_phantom_value
                };
                #xui_core::element::ComponentElement { props, key }
            }

            impl #impl_params #xui_core::dsl::NodeElement<#state_ty, #xui_core::dsl::NoChildren>
                for #marker_ty
            #complete_where
            {
                type Output = #xui_core::element::ComponentElement<#props_ty>;

                fn into_element(
                    draft: #xui_core::node::Draft<Self, #state_ty>,
                    _: #xui_core::dsl::NoChildren,
                ) -> Self::Output {
                    __xui_into_element(draft, ::core::option::Option::None)
                }
            }

            #children_impls
        },
        bindings: props.iter().map(|prop| prop.binding.clone()).collect(),
        has_props: true,
    })
}

struct RequiredPropState {
    param: TokenIdent,
    missing: TokenIdent,
    set: TokenIdent,
    required_trait: TokenIdent,
}

struct ComponentProp {
    field: Ident,
    /// The field type names one of the component's generic parameters.
    generic: bool,
    field_type: Type,
    default_value: Option<TokenStream2>,
    required: Option<RequiredPropState>,
    binding: TokenStream2,
}

fn component_prop_field_type_and_binding(
    field: &Ident,
    ty: &Type,
) -> Result<Option<(Type, TokenStream2)>> {
    let Type::Reference(TypeReference {
        mutability: None,
        elem,
        ..
    }) = ty
    else {
        return Ok(None);
    };

    if type_ends_with_ident(elem, "str") {
        return Ok(Some((
            parse_quote!(::std::string::String),
            quote!(let #field: &str = __xui_props.#field.as_str();),
        )));
    }

    let field_type = (**elem).clone();
    Ok(Some((
        field_type,
        quote!(let #field = &*__xui_props.#field;),
    )))
}

fn component_props_type(sig: &Signature) -> Result<Option<Type>> {
    let mut props = sig.inputs.iter().skip(1);
    let Some(arg) = props.next() else {
        return Ok(None);
    };
    if let Some(extra) = props.next() {
        return Err(Error::new(
            extra.span(),
            "component functions support at most one props argument; wrap multiple values in a props struct",
        ));
    }

    let FnArg::Typed(arg) = arg else {
        return Err(Error::new(arg.span(), "component props cannot be self"));
    };
    let Type::Reference(TypeReference {
        mutability: None,
        elem,
        ..
    }) = arg.ty.as_ref()
    else {
        return Err(Error::new(
            arg.ty.span(),
            "component props argument must be a shared reference like `props: &Props`",
        ));
    };
    Ok(Some((**elem).clone()))
}

/// Support module for one component. Everything the component needs but
/// nobody names directly lives here, so a module that re-exports its
/// components with a glob puts a handful of items in scope instead of one per
/// prop plus four free functions.
fn component_module_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(&format!("__xui_{}", original_name), original_name.span())
}

fn component_render_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(
        &format!("{}_component", original_name),
        original_name.span(),
    )
}

fn component_type_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(
        &format!("{}_component_type", original_name),
        original_name.span(),
    )
}

fn component_call_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(
        &format!("{}_component_call", original_name),
        original_name.span(),
    )
}

fn component_handle_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(
        &format!("{}_component_render", original_name),
        original_name.span(),
    )
}

fn component_props_name(original_name: &Ident) -> TokenIdent {
    let mut name = String::new();
    let mut uppercase_next = true;
    for ch in original_name.to_string().chars() {
        if ch == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            name.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            name.push(ch);
        }
    }
    name.push_str("Props");
    TokenIdent::new(&name, original_name.span())
}

/// The component's node marker: `badge` → `Badge`, inside the support module.
fn component_marker_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(&ident_pascal_case(original_name), original_name.span())
}

fn component_table_name(original_name: &Ident) -> TokenIdent {
    TokenIdent::new(
        &format!("{}Properties", ident_pascal_case(original_name)),
        original_name.span(),
    )
}

fn component_prop_state_name(props_name: &TokenIdent, field: &Ident, suffix: &str) -> TokenIdent {
    let field = ident_pascal_case(field);
    TokenIdent::new(&format!("{props_name}{field}{suffix}"), Span::call_site())
}

pub(crate) fn ident_pascal_case(ident: &Ident) -> String {
    let source = ident.to_string();
    let source = source.strip_prefix("r#").unwrap_or(&source);
    let mut output = String::new();
    let mut uppercase_next = true;
    for ch in source.chars() {
        if ch == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }
    if output.is_empty() {
        output.push_str("Prop");
    }
    output
}

/// The component function's own generic parameters, threaded through every
/// generated item ahead of the builder's typestate parameters. Props travel as
/// `Rc<dyn Any>`, so each type parameter is bounded by `'static` and lifetime
/// parameters are rejected.
struct ComponentGenerics {
    /// As declared, bounds kept and defaults dropped.
    params: Vec<GenericParam>,
    /// Every parameter's name, in order, as a generic argument.
    args: Vec<Ident>,
    type_params: Vec<Ident>,
    predicates: Vec<WherePredicate>,
}

impl ComponentGenerics {
    fn new(generics: &mut Generics) -> Result<Self> {
        for param in &mut generics.params {
            match param {
                GenericParam::Lifetime(param) => {
                    return Err(Error::new(
                        param.span(),
                        "component functions cannot take lifetime parameters: props are \
                         stored as `'static` values, so give them owned types",
                    ));
                }
                GenericParam::Type(param) => {
                    param.eq_token = None;
                    param.default = None;
                }
                GenericParam::Const(param) => {
                    param.eq_token = None;
                    param.default = None;
                }
            }
        }
        let type_params = generics
            .type_params()
            .map(|param| param.ident.clone())
            .collect::<Vec<_>>();
        if !type_params.is_empty() {
            let where_clause = generics.make_where_clause();
            for param in &type_params {
                where_clause.predicates.push(parse_quote!(#param: 'static));
            }
        }
        let args = generics
            .params
            .iter()
            .map(|param| match param {
                GenericParam::Type(param) => param.ident.clone(),
                GenericParam::Const(param) => param.ident.clone(),
                GenericParam::Lifetime(_) => unreachable!("rejected above"),
            })
            .collect();
        let predicates = generics
            .where_clause
            .as_ref()
            .map(|clause| clause.predicates.iter().cloned().collect())
            .unwrap_or_default();
        Ok(Self {
            params: generics.params.iter().cloned().collect(),
            args,
            type_params,
            predicates,
        })
    }

    fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// `<user params.., extra..>` for an `impl`, `struct` or `fn`.
    fn params(&self, extra: &[TokenIdent]) -> TokenStream2 {
        let params = &self.params;
        if params.is_empty() && extra.is_empty() {
            quote!()
        } else {
            quote!(<#(#params,)* #(#extra),*>)
        }
    }

    /// `name<user args.., extra..>`.
    fn ty(&self, name: &TokenIdent, extra: &[TokenIdent]) -> TokenStream2 {
        let args = &self.args;
        if args.is_empty() && extra.is_empty() {
            quote!(#name)
        } else {
            quote!(#name<#(#args,)* #(#extra),*>)
        }
    }

    /// `::<user args..>`, to name one monomorphization of a generated fn.
    fn turbofish(&self) -> TokenStream2 {
        let args = &self.args;
        if args.is_empty() {
            quote!()
        } else {
            quote!(::<#(#args),*>)
        }
    }

    fn where_clause(&self, extra: &[TokenStream2]) -> TokenStream2 {
        let predicates = &self.predicates;
        if predicates.is_empty() && extra.is_empty() {
            quote!()
        } else {
            quote!(where #(#predicates,)* #(#extra,)*)
        }
    }

    fn mentioned_by(&self, ty: &Type) -> bool {
        tokens_mention(quote!(#ty), &|ident| self.args.contains(ident))
    }
}

fn tokens_mention(tokens: TokenStream2, matches: &dyn Fn(&Ident) -> bool) -> bool {
    find_ident(tokens, matches).is_some()
}

fn find_ident(tokens: TokenStream2, matches: &dyn Fn(&Ident) -> bool) -> Option<Ident> {
    tokens.into_iter().find_map(|token| match token {
        TokenTree::Ident(ident) => matches(&ident).then_some(ident),
        TokenTree::Group(group) => find_ident(group.stream(), matches),
        _ => None,
    })
}

fn is_hook_context_arg(arg: &FnArg) -> bool {
    let FnArg::Typed(arg) = arg else {
        return false;
    };
    type_ends_with_ident(&arg.ty, "HookContext")
}

fn type_ends_with_ident(ty: &Type, ident: &str) -> bool {
    match ty {
        Type::Reference(reference) => type_ends_with_ident(&reference.elem, ident),
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == ident),
        _ => false,
    }
}
