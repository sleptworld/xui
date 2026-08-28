//! `#[component]` / `component_fn!` — props struct, typed builder, and the tag
//! constructor that lets `xui!` treat a component exactly like a host widget.

use proc_macro2::{Delimiter, Group, Ident as TokenIdent, Span, TokenStream as TokenStream2, TokenTree};
use quote::quote;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute as SynAttribute, Error, Expr, FnArg, Ident, Pat, Result, ReturnType, Signature,
    Token, Type, TypeReference, Visibility, parse_quote,
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

struct GeneratedComponentProps {
    tokens: TokenStream2,
    bindings: Vec<TokenStream2>,
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
                && group.delimiter() == Delimiter::Brace {
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
    for token in sig_tokens {
        match token {
            TokenTree::Group(group)
                if group.delimiter() == Delimiter::Parenthesis && defaults.is_none() =>
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
    let element_name = component_element_name(&original_name);
    let xui = krate::xui()?;
    function.sig.ident = component_name.clone();
    function.sig.output = ReturnType::Type(
        Default::default(),
        Box::new(parse_quote!(#xui::ElementDesc)),
    );

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
            *arg.ty = parse_quote!(&mut ::xui::HookContext<'_>);
        }
    } else {
        function
            .sig
            .inputs
            .insert(0, parse_quote!(cx: &mut ::xui::HookContext<'_>));
        function.input_defaults.insert(0, None);
    }

    let props_arg_count = function.sig.inputs.len().saturating_sub(1);
    // Every parameter after `cx` becomes a field of the generated props
    // struct, whatever the count. Treating a lone parameter as an
    // already-written props type instead would be indistinguishable from a
    // one-field component, and would leave `{Name}Props` undefined for the
    // `xui!` call site.
    let generated_props = if props_arg_count >= 1 {
        let generated =
            generate_component_props(
                function,
                &props_name,
                &xui,
                &component_handle_name,
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
        inputs.push(parse_quote!(__xui_props: &#props_name));
        function.sig.inputs = inputs;
        Some(generated)
    } else {
        None
    };

    let props_type = component_props_type(&function.sig)?;
    let component_call = if let Some(props_type) = props_type {
        quote! {
            fn #component_call_name(
                cx: &mut ::xui::HookContext<'_>,
                props: ::std::option::Option<#xui::ErasedPropsRef<'_>>,
            ) -> #xui::ElementDesc {
                let props = props
                    .expect("component props missing")
                    .downcast_ref::<#props_type>()
                    .unwrap_or_else(|| panic!("component props type mismatch"));
                #component_name(cx, props)
            }
        }
    } else {
        quote! {
            fn #component_call_name(
                cx: &mut ::xui::HookContext<'_>,
                props: ::std::option::Option<#xui::ErasedPropsRef<'_>>,
            ) -> #xui::ElementDesc {
                let _ = props;
                #component_name(cx)
            }
        }
    };
    let attrs = &function.attrs;
    let vis = &function.vis;
    let sig = &function.sig;
    let body = &function.body;
    let props_tokens = generated_props
        .as_ref()
        .map(|props| props.tokens.clone())
        .unwrap_or_default();
    let prop_bindings = generated_props
        .as_ref()
        .map(|props| props.bindings.as_slice())
        .unwrap_or(&[]);
    // A component with no props has no props struct to surface.
    let props_reexport = generated_props
        .as_ref()
        .map(|_| quote!(#[allow(unused_imports)] #vis use #module_name::#props_name;))
        .unwrap_or_default();

    // `<tag>` resolves to a plain `tag()` call, so a component needs a
    // zero-argument constructor under its own name. With props, the props
    // builder already is that builder; without them there is nothing to build,
    // so a minimal one is generated.
    let element_builder = if generated_props.is_some() {
        // Emitted by `generate_component_props`, which knows the builder's
        // initial typestate.
        quote!()
    } else {
        quote! {
            #[derive(Default)]
            pub struct #element_name {
                __xui_key: ::std::option::Option<#xui::fiber::Key>,
            }

            impl #element_name {
                pub fn key(
                    mut self,
                    key: impl ::std::convert::Into<#xui::fiber::Key>,
                ) -> Self {
                    self.__xui_key = ::std::option::Option::Some(key.into());
                    self
                }
            }

            impl #xui::dsl::IntoElement<#xui::dsl::NoChildren> for #element_name {
                fn into_element(self, _: #xui::dsl::NoChildren) -> #xui::ElementDesc {
                    let element = #xui::component(#component_handle_name());
                    let element = match self.__xui_key {
                        ::std::option::Option::Some(key) => element.key(key),
                        ::std::option::Option::None => element,
                    };
                    ::std::convert::Into::into(element)
                }
            }

            pub fn #original_name() -> #element_name {
                #element_name::default()
            }
        }
    };

    // The typestate markers appear in `Props::builder()`'s signature, so the
    // module has to be reachable or every builder method trips
    // `private_interfaces`. It is hidden from docs instead.
    Ok(ExpandedComponentFunction {
        tokens: quote! {
            #[doc(hidden)]
            #[allow(non_snake_case)]
            #vis mod #module_name {
                #[allow(unused_imports)]
                use super::*;

                #props_tokens

                #(#attrs)*
                pub #sig {
                    #(#prop_bindings)*
                    #body
                }

                pub fn #component_type_name() -> #xui::ComponentType {
                    #xui::ComponentType::new(
                        concat!(module_path!(), "::", stringify!(#original_name)),
                    )
                }

                #component_call

                pub fn #component_handle_name() -> #xui::ComponentRender {
                    #xui::ComponentRender::new(#component_type_name(), #component_call_name)
                }

                #element_builder
            }

            #[allow(unused_imports)]
            #vis use #module_name::{#component_handle_name, #component_name, #original_name};
            #props_reexport
        },
    })
}

fn generate_component_props(
    function: &ComponentFunction,
    props_name: &TokenIdent,
    xui: &TokenStream2,
    handle_name: &TokenIdent,
    tag_name: &Ident,
) -> Result<GeneratedComponentProps> {
    // Unconditionally public: these items live inside the support module, and
    // the facade decides what the outside world can reach.
    let vis = quote!(pub);
    let builder_name = component_props_builder_name(props_name);
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
        let Some((field_type, binding)) = component_prop_field_type_and_binding(field, &arg.ty)?
        else {
            return Err(Error::new(
                arg.ty.span(),
                "named component props must be shared references like `name: &Type`",
            ));
        };
        if field == "children" {
            has_children = true;
        }
        let field_pascal = ident_pascal_case(field);
        let field_state = TokenIdent::new(&format!("__Xui{field_pascal}State"), Span::call_site());
        let field_missing = component_prop_state_name(props_name, field, "Missing");
        let field_set = component_prop_state_name(props_name, field, "Set");
        let field_required_trait =
            component_prop_state_name(props_name, field, "RequiredPropIsSet");
        let is_children = field == "children";
        let default_value = default
            .as_ref()
            .map(|expr| quote!(#expr))
            .or_else(|| is_children.then(|| quote!(::std::vec::Vec::new())));

        let required_state = default_value.is_none().then_some(RequiredPropState {
            param: field_state,
            missing: field_missing,
            set: field_set,
            required_trait: field_required_trait,
        });

        props.push(ComponentProp {
            field: field.clone(),
            field_type,
            default_value,
            required_state,
            binding,
        });
    }

    let fields = props
        .iter()
        .map(|prop| {
            let field = &prop.field;
            let field_type = &prop.field_type;
            quote!(pub #field: #field_type)
        })
        .collect::<Vec<_>>();
    let builder_fields = props
        .iter()
        .map(|prop| {
            let field = &prop.field;
            let field_type = &prop.field_type;
            quote!(#field: ::std::option::Option<#field_type>)
        })
        .collect::<Vec<_>>();
    let builder_init_values = props
        .iter()
        .map(|prop| {
            let field = &prop.field;
            quote!(#field: ::std::option::Option::None)
        })
        .collect::<Vec<_>>();
    let build_values = props
        .iter()
        .map(|prop| {
            let field = &prop.field;
            if let Some(default_value) = &prop.default_value {
                quote!(#field: self.#field.unwrap_or_else(|| #default_value))
            } else {
                quote! {
                    #field: self.#field.expect(concat!(
                        "required component prop `",
                        stringify!(#field),
                        "` was marked as set by its typed builder state"
                    ))
                }
            }
        })
        .collect::<Vec<_>>();
    let bindings = props
        .iter()
        .map(|prop| prop.binding.clone())
        .collect::<Vec<_>>();
    let required_states = props
        .iter()
        .filter_map(|prop| prop.required_state.as_ref())
        .collect::<Vec<_>>();
    let required_markers = required_states
        .iter()
        .zip(props.iter().filter(|prop| prop.required_state.is_some()))
        .map(|(state, prop)| {
            let field = &prop.field;
            let required_trait = &state.required_trait;
            let missing = &state.missing;
            let set = &state.set;
            let missing_message =
                format!("required prop `{field}` of `<{tag_name}>` was never set");
            let missing_label = format!("`{field}` is missing here");
            let missing_note = format!(
                "set it with `{field}={{..}}` on the tag, or give the parameter a \
                 default value with `#[defaults({field} = ..)]`"
            );
            quote! {
                #[doc = "Implemented only when this required component prop has been set on the typed builder."]
                #[diagnostic::on_unimplemented(
                    message = #missing_message,
                    label = #missing_label,
                    note = #missing_note,
                )]
                #vis trait #required_trait {}
                #vis struct #missing;
                #vis struct #set;
                impl #required_trait for #set {}
            }
        })
        .collect::<Vec<_>>();
    let state_params = required_states
        .iter()
        .map(|state| state.param.clone())
        .collect::<Vec<_>>();
    let missing_state_args = required_states
        .iter()
        .map(|state| state.missing.clone())
        .collect::<Vec<_>>();
    let required_trait_bounds = required_states
        .iter()
        .map(|state| {
            let param = &state.param;
            let required_trait = &state.required_trait;
            quote!(#param: #required_trait)
        })
        .collect::<Vec<_>>();
    let builder_generics = generics(&state_params);
    let builder_current_type = builder_type(&builder_name, &state_params);
    let builder_initial_type = builder_type(&builder_name, &missing_state_args);
    let state_phantom_type = phantom_type(&state_params);
    let build_where_clause = (!required_trait_bounds.is_empty()).then(|| {
        quote! {
            where
                #(#required_trait_bounds),*
        }
    });
    let setters = props
        .iter()
        .map(|prop| {
            let field = &prop.field;
            let field_type = &prop.field_type;
            if let Some(required_state) = prop.required_state.as_ref() {
                let result_state_args = required_states
                    .iter()
                    .map(|state| {
                        if state.param == required_state.param {
                            required_state.set.clone()
                        } else {
                            state.param.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                let builder_type = builder_type(&builder_name, &result_state_args);
                let destructure_fields = props
                    .iter()
                    .map(|prop| {
                        let name = &prop.field;
                        if name == field {
                            quote!(#name: _)
                        } else {
                            quote!(#name)
                        }
                    })
                    .collect::<Vec<_>>();
                let reconstruct_fields = props
                    .iter()
                    .map(|prop| {
                        let name = &prop.field;
                        if name == field {
                            quote!(#name: ::std::option::Option::Some(#field.into()))
                        } else {
                            quote!(#name)
                        }
                    })
                    .collect::<Vec<_>>();
                quote! {
                    pub fn #field(self, #field: impl ::std::convert::Into<#field_type>) -> #builder_type {
                        let Self {
                            #(#destructure_fields),*,
                            __xui_key,
                            _states: _,
                        } = self;
                        #builder_name {
                            #(#reconstruct_fields),*,
                            __xui_key,
                            _states: ::std::marker::PhantomData,
                        }
                    }
                }
            } else {
                quote! {
                    pub fn #field(mut self, #field: impl ::std::convert::Into<#field_type>) -> Self {
                        self.#field = ::std::option::Option::Some(#field.into());
                        self
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    // Only a component that declares a `children` prop accepts a body, and that
    // is expressed by *not* implementing the child-bearing `IntoElement`s —
    // `<no_children_component>{x}</no_children_component>` is a type error.
    let take_children = if has_children {
        quote! {
            if let ::std::option::Option::Some(children) = children {
                self.children = ::std::option::Option::Some(children);
            }
        }
    } else {
        quote!(let _ = children;)
    };

    let children_element_impls = has_children.then(|| {
        let mut content_params = vec![TokenIdent::new("__XuiChildren", Span::call_site())];
        content_params.extend(state_params.iter().cloned());
        let content_generics = generics(&content_params);
        let content_where = quote! {
            where
                __XuiChildren: #xui::IntoChildren,
                #(#required_trait_bounds,)*
        };
        quote! {
            impl #builder_generics #xui::dsl::IntoElement<#xui::dsl::Children>
                for #builder_current_type
            #build_where_clause
            {
                fn into_element(self, children: #xui::dsl::Children) -> #xui::ElementDesc {
                    self.__xui_into_element(::std::option::Option::Some(children.collect()))
                }
            }

            impl #content_generics #xui::dsl::IntoElement<#xui::dsl::Content<__XuiChildren>>
                for #builder_current_type
            #content_where
            {
                fn into_element(
                    self,
                    content: #xui::dsl::Content<__XuiChildren>,
                ) -> #xui::ElementDesc {
                    self.__xui_into_element(::std::option::Option::Some(content.collect()))
                }
            }
        }
    });

    let children_impl = has_children.then(|| {
        quote! {
            impl ::xui::WithChildren for #props_name {
                fn with_children(
                    mut self,
                    children: ::std::vec::Vec<::xui::ElementDesc>,
                ) -> Self {
                    self.children = children;
                    self
                }
            }
        }
    });

    Ok(GeneratedComponentProps {
        tokens: quote! {
            #vis struct #props_name {
                #(#fields),*
            }

            #(#required_markers)*

            #vis struct #builder_name #builder_generics {
                #(#builder_fields),*,
                __xui_key: ::std::option::Option<#xui::fiber::Key>,
                _states: ::std::marker::PhantomData<#state_phantom_type>,
            }

            /// `<tag ... />` expands to `tag()`, the same as a host widget.
            pub fn #tag_name() -> #builder_initial_type {
                #props_name::builder()
            }

            impl #props_name {
                pub fn builder() -> #builder_initial_type {
                    #builder_name {
                        #(#builder_init_values),*,
                        __xui_key: ::std::option::Option::None,
                        _states: ::std::marker::PhantomData,
                    }
                }
            }

            impl #builder_generics #builder_current_type {
                #(#setters)*

                /// Available in every builder state: a key never affects
                /// whether the required props are satisfied.
                pub fn key(mut self, key: impl ::std::convert::Into<#xui::fiber::Key>) -> Self {
                    self.__xui_key = ::std::option::Option::Some(key.into());
                    self
                }
            }

            impl #builder_generics #builder_current_type
            #build_where_clause
            {
                pub fn build(self) -> #props_name {
                    #props_name {
                        #(#build_values),*
                    }
                }

                fn __xui_into_element(
                    mut self,
                    children: ::std::option::Option<
                        ::std::vec::Vec<#xui::ElementDesc>,
                    >,
                ) -> #xui::ElementDesc {
                    let key = self.__xui_key.take();
                    #take_children
                    let element = #xui::component(#handle_name()).props(self.build());
                    let element = match key {
                        ::std::option::Option::Some(key) => element.key(key),
                        ::std::option::Option::None => element,
                    };
                    ::std::convert::Into::into(element)
                }
            }

            impl #builder_generics #xui::dsl::IntoElement<#xui::dsl::NoChildren>
                for #builder_current_type
            #build_where_clause
            {
                fn into_element(self, _: #xui::dsl::NoChildren) -> #xui::ElementDesc {
                    self.__xui_into_element(::std::option::Option::None)
                }
            }

            #children_element_impls

            #children_impl
        },
        bindings,
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
    field_type: Type,
    default_value: Option<TokenStream2>,
    required_state: Option<RequiredPropState>,
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
        quote!(let #field = &__xui_props.#field;),
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

fn component_element_name(original_name: &TokenIdent) -> TokenIdent {
    let mut name = ident_pascal_case(original_name);
    name.push_str("Element");
    TokenIdent::new(&name, original_name.span())
}

fn component_props_builder_name(props_name: &TokenIdent) -> TokenIdent {
    TokenIdent::new(&format!("{props_name}Builder"), props_name.span())
}

fn component_prop_state_name(props_name: &TokenIdent, field: &Ident, suffix: &str) -> TokenIdent {
    let field = ident_pascal_case(field);
    TokenIdent::new(&format!("{props_name}{field}{suffix}"), Span::call_site())
}

fn ident_pascal_case(ident: &Ident) -> String {
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

fn generics(params: &[TokenIdent]) -> TokenStream2 {
    if params.is_empty() {
        quote!()
    } else {
        quote!(<#(#params),*>)
    }
}

fn builder_type(builder_name: &TokenIdent, args: &[TokenIdent]) -> TokenStream2 {
    if args.is_empty() {
        quote!(#builder_name)
    } else {
        quote!(#builder_name<#(#args),*>)
    }
}

fn phantom_type(params: &[TokenIdent]) -> TokenStream2 {
    match params {
        [] => quote!(()),
        [param] => quote!(#param),
        _ => quote!((#(#params),*)),
    }
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

