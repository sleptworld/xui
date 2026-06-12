mod tools;
use proc_macro::TokenStream;
use proc_macro2::{
    Delimiter, Group, Ident as TokenIdent, Span, TokenStream as TokenStream2, TokenTree,
};
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{quote, ToTokens};
use syn::parse::Parser;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    braced, parse_macro_input, parse_quote, Attribute as SynAttribute, Data, DeriveInput, Error,
    Expr, Fields, FnArg, Ident, LitStr, Pat, Result, ReturnType, Signature, Token, Type,
    TypeReference, Visibility,
};

use crate::tools::{
    event_attr_stmt, parse_attrs_helper, parse_base_attr, parse_layout_style_attr,
    parse_paint_style_attr, parse_scroll_style_attr, parse_stack_attr, parse_text_style_attr,
    unsupported_attr,
};

#[proc_macro]
pub fn xui(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as ElementNode);
    match expand_node(&root) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn component(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(item as ComponentFunction);
    if let Err(error) = reject_signature_defaults(&function) {
        return error.to_compile_error().into();
    }
    if let Err(error) = apply_defaults_attr(&mut function) {
        return error.to_compile_error().into();
    }
    match expand_component_function(&mut function) {
        Ok(expanded) => expanded.tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn defaults(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro]
pub fn component_fn(input: TokenStream) -> TokenStream {
    let mut functions = parse_macro_input!(input as ComponentFunctions);
    match expand_component_functions(&mut functions) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_derive(Animatable)]
pub fn derive_animatable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_derive_animatable(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_derive_animatable(input: &DeriveInput) -> Result<TokenStream2> {
    let Data::Struct(data) = &input.data else {
        return Err(Error::new(
            input.span(),
            "Animatable can only be derived for structs",
        ));
    };

    let animatable_path = animatable_crate_path()?;
    let field_types = data
        .fields
        .iter()
        .map(|field| field.ty.clone())
        .collect::<Vec<_>>();

    let mut generics = input.generics.clone();
    if !field_types.is_empty() {
        let where_clause = generics.make_where_clause();
        for field_type in &field_types {
            where_clause
                .predicates
                .push(parse_quote!(#field_type: #animatable_path::Animatable));
        }
    }

    let ident = &input.ident;
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let body = expand_animatable_struct_body(&data.fields, &animatable_path)?;

    Ok(quote! {
        impl #impl_generics #animatable_path::Animatable for #ident #type_generics #where_clause {
            fn interpolate(from: &Self, to: &Self, progress: f32) -> Self {
                #body
            }
        }
    })
}

fn animatable_crate_path() -> Result<TokenStream2> {
    match crate_name("xui-animation") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = TokenIdent::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(Error::new(
            Span::call_site(),
            format!("failed to find xui-animation dependency: {error}"),
        )),
    }
}

fn expand_animatable_struct_body(
    fields: &Fields,
    animatable_path: &TokenStream2,
) -> Result<TokenStream2> {
    match fields {
        Fields::Named(fields) => {
            let field_values = fields
                .named
                .iter()
                .map(|field| {
                    let ident = field
                        .ident
                        .as_ref()
                        .expect("named fields always have identifiers");
                    let ty = &field.ty;
                    quote! {
                        #ident: <#ty as #animatable_path::Animatable>::interpolate(
                            &from.#ident,
                            &to.#ident,
                            progress,
                        )
                    }
                })
                .collect::<Vec<_>>();

            Ok(quote! {
                Self {
                    #(#field_values),*
                }
            })
        }
        Fields::Unnamed(fields) => {
            let field_values = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    let index = syn::Index::from(index);
                    let ty = &field.ty;
                    quote! {
                        <#ty as #animatable_path::Animatable>::interpolate(
                            &from.#index,
                            &to.#index,
                            progress,
                        )
                    }
                })
                .collect::<Vec<_>>();

            Ok(quote! {
                Self(#(#field_values),*)
            })
        }
        Fields::Unit => Ok(quote!(Self)),
    }
}

struct ComponentFunctions {
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

struct ComponentFunction {
    attrs: Vec<SynAttribute>,
    vis: Visibility,
    sig: Signature,
    input_defaults: Vec<Option<Expr>>,
    body: TokenStream2,
}

struct ExpandedComponentFunction {
    tokens: TokenStream2,
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
            if let TokenTree::Group(group) = &token {
                if group.delimiter() == Delimiter::Brace {
                    body = Some(group.stream());
                    break;
                }
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

fn strip_component_param_defaults(
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

fn reject_signature_defaults(function: &ComponentFunction) -> Result<()> {
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

fn apply_defaults_attr(function: &mut ComponentFunction) -> Result<()> {
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

fn expand_component_functions(functions: &mut ComponentFunctions) -> Result<TokenStream2> {
    let mut output = TokenStream2::new();
    for function in &mut functions.functions {
        let expanded = expand_component_function(function)?;
        output.extend(expanded.tokens);
    }
    Ok(output)
}

fn expand_component_function(
    function: &mut ComponentFunction,
) -> Result<ExpandedComponentFunction> {
    let original_name = function.sig.ident.clone();
    let component_name = component_render_name(&original_name);
    let component_type_name = component_type_name(&original_name);
    let component_call_name = component_call_name(&original_name);
    let component_handle_name = component_handle_name(&original_name);
    let props_name = component_props_name(&original_name);
    function.sig.ident = component_name.clone();
    function.sig.output = ReturnType::Type(
        Default::default(),
        Box::new(parse_quote!(::xui::ElementDesc)),
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
            arg.ty = Box::new(parse_quote!(&mut ::xui::HookContext<'_>));
        }
    } else {
        function
            .sig
            .inputs
            .insert(0, parse_quote!(cx: &mut ::xui::HookContext<'_>));
        function.input_defaults.insert(0, None);
    }

    let props_arg_count = function.sig.inputs.len().saturating_sub(1);
    let generated_props = if props_arg_count > 1 {
        let generated = generate_component_props(function, &props_name)?;
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
        if let Some(default) = function
            .input_defaults
            .iter()
            .skip(1)
            .find_map(|default| default.as_ref())
        {
            return Err(Error::new(
                default.span(),
                "default component parameters require at least two props parameters",
            ));
        }
        None
    };

    let props_type = component_props_type(&function.sig)?;
    let component_call = if let Some(props_type) = props_type {
        quote! {
            fn #component_call_name(
                cx: &mut ::xui::HookContext<'_>,
                props: ::std::option::Option<::xui::ErasedPropsRef<'_>>,
            ) -> ::xui::ElementDesc {
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
                props: ::std::option::Option<::xui::ErasedPropsRef<'_>>,
            ) -> ::xui::ElementDesc {
                let _ = props;
                #component_name(cx)
            }
        }
    };
    let attrs = &function.attrs;
    let vis = &function.vis;
    let sig = &function.sig;
    let body = expand_component_body(&function.body)?;
    let props_tokens = generated_props
        .as_ref()
        .map(|props| props.tokens.clone())
        .unwrap_or_default();
    let prop_bindings = generated_props
        .as_ref()
        .map(|props| props.bindings.as_slice())
        .unwrap_or(&[]);

    Ok(ExpandedComponentFunction {
        tokens: quote! {
            #props_tokens

            #(#attrs)*
            #vis #sig {
                #(#prop_bindings)*
                #body
            }

            #vis fn #component_type_name() -> ::xui::ComponentType {
                ::xui::ComponentType::new(concat!(module_path!(), "::", stringify!(#original_name)))
            }

            #component_call

            #vis fn #component_handle_name() -> ::xui::ComponentRender {
                ::xui::ComponentRender::new(#component_type_name(), #component_call_name)
            }
        },
    })
}

fn generate_component_props(
    function: &ComponentFunction,
    props_name: &TokenIdent,
) -> Result<GeneratedComponentProps> {
    let vis = &function.vis;
    let mut fields = Vec::new();
    let mut default_values = Vec::new();
    let mut setters = Vec::new();
    let mut bindings = Vec::new();
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
        let default_value = default
            .as_ref()
            .map(|expr| quote!(#expr))
            .unwrap_or_else(|| quote!(::std::default::Default::default()));
        fields.push(quote!(pub #field: #field_type));
        default_values.push(quote!(#field: #default_value));
        setters.push(quote! {
            pub fn #field(mut self, #field: impl ::std::convert::Into<#field_type>) -> Self {
                self.#field = #field.into();
                self
            }
        });
        bindings.push(binding);
    }

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
            #[derive(Hash)]
            #vis struct #props_name {
                #(#fields),*
            }

            impl ::std::default::Default for #props_name {
                fn default() -> Self {
                    Self {
                        #(#default_values),*
                    }
                }
            }

            impl #props_name {
                #(#setters)*
            }

            #children_impl
        },
        bindings,
    })
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

fn expand_component_body(body: &TokenStream2) -> Result<TokenStream2> {
    let tokens: Vec<_> = body.clone().into_iter().collect();
    let Some(xml_start) = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == '<'))
    else {
        return Ok(body.clone());
    };

    let prefix = tokens[..xml_start]
        .iter()
        .cloned()
        .collect::<TokenStream2>();
    let xml = tokens[xml_start..]
        .iter()
        .cloned()
        .collect::<TokenStream2>();
    let node = syn::parse2::<ElementNode>(xml)?;
    let element = expand_node(&node)?;

    Ok(quote! {
        #prefix
        #element
    })
}

struct ElementNode {
    name: Ident,
    attrs: Vec<XuiAttribute>,
    children: Vec<Child>,
}

struct XuiAttribute {
    name: Ident,
    value: TokenStream2,
}

enum Child {
    Element(ElementNode),
    Expr(Expr),
}

impl Parse for ElementNode {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse::<Token![<]>()?;
        let name: Ident = input.parse()?;
        let attrs = parse_attrs(input)?;

        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(Self {
                name,
                attrs,
                children: Vec::new(),
            });
        }

        input.parse::<Token![>]>()?;
        let mut children = Vec::new();

        loop {
            if input.is_empty() {
                return Err(Error::new(name.span(), "missing closing tag"));
            }

            if starts_closing_tag(input) {
                input.parse::<Token![<]>()?;
                input.parse::<Token![/]>()?;
                let close_name: Ident = input.parse()?;
                input.parse::<Token![>]>()?;
                if close_name != name {
                    return Err(Error::new(
                        close_name.span(),
                        format!("expected closing tag </{}>", name),
                    ));
                }
                break;
            }

            if input.peek(Token![<]) {
                children.push(Child::Element(input.parse()?));
            } else if input.peek(syn::token::Brace) {
                let content;
                braced!(content in input);
                children.push(Child::Expr(content.parse()?));
            } else {
                return Err(Error::new(
                    input.span(),
                    "children must be nested tags or braced Rust expressions",
                ));
            }
        }

        Ok(Self {
            name,
            attrs,
            children,
        })
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

fn parse_attrs(input: ParseStream<'_>) -> Result<Vec<XuiAttribute>> {
    let mut attrs = Vec::new();
    while !(input.peek(Token![>]) || input.peek(Token![/])) {
        let name: Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value = if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            let expr: Expr = content.parse()?;
            expr.into_token_stream()
        } else {
            let literal: LitStr = input.parse()?;
            literal.into_token_stream()
        };
        attrs.push(XuiAttribute { name, value });
    }
    Ok(attrs)
}

fn starts_closing_tag(input: ParseStream<'_>) -> bool {
    let fork = input.fork();
    fork.parse::<Token![<]>().is_ok() && fork.parse::<Token![/]>().is_ok()
}

fn expand_node(node: &ElementNode) -> Result<TokenStream2> {
    match node.name.to_string().as_str() {
        "label" => expand_label(node),
        "text" => expand_text(node),
        "button" => expand_button(node),
        "column" => expand_stack(node, "column", quote!(::xui::column())),
        "row" => expand_stack(node, "row", quote!(::xui::row())),
        "container" => expand_container(node),
        "style_scope" => expand_style_scope(node),
        "component" => expand_component(node),
        _ => expand_function_component(node),
    }
}

fn expand_style_scope(node: &ElementNode) -> Result<TokenStream2> {
    let mut style = None;
    let mut style_stmts = Vec::new();
    let mut element_stmts = Vec::new();
    for attr in &node.attrs {
        let value = &attr.value;
        match attr.name.to_string().as_str() {
            "style" => style = Some(value.clone()),
            "key" => element_stmts.push(quote! {
                __xui_element = __xui_element.key(#value);
            }),
            "color" => style_stmts.push(quote! {
                __xui_scope_style.merge(&::xui::Style::new().color(#value));
            }),
            "font_size" => style_stmts.push(quote! {
                __xui_scope_style.merge(&::xui::Style::new().font_size(#value));
            }),
            other => {
                if let Some(stmt) = event_attr_stmt(attr) {
                    element_stmts.push(stmt);
                } else {
                    return unsupported_attr(attr, "style_scope", other);
                }
            }
        }
    }
    let children = node
        .children
        .iter()
        .map(expand_child)
        .collect::<Result<Vec<_>>>()?;
    let style_init = style
        .map(|style| quote!(#style))
        .unwrap_or_else(|| quote!(::xui::Style::new()));
    Ok(quote! {{
        let mut __xui_scope_style = #style_init;
        #(#style_stmts)*
        let mut __xui_element = ::xui::style_scope(__xui_scope_style);
        #(#element_stmts)*
        #(
            __xui_element = __xui_element.child(#children);
        )*
        __xui_element.into_element_desc(::std::vec![#(#children),*])
    }})
}

fn expand_label(node: &ElementNode) -> Result<TokenStream2> {
    let text = required_text(node, "label")?;
    let mut attr_stmts = Vec::new();
    for attr in &node.attrs {
        let value = &attr.value;
        match attr.name.to_string().as_str() {
            "key" => attr_stmts.push(quote! { __xui_element = __xui_element.key(#value); }),
            "text" => {}
            "style" => attr_stmts.push(quote! { __xui_style.merge(&#value); }),
            "color" => {
                attr_stmts.push(quote! { __xui_style.merge(&::xui::Style::new().color(#value)); })
            }
            "font_size" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_size(#value)); }),
            other => {
                if let Some(stmt) = event_attr_stmt(attr) {
                    attr_stmts.push(stmt);
                } else {
                    return unsupported_attr(attr, "label", other);
                }
            }
        }
    }
    no_children_except_text(node, "label")?;
    Ok(quote! {{
        let mut __xui_element = ::xui::label(#text);
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc()
    }})
}

fn expand_text(node: &ElementNode) -> Result<TokenStream2> {
    let text = optional_text(node);
    let mut attr_stmts = Vec::new();
    for attr in &node.attrs {
        let value = &attr.value;
        match attr.name.to_string().as_str() {
            "key" => attr_stmts.push(quote! { __xui_element = __xui_element.key(#value); }),
            "text" => {}
            "props" => attr_stmts.push(quote! { __xui_element = __xui_element.props(#value); }),
            "paragraph" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.paragraph(#value); })
            }
            "text_box" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.text_box(#value); })
            }
            "overflow_wrap" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.overflow_wrap(#value); })
            }
            "overflow" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.overflow(#value); })
            }
            "max_lines" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.max_lines(#value); })
            }
            "style" => attr_stmts.push(quote! { __xui_style.merge(&#value); }),
            "color" => {
                attr_stmts.push(quote! { __xui_style.merge(&::xui::Style::new().color(#value)); })
            }
            "font_family" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_family(#value)); }),
            "font_size" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_size(#value)); }),
            "font_weight" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_weight(#value)); }),
            "font_style" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_style(#value)); }),
            "line_height" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().line_height(#value)); }),
            "letter_spacing" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().letter_spacing(#value)); }),
            "decoration" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().decoration(#value)); }),
            other => {
                if let Some(stmt) = event_attr_stmt(attr) {
                    attr_stmts.push(stmt);
                } else {
                    return unsupported_attr(attr, "text", other);
                }
            }
        }
    }
    no_children_except_text(node, "text")?;
    Ok(quote! {{
        let mut __xui_element = ::xui::text(#text);
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc()
    }})
}

fn expand_button(node: &ElementNode) -> Result<TokenStream2> {
    let text = required_text(node, "button")?;
    let mut attr_stmts = Vec::new();
    for attr in &node.attrs {
        let value = &attr.value;
        match attr.name.to_string().as_str() {
            "key" => attr_stmts.push(quote! { __xui_element = __xui_element.key(#value); }),
            "text" => {}
            "on_click" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.on_click(#value); });
            }
            "disabled" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.disabled(#value); });
            }
            "style" => attr_stmts.push(quote! { __xui_style.merge(&#value); }),
            "hover_style" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.hover_style(#value); })
            }
            "pressed_style" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.pressed_style(#value); })
            }
            "disabled_style" => {
                attr_stmts.push(quote! { __xui_element = __xui_element.disabled_style(#value); })
            }
            "background" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().background(#value)); }),
            "color" => {
                attr_stmts.push(quote! { __xui_style.merge(&::xui::Style::new().color(#value)); })
            }
            "font_size" => attr_stmts
                .push(quote! { __xui_style.merge(&::xui::Style::new().font_size(#value)); }),
            other => {
                if let Some(stmt) = event_attr_stmt(attr) {
                    attr_stmts.push(stmt);
                } else {
                    return unsupported_attr(attr, "button", other);
                }
            }
        }
    }
    no_children_except_text(node, "button")?;
    Ok(quote! {{
        let mut __xui_element = ::xui::button(#text);
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(::std::vec::Vec::new())
    }})
}

fn expand_stack(node: &ElementNode, tag: &str, constructor: TokenStream2) -> Result<TokenStream2> {
    let mut attr_stmts = Vec::new();

    parse_attrs_helper(
        node,
        |name, value| {
            parse_base_attr(name, value).or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_stack_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    let children = node
        .children
        .iter()
        .map(expand_child)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {{
        let mut __xui_element = #constructor;
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(::std::vec![#(#children),*])
    }})
}

fn expand_container(node: &ElementNode) -> Result<TokenStream2> {
    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            parse_base_attr(name, value).or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    let children = node
        .children
        .iter()
        .map(expand_child)
        .collect::<Result<Vec<_>>>()?;

    Ok(quote! {{
        let mut __xui_element = ::xui::container();
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(::std::vec![#(#children),*])
    }})
}

fn expand_component(node: &ElementNode) -> Result<TokenStream2> {
    let mut render = None;
    let mut key = None;
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "render" => render = Some(attr.value.clone()),
            "key" => key = Some(attr.value.clone()),
            other => return unsupported_attr(attr, "component", other),
        }
    }
    if !node.children.is_empty() {
        return Err(Error::new(
            node.name.span(),
            "component tags must be self-closing in xui! v1",
        ));
    }
    let render = render.ok_or_else(|| Error::new(node.name.span(), "component requires render"))?;
    let mut expr = quote!(::xui::component(#render));
    if let Some(key) = key {
        expr = quote!(#expr.key(#key));
    }
    Ok(to_element(expr))
}

fn expand_function_component(node: &ElementNode) -> Result<TokenStream2> {
    let mut key = None;
    let mut props_value = None;
    let mut named_props = Vec::new();
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => key = Some(attr.value.clone()),
            "props" => props_value = Some(attr.value.clone()),
            _ => named_props.push(attr),
        }
    }
    if props_value.is_some() && !named_props.is_empty() {
        return Err(Error::new(
            node.name.span(),
            "registered function components cannot mix `props` with named props attributes",
        ));
    }

    let component_handle_name =
        TokenIdent::new(&format!("{}_component_render", node.name), Span::call_site());
    let component_props_name = component_props_name(&node.name);
    let has_children = !node.children.is_empty();
    let named_props_value = if named_props.is_empty() {
        None
    } else {
        let mut props_expr = quote!(#component_props_name::default());
        for attr in named_props {
            let name = &attr.name;
            let value = &attr.value;
            props_expr = quote!(#props_expr.#name(#value));
        }
        Some(props_expr)
    };

    let expr = if has_children {
        let props_value = props_value.or(named_props_value).ok_or_else(|| {
            Error::new(
                node.name.span(),
                "registered function components with children require `props` or named props attributes",
            )
        })?;
        let children = node
            .children
            .iter()
            .map(expand_child)
            .collect::<Result<Vec<_>>>()?;
        let mut element_expr = quote! {{
            let __xui_children = ::std::vec![#(#children),*];
            let __xui_props = ::xui::WithChildren::with_children(
                #props_value,
                __xui_children.clone(),
            );
            ::xui::component(#component_handle_name())
                .props(__xui_props)
                .with_children(__xui_children)
        }};
        if let Some(key) = key {
            element_expr = quote! {{
                let __xui_element = #element_expr;
                __xui_element.key(#key)
            }};
        }
        element_expr
    } else {
        let mut expr = quote!(::xui::component(#component_handle_name()));
        if let Some(key) = key {
            expr = quote!(#expr.key(#key));
        }
        if let Some(props_value) = props_value.or(named_props_value) {
            expr = quote!(#expr.props(#props_value));
        }
        expr
    };

    Ok(to_element(expr))
}

fn to_element(expr: TokenStream2) -> TokenStream2 {
    quote!(::std::convert::Into::<::xui::ElementDesc>::into(#expr))
}

fn expand_child(child: &Child) -> Result<TokenStream2> {
    match child {
        Child::Element(node) => expand_node(node),
        Child::Expr(expr) => Ok(quote!(::std::convert::Into::<::xui::ElementDesc>::into(#expr))),
    }
}

fn required_text(node: &ElementNode, tag: &str) -> Result<TokenStream2> {
    for attr in &node.attrs {
        if attr.name == "text" {
            return Ok(attr.value.clone());
        }
    }
    if node.children.len() == 1 {
        if let Child::Expr(expr) = &node.children[0] {
            return Ok(expr.into_token_stream());
        }
    }
    Err(Error::new(
        node.name.span(),
        format!("{tag} requires text=\"...\" or a single braced text expression"),
    ))
}

fn optional_text(node: &ElementNode) -> TokenStream2 {
    for attr in &node.attrs {
        if attr.name == "text" {
            return attr.value.clone();
        }
    }
    if node.children.len() == 1 {
        if let Child::Expr(expr) = &node.children[0] {
            return expr.into_token_stream();
        }
    }
    quote!("")
}

fn no_children_except_text(node: &ElementNode, tag: &str) -> Result<()> {
    if node.children.is_empty()
        || (node.children.len() == 1 && matches!(node.children[0], Child::Expr(_)))
    {
        return Ok(());
    }
    Err(Error::new(
        node.name.span(),
        format!("{tag} does not support element children"),
    ))
}
