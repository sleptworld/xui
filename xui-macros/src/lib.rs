mod tools;
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{
    Delimiter, Group, Ident as TokenIdent, Span, TokenStream as TokenStream2, TokenTree,
};
use quote::{ToTokens, quote};
use syn::parse::Parser;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Attribute as SynAttribute, Data, DeriveInput, Error, Expr, Fields, FnArg, Ident, LitStr, Pat,
    Result, ReturnType, Signature, Token, Type, TypeReference, Visibility, braced,
    parse_macro_input, parse_quote,
};

use crate::tools::{
    event_attr_stmt, parse_attrs_helper, parse_base_attr, parse_event_attr,
    parse_layout_style_attr, parse_paint_style_attr, parse_scroll_style_attr,
    parse_text_style_attr, parse_transform_style_attr, unsupported_attr,
};

#[proc_macro]
pub fn xui(input: TokenStream) -> TokenStream {
    let root = parse_macro_input!(input as ElementNode);
    match expand_node(&root) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro]
pub fn style(input: TokenStream) -> TokenStream {
    let style = parse_macro_input!(input as StyleInput);
    match expand_style(&style) {
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
pub fn main(_attrs: TokenStream, item: TokenStream) -> TokenStream {
    let function = parse_macro_input!(item as MainFunction);
    match expand_main_function(function) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn xui_crate_path() -> Result<TokenStream2> {
    match crate_name("xui") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = TokenIdent::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(Error::new(
            Span::call_site(),
            format!("failed to find xui dependency: {error}"),
        )),
    }
}

fn expand_main_function(function: MainFunction) -> Result<TokenStream2> {
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

    let xui = xui_crate_path()?;
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

struct MainFunction {
    attrs: Vec<SynAttribute>,
    vis: Visibility,
    sig: Signature,
    input_defaults: Vec<Option<Expr>>,
    body: TokenStream2,
}

struct ExpandedComponentFunction {
    tokens: TokenStream2,
}

struct StyleInput {
    entries: Punctuated<StyleEntry, Token![,]>,
}

struct StyleEntry {
    name: Ident,
    value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StyleConditionMask {
    required: u32,
    forbidden: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StyleConditionExpr {
    State(u32),
    Not(Box<StyleConditionExpr>),
    And(Box<StyleConditionExpr>, Box<StyleConditionExpr>),
    Or(Box<StyleConditionExpr>, Box<StyleConditionExpr>),
}

struct StyleRuleEntries {
    mask: StyleConditionMask,
    entries: Vec<StyleEntryTokens>,
}

struct StyleEntryTokens {
    name: Ident,
    value: TokenStream2,
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

impl Parse for StyleInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        Ok(Self {
            entries: Punctuated::<StyleEntry, Token![,]>::parse_terminated(input)?,
        })
    }
}

impl Parse for StyleEntry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        let value = input.parse()?;
        Ok(Self { name, value })
    }
}

const STATE_HOVERED: u32 = 1 << 0;
const STATE_PRESSED: u32 = 1 << 1;
const STATE_FOCUSED: u32 = 1 << 2;
const STATE_DISABLED: u32 = 1 << 3;
const STATE_SELECTED: u32 = 1 << 4;
const STATE_CHECKED: u32 = 1 << 5;
const STATE_DRAGGING: u32 = 1 << 6;

fn expand_style(style: &StyleInput) -> Result<TokenStream2> {
    let xui = quote!(::xui);
    let mut base_entries = Vec::new();
    let mut rules: Vec<StyleRuleEntries> = Vec::new();

    for entry in &style.entries {
        collect_style_value(
            &entry.name,
            &entry.value,
            &[StyleConditionMask::empty()],
            &mut base_entries,
            &mut rules,
        )?;
    }

    let mut base_patch = quote!(#xui::StylePatch::default());
    for entry in &base_entries {
        let name = &entry.name;
        let value = &entry.value;
        base_patch = quote!(#base_patch.#name(#value));
    }

    let mut style_expr = quote!(#xui::Style::from_patch(#base_patch));
    for rule in &rules {
        let required = widget_state_tokens(rule.mask.required, &xui);
        let forbidden = widget_state_tokens(rule.mask.forbidden, &xui);
        let mut rule_patch = quote!(s);
        for entry in &rule.entries {
            let name = &entry.name;
            let value = &entry.value;
            rule_patch = quote!(#rule_patch.#name(#value));
        }
        style_expr = quote! {
            #style_expr.when_state(
                #xui::WidgetStateMatcher::new(#required, #forbidden),
                |s| #rule_patch,
            )
        };
    }

    Ok(style_expr)
}

impl StyleConditionMask {
    fn empty() -> Self {
        Self {
            required: 0,
            forbidden: 0,
        }
    }
}

impl StyleConditionExpr {
    fn true_masks(&self) -> Vec<StyleConditionMask> {
        match self {
            Self::State(flag) => vec![StyleConditionMask {
                required: *flag,
                forbidden: 0,
            }],
            Self::Not(expr) => expr.false_masks(),
            Self::And(left, right) => cross_condition_masks(left.true_masks(), right.true_masks()),
            Self::Or(left, right) => {
                dedupe_condition_masks([left.true_masks(), right.true_masks()].concat())
            }
        }
    }

    fn false_masks(&self) -> Vec<StyleConditionMask> {
        match self {
            Self::State(flag) => vec![StyleConditionMask {
                required: 0,
                forbidden: *flag,
            }],
            Self::Not(expr) => expr.true_masks(),
            Self::And(left, right) => {
                dedupe_condition_masks([left.false_masks(), right.false_masks()].concat())
            }
            Self::Or(left, right) => cross_condition_masks(left.false_masks(), right.false_masks()),
        }
    }
}

fn collect_style_value(
    name: &Ident,
    value: &Expr,
    conditions: &[StyleConditionMask],
    base_entries: &mut Vec<StyleEntryTokens>,
    rules: &mut Vec<StyleRuleEntries>,
) -> Result<()> {
    if let Expr::If(if_expr) = value {
        if let Some(condition) = parse_style_condition(&if_expr.cond)? {
            let then_conditions =
                cross_condition_masks(conditions.to_vec(), condition.true_masks());
            collect_style_block_value(
                name,
                &if_expr.then_branch,
                &then_conditions,
                base_entries,
                rules,
            )?;

            if let Some((_, else_expr)) = &if_expr.else_branch {
                let else_conditions =
                    cross_condition_masks(conditions.to_vec(), condition.false_masks());
                collect_style_value(name, else_expr, &else_conditions, base_entries, rules)?;
            }

            return Ok(());
        }
    }

    push_style_value(
        name,
        style_expr_value_tokens(value),
        conditions,
        base_entries,
        rules,
    );
    Ok(())
}

fn collect_style_block_value(
    name: &Ident,
    block: &syn::Block,
    conditions: &[StyleConditionMask],
    base_entries: &mut Vec<StyleEntryTokens>,
    rules: &mut Vec<StyleRuleEntries>,
) -> Result<()> {
    if let Some(expr) = single_tail_expr(block) {
        collect_style_value(name, expr, conditions, base_entries, rules)
    } else {
        push_style_value(name, quote!(#block), conditions, base_entries, rules);
        Ok(())
    }
}

fn push_style_value(
    name: &Ident,
    value: TokenStream2,
    conditions: &[StyleConditionMask],
    base_entries: &mut Vec<StyleEntryTokens>,
    rules: &mut Vec<StyleRuleEntries>,
) {
    for condition in conditions {
        let entry = StyleEntryTokens {
            name: name.clone(),
            value: value.clone(),
        };
        if *condition == StyleConditionMask::empty() {
            base_entries.push(entry);
        } else {
            push_style_rule_entry(rules, *condition, entry);
        }
    }
}

fn style_expr_value_tokens(expr: &Expr) -> TokenStream2 {
    if let Expr::Block(block) = expr {
        style_block_value_tokens(&block.block)
    } else {
        quote!(#expr)
    }
}

fn style_block_value_tokens(block: &syn::Block) -> TokenStream2 {
    if let Some(expr) = single_tail_expr(block) {
        return quote!(#expr);
    }
    quote!(#block)
}

fn single_tail_expr(block: &syn::Block) -> Option<&Expr> {
    if block.stmts.len() == 1 {
        if let syn::Stmt::Expr(expr, None) = &block.stmts[0] {
            return Some(expr);
        }
    }
    None
}

fn push_style_rule_entry(
    rules: &mut Vec<StyleRuleEntries>,
    mask: StyleConditionMask,
    entry: StyleEntryTokens,
) {
    if let Some(rule) = rules.iter_mut().find(|rule| rule.mask == mask) {
        rule.entries.push(entry);
    } else {
        rules.push(StyleRuleEntries {
            mask,
            entries: vec![entry],
        });
    }
}

fn cross_condition_masks(
    left: Vec<StyleConditionMask>,
    right: Vec<StyleConditionMask>,
) -> Vec<StyleConditionMask> {
    let mut masks = Vec::new();
    for left in left {
        for right in &right {
            if let Some(mask) = combine_condition_masks(left, *right) {
                masks.push(mask);
            }
        }
    }
    dedupe_condition_masks(masks)
}

fn combine_condition_masks(
    left: StyleConditionMask,
    right: StyleConditionMask,
) -> Option<StyleConditionMask> {
    let required = left.required | right.required;
    let forbidden = left.forbidden | right.forbidden;
    (required & forbidden == 0).then_some(StyleConditionMask {
        required,
        forbidden,
    })
}

fn dedupe_condition_masks(masks: Vec<StyleConditionMask>) -> Vec<StyleConditionMask> {
    let mut deduped = Vec::new();
    for mask in masks {
        if !deduped.contains(&mask) {
            deduped.push(mask);
        }
    }
    deduped
}

fn parse_style_condition(expr: &Expr) -> Result<Option<StyleConditionExpr>> {
    match expr {
        Expr::Path(path) => Ok(path
            .path
            .get_ident()
            .and_then(state_flag)
            .map(StyleConditionExpr::State)),
        Expr::Paren(paren) => parse_style_condition(&paren.expr),
        Expr::Group(group) => parse_style_condition(&group.expr),
        Expr::Unary(unary) => {
            if !matches!(unary.op, syn::UnOp::Not(_)) {
                return unsupported_state_condition_if_needed(expr);
            }
            Ok(parse_style_condition(&unary.expr)?
                .map(|condition| StyleConditionExpr::Not(Box::new(condition))))
        }
        Expr::Binary(binary) => match &binary.op {
            syn::BinOp::And(_) => {
                let left = parse_style_condition(&binary.left)?;
                let right = parse_style_condition(&binary.right)?;
                Ok(match (left, right) {
                    (Some(left), Some(right)) => {
                        Some(StyleConditionExpr::And(Box::new(left), Box::new(right)))
                    }
                    (None, None) => None,
                    _ => {
                        return Err(Error::new(
                            expr.span(),
                            "`style!` state conditions cannot mix state names with runtime expressions",
                        ));
                    }
                })
            }
            syn::BinOp::Or(_) => {
                let left = parse_style_condition(&binary.left)?;
                let right = parse_style_condition(&binary.right)?;
                Ok(match (left, right) {
                    (Some(left), Some(right)) => {
                        Some(StyleConditionExpr::Or(Box::new(left), Box::new(right)))
                    }
                    (None, None) => None,
                    _ => {
                        return Err(Error::new(
                            expr.span(),
                            "`style!` state conditions cannot mix state names with runtime expressions",
                        ));
                    }
                })
            }
            _ => unsupported_state_condition_if_needed(expr),
        },
        _ => unsupported_state_condition_if_needed(expr),
    }
}

fn unsupported_state_condition_if_needed(expr: &Expr) -> Result<Option<StyleConditionExpr>> {
    if expr_contains_state_ident(expr) {
        Err(Error::new(
            expr.span(),
            "`style!` state conditions only support state names, `&&`, `!`, and parentheses",
        ))
    } else {
        Ok(None)
    }
}

fn expr_contains_state_ident(expr: &Expr) -> bool {
    match expr {
        Expr::Path(path) => path.path.get_ident().and_then(state_flag).is_some(),
        Expr::Paren(paren) => expr_contains_state_ident(&paren.expr),
        Expr::Group(group) => expr_contains_state_ident(&group.expr),
        Expr::Unary(unary) => expr_contains_state_ident(&unary.expr),
        Expr::Binary(binary) => {
            expr_contains_state_ident(&binary.left) || expr_contains_state_ident(&binary.right)
        }
        Expr::Call(call) => {
            expr_contains_state_ident(&call.func) || call.args.iter().any(expr_contains_state_ident)
        }
        Expr::MethodCall(call) => {
            expr_contains_state_ident(&call.receiver)
                || call.args.iter().any(expr_contains_state_ident)
        }
        Expr::If(if_expr) => {
            expr_contains_state_ident(&if_expr.cond)
                || if_expr
                    .else_branch
                    .as_ref()
                    .is_some_and(|(_, expr)| expr_contains_state_ident(expr))
        }
        _ => false,
    }
}

fn state_flag(ident: &Ident) -> Option<u32> {
    match ident.to_string().as_str() {
        "hovered" => Some(STATE_HOVERED),
        "pressed" => Some(STATE_PRESSED),
        "focused" => Some(STATE_FOCUSED),
        "disabled" => Some(STATE_DISABLED),
        "selected" => Some(STATE_SELECTED),
        "checked" => Some(STATE_CHECKED),
        "dragging" => Some(STATE_DRAGGING),
        _ => None,
    }
}

fn widget_state_tokens(mask: u32, xui: &TokenStream2) -> TokenStream2 {
    let mut tokens = Vec::new();
    if mask & STATE_HOVERED != 0 {
        tokens.push(quote!(#xui::WidgetState::HOVERED));
    }
    if mask & STATE_PRESSED != 0 {
        tokens.push(quote!(#xui::WidgetState::PRESSED));
    }
    if mask & STATE_FOCUSED != 0 {
        tokens.push(quote!(#xui::WidgetState::FOCUSED));
    }
    if mask & STATE_DISABLED != 0 {
        tokens.push(quote!(#xui::WidgetState::DISABLED));
    }
    if mask & STATE_SELECTED != 0 {
        tokens.push(quote!(#xui::WidgetState::SELECTED));
    }
    if mask & STATE_CHECKED != 0 {
        tokens.push(quote!(#xui::WidgetState::CHECKED));
    }
    if mask & STATE_DRAGGING != 0 {
        tokens.push(quote!(#xui::WidgetState::DRAGGING));
    }

    tokens
        .into_iter()
        .reduce(|acc, token| quote!(#acc | #token))
        .unwrap_or_else(|| quote!(#xui::WidgetState::empty()))
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

impl Parse for MainFunction {
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

        let required_state = default_value.is_none().then(|| RequiredPropState {
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
        .map(|state| {
            let required_trait = &state.required_trait;
            let missing = &state.missing;
            let set = &state.set;
            quote! {
                #[doc = "Implemented only when this required component prop has been set on the typed builder."]
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
                            _states: _,
                        } = self;
                        #builder_name {
                            #(#reconstruct_fields),*,
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
                _states: ::std::marker::PhantomData<#state_phantom_type>,
            }

            impl #props_name {
                pub fn builder() -> #builder_initial_type {
                    #builder_name {
                        #(#builder_init_values),*,
                        _states: ::std::marker::PhantomData,
                    }
                }
            }

            impl #builder_generics #builder_current_type {
                #(#setters)*
            }

            impl #builder_generics #builder_current_type
            #build_where_clause
            {
                pub fn build(self) -> #props_name {
                    #props_name {
                        #(#build_values),*
                    }
                }
            }

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
        "text" => expand_text(node),
        "container" => expand_container(node),
        "grid" => expand_grid(node),
        "canvas" => expand_canvas(node),
        "icon" => expand_icon(node),
        _ => expand_function_component(node),
    }
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

fn expand_container(node: &ElementNode) -> Result<TokenStream2> {
    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            match name {
                "transition" => Some(quote! {
                    __xui_style = __xui_style.transition(#value);
                }),
                _ => None,
            }
            .or(parse_base_attr(name, value))
            .or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_transform_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_event_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    let children = expand_children(&node.children)?;

    Ok(quote! {{
        let mut __xui_element = ::xui::container();
        let mut __xui_style = ::xui::Style::new();
        let mut __xui_children = ::std::vec::Vec::new();
        #(#attr_stmts)*
        #(#children)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(__xui_children)
    }})
}

fn expand_grid(node: &ElementNode) -> Result<TokenStream2> {
    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            match name {
                "columns" => Some(quote! {
                    __xui_element = __xui_element.columns(#value);
                }),
                "rows" => Some(quote! {
                    __xui_element = __xui_element.rows(#value);
                }),
                "flow" => Some(quote! {
                    __xui_element = __xui_element.flow(#value);
                }),
                "columns_count" => Some(quote! {
                    __xui_element = __xui_element.columns_count(#value);
                }),
                "adaptive_columns" => Some(quote! {
                    __xui_element = __xui_element.adaptive_columns(#value);
                }),
                _ => None,
            }
            .or(parse_base_attr(name, value))
            .or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_transform_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_event_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    let children = expand_children(&node.children)?;

    Ok(quote! {{
        let mut __xui_element = ::xui::grid();
        let mut __xui_style = ::xui::Style::new();
        let mut __xui_children = ::std::vec::Vec::new();
        #(#attr_stmts)*
        #(#children)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(__xui_children)
    }})
}

fn expand_canvas(node: &ElementNode) -> Result<TokenStream2> {
    let mut controllers = node.attrs.iter().filter(|attr| attr.name == "controller");
    let controller = controllers
        .next()
        .map(|attr| attr.value.clone())
        .ok_or_else(|| Error::new(node.name.span(), "canvas requires a `controller` attribute"))?;
    if let Some(duplicate) = controllers.next() {
        return Err(Error::new(
            duplicate.name.span(),
            "canvas accepts only one `controller` attribute",
        ));
    }

    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            match name {
                "controller" => Some(quote! {}),
                _ => None,
            }
            .or(parse_base_attr(name, value))
            .or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_transform_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_event_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    if !node.children.is_empty() {
        return Err(Error::new(
            node.name.span(),
            "canvas does not support children",
        ));
    }

    Ok(quote! {{
        let mut __xui_element = ::xui::canvas(#controller);
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc()
    }})
}

fn expand_icon(node: &ElementNode) -> Result<TokenStream2> {
    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            match name {
                "transition" => Some(quote! {
                    __xui_style = __xui_style.transition(#value);
                }),
                "asset" => Some(quote! {
                    __xui_element = __xui_element.asset(#value);
                }),
                "size" => Some(quote! {
                    let __xui_icon_size: ::xui::Sizing = (#value).into();
                    __xui_style = __xui_style.size(::xui::Size::new(
                        __xui_icon_size,
                        __xui_icon_size,
                    ));
                }),
                _ => None,
            }
            .or(parse_base_attr(name, value))
            .or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_transform_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_event_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    Ok(quote! {{
        let mut __xui_element = ::xui::icon();
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc()
    }})
}

#[cfg(test)]
fn expand_image(node: &ElementNode) -> Result<TokenStream2> {
    let has_asset = node.attrs.iter().any(|attr| attr.name == "asset");
    let has_image_key = node.attrs.iter().any(|attr| attr.name == "image_key");
    if has_asset && has_image_key {
        return Err(Error::new(
            node.name.span(),
            "image cannot use `asset` and `image_key` together",
        ));
    }

    let mut attr_stmts = Vec::new();
    parse_attrs_helper(
        node,
        |name, value| {
            match name {
                "asset" => Some(match syn::parse2::<LitStr>(value.clone()) {
                    Ok(path) => quote! {
                        __xui_element = __xui_element.asset_path(#path);
                    },
                    Err(_) => quote! {
                        __xui_element = __xui_element.asset(#value);
                    },
                }),
                "image_key" => Some(quote! {
                    __xui_element = __xui_element.image_key(#value);
                }),
                "opacity" => Some(quote! {
                    __xui_element = __xui_element.opacity(#value);
                }),
                _ => None,
            }
            .or(parse_base_attr(name, value))
            .or(parse_text_style_attr(name, value)
                .or(parse_layout_style_attr(name, value))
                .or(parse_paint_style_attr(name, value))
                .or(parse_transform_style_attr(name, value))
                .or(parse_scroll_style_attr(name, value))
                .or(parse_event_attr(name, value)))
        },
        &mut attr_stmts,
    )?;

    if !node.children.is_empty() {
        return Err(Error::new(
            node.name.span(),
            "image does not support children",
        ));
    }

    Ok(quote! {{
        let mut __xui_element = ::xui::image();
        let mut __xui_style = ::xui::Style::new();
        #(#attr_stmts)*
        __xui_element = __xui_element.style(__xui_style);
        __xui_element.into_element_desc(::std::vec::Vec::new())
    }})
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

    let component_handle_name = TokenIdent::new(
        &format!("{}_component_render", node.name),
        Span::call_site(),
    );
    let component_props_name = component_props_name(&node.name);
    let has_children = !node.children.is_empty();
    let named_props_value = if named_props.is_empty() {
        None
    } else {
        let mut props_expr = quote!(#component_props_name::builder());
        for attr in named_props {
            let name = &attr.name;
            let value = &attr.value;
            props_expr = quote!(#props_expr.#name(#value));
        }
        Some(quote!(#props_expr.build()))
    };

    let expr = if has_children {
        let props_value = props_value
            .or(named_props_value)
            .unwrap_or_else(|| quote!(#component_props_name::builder().build()));
        let children = expand_children(&node.children)?;
        let mut element_expr = quote! {{
            let mut __xui_children = ::std::vec::Vec::new();
            #(#children)*
            let __xui_props = ::xui::WithChildren::with_children(
                #props_value,
                __xui_children,
            );
            ::xui::component(#component_handle_name())
                .props(__xui_props)
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

fn expand_children(children: &[Child]) -> Result<Vec<TokenStream2>> {
    children
        .iter()
        .map(|child| match child {
            Child::Element(node) => {
                let child = expand_node(node)?;
                Ok(quote! {
                    __xui_children.push(#child);
                })
            }
            Child::Expr(expr) => Ok(quote! {
                ::xui::IntoChildren::append_children(
                    #expr,
                    &mut __xui_children,
                );
            }),
        })
        .collect()
}

fn optional_icon_data(node: &ElementNode) -> TokenStream2 {
    for attr in &node.attrs {
        if attr.name == "data" {
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

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn expand_image_tokens(tokens: TokenStream2) -> String {
        let node = syn::parse2::<ElementNode>(tokens).unwrap();
        expand_image(&node).unwrap().to_string()
    }

    #[test]
    fn image_asset_literal_expands_as_path() {
        let expanded = expand_image_tokens(quote!(<image asset="images/demo.png" />));
        assert!(expanded.contains("asset_path"));
        assert!(expanded.contains("images/demo.png"));
    }

    #[test]
    fn image_asset_expression_expands_as_asset_id() {
        let expanded = expand_image_tokens(quote!(
            <image asset={xui_assets::refs::images::DEMO_PNG} />
        ));
        assert!(expanded.contains("asset (xui_assets :: refs :: images :: DEMO_PNG)"));
        assert!(!expanded.contains("asset_path"));
    }

    #[test]
    fn image_rejects_asset_with_manual_image_key() {
        let node = syn::parse2::<ElementNode>(quote!(
            <image asset="images/demo.png" image_key="manual" />
        ))
        .unwrap();
        let error = expand_image(&node).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot use `asset` and `image_key`")
        );
    }

    #[test]
    fn canvas_expands_as_host_widget() {
        let node = syn::parse2::<ElementNode>(quote!(
            <canvas
                controller={controller.clone()}
                key="plot"
                width={320.0}
                height={180.0}
                on_click={handle_click}
            />
        ))
        .unwrap();
        let expanded = expand_node(&node).unwrap().to_string();

        assert!(expanded.contains("xui :: canvas (controller . clone ())"));
        assert!(expanded.contains("key (\"plot\")"));
        assert!(expanded.contains("width (320.0)"));
        assert!(expanded.contains("height (180.0)"));
        assert!(expanded.contains("on_click (handle_click)"));
        assert!(expanded.contains("into_element_desc ()"));
        assert!(!expanded.contains("canvas_component_render"));
    }

    #[test]
    fn canvas_requires_controller() {
        let node = syn::parse2::<ElementNode>(quote!(<canvas width={320.0} />)).unwrap();
        let error = expand_node(&node).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("requires a `controller` attribute")
        );
    }

    #[test]
    fn canvas_rejects_children() {
        let node = syn::parse2::<ElementNode>(quote!(
            <canvas controller={controller}>
                <text text="unsupported" />
            </canvas>
        ))
        .unwrap();
        let error = expand_node(&node).unwrap_err();

        assert!(error.to_string().contains("does not support children"));
    }

    #[test]
    fn canvas_rejects_duplicate_controller() {
        let node = syn::parse2::<ElementNode>(quote!(
            <canvas controller={first} controller={second} />
        ))
        .unwrap();
        let error = expand_node(&node).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("accepts only one `controller` attribute")
        );
    }

    #[test]
    fn function_component_named_props_use_builder_build() {
        let node = syn::parse2::<ElementNode>(quote!(
            <pbutton text={"hello".to_string()} ps={button_prop} />
        ))
        .unwrap();
        let expanded = expand_function_component(&node).unwrap().to_string();
        assert!(expanded.contains("PbuttonProps :: builder"));
        assert!(expanded.contains(". build"));
    }

    #[test]
    fn braced_children_are_appended_instead_of_forced_into_one_element() {
        let node = syn::parse2::<ElementNode>(quote!(
            <container>
                <text text="before" />
                {children}
                <text text="after" />
            </container>
        ))
        .unwrap();
        let expanded = expand_container(&node).unwrap().to_string();

        assert!(expanded.contains("IntoChildren :: append_children (children"));
        assert_eq!(expanded.matches("__xui_children . push").count(), 2);
        assert!(expanded.contains("into_element_desc (__xui_children)"));
    }

    #[test]
    fn container_transition_attribute_is_stored_on_style() {
        let node = syn::parse2::<ElementNode>(quote!(
            <container transition={transition} />
        ))
        .unwrap();
        let expanded = expand_container(&node).unwrap().to_string();

        assert!(expanded.contains("__xui_style = __xui_style . transition (transition)"));
        assert!(!expanded.contains("__xui_element . transition"));
    }

    #[test]
    fn grid_expands_as_host_widget_with_adaptive_columns() {
        let node = syn::parse2::<ElementNode>(quote!(
            <grid
                adaptive_columns={220.0}
                gap={12.0}
                flow={GridFlow::RowDense}
            >
                <container />
                {cards}
            </grid>
        ))
        .unwrap();
        let expanded = expand_grid(&node).unwrap().to_string();

        assert!(expanded.contains("xui :: grid ()"));
        assert!(expanded.contains("adaptive_columns (220.0)"));
        assert!(expanded.contains("gap (12.0)"));
        assert!(expanded.contains("flow (GridFlow :: RowDense)"));
        assert!(expanded.contains("IntoChildren :: append_children (cards"));
        assert!(expanded.contains("into_element_desc (__xui_children)"));
        assert!(!expanded.contains("grid_component_render"));
    }

    #[test]
    fn generated_component_props_include_required_prop_bounds() {
        let mut function = syn::parse2::<ComponentFunction>(quote!(
            fn pair(text: &String, color: &Color) {
                todo!()
            }
        ))
        .unwrap();
        let expanded = expand_component_function(&mut function)
            .unwrap()
            .tokens
            .to_string();
        assert!(expanded.contains("PairPropsTextRequiredPropIsSet"));
        assert!(expanded.contains("PairPropsTextMissing"));
        assert!(expanded.contains("PairPropsColorRequiredPropIsSet"));
        assert!(expanded.contains("PairPropsColorMissing"));
    }

    #[test]
    fn style_macro_expands_base_and_hover_rule() {
        let input = syn::parse2::<StyleInput>(quote!(
            background: Color::BLACK,
            color: if hovered { Color::BLACK } else { Color::WHITE },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert!(expanded.contains("Style :: from_patch"));
        assert!(expanded.contains("StylePatch :: default"));
        assert!(expanded.contains(". background (Color :: BLACK)"));
        assert!(expanded.contains(". color (Color :: WHITE)"));
        assert!(expanded.contains(". when_state"));
        assert_eq!(expanded.matches("when_state").count(), 2);
        assert!(expanded.contains("WidgetState :: HOVERED"));
        assert!(expanded.contains(". color (Color :: BLACK)"));
    }

    #[test]
    fn style_macro_groups_entries_with_the_same_state_matcher() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if hovered { Color::BLACK } else { Color::WHITE },
            background: if hovered { Color::WHITE } else { Color::BLACK },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert_eq!(expanded.matches("when_state").count(), 2);
        assert!(expanded.contains(". color (Color :: BLACK)"));
        assert!(expanded.contains(". background (Color :: WHITE)"));
    }

    #[test]
    fn style_macro_expands_combined_required_and_forbidden_states() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if hovered && pressed && !disabled { Color::BLACK } else { Color::WHITE },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert!(expanded.contains("WidgetState :: HOVERED"));
        assert!(expanded.contains("WidgetState :: PRESSED"));
        assert!(expanded.contains("WidgetState :: DISABLED"));
        assert!(expanded.contains("WidgetStateMatcher :: new"));
    }

    #[test]
    fn style_macro_keeps_runtime_if_expression_in_base() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if is_hovered { Color::BLACK } else { Color::WHITE },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert!(expanded.contains("if is_hovered"));
        assert!(!expanded.contains("when_state"));
    }

    #[test]
    fn style_macro_supports_or_conditions() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if hovered || pressed { Color::BLACK } else { Color::WHITE },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert_eq!(expanded.matches("when_state").count(), 3);
        assert!(expanded.contains("WidgetState :: HOVERED"));
        assert!(expanded.contains("WidgetState :: PRESSED"));
        assert!(expanded.contains(". color (Color :: BLACK)"));
        assert!(expanded.contains(". color (Color :: WHITE)"));
    }

    #[test]
    fn style_macro_parses_nested_state_branches_into_rules() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if hovered {
                if pressed { Color::BLACK } else { Color::WHITE }
            } else {
                Color::BLUE
            },
        ))
        .unwrap();
        let expanded = expand_style(&input).unwrap().to_string();

        assert_eq!(expanded.matches("when_state").count(), 3);
        assert!(expanded.contains("WidgetState :: HOVERED"));
        assert!(expanded.contains("WidgetState :: PRESSED"));
        assert!(expanded.contains(". color (Color :: BLACK)"));
        assert!(expanded.contains(". color (Color :: WHITE)"));
        assert!(expanded.contains(". color (Color :: BLUE)"));
    }

    #[test]
    fn style_macro_rejects_mixed_state_and_runtime_conditions() {
        let input = syn::parse2::<StyleInput>(quote!(
            color: if hovered && is_pressed { Color::BLACK } else { Color::WHITE },
        ))
        .unwrap();
        let error = expand_style(&input).unwrap_err();

        assert!(error.to_string().contains("cannot mix state names"));
    }

    #[test]
    fn container_transform_attributes_write_style() {
        let node = syn::parse2::<ElementNode>(quote!(
            <container translate_y={2.0} scale={0.98}></container>
        ))
        .unwrap();
        let expanded = expand_container(&node).unwrap().to_string();

        assert!(expanded.contains("translate_y (2.0)"));
        assert!(expanded.contains("scale (0.98)"));
    }

    #[test]
    fn icon_expands_without_children_argument() {
        let node = syn::parse2::<ElementNode>(quote!(
            <icon asset={xui_assets::icons::SEARCH_SVG} size={16.0}/>
        ))
        .unwrap();
        let expanded = expand_icon(&node).unwrap().to_string();

        assert!(expanded.contains("into_element_desc ()"));
        assert!(expanded.contains("let __xui_icon_size : :: xui :: Sizing = (16.0) . into ()"));
        assert!(expanded.contains("Size :: new (__xui_icon_size , __xui_icon_size"));
        assert!(!expanded.contains("into_element_desc (:: std :: vec :: Vec :: new ())"));
    }
}
