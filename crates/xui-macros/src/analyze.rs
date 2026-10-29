//! Dependency analysis: which of the component's names a piece of code
//! mentions, resolved the way Rust resolves them — a closure parameter, a
//! `let`, or a pattern inside the code shadows a name of the same spelling.
//! It errs towards finding too much (a mention that is not a read costs a
//! spurious re-run); finding too little is impossible, since generated code
//! only puts found names in scope.
use std::collections::BTreeMap;

use proc_macro2::{Delimiter, Group, Span, Spacing, TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{
    Arm, Block, Error, Expr, ExprClosure, ExprForLoop, ExprIf, ExprLet, ExprLit, ExprPath, ExprWhile, Ident, Item, Lit,
    LitStr, Local, Macro, Pat, Result, Token,
};

/// How a name is used; a name used both ways counts as read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Use {
    /// Inside `untrack!(..)`: in scope, but not a dependency.
    Untracked,
    Read,
}

/// Name → its strongest use and the span of its first mention.
pub type Found = BTreeMap<String, (Use, Span)>;

/// Every free name accepted by `known` that `expr` mentions. `shadow` are
/// names bound around `expr` — a match arm's pattern, for its guard.
pub fn uses(expr: &Expr, shadow: &[Ident], known: &dyn Fn(&str) -> bool, out: &mut Found) {
    let scope = shadow.iter().map(Ident::to_string).collect();
    let mut visitor = Visitor { known, scopes: vec![scope], untracked: 0, out };
    visitor.visit_expr(expr);
}

/// The variables `pat` binds. A lone capitalised identifier is taken to be
/// a unit variant or constant (`None`), as rustc does in practice.
pub fn bindings(pat: &Pat, out: &mut Vec<Ident>) {
    match pat {
        Pat::Ident(p) => {
            if !p.ident.to_string().starts_with(|c: char| c.is_uppercase()) {
                out.push(p.ident.clone());
            }
            if let Some((_, sub)) = &p.subpat {
                bindings(sub, out);
            }
        }
        Pat::Tuple(p) => p.elems.iter().for_each(|p| bindings(p, out)),
        Pat::TupleStruct(p) => p.elems.iter().for_each(|p| bindings(p, out)),
        Pat::Struct(p) => p.fields.iter().for_each(|f| bindings(&f.pat, out)),
        Pat::Slice(p) => p.elems.iter().for_each(|p| bindings(p, out)),
        Pat::Reference(p) => bindings(&p.pat, out),
        Pat::Paren(p) => bindings(&p.pat, out),
        Pat::Type(p) => bindings(&p.pat, out),
        // Every alternative binds the same names.
        Pat::Or(p) => p.cases.iter().take(1).for_each(|p| bindings(p, out)),
        _ => {}
    }
}

/// The last segment of a macro's path: `untrack` of `xui_core::untrack!`.
pub fn macro_name(mac: &Macro) -> Option<String> {
    mac.path.segments.last().map(|s| s.ident.to_string())
}

struct Visitor<'a, 'o> {
    known: &'a dyn Fn(&str) -> bool,
    /// Names the code binds itself, innermost scope last.
    scopes: Vec<Vec<String>>,
    /// Inside how many `untrack!`s.
    untracked: usize,
    out: &'o mut Found,
}

impl Visitor<'_, '_> {
    fn mention(&mut self, name: String, span: Span) {
        if !(self.known)(&name) || self.scopes.iter().any(|scope| scope.contains(&name)) {
            return;
        }
        let used = if self.untracked > 0 { Use::Untracked } else { Use::Read };
        self.out.entry(name).and_modify(|(u, _)| *u = (*u).max(used)).or_insert((used, span));
    }

    fn bind(&mut self, pat: &Pat) {
        let mut names = Vec::new();
        bindings(pat, &mut names);
        self.scopes.last_mut().expect("a scope").extend(names.iter().map(Ident::to_string));
    }

    fn scoped(&mut self, f: impl FnOnce(&mut Self)) {
        self.scopes.push(Vec::new());
        f(self);
        self.scopes.pop();
    }

    fn untracked(&mut self, f: impl FnOnce(&mut Self)) {
        self.untracked += 1;
        f(self);
        self.untracked -= 1;
    }

    /// A macro's arguments, as expressions when they parse as a comma list
    /// (`format!`, `println!`, `assert!`), as tokens otherwise.
    fn mac(&mut self, mac: &Macro) {
        if macro_name(mac).as_deref() == Some("untrack") {
            return self.untracked(|v| match mac.parse_body::<Expr>() {
                Ok(expr) => v.visit_expr(&expr),
                Err(_) => v.tokens(mac.tokens.clone()),
            });
        }
        match mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(args) => {
                for arg in &args {
                    match arg {
                        Expr::Lit(ExprLit { lit: Lit::Str(lit), .. }) => self.captures(lit),
                        arg => self.visit_expr(arg),
                    }
                }
            }
            Err(_) => self.tokens(mac.tokens.clone()),
        }
    }

    /// Inline captures in a format string: `"{name}"`, `"{name:>4}"`.
    fn captures(&mut self, lit: &LitStr) {
        let text = lit.value();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '{' {
                continue;
            }
            if chars.peek() == Some(&'{') {
                chars.next();
                continue;
            }
            let mut name = String::new();
            while let Some(&c) = chars.peek()
                && (c.is_alphanumeric() || c == '_')
            {
                name.push(c);
                chars.next();
            }
            if name.chars().next().is_some_and(|c| !c.is_ascii_digit()) {
                self.mention(name, lit.span());
            }
        }
    }

    /// Macro input that is not Rust: every identifier that could be a
    /// variable counts, as does every string literal's captures.
    fn tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        for (i, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) => {
                    let untrack = i >= 2
                        && is_punct(&tokens[i - 1], '!')
                        && matches!(&tokens[i - 2], TokenTree::Ident(id) if id == "untrack");
                    if untrack {
                        self.untracked(|v| v.tokens(group.stream()));
                    } else {
                        self.tokens(group.stream());
                    }
                }
                TokenTree::Ident(ident) if !is_not_a_variable(&tokens, i) => self.mention(ident.to_string(), ident.span()),
                TokenTree::Literal(literal) => {
                    if let Ok(lit) = syn::parse2::<LitStr>(TokenTree::Literal(literal.clone()).into()) {
                        self.captures(&lit);
                    }
                }
                _ => {}
            }
        }
    }
}

impl<'ast> Visit<'ast> for Visitor<'_, '_> {
    fn visit_expr_path(&mut self, expr: &'ast ExprPath) {
        if expr.qself.is_none()
            && let Some(ident) = expr.path.get_ident()
        {
            self.mention(ident.to_string(), ident.span());
        }
    }

    // Patterns bind; they are handled where they occur.
    fn visit_pat(&mut self, _: &'ast Pat) {}

    // Nested items cannot capture.
    fn visit_item(&mut self, _: &'ast Item) {}

    fn visit_block(&mut self, block: &'ast Block) {
        self.scoped(|v| visit::visit_block(v, block));
    }

    fn visit_local(&mut self, local: &'ast Local) {
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);
            if let Some((_, diverge)) = &init.diverge {
                self.visit_expr(diverge);
            }
        }
        self.bind(&local.pat);
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        self.scoped(|v| {
            closure.inputs.iter().for_each(|pat| v.bind(pat));
            v.visit_expr(&closure.body);
        });
    }

    fn visit_arm(&mut self, arm: &'ast Arm) {
        self.scoped(|v| {
            v.bind(&arm.pat);
            if let Some((_, guard)) = &arm.guard {
                v.visit_expr(guard);
            }
            v.visit_expr(&arm.body);
        });
    }

    /// Binds into the enclosing scope, which `if`/`while` open, so a `let`
    /// chain's later conditions and its body see it.
    fn visit_expr_let(&mut self, expr: &'ast ExprLet) {
        self.visit_expr(&expr.expr);
        self.bind(&expr.pat);
    }

    fn visit_expr_if(&mut self, expr: &'ast ExprIf) {
        self.scoped(|v| {
            v.visit_expr(&expr.cond);
            v.visit_block(&expr.then_branch);
        });
        if let Some((_, otherwise)) = &expr.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, expr: &'ast ExprWhile) {
        self.scoped(|v| {
            v.visit_expr(&expr.cond);
            v.visit_block(&expr.body);
        });
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast ExprForLoop) {
        self.visit_expr(&expr.expr);
        self.scoped(|v| {
            v.bind(&expr.pat);
            v.visit_block(&expr.body);
        });
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.mac(mac);
    }
}

fn is_punct(token: &TokenTree, ch: char) -> bool {
    matches!(token, TokenTree::Punct(p) if p.as_char() == ch)
}

/// `a.name`, `'name`, `path::name`, `name!`, `name::path`.
fn is_not_a_variable(tokens: &[TokenTree], i: usize) -> bool {
    let at = |j: Option<usize>| j.and_then(|j| tokens.get(j));
    let prev = at(i.checked_sub(1));
    let prev2 = at(i.checked_sub(2));
    let next = at(Some(i + 1));
    let next2 = at(Some(i + 2));
    let after = match prev {
        // `..name` is a range, not a field.
        Some(t) if is_punct(t, '.') => !prev2.is_some_and(|t| is_punct(t, '.')),
        Some(t) if is_punct(t, '\'') => true,
        Some(t) if is_punct(t, ':') => prev2.is_some_and(|t| is_punct(t, ':')),
        _ => false,
    };
    let before = match next {
        // `name!` is a macro, `name != x` is not.
        Some(TokenTree::Punct(p)) if p.as_char() == '!' => p.spacing() == Spacing::Alone,
        Some(t) if is_punct(t, ':') => next2.is_some_and(|t| is_punct(t, ':')),
        _ => false,
    };
    after || before
}

/// User code as it is emitted: `untrack!(e)` becomes `e`, which is all it
/// means once the analysis has seen it. The declaring macros are only
/// statements of the component's body; anywhere else they are an error.
pub fn rust(tokens: TokenStream) -> Result<TokenStream> {
    let mut out: Vec<TokenTree> = Vec::new();
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Ident(ident)
                if matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '!' && p.spacing() == Spacing::Alone) =>
            {
                match ident.to_string().as_str() {
                    "untrack" => {
                        trim_path(&mut out);
                        tokens.next();
                        let Some(TokenTree::Group(group)) = tokens.next() else {
                            return Err(Error::new(ident.span(), "`untrack!` takes an expression: `untrack!(x)`"));
                        };
                        let inner: Vec<TokenTree> = rust(group.stream())?.into_iter().collect();
                        match inner.as_slice() {
                            [TokenTree::Ident(name)] => out.push(TokenTree::Ident(name.clone())),
                            _ => {
                                // Not the user's parentheses, so no `unused_parens` on them.
                                let mut parens = Group::new(Delimiter::Parenthesis, inner.into_iter().collect());
                                parens.set_span(Span::mixed_site());
                                out.push(parens.into());
                            }
                        }
                    }
                    "state" | "memo" | "effect" | "context" | "provide" | "ticker" | "size" | "resource" => {
                        return Err(Error::new(
                            ident.span(),
                            format!(
                                "`{ident}!` is a statement of the component's body: \
                                 `let name: T = state!(..)`, `memo!(..)` or `context!()`, `effect!(|cx| ..)`, `provide!(..)`"
                            ),
                        ));
                    }
                    _ => out.push(ident.into()),
                }
            }
            TokenTree::Group(group) => {
                let mut inner = Group::new(group.delimiter(), rust(group.stream())?);
                inner.set_span(group.span());
                out.push(inner.into());
            }
            token => out.push(token),
        }
    }
    Ok(out.into_iter().collect())
}

/// Drops the path leading to a macro name: `xui_core::` of `xui_core::untrack!`.
fn trim_path(out: &mut Vec<TokenTree>) {
    while let [.., a, b] = out.as_slice()
        && is_punct(a, ':')
        && is_punct(b, ':')
    {
        out.truncate(out.len() - 2);
        if matches!(out.last(), Some(TokenTree::Ident(_))) {
            out.pop();
        }
    }
}
