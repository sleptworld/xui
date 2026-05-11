use proc_macro::TokenStream;
use proc_macro2::{Ident as TokenIdent, Span, TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Attribute as SynAttribute, Error, Expr, FnArg, Ident, LitStr, Pat, Result, ReturnType,
    Signature, Token, Visibility, braced, parse_macro_input, parse_quote,
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
    match expand_component_function(&mut function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn component_fn(input: TokenStream) -> TokenStream {
    let mut functions = parse_macro_input!(input as ComponentFunctions);
    match expand_component_functions(&mut functions) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
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
            return Err(Error::new(input.span(), "component_fn! requires at least one function"));
        }

        Ok(Self { functions })
    }
}

struct ComponentFunction {
    attrs: Vec<SynAttribute>,
    vis: Visibility,
    sig: Signature,
    body: TokenStream2,
}

impl Parse for ComponentFunction {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = input.call(SynAttribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;
        let content;
        braced!(content in input);
        let body: TokenStream2 = content.parse()?;
        Ok(Self {
            attrs,
            vis,
            sig,
            body,
        })
    }
}

fn expand_component_functions(functions: &mut ComponentFunctions) -> Result<TokenStream2> {
    let mut output = TokenStream2::new();
    for function in &mut functions.functions {
        output.extend(expand_component_function(function)?);
    }
    Ok(output)
}

fn expand_component_function(function: &mut ComponentFunction) -> Result<TokenStream2> {
    let original_name = function.sig.ident.clone();
    function.sig.ident = TokenIdent::new(
        &format!("{}_component", original_name),
        original_name.span(),
    );
    function.sig.output =
        ReturnType::Type(Default::default(), Box::new(parse_quote!(::xui::Element)));

    match function.sig.inputs.len() {
        0 => {
            function
                .sig
                .inputs
                .push(parse_quote!(cx: &mut ::xui::HookContext<'_>));
        }
        1 => {
            let Some(first) = function.sig.inputs.first_mut() else {
                unreachable!("checked one argument");
            };
            match first {
                FnArg::Typed(arg) => {
                    if let Pat::Ident(pat) = arg.pat.as_mut() {
                        pat.ident = TokenIdent::new("cx", pat.ident.span());
                    }
                    arg.ty = Box::new(parse_quote!(&mut ::xui::HookContext<'_>));
                }
                FnArg::Receiver(receiver) => {
                    return Err(Error::new(
                        receiver.span(),
                        "component functions cannot take self",
                    ));
                }
            }
        }
        _ => {
            return Err(Error::new(
                function.sig.inputs.span(),
                "component functions accept at most one HookContext argument",
            ));
        }
    }

    let attrs = &function.attrs;
    let vis = &function.vis;
    let sig = &function.sig;
    let body = expand_component_body(&function.body)?;

    Ok(quote! {
        #(#attrs)*
        #vis #sig {
            #body
        }
    })
}

fn expand_component_body(body: &TokenStream2) -> Result<TokenStream2> {
    let tokens: Vec<_> = body.clone().into_iter().collect();
    let Some(xml_start) = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == '<'))
    else {
        return Ok(body.clone());
    };

    let prefix = tokens[..xml_start].iter().cloned().collect::<TokenStream2>();
    let xml = tokens[xml_start..].iter().cloned().collect::<TokenStream2>();
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
        "container" => expand_container(node),
        "column" => expand_children_widget(node, quote!(::xui::column())),
        "row" => expand_children_widget(node, quote!(::xui::row())),
        "label" => expand_label(node),
        "button" => expand_button(node),
        "component" => expand_component(node),
        _ => expand_function_component(node),
    }
}

fn expand_container(node: &ElementNode) -> Result<TokenStream2> {
    let mut expr = quote!(::xui::container());
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => {
                let value = &attr.value;
                expr = quote!(#expr.key(#value));
            }
            "size" => {
                let value = &attr.value;
                expr = quote!(#expr.size(#value));
            }
            "padding" => {
                let value = &attr.value;
                expr = quote!(#expr.padding(#value));
            }
            "background" => {
                let value = &attr.value;
                expr = quote!(#expr.background(#value));
            }
            other => return unsupported_attr(attr, "container", other),
        }
    }
    add_children(expr, &node.children)
}

fn expand_children_widget(node: &ElementNode, mut expr: TokenStream2) -> Result<TokenStream2> {
    let tag = node.name.to_string();
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => {
                let value = &attr.value;
                expr = quote!(#expr.key(#value));
            }
            "gap" => {
                let value = &attr.value;
                expr = quote!(#expr.gap(#value));
            }
            other => return unsupported_attr(attr, &tag, other),
        }
    }
    add_children(expr, &node.children)
}

fn expand_label(node: &ElementNode) -> Result<TokenStream2> {
    let text = required_text(node, "label")?;
    let mut expr = quote!(::xui::label(#text));
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => {
                let value = &attr.value;
                expr = quote!(#expr.key(#value));
            }
            "text" => {}
            "color" => {
                let value = &attr.value;
                expr = quote!(#expr.color(#value));
            }
            other => return unsupported_attr(attr, "label", other),
        }
    }
    no_children_except_text(node, "label")?;
    Ok(to_element(expr))
}

fn expand_button(node: &ElementNode) -> Result<TokenStream2> {
    let text = required_text(node, "button")?;
    let mut expr = quote!(::xui::button(#text));
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => {
                let value = &attr.value;
                expr = quote!(#expr.key(#value));
            }
            "text" => {}
            "on_click" => {
                let value = &attr.value;
                expr = quote!(#expr.on_click(#value));
            }
            other => return unsupported_attr(attr, "button", other),
        }
    }
    no_children_except_text(node, "button")?;
    Ok(to_element(expr))
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
    if !node.children.is_empty() {
        return Err(Error::new(
            node.name.span(),
            "function component tags must be self-closing in xui! v1",
        ));
    }

    let mut key = None;
    for attr in &node.attrs {
        match attr.name.to_string().as_str() {
            "key" => key = Some(attr.value.clone()),
            other => return unsupported_attr(attr, &node.name.to_string(), other),
        }
    }

    let component_name = TokenIdent::new(&format!("{}_component", node.name), Span::call_site());
    let mut expr = quote!(::xui::component(#component_name));
    if let Some(key) = key {
        expr = quote!(#expr.key(#key));
    }
    Ok(to_element(expr))
}

fn add_children(mut expr: TokenStream2, children: &[Child]) -> Result<TokenStream2> {
    for child in children {
        let child = expand_child(child)?;
        expr = quote!(#expr.child(#child));
    }
    Ok(to_element(expr))
}

fn to_element(expr: TokenStream2) -> TokenStream2 {
    quote!(::xui::Element::from(#expr))
}

fn expand_child(child: &Child) -> Result<TokenStream2> {
    match child {
        Child::Element(node) => expand_node(node),
        Child::Expr(expr) => Ok(quote!(#expr)),
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

fn unsupported_attr<T>(attr: &XuiAttribute, tag: &str, attr_name: &str) -> Result<T> {
    Err(Error::new(
        attr.name.span(),
        format!("unsupported attribute `{attr_name}` on <{tag}>"),
    ))
}
