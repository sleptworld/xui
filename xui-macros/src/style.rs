//! `style!(padding: 16.0, color: if hovered { A } else { B })`.
//!
//! Property names are passed straight through to `StylePatch` methods, so the
//! macro carries no property vocabulary of its own. State conditions are
//! lowered to `WidgetStateMatcher` rules at compile time.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, Result, Token};

use crate::krate;

pub struct StyleInput {
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

pub fn expand_style(style: &StyleInput) -> Result<TokenStream2> {
    let xui = krate::xui()?;
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
