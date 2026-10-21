//! `style!(container { padding: 16.0, background: if hovered { A } else { B } })`.
//!
//! A style is typed by the node it is for: the tag names it, and the result
//! is a `StyleOf<N>` for that tag's node. Property names are looked up in the
//! node's property table, as `xui!` attributes are, so the macro carries no
//! property vocabulary of its own, and a property the node's style cannot
//! hold — a prop, a handler, a capability it did not select — fails to
//! compile. State conditions are lowered to `WidgetStateMatcher` rules at
//! compile time.

use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, Result, Token};

use crate::krate;

pub struct StyleInput {
    /// The tag whose node the style is for: its constructor, as in `xui!`.
    tag: syn::Path,
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

impl Parse for StyleInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let tag = input.parse::<syn::Path>().map_err(|error| {
            Error::new(
                error.span(),
                "`style!` names the node it styles first, as in `style!(container { padding: 8.0 })`",
            )
        })?;
        let content;
        syn::braced!(content in input);
        Ok(Self {
            tag,
            entries: Punctuated::<StyleEntry, Token![,]>::parse_terminated(&content)?,
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

pub fn expand_style(style: &StyleInput) -> Result<TokenStream2> {
    let xui_core = krate::xui()?;
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

    let tag = &style.tag;
    let new_style = quote_spanned!(tag.span()=> #xui_core::node::StyleOf::for_tag(#tag));
    let sets = |entries: &[StyleEntryTokens]| {
        entries
            .iter()
            .map(|entry| {
                let name = &entry.name;
                let value = &entry.value;
                quote_spanned! {name.span()=>
                    let __xui_property = __xui_style.__properties().#name();
                    let __xui_style = __xui_style.__with(__xui_property, #value);
                }
            })
            .collect::<Vec<_>>()
    };
    let base = sets(&base_entries);
    let rules = rules.iter().map(|rule| {
        let required = widget_state_tokens(rule.mask.required, &xui_core);
        let forbidden = widget_state_tokens(rule.mask.forbidden, &xui_core);
        let sets = sets(&rule.entries);
        quote! {
            let __xui_style = __xui_style.__rule(
                #xui_core::WidgetStateMatcher::new(#required, #forbidden),
                {
                    let __xui_style = #new_style;
                    #(#sets)*
                    __xui_style
                },
            );
        }
    });
    let style_expr = quote! {
        {
            #[allow(unused_imports)]
            use #xui_core::caps::tables::*;
            let __xui_style = #new_style;
            #(#base)*
            #(#rules)*
            __xui_style
        }
    };

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
    if let Expr::If(if_expr) = value
        && let Some(condition) = parse_style_condition(&if_expr.cond)?
    {
        let then_conditions = cross_condition_masks(conditions.to_vec(), condition.true_masks());
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
    if block.stmts.len() == 1
        && let syn::Stmt::Expr(expr, None) = &block.stmts[0]
    {
        return Some(expr);
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

fn widget_state_tokens(mask: u32, xui_core: &TokenStream2) -> TokenStream2 {
    let mut tokens = Vec::new();
    if mask & STATE_HOVERED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::HOVERED));
    }
    if mask & STATE_PRESSED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::PRESSED));
    }
    if mask & STATE_FOCUSED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::FOCUSED));
    }
    if mask & STATE_DISABLED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::DISABLED));
    }
    if mask & STATE_SELECTED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::SELECTED));
    }
    if mask & STATE_CHECKED != 0 {
        tokens.push(quote!(#xui_core::WidgetState::CHECKED));
    }
    if mask & STATE_DRAGGING != 0 {
        tokens.push(quote!(#xui_core::WidgetState::DRAGGING));
    }

    tokens
        .into_iter()
        .reduce(|acc, token| quote!(#acc | #token))
        .unwrap_or_else(|| quote!(#xui_core::WidgetState::empty()))
}
