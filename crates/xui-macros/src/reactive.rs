//! `#[component]`: a function run once, at mount, whose body declares
//! sources and effects and whose tail is `xui! { .. }` or `view! { .. }`.
//! It works out statically what each dynamic part of the view reads, and
//! emits a `Template` whose effects carry those dependency lists for the
//! runtime to bind at mount. See `docs/reactive.md`.
use std::collections::BTreeMap;

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use proc_macro2::TokenTree;
use syn::{
    BinOp, Error, Expr, ExprClosure, FnArg, GenericArgument, GenericParam, Ident, ItemFn, Local, LocalInit, Macro,
    Meta, Pat, PathArguments, Result, ReturnType, Stmt, Type,
};

use crate::analyze::{self, Found, Use, bindings, macro_name, rust};
use crate::element::{Body, Child, Element, ForChild, IfChild, MatchChild};
use crate::hosts::{HostKind, host_kind};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A parameter; `&T` in the signature, a field of the instance.
    Prop,
    /// A typed `let`, `state!` or `memo!`, kept on the instance.
    Field,
    /// An untyped `let`: setup only, unreachable from bindings.
    Local,
    /// Bound by the pattern of a block's branch or row, at `level`.
    Bound { level: usize },
}

/// The handle the instance keeps a source in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Handle {
    Signal,
    Memo,
    ReadSignal,
}

impl Handle {
    fn ident(self) -> Ident {
        let name = match self {
            Self::Signal => "Signal",
            Self::Memo => "Memo",
            Self::ReadSignal => "ReadSignal",
        };
        Ident::new(name, Span::call_site())
    }
}

#[derive(Clone)]
struct Name {
    ident: Ident,
    kind: Kind,
    /// The value's type: a prop's `T` of `&T`, a field's declared type.
    /// `None` for locals and bound names.
    ty: Option<Type>,
    /// For a source, the handle around its value.
    handle: Option<Handle>,
    /// Source index: the component's sources in declaration order, then
    /// one per row level for bound names.
    source: Option<u16>,
    /// Declaration position: props are 0, the body's statements count from 1.
    order: usize,
    /// A prop's `#[defaults(..)]` value, which makes it optional.
    default: Option<Expr>,
    /// A prop that is not a value: a callback, or children.
    special: Option<Special>,
}

#[derive(Clone)]
enum Special {
    /// `&Callback<A>`.
    Event,
    /// `&Children`.
    Children,
    /// `size!()`: a signal a host's layout writes.
    Size,
}

/// What a prop's type says it is beyond a value: `Callback<A>` (and its
/// `A`), or `Children`.
fn special(ty: &Type) -> Option<Special> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    match (last.ident.to_string().as_str(), &last.arguments) {
        ("Callback", PathArguments::AngleBracketed(args)) => match args.args.first() {
            Some(GenericArgument::Type(_)) if args.args.len() == 1 => Some(Special::Event),
            _ => None,
        },
        ("Children", PathArguments::None) => Some(Special::Children),
        _ => None,
    }
}

impl Name {
    /// Kept on the instance.
    fn instance(&self) -> bool {
        matches!(self.kind, Kind::Prop | Kind::Field)
    }

    fn field_ty(&self, k: &TokenStream) -> TokenStream {
        let ty = &self.ty;
        match self.handle {
            Some(handle) => {
                let handle = handle.ident();
                quote!(#k::reactive::#handle<#ty>)
            }
            None => quote!(#ty),
        }
    }
}

/// Where the view is, for name resolution: after every statement.
const VIEW: usize = usize::MAX;

/// How generated code hands names to user code.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Code that reruns when what it reads changes — memos, attribute
    /// values, block conditions: a source is its value, `&T`.
    Tracked,
    /// Effects, handlers and child props: a source is its handle.
    Handles,
}

type Uses = Vec<(Name, Use, Span)>;

/// A binding depends on the sources it reads, except under `untrack!`.
fn deps(uses: &Uses) -> Vec<u16> {
    let mut deps: Vec<u16> = uses.iter().filter(|u| u.1 == Use::Read).filter_map(|u| u.0.source).collect();
    deps.sort();
    deps.dedup();
    deps
}

fn option_u16(value: Option<u16>) -> TokenStream {
    match value {
        Some(v) => quote!(::core::option::Option::Some(#v)),
        None => quote!(::core::option::Option::None),
    }
}

/// Generated locals and parameters, unreachable from user code.
fn hidden(name: &str) -> Ident {
    Ident::new(name, Span::mixed_site())
}

fn pascal_case(ident: &Ident) -> Ident {
    let name: String = ident
        .to_string()
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect();
    Ident::new(&name, ident.span())
}

/// A `Signal<T>`, `Memo<T>` or `ReadSignal<T>` written out as a type: what
/// `state!`, `memo!` and `#[source]` declare instead.
fn handle_type(ty: &Type) -> Option<&'static str> {
    let Type::Path(path) = ty else { return None };
    let last = path.path.segments.last()?;
    let handle = match last.ident.to_string().as_str() {
        "Signal" => "Signal",
        "Memo" => "Memo",
        "ReadSignal" => "ReadSignal",
        _ => return None,
    };
    let PathArguments::AngleBracketed(args) = &last.arguments else { return None };
    matches!(args.args.first(), Some(GenericArgument::Type(_))).then_some(handle)
}

/// The variables an `if` condition's `let`s bind, through `&&` chains.
fn cond_bindings(cond: &Expr, out: &mut Vec<Ident>) {
    match cond {
        Expr::Let(l) => bindings(&l.pat, out),
        Expr::Binary(b) if matches!(b.op, BinOp::And(_)) => {
            cond_bindings(&b.left, out);
            cond_bindings(&b.right, out);
        }
        Expr::Paren(p) => cond_bindings(&p.expr, out),
        _ => {}
    }
}

/// `effect!(|cx| ..)`'s closure, which takes the `Cx` or nothing.
fn effect_closure(mac: &Macro) -> Result<ExprClosure> {
    const MESSAGE: &str = "`effect!` takes a closure of the `Cx`, or of nothing: `effect!(|cx| ..)`";
    match mac.parse_body::<Expr>() {
        Ok(Expr::Closure(closure)) if closure.inputs.len() <= 1 => Ok(closure),
        _ => Err(Error::new(mac.span(), MESSAGE)),
    }
}

/// `3`, `-1.5`: a literal whose type is whatever it is written for. A
/// suffixed one (`3usize`) has a type of its own, and converts like any value.
fn is_numeric_literal(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(int), .. }) => int.suffix().is_empty(),
        Expr::Lit(syn::ExprLit { lit: syn::Lit::Float(float), .. }) => float.suffix().is_empty(),
        Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => is_numeric_literal(&unary.expr),
        _ => false,
    }
}

/// `ticker!(|cx, frame| ..)`'s closure.
fn ticker_closure(mac: &Macro) -> Result<ExprClosure> {
    const MESSAGE: &str = "`ticker!` takes a closure of the `Cx` and the frame: `ticker!(|cx, frame| ..)`";
    match mac.parse_body::<Expr>() {
        Ok(Expr::Closure(closure)) if closure.inputs.len() <= 2 => Ok(closure),
        _ => Err(Error::new(mac.span(), MESSAGE)),
    }
}

/// The pattern binding a closure's `Cx`, if it takes one.
fn closure_cx(closure: &ExprClosure) -> Option<Pat> {
    closure.inputs.first().map(|input| match input {
        Pat::Type(typed) => (*typed.pat).clone(),
        pat => pat.clone(),
    })
}

fn closure_params(closure: &ExprClosure) -> Vec<Ident> {
    let mut idents = Vec::new();
    closure.inputs.iter().for_each(|pat| bindings(pat, &mut idents));
    idents
}

/// An attribute's value as an expression: `attr="lit"` is `attr={"lit"}`.
fn attr_expr(value: &TokenStream) -> Result<Expr> {
    syn::parse2(value.clone())
}

/// What the body's statements are to the macro.
#[allow(clippy::large_enum_variant)]
enum Setup {
    /// Copied into `create` as written.
    Verbatim(Stmt),
    /// `let name: T = state!(init);`, `name` an index into the names.
    State { name: usize, init: Expr },
    /// `let name: T = memo!(body);`
    Memo { name: usize, body: Expr },
    /// `effect!(|cx| ..);`
    Effect { closure: ExprClosure },
    /// `let name: T = context!();`
    Context { name: usize },
    /// `provide!(source_or_value);`
    Provide { expr: Expr },
    /// `ticker!(|cx, frame| ..)`, kept as field `name` when a `let` names it.
    Ticker { closure: ExprClosure, name: Option<usize> },
    /// `let name: Option<Size<f32>> = size!();`
    Size { name: usize },
    /// `let name: AsyncValue<T, E> = resource!(future);`
    Resource { name: usize, future: Expr },
}

/// Where generated code runs: the component itself (depth 0), or a row of
/// a block, `depth` blocks deep. A row's closures take `&Rc<Row<..>>` and
/// reach the component through `parent`s and their bound names through the
/// `data` of the row at each level.
#[derive(Clone, Default)]
struct Scope {
    depth: usize,
    /// Per level from 1: the pattern that destructures that row's data.
    patterns: Vec<TokenStream>,
    /// Innermost last.
    bound: Vec<Name>,
}

impl Scope {
    /// The closure parameter holding the scope.
    fn param(&self) -> Ident {
        hidden(if self.depth == 0 { "__this" } else { "__scope" })
    }

    fn ty(&self, k: &TokenStream) -> TokenStream {
        (0..self.depth).fold(quote!(Self), |inner, _| quote!(#k::reactive::Row<#inner, _>))
    }

    /// `|scope, rest|`, typed so the rows' data types are inferred.
    fn header(&self, k: &TokenStream, rest: TokenStream) -> TokenStream {
        let (param, ty) = (self.param(), self.ty(k));
        quote!(|#param: &::std::rc::Rc<#ty>, #rest|)
    }

    fn header1(&self, k: &TokenStream) -> TokenStream {
        let (param, ty) = (self.param(), self.ty(k));
        quote!(|#param: &::std::rc::Rc<#ty>|)
    }
}

/// One scope's template, as tokens.
#[derive(Default)]
struct Out {
    nodes: Vec<TokenStream>,
    effects: Vec<TokenStream>,
    children: Vec<TokenStream>,
}

struct Gen<'a> {
    /// The runtime crate's path.
    k: TokenStream,
    names: &'a [Name],
    /// How many sources the component has; row levels number after them.
    sources: u16,
    /// Hidden associated functions.
    fns: Vec<TokenStream>,
    effect_blocks: usize,
}

impl Gen<'_> {
    /// The names `parts` mention, resolved as Rust would at position `at`:
    /// the innermost bound name, else the last declaration before `at`.
    /// Each part is an expression and the names bound around it.
    fn uses(&self, scope: &Scope, at: usize, parts: &[(&Expr, &[Ident])]) -> Result<Uses> {
        let known = |n: &str| self.names.iter().chain(&scope.bound).any(|x| x.ident == n);
        let mut found = Found::new();
        for (expr, shadow) in parts {
            analyze::uses(expr, shadow, &known, &mut found);
        }
        let mut uses = Vec::new();
        for (ident, (used, span)) in found {
            let name = scope
                .bound
                .iter()
                .rev()
                .find(|n| n.ident == ident)
                .or_else(|| self.names.iter().rev().find(|n| n.ident == ident && n.order < at));
            let Some(name) = name else {
                let message = if self.names.iter().any(|n| n.ident == ident && n.order == at) {
                    format!("`{ident}` reads itself")
                } else {
                    format!("`{ident}` is declared after this")
                };
                return Err(Error::new(span, message));
            };
            if name.kind == Kind::Local {
                return Err(Error::new(
                    span,
                    format!(
                        "`{ident}` only exists while the component is created; give it a type \
                         (`let {ident}: T = ..`) to keep it on the instance"
                    ),
                ));
            }
            uses.push((name.clone(), used, span));
        }
        uses.sort_by_key(|(n, ..)| match n.kind {
            Kind::Bound { level } => (1, level),
            _ => (0, n.order),
        });
        Ok(uses)
    }

    /// Binds exactly the names `uses` lists: the component's by reference to
    /// its fields (to their values, in tracked code), bound ones by
    /// destructuring their row's data. Inner levels come last, so they shadow.
    fn prelude(&self, scope: &Scope, uses: &Uses, mode: Mode) -> TokenStream {
        let this = hidden("__this");
        let mut out = TokenStream::new();
        let component: Vec<&Name> = uses.iter().map(|u| &u.0).filter(|n| n.instance()).collect();
        if scope.depth > 0 && !component.is_empty() {
            let (param, parents) = (scope.param(), (0..scope.depth).map(|_| quote!(.parent)));
            out.extend(quote!(let #this: &Self = &#param #(#parents)*;));
        }
        for name in component {
            let id = &name.ident;
            if mode == Mode::Tracked && name.handle.is_some() {
                let guard = hidden(&format!("__read_{id}"));
                out.extend(quote! {
                    let #guard = #this.#id.read();
                    #[allow(unused_variables)]
                    let #id = &*#guard;
                });
            } else {
                out.extend(quote! {
                    #[allow(unused_variables)]
                    let #id = &#this.#id;
                });
            }
        }
        for level in 1..=scope.depth {
            if !uses.iter().any(|u| u.0.kind == Kind::Bound { level }) {
                continue;
            }
            let (param, parents) = (scope.param(), (level..scope.depth).map(|_| quote!(.parent)));
            let guard = hidden(&format!("__guard_{level}"));
            let pattern = &scope.patterns[level - 1];
            out.extend(quote! {
                let #guard = #param #(#parents)* .data.read();
                #[allow(unused_variables)]
                let #pattern = &*#guard;
            });
        }
        out
    }

    fn effect(&self, out: &mut Out, label: String, lane: &str, deps: Vec<u16>, output: Option<u16>, run: TokenStream) {
        let k = &self.k;
        let lane = Ident::new(lane, Span::call_site());
        let output = option_u16(output);
        out.effects.push(quote! {
            #k::reactive::EffectSpec {
                label: #label,
                lane: #k::reactive::Lane::#lane,
                deps: &[#(#deps),*],
                output: #output,
                run: #run,
            }
        });
    }

    /// Returns the statement creating the memo, with its first value, in `create`.
    fn memo(&mut self, out: &mut Out, memo: &Name, body: &Expr) -> Result<TokenStream> {
        let k = self.k.clone();
        let uses = self.uses(&Scope::default(), memo.order, &[(body, &[])])?;
        // A function of the values of exactly the names it reads, shared by
        // `create` and the effect.
        let (name, value) = (&memo.ident, &memo.ty);
        let f = format_ident!("__memo_{}", name);
        let params = uses.iter().map(|(n, ..)| {
            let (id, ty) = (&n.ident, &n.ty);
            quote!(#id: &#ty)
        });
        let body = rust(body.to_token_stream())?;
        self.fns.push(quote! {
            #[doc(hidden)]
            #[allow(unused_variables, clippy::all)]
            fn #f(#(#params),*) -> #value { #body }
        });
        let (this, cx) = (hidden("__this"), hidden("__cx"));
        let args = uses.iter().map(|(n, ..)| {
            let id = &n.ident;
            match n.handle {
                Some(_) => quote!(&*#this.#id.read()),
                None => quote!(&#this.#id),
            }
        });
        self.effect(
            out,
            format!("memo {name}"),
            "Memo",
            deps(&uses),
            memo.source,
            quote! {
                #k::reactive::Run::Cx(|#this: &::std::rc::Rc<Self>, #cx: &mut #k::reactive::Cx<'_>| {
                    let __value = Self::#f(#(#args),*);
                    #this.#name.__replace(#cx, __value);
                })
            },
        );

        // In `create`, props are already `&T` locals and fields are owned.
        let create_args = uses.iter().map(|(n, ..)| {
            let id = &n.ident;
            match (n.handle, n.kind) {
                (Some(_), _) => quote!(&*#id.read()),
                (None, Kind::Prop) => quote!(#id),
                (None, _) => quote!(&#id),
            }
        });
        let (ty, cx) = (memo.field_ty(&k), Ident::new("cx", Span::call_site()));
        Ok(quote!(let #name: #ty = #cx.memo(Self::#f(#(#create_args),*));))
    }

    /// A closure given the `Cx` and handles — an `effect!` or an `on_*`
    /// handler — as `prelude`, the `Cx` bound to the user's pattern, and the body.
    /// The closure's body with its `Cx` (`__cx`) and argument (`__arg`)
    /// bound to its patterns; it evaluates to the body's value when `returns`.
    fn with_cx(&self, scope: &Scope, at: usize, closure: &ExprClosure, returns: bool) -> Result<(Uses, TokenStream)> {
        self.with_cx_ecx(scope, at, closure, returns, None)
    }

    /// [`with_cx`](Self::with_cx), binding a third closure parameter to `ecx`:
    /// a host handler's `EventContext`.
    fn with_cx_ecx(
        &self,
        scope: &Scope,
        at: usize,
        closure: &ExprClosure,
        returns: bool,
        ecx: Option<&Ident>,
    ) -> Result<(Uses, TokenStream)> {
        let params = closure_params(closure);
        let uses = self.uses(scope, at, &[(&closure.body, &params)])?;
        let prelude = self.prelude(scope, &uses, Mode::Handles);
        let (cx, arg) = (hidden("__cx"), hidden("__arg"));
        let bind_cx = closure_cx(closure).map(|pat| quote!(let #pat = #cx;));
        let bind_arg = closure.inputs.iter().nth(1).map(|pat| quote!(let #pat = #arg;));
        let bind_ecx = ecx.and_then(|ecx| closure.inputs.iter().nth(2).map(|pat| quote!(let #pat = #ecx;)));
        let body = rust(closure.body.to_token_stream())?;
        let body = if returns { quote!({ #body }) } else { quote!({ #body };) };
        Ok((uses, quote!(#prelude #bind_cx #bind_arg #bind_ecx #body)))
    }

    /// `resource!(future)`: tracked code that starts `future` whenever what it
    /// reads changes; the result is written to the field.
    fn resource(&mut self, out: &mut Out, field: &Name, future: &Expr) -> Result<()> {
        let k = self.k.clone();
        let scope = Scope::default();
        let uses = self.uses(&scope, field.order, &[(future, &[])])?;
        let prelude = self.prelude(&scope, &uses, Mode::Tracked);
        let code = rust(future.to_token_stream())?;
        let (this, cx, id) = (hidden("__this"), hidden("__cx"), &field.ident);
        let run = quote! {
            #k::reactive::Run::Cx(|#this: &::std::rc::Rc<Self>, #cx: &mut #k::reactive::Cx<'_>| {
                let __future = { #prelude #code };
                #cx.__resource(&#this.#id, __future);
            })
        };
        self.effect(out, format!("resource {id}"), "Effect", deps(&uses), None, run);
        Ok(())
    }

    /// `ticker!(|cx, frame| ..)`: run by the frame loop, given handles and a `Cx`.
    fn ticker(&mut self, out: &mut Out, at: usize, closure: &ExprClosure) -> Result<()> {
        let k = self.k.clone();
        let (_, body) = self.with_cx(&Scope::default(), at, closure, true)?;
        let (this, cx, arg) = (hidden("__this"), hidden("__cx"), hidden("__arg"));
        let run = quote! {
            #k::reactive::Run::Tick(|#this: &::std::rc::Rc<Self>, #cx: &mut #k::reactive::Cx<'_>, #arg: #k::clock::FrameTime| -> #k::ticker::Tick {
                #body
            })
        };
        self.effect(out, format!("ticker {}", out.effects.len()), "Tick", Vec::new(), None, run);
        Ok(())
    }

    fn effect_block(&mut self, out: &mut Out, at: usize, closure: &ExprClosure) -> Result<()> {
        let k = self.k.clone();
        let index = self.effect_blocks;
        self.effect_blocks += 1;
        let scope = Scope::default();
        let (uses, body) = self.with_cx(&scope, at, closure, false)?;
        let cx = hidden("__cx");
        let header = scope.header(&k, quote!(#cx: &mut #k::reactive::Cx<'_>));
        let run = quote!(#k::reactive::Run::Cx(#header { #body }));
        self.effect(out, format!("effect {index}"), "Effect", deps(&uses), None, run);
        Ok(())
    }

    fn element(&mut self, scope: &Scope, out: &mut Out, element: &Element, parent: Option<u16>) -> Result<()> {
        if let Some(twice) = element.attrs.iter().enumerate().find_map(|(i, a)| {
            element.attrs[..i].iter().any(|b| b.name == a.name).then_some(&a.name)
        }) {
            return Err(Error::new(twice.span(), format!("`{twice}` is set twice")));
        }
        if element.tag.is_ident("slot") {
            return self.slot(scope, out, element, parent);
        }
        if element.tag.is_ident("portal") {
            return self.portal(scope, out, element, parent);
        }
        match host_kind(&element.tag) {
            Some(kind) => self.host(scope, out, element, kind, parent),
            None => self.component(scope, out, element, parent),
        }
    }

    /// `portal(z_index: 10, modal: true, on_dismiss: |cx, reason| ..) { .. }`:
    /// content mounted under the overlayer. Attributes are evaluated once, at
    /// mount, as `PortalSpec` builder calls.
    fn portal(&mut self, scope: &Scope, out: &mut Out, element: &Element, parent: Option<u16>) -> Result<()> {
        let k = self.k.clone();
        let index = out.nodes.len() as u16;
        out.nodes.push(TokenStream::new());
        let Body::Children(list) = &element.body else {
            return Err(Error::new(element.tag.span(), "a portal's content is its children: `portal { .. }`"));
        };
        let inner = self.row_scope(scope, quote!(()), &[]);
        let template = self.row_template(&inner, list)?;
        let (param, notifier, spec) = (scope.param(), hidden("__notifier"), hidden("__spec"));
        let mut calls = Vec::new();
        for attr in &element.attrs {
            let name = &attr.name;
            let expr = attr_expr(&attr.value)?;
            match (&expr, name == "on_dismiss") {
                (Expr::Closure(closure), true) => {
                    if closure.inputs.len() > 2 {
                        return Err(Error::new(closure.inputs.span(), "`on_dismiss` takes the `Cx` and the reason: `|cx, reason| ..`"));
                    }
                    let (_, body) = self.with_cx(scope, VIEW, closure, false)?;
                    let (kept, cx, arg) = (hidden("__kept_scope"), hidden("__cx"), hidden("__arg"));
                    calls.push(quote! {
                        let #spec = {
                            let #kept = ::std::rc::Rc::clone(#param);
                            #spec.__on_dismiss(#notifier, move |#cx: &mut #k::reactive::Cx<'_>, #arg| {
                                #[allow(unused_variables)]
                                let #param = &#kept;
                                #body
                            })
                        };
                    });
                }
                (_, true) => return Err(Error::new(name.span(), "`on_dismiss` takes a closure")),
                (_, false) => {
                    let uses = self.uses(scope, VIEW, &[(&expr, &[])])?;
                    let prelude = self.prelude(scope, &uses, Mode::Tracked);
                    let code = rust(expr.to_token_stream())?;
                    calls.push(quote!(let #spec = #spec.#name({ #prelude #code });));
                }
            }
        }
        let header = scope.header(&k, quote!(#notifier: &#k::reactive::Notifier));
        let parent = option_u16(parent);
        out.nodes[usize::from(index)] = quote! {
            #k::reactive::StaticNode {
                kind: #k::reactive::NodeKind::Portal(#header -> #k::reactive::PortalSpec {
                    let #spec = #k::reactive::PortalSpec::new(#k::reactive::__children(#param, #template));
                    #(#calls)*
                    #spec
                }),
                parent: #parent,
            }
        };
        Ok(())
    }

    /// `slot(children)`: where content a parent passed is mounted. Its
    /// expression is read once, as the slot mounts, with sources as values.
    fn slot(&mut self, scope: &Scope, out: &mut Out, element: &Element, parent: Option<u16>) -> Result<()> {
        let k = self.k.clone();
        let (Body::Content(content), true) = (&element.body, element.attrs.is_empty()) else {
            return Err(Error::new(element.tag.span(), "a slot takes the children it mounts: `slot(children)`"));
        };
        let uses = self.uses(scope, VIEW, &[(content, &[])])?;
        let prelude = self.prelude(scope, &uses, Mode::Tracked);
        let code = rust(content.to_token_stream())?;
        let header = scope.header1(&k);
        let parent = option_u16(parent);
        out.nodes.push(quote! {
            #k::reactive::StaticNode {
                kind: #k::reactive::NodeKind::Outlet(#header -> #k::reactive::Children {
                    #prelude
                    (#code).clone()
                }),
                parent: #parent,
            }
        });
        Ok(())
    }

    /// A host: a node whose draft its build function makes, and a render
    /// binding per attribute that reads a source.
    fn host(&mut self, scope: &Scope, out: &mut Out, element: &Element, kind: HostKind, parent: Option<u16>) -> Result<()> {
        let k = self.k.clone();
        let index = out.nodes.len() as u16;
        out.nodes.push(TokenStream::new());
        let tag = &element.tag;
        let (draft, table, notifier) = (hidden("__draft"), hidden("__table"), hidden("__notifier"));

        // `text(expr)`'s content is its `content` attribute.
        let mut attrs: Vec<(Ident, Expr)> = Vec::new();
        for attr in &element.attrs {
            attrs.push((attr.name.clone(), attr_expr(&attr.value)?));
        }
        let mut children: &[Child] = &[];
        match (&element.body, kind) {
            (Body::None, _) => {}
            (Body::Content(content), HostKind::Text) => attrs.push((Ident::new("content", content.span()), content.clone())),
            (Body::Children(list), HostKind::Parent) => children = list,
            (Body::Content(content), HostKind::Parent) => {
                return Err(Error::new(
                    content.span(),
                    "children are elements here; a component's own children are a `slot`",
                ));
            }
            (_, _) => return Err(Error::new(tag.span(), "this tag takes no children")),
        }

        let mut sets = Vec::new();
        let mut tracked: Vec<(&Expr, &[Ident])> = Vec::new();
        for (name, value) in &attrs {
            if name == "key" {
                return Err(Error::new(name.span(), "`key` keys the rows of a `for`; it goes on an element of its body"));
            }
            if name == "draft" {
                // An escape hatch: `draft: |d| d.animate(..)`, any draft method,
                // applied as the host is built; rebuilt when what it reads changes.
                let uses = self.uses(scope, VIEW, &[(value, &[])])?;
                let code = rust(value.to_token_stream())?;
                sets.push(quote!(let #draft = #k::reactive::__apply(#draft, #code);));
                tracked.push((value, &[]));
                let deps = deps(&uses);
                if !deps.is_empty() {
                    let header = scope.header(&k, quote!(__cx: &mut #k::reactive::Cx<'_>));
                    let run = quote!(#k::reactive::Run::Cx(#header { __cx.__rebuild(#index); }));
                    self.effect(out, format!("{} {index}.draft", crate::element::path_name(tag)), "Render", deps, None, run);
                }
                continue;
            }
            match (name.to_string().strip_prefix("on_"), value) {
                (Some(_), Expr::Closure(closure)) => {
                    if closure.inputs.len() > 3 {
                        return Err(Error::new(
                            closure.inputs.span(),
                            "a handler takes the `Cx`, the event and the event context: `|cx, event, ecx| ..`",
                        ));
                    }
                    let ecx = hidden("__ecx");
                    let (_, body) = self.with_cx_ecx(scope, VIEW, closure, true, Some(&ecx))?;
                    let param = scope.param();
                    let (kept_scope, kept_notifier, cx, arg) =
                        (hidden("__kept_scope"), hidden("__kept_notifier"), hidden("__cx"), hidden("__arg"));
                    sets.push(quote! {
                        let #draft = {
                            let #kept_scope = ::std::rc::Rc::clone(#param);
                            let #kept_notifier = ::core::clone::Clone::clone(#notifier);
                            #draft.__with_on(#table.#name(), move |#arg, #ecx| {
                                #[allow(unused_variables)]
                                let #param = &#kept_scope;
                                let mut #cx = #k::reactive::Cx::__handler(&#kept_notifier);
                                let #cx = &mut #cx;
                                #body
                            })
                        };
                    });
                }
                (Some(_), _) => return Err(Error::new(name.span(), format!("`{name}` takes a closure"))),
                (None, Expr::Path(path))
                    if path.qself.is_none()
                        && self.uses(scope, VIEW, &[(value, &[])])?.iter().any(|(n, ..)| {
                            matches!(n.special, Some(Special::Size)) && path.path.is_ident(&n.ident)
                        }) =>
                {
                    // A `size!()` source binds the host: its layout writes it.
                    let uses = self.uses(scope, VIEW, &[(value, &[])])?;
                    let prelude = self.prelude(scope, &uses, Mode::Handles);
                    sets.push(quote! {
                        let #draft = #draft.__with(#table.#name(), { #prelude #k::reactive::__size_observer(#path, #notifier) });
                    });
                }
                (None, value) => {
                    let uses = self.uses(scope, VIEW, &[(value, &[])])?;
                    let code = rust(value.to_token_stream())?;
                    sets.push(quote!(let #draft = #draft.__with(#table.#name(), { #code });));
                    tracked.push((value, &[]));
                    let deps = deps(&uses);
                    if deps.is_empty() {
                        continue;
                    }
                    // Rewritten whenever what it reads changes.
                    let prelude = self.prelude(scope, &uses, Mode::Tracked);
                    let header = scope.header(&k, quote!(__cx: &mut #k::reactive::Cx<'_>));
                    let run = quote! {
                        #k::reactive::Run::Cx(#header {
                            #prelude
                            #[allow(unused_imports)]
                            use #k::caps::tables::*;
                            let __value = { #code };
                            __cx.__attr(#index, #tag, |__table| __table.#name(), __value);
                        })
                    };
                    self.effect(out, format!("{} {index}.{name}", crate::element::path_name(tag)), "Render", deps, None, run);
                }
            }
        }

        // The build function evaluates every attribute, so it binds what any reads.
        let uses = self.uses(scope, VIEW, &tracked)?;
        let prelude = self.prelude(scope, &uses, Mode::Tracked);
        let header = scope.header(&k, quote!(#notifier: &#k::reactive::Notifier));
        let parent = option_u16(parent);
        out.nodes[usize::from(index)] = quote! {
            #k::reactive::StaticNode {
                kind: #k::reactive::NodeKind::Host(#header -> #k::reactive::HostDraft {
                    #prelude
                    #[allow(unused_imports)]
                    use #k::caps::tables::*;
                    let #draft = (#k::reactive::__host(#tag))();
                    #[allow(unused_variables)]
                    let #table = #draft.__properties();
                    #(#sets)*
                    #k::reactive::HostDraft::new(#draft)
                }),
                parent: #parent,
            }
        };
        self.children(scope, out, children, Some(index))
    }

    fn children(&mut self, scope: &Scope, out: &mut Out, children: &[Child], parent: Option<u16>) -> Result<()> {
        for child in children {
            match child {
                Child::Element(element) => self.element(scope, out, element, parent)?,
                Child::Expr(expr) => {
                    return Err(Error::new(expr.span(), "a child is an element or a block; a component's own children are a `slot`"));
                }
                Child::Let(stmt) => {
                    return Err(Error::new(stmt.span(), "a `let` among children is not supported; use a `memo!`"));
                }
                Child::If(block) => self.if_block(scope, out, block, parent)?,
                Child::For(block) => self.for_block(scope, out, block, parent)?,
                Child::Match(block) => self.match_block(scope, out, block, parent)?,
            }
        }
        Ok(())
    }

    /// `display(value: x)` builds `display()`'s draft through its property
    /// table and mounts it: each prop takes `x` as its declaration says
    /// (see `xui_core::reactive::props`). A source passed by name is passed
    /// as its handle; anything else as an expression of this scope.
    fn component(&mut self, scope: &Scope, out: &mut Out, element: &Element, parent: Option<u16>) -> Result<()> {
        let k = self.k.clone();
        let child = out.children.len() as u16;
        let parent = option_u16(parent);
        out.nodes.push(quote! {
            #k::reactive::StaticNode { kind: #k::reactive::NodeKind::Component(#child), parent: #parent }
        });
        let param = scope.param();
        let (draft, table) = (hidden("__draft"), hidden("__table"));
        let mut sets = Vec::new();
        // The content after the tag is the `children` prop.
        match &element.body {
            Body::None => {}
            Body::Children(list) => {
                let inner = self.row_scope(scope, quote!(()), &[]);
                let template = self.row_template(&inner, list)?;
                let children = Ident::new("children", element.tag.span());
                sets.push(quote! {
                    let #draft = #draft.__with(#table.#children(), #k::reactive::__children(#param, #template));
                });
            }
            Body::Content(content) => {
                return Err(Error::new(content.span(), "a component's children are elements: `tag { .. }`"));
            }
        }
        for attr in &element.attrs {
            let name = &attr.name;
            let expr = attr_expr(&attr.value)?;
            if let Expr::Macro(mac) = &expr
                && let Some(kind @ ("view" | "xui")) = macro_name(&mac.mac).as_deref()
            {
                // Elements given to a `Children` prop: a row of this scope.
                let list = if kind == "view" {
                    crate::view::child_list(mac.mac.tokens.clone())?
                } else {
                    vec![Child::Element(syn::parse2(mac.mac.tokens.clone())?)]
                };
                let inner = self.row_scope(scope, quote!(()), &[]);
                let template = self.row_template(&inner, &list)?;
                let with = quote_spanned!(name.span()=> #draft.__with(#table.#name(), #k::reactive::__children(#param, #template)));
                sets.push(quote!(let #draft = #with;));
                continue;
            }
            if let Expr::Closure(closure) = &expr {
                // A callback: the closure runs in this scope, with the `Cx` it is called with.
                if closure.inputs.len() > 2 {
                    return Err(Error::new(closure.inputs.span(), "a callback takes the `Cx` and its value: `|cx, value| ..`"));
                }
                let (_, body) = self.with_cx(scope, VIEW, closure, false)?;
                let (kept, cx, arg) = (hidden("__kept_scope"), hidden("__cx"), hidden("__arg"));
                let callback = quote! {
                    #k::reactive::__callback(#table.#name(), {
                        let #kept = ::std::rc::Rc::clone(#param);
                        move |#cx: &mut #k::reactive::Cx<'_>, #arg| {
                            #[allow(unused_variables)]
                            let #param = &#kept;
                            #body
                        }
                    })
                };
                let with = quote_spanned!(name.span()=> #draft.__with(#table.#name(), #callback));
                sets.push(quote!(let #draft = #with;));
                continue;
            }
            let uses = self.uses(scope, VIEW, &[(&expr, &[])])?;
            let source = match &expr {
                Expr::Path(path) if path.qself.is_none() && path.path.get_ident().is_some() => {
                    uses.iter().find(|(n, ..)| n.handle.is_some() && path.path.is_ident(&n.ident))
                }
                _ => None,
            };
            let arg = match source {
                Some((name, ..)) => {
                    let prelude = self.prelude(scope, &uses, Mode::Handles);
                    let id = &name.ident;
                    quote!({ #prelude #k::reactive::SourceArg(::core::clone::Clone::clone(#id)) })
                }
                None => {
                    let deps = deps(&uses);
                    let prelude = self.prelude(scope, &uses, Mode::Tracked);
                    // `(|v| ..)`: a closure as a value, not a callback. The
                    // parentheses said so; past here they would only be linted.
                    let value_expr = match &expr {
                        Expr::Paren(paren) if matches!(&*paren.expr, Expr::Closure(_)) => &*paren.expr,
                        other => other,
                    };
                    let code = rust(value_expr.to_token_stream())?;
                    // A numeric literal takes the prop's type; anything else is
                    // converted by the prop, which may infer the child's
                    // parameters from it.
                    let (kept, marker) = (hidden("__kept_scope"), hidden("__marker"));
                    let value = if is_numeric_literal(&expr) {
                        quote!(#code)
                    } else {
                        quote!(#k::reactive::__convert(#marker, { #code }))
                    };
                    quote! {
                        #k::reactive::__expr(
                            #table.#name(),
                            ::std::vec![#(#k::reactive::Scope::source(&**#param, #deps)),*],
                            {
                                let #kept = ::std::rc::Rc::clone(#param);
                                let #marker = #table.#name();
                                move || {
                                    #[allow(unused_variables)]
                                    let #param = &#kept;
                                    #prelude
                                    #value
                                }
                            },
                        )
                    }
                }
            };
            // Errors about the prop point at it.
            let with = quote_spanned!(name.span()=> #draft.__with(#table.#name(), #arg));
            sets.push(quote!(let #draft = #with;));
        }

        let tag = &element.tag;

        let header = scope.header1(&k);
        let label = crate::element::path_name(&element.tag);
        // A missing required prop is reported at the tag.
        let mount = quote_spanned!(tag.span()=> #k::reactive::MountDraft::__mount(#draft));
        out.children.push(quote! {
            #k::reactive::ChildSpec {
                label: #label,
                prepare: #header -> #k::reactive::PendingMount {
                    let #draft = #tag();
                    #[allow(unused_variables)]
                    let #table = #draft.__properties();
                    #(#sets)*
                    #mount
                },
            }
        });
        Ok(())
    }

    /// The scope one block deeper, whose rows' data `pattern` destructures.
    /// Its names shadow every outer one of the same spelling.
    fn row_scope(&self, scope: &Scope, pattern: TokenStream, idents: &[Ident]) -> Scope {
        let mut inner = scope.clone();
        inner.depth += 1;
        inner.patterns.push(pattern);
        let source = self.sources + inner.depth as u16 - 1;
        for ident in idents {
            inner.bound.push(Name {
                ident: ident.clone(),
                kind: Kind::Bound { level: inner.depth },
                ty: None,
                handle: None,
                source: Some(source),
                order: 0,
                default: None,
                special: None,
            });
        }
        inner
    }

    /// A row's template, as a `RowTemplate` literal.
    fn row_template(&mut self, scope: &Scope, body: &[Child]) -> Result<TokenStream> {
        let k = self.k.clone();
        let mut out = Out::default();
        self.children(scope, &mut out, body, None)?;
        let level = self.sources + scope.depth as u16 - 1;
        let Out { nodes, effects, children } = out;
        Ok(quote! {
            #k::reactive::RowTemplate {
                level: #level,
                nodes: &[#(#nodes),*],
                effects: &[#(#effects),*],
                children: &[#(#children),*],
            }
        })
    }

    /// Pushes the block's node and its effect, whose closure runs `control`
    /// after binding the names `parts` mention, as values.
    fn block(
        &mut self,
        scope: &Scope,
        out: &mut Out,
        parent: Option<u16>,
        kind: &str,
        parts: &[(&Expr, &[Ident])],
        control: TokenStream,
    ) -> Result<()> {
        let k = self.k.clone();
        let node = out.nodes.len() as u16;
        let parent_node = option_u16(parent);
        out.nodes.push(quote! {
            #k::reactive::StaticNode { kind: #k::reactive::NodeKind::Block, parent: #parent_node }
        });
        let uses = self.uses(scope, VIEW, parts)?;
        let prelude = self.prelude(scope, &uses, Mode::Tracked);
        let header = scope.header(&k, quote!(__block: &mut #k::reactive::BlockCx<'_>));
        let run = quote!(#k::reactive::Run::Block(#node, #header { #prelude #control }));
        self.effect(out, format!("{kind} {node}"), "Block", deps(&uses), None, run);
        Ok(())
    }

    /// `__block.arm(scope, index, (bound,), template)` for one branch.
    fn arm(&mut self, scope: &Scope, index: u16, idents: &[Ident], body: &[Child]) -> Result<TokenStream> {
        let data = quote!((#(#idents,)*));
        let inner = self.row_scope(scope, data.clone(), idents);
        let template = self.row_template(&inner, body)?;
        let parent = scope.param();
        Ok(quote!(__block.arm(#parent, #index, #data, #template)))
    }

    fn if_block(&mut self, scope: &Scope, out: &mut Out, block: &IfChild, parent: Option<u16>) -> Result<()> {
        // Flatten `else if` chains into arms.
        let mut arms: Vec<(Option<&Expr>, &[Child])> = Vec::new();
        let mut next = Some(block);
        while let Some(IfChild { cond, then, otherwise, .. }) = next {
            arms.push((Some(cond), then));
            next = match otherwise.as_deref() {
                Some([Child::If(inner)]) => Some(inner),
                Some(body) => {
                    arms.push((None, body));
                    None
                }
                None => None,
            };
        }
        let mut control = TokenStream::new();
        let mut parts: Vec<(&Expr, &[Ident])> = Vec::new();
        for (index, (cond, body)) in arms.iter().enumerate() {
            let mut idents = Vec::new();
            if let Some(cond) = cond {
                cond_bindings(cond, &mut idents);
                parts.push((cond, &[]));
            }
            let arm = self.arm(scope, index as u16, &idents, body)?;
            let cond = cond.map(|c| rust(c.to_token_stream())).transpose()?;
            control.extend(match (index, cond) {
                (0, Some(cond)) => quote!(if #cond { #arm }),
                (_, Some(cond)) => quote!(else if #cond { #arm }),
                (_, None) => quote!(else { #arm }),
            });
        }
        if arms.last().is_some_and(|(cond, _)| cond.is_some()) {
            control.extend(quote!(else { __block.none() }));
        }
        self.block(scope, out, parent, "if", &parts, control)
    }

    fn match_block(&mut self, scope: &Scope, out: &mut Out, block: &MatchChild, parent: Option<u16>) -> Result<()> {
        let arm_idents: Vec<Vec<Ident>> = block
            .arms
            .iter()
            .map(|arm| {
                let mut idents = Vec::new();
                bindings(&arm.pat, &mut idents);
                idents
            })
            .collect();
        let mut parts: Vec<(&Expr, &[Ident])> = vec![(&block.scrutinee, &[])];
        let mut arms = Vec::new();
        for (index, (arm, idents)) in block.arms.iter().zip(&arm_idents).enumerate() {
            let call = self.arm(scope, index as u16, idents, &arm.body)?;
            let pat = &arm.pat;
            let guard = match &arm.guard {
                Some(guard) => {
                    // The guard sees the arm's bindings.
                    parts.push((guard, idents));
                    let guard = rust(guard.to_token_stream())?;
                    Some(quote!(if #guard))
                }
                None => None,
            };
            arms.push(quote!(#pat #guard => #call,));
        }
        let scrutinee = rust(block.scrutinee.to_token_stream())?;
        let control = quote!(match #scrutinee { #(#arms)* });
        self.block(scope, out, parent, "match", &parts, control)
    }

    fn for_block(&mut self, scope: &Scope, out: &mut Out, block: &ForChild, parent: Option<u16>) -> Result<()> {
        let ForChild { pat, iter, body, .. } = block;
        // `.key(expr)` on an element of the body keys the rows.
        let mut body: Vec<Child> = body.iter().map(clone_child).collect();
        let mut key = None;
        for child in &mut body {
            if let Child::Element(element) = child
                && let Some(at) = element.attrs.iter().position(|a| a.name == "key")
            {
                let attr = element.attrs.remove(at);
                if key.is_some() {
                    return Err(Error::new(attr.name.span(), "a `for` row takes one `key`"));
                }
                key = Some(rust(attr.value)?);
            }
        }
        let mut idents = Vec::new();
        bindings(pat, &mut idents);
        let inner = self.row_scope(scope, pat.to_token_stream(), &idents);
        let template = self.row_template(&inner, &body)?;
        let parent_scope = scope.param();
        let items = rust(iter.to_token_stream())?;
        let control = match key {
            Some(key) => {
                let item = hidden("__item");
                quote! {
                    __block.each(#parent_scope, #items, |#item: &_| {
                        #[allow(unused_variables)]
                        let #pat = #item;
                        #key
                    }, #template)
                }
            }
            None => quote!(__block.each_indexed(#parent_scope, #items, #template)),
        };
        self.block(scope, out, parent, "for", &[(iter, &[])], control)
    }
}

/// `Child` is not `Clone`; a `for` body is copied to take its `key` out.
fn clone_child(child: &Child) -> Child {
    match child {
        Child::Element(element) => Child::Element(clone_element(element)),
        Child::Expr(expr) => Child::Expr(expr.clone()),
        Child::Let(stmt) => Child::Let(stmt.clone()),
        Child::If(block) => Child::If(IfChild {
            if_token: block.if_token,
            cond: block.cond.clone(),
            then: block.then.iter().map(clone_child).collect(),
            otherwise: block.otherwise.as_ref().map(|o| o.iter().map(clone_child).collect()),
        }),
        Child::For(block) => Child::For(ForChild {
            for_token: block.for_token,
            pat: block.pat.clone(),
            iter: block.iter.clone(),
            body: block.body.iter().map(clone_child).collect(),
        }),
        Child::Match(block) => Child::Match(MatchChild {
            match_token: block.match_token,
            scrutinee: block.scrutinee.clone(),
            arms: block
                .arms
                .iter()
                .map(|arm| crate::element::MatchArm {
                    pat: arm.pat.clone(),
                    guard: arm.guard.clone(),
                    body: arm.body.iter().map(clone_child).collect(),
                })
                .collect(),
        }),
    }
}

fn clone_element(element: &Element) -> Element {
    Element {
        tag: element.tag.clone(),
        attrs: element
            .attrs
            .iter()
            .map(|a| crate::element::Attribute { name: a.name.clone(), value: a.value.clone() })
            .collect(),
        body: match &element.body {
            Body::None => Body::None,
            Body::Content(expr) => Body::Content(expr.clone()),
            Body::Children(children) => Body::Children(children.iter().map(clone_child).collect()),
        },
    }
}

fn declare(
    names: &mut Vec<Name>,
    ident: &Ident,
    kind: Kind,
    ty: Option<Type>,
    handle: Option<Handle>,
    order: usize,
) -> Result<usize> {
    if ident == "cx" {
        return Err(Error::new(ident.span(), "`cx` is the component's context"));
    }
    // Locals shadow freely; the instance has one field per name.
    if kind != Kind::Local && names.iter().any(|n| n.instance() && n.ident == *ident) {
        return Err(Error::new(
            ident.span(),
            format!("`{ident}` is already kept on the instance; a prop, state, memo or typed `let` needs a name of its own"),
        ));
    }
    let source = handle.map(|_| names.iter().filter(|n| n.source.is_some()).count() as u16);
    names.push(Name { ident: ident.clone(), kind, ty, handle, source, order, default: None, special: None });
    Ok(names.len() - 1)
}

/// `effect!(..)` as a statement.
fn statement_macro(stmt: &Stmt) -> Option<&Macro> {
    match stmt {
        Stmt::Macro(mac) => Some(&mac.mac),
        Stmt::Expr(Expr::Macro(mac), _) => Some(&mac.mac),
        _ => None,
    }
}

/// Sorts a statement into [`Setup`], declaring the names a `let` introduces.
fn classify(stmt: Stmt, order: usize, names: &mut Vec<Name>) -> Result<Setup> {
    if let Some(mac) = statement_macro(&stmt) {
        match macro_name(mac).as_deref() {
            Some("effect") => return Ok(Setup::Effect { closure: effect_closure(mac)? }),
            Some("provide") => return Ok(Setup::Provide { expr: mac.parse_body()? }),
            Some("ticker") => return Ok(Setup::Ticker { closure: ticker_closure(mac)?, name: None }),
            _ => {}
        }
    }
    let Stmt::Local(Local { pat, init, .. }) = &stmt else { return Ok(Setup::Verbatim(stmt)) };
    let declaring = match init {
        Some(LocalInit { expr, diverge: None, .. }) => match &**expr {
            Expr::Macro(mac) => match macro_name(&mac.mac).as_deref() {
                Some("state") => Some(("state", &mac.mac)),
                Some("memo") => Some(("memo", &mac.mac)),
                Some("context") => Some(("context", &mac.mac)),
                Some("ticker") => Some(("ticker", &mac.mac)),
                Some("size") => Some(("size", &mac.mac)),
                Some("resource") => Some(("resource", &mac.mac)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };
    if let Some((what, mac)) = declaring {
        let (ident, ty) = match pat {
            Pat::Type(typed) => match &*typed.pat {
                Pat::Ident(ident) if ident.by_ref.is_none() && ident.mutability.is_none() && ident.subpat.is_none() => {
                    (&ident.ident, (*typed.ty).clone())
                }
                pat => return Err(Error::new(pat.span(), format!("a `{what}!` is `let name: T = {what}!(..)`"))),
            },
            Pat::Ident(ident) => {
                return Err(Error::new(
                    ident.span(),
                    format!("give `{0}` its value's type: `let {0}: T = {what}!(..)`", ident.ident),
                ));
            }
            pat => return Err(Error::new(pat.span(), format!("a `{what}!` is `let name: T = {what}!(..)`"))),
        };
        if handle_type(&ty).is_some() {
            return Err(Error::new(ty.span(), format!("type `{ident}` by its value: `let {ident}: T = {what}!(..)`")));
        }
        return Ok(match what {
            "state" => {
                let init = mac.parse_body()?;
                let name = declare(names, ident, Kind::Field, Some(ty), Some(Handle::Signal), order)?;
                Setup::State { name, init }
            }
            "memo" => {
                let body = mac.parse_body()?;
                let name = declare(names, ident, Kind::Field, Some(ty), Some(Handle::Memo), order)?;
                Setup::Memo { name, body }
            }
            "ticker" => {
                let closure = ticker_closure(mac)?;
                let name = declare(names, ident, Kind::Field, Some(ty), None, order)?;
                Setup::Ticker { closure, name: Some(name) }
            }
            "resource" => {
                let future = mac.parse_body()?;
                let name = declare(names, ident, Kind::Field, Some(ty), Some(Handle::Signal), order)?;
                Setup::Resource { name, future }
            }
            "size" => {
                if !mac.tokens.is_empty() {
                    return Err(Error::new(mac.tokens.span(), "`size!()` takes nothing; pass it to a host's `size_observer`"));
                }
                let name = declare(names, ident, Kind::Field, Some(ty), Some(Handle::Signal), order)?;
                names[name].special = Some(Special::Size);
                Setup::Size { name }
            }
            _ => {
                if !mac.tokens.is_empty() {
                    return Err(Error::new(mac.tokens.span(), "`context!()` takes nothing: its type is the `let`'s"));
                }
                let name = declare(names, ident, Kind::Field, Some(ty), Some(Handle::ReadSignal), order)?;
                Setup::Context { name }
            }
        });
    }
    match pat {
        Pat::Type(typed) => {
            let Pat::Ident(ident) = &*typed.pat else {
                return Err(Error::new(typed.pat.span(), "a typed `let` here names one field"));
            };
            if let Some(handle) = handle_type(&typed.ty) {
                let what = if handle == "Memo" { "memo" } else { "state" };
                return Err(Error::new(
                    typed.ty.span(),
                    format!("declare it by its value's type: `let {}: T = {what}!(..)`", ident.ident),
                ));
            }
            declare(names, &ident.ident, Kind::Field, Some((*typed.ty).clone()), None, order)?;
        }
        pat => {
            let mut idents = Vec::new();
            bindings(pat, &mut idents);
            for ident in &idents {
                declare(names, ident, Kind::Local, None, None, order)?;
            }
        }
    }
    Ok(Setup::Verbatim(stmt))
}

/// A prop: `name: &T`, with `#[source]` (a `ReadSignal<T>` it subscribes
/// to) or `#[state]` (a `Signal<T>` it may also write).
fn prop(input: &FnArg, names: &mut Vec<Name>, defaults: &mut Vec<(Ident, Expr)>) -> Result<()> {
    let FnArg::Typed(typed) = input else {
        return Err(Error::new(input.span(), "a component takes no `self`"));
    };
    let (Pat::Ident(ident), Type::Reference(reference)) = (&*typed.pat, &*typed.ty) else {
        return Err(Error::new(typed.span(), "a prop is `name: &T`"));
    };
    if reference.mutability.is_some() {
        return Err(Error::new(typed.ty.span(), "a prop is `name: &T`"));
    }
    let mut handle = None;
    for attr in &typed.attrs {
        let marker = match &attr.meta {
            Meta::Path(path) if path.is_ident("source") => Handle::ReadSignal,
            Meta::Path(path) if path.is_ident("state") => Handle::Signal,
            _ => return Err(Error::new(attr.span(), "a prop takes `#[source]` or `#[state]`")),
        };
        if handle.replace(marker).is_some() {
            return Err(Error::new(attr.span(), "a prop takes one of `#[source]` and `#[state]`"));
        }
    }
    let ty = (*reference.elem).clone();
    if has_impl_trait(&ty) {
        return Err(Error::new(
            reference.elem.span(),
            "a prop is stored on the component, so its type cannot be `impl Trait`: name a parameter instead, \
             as in `fn name<T: Trait>(prop: &T)`",
        ));
    }
    if handle.is_none() && handle_type(&ty).is_some() {
        let name = &ident.ident;
        return Err(Error::new(
            typed.ty.span(),
            format!(
                "declare a source prop by its value's type: `#[source] {name}: &T` to read it, \
                 `#[state] {name}: &T` to also write it"
            ),
        ));
    }
    let special = special(&ty);
    if special.is_some() && handle.is_some() {
        return Err(Error::new(typed.span(), "a callback or children prop is neither `#[source]` nor `#[state]`"));
    }
    let index = declare(names, &ident.ident, Kind::Prop, Some(ty), handle, 0)?;
    names[index].special = special;
    if let Some(at) = defaults.iter().position(|(name, _)| *name == ident.ident) {
        names[index].default = Some(defaults.remove(at).1);
    }
    Ok(())
}

/// A component's type and const parameters, which every generated item
/// carries: the marker, instance, props, draft, table and each prop's
/// property.
struct Generics {
    /// As declared, bounds included: `T: Display`, `const N: usize`.
    decl: Vec<TokenStream>,
    /// As arguments: `T`, `N`.
    args: Vec<Ident>,
    /// The `where` clause's predicates, plus `T: 'static` per type
    /// parameter, since an instance lives behind `Rc<dyn Any>`, and
    /// `PartialEq` for what a memo holds.
    preds: Vec<TokenStream>,
    /// `PhantomData` over every parameter, for an item no field of which
    /// names them all.
    phantom: TokenStream,
}

impl Generics {
    fn new(generics: &syn::Generics) -> Result<Self> {
        let (mut decl, mut args, mut preds, mut carried) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for param in &generics.params {
            match param {
                GenericParam::Lifetime(lifetime) => {
                    return Err(Error::new(
                        lifetime.span(),
                        "a component outlives its parent's borrows, so it takes no lifetime parameters",
                    ));
                }
                GenericParam::Type(ty) => {
                    let id = &ty.ident;
                    preds.push(quote!(#id: 'static));
                    carried.push(quote!(#id));
                    args.push(id.clone());
                }
                GenericParam::Const(c) => {
                    let id = &c.ident;
                    carried.push(quote!([(); #id]));
                    args.push(id.clone());
                }
            }
            // Defaults are not allowed on a function's parameters anyway.
            decl.push(param.to_token_stream());
        }
        if let Some(clause) = &generics.where_clause {
            preds.extend(clause.predicates.iter().map(ToTokens::to_token_stream));
        }
        let phantom = quote!(::core::marker::PhantomData<fn() -> (#(#carried,)*)>);
        Ok(Self { decl, args, preds, phantom })
    }

    fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    /// `<T: Bound, .., extra..>` for an `impl`, or nothing.
    fn params(&self, extra: &[Ident]) -> TokenStream {
        if self.decl.is_empty() && extra.is_empty() {
            return quote!();
        }
        let decl = &self.decl;
        quote!(<#(#decl,)* #(#extra,)*>)
    }

    /// `<T, N>` after a type's name, or nothing.
    fn args(&self) -> TokenStream {
        if self.args.is_empty() {
            return quote!();
        }
        let args = &self.args;
        quote!(<#(#args),*>)
    }

    /// `where ..`, with `extra` predicates, or nothing.
    fn where_clause(&self, extra: &[TokenStream]) -> TokenStream {
        if self.preds.is_empty() && extra.is_empty() {
            return quote!();
        }
        let preds = &self.preds;
        quote!(where #(#preds,)* #(#extra,)*)
    }

    /// Whether `ty` names one of the parameters: a prop of that type is
    /// where the parameter is inferred from.
    fn mentioned_by(&self, ty: &Type) -> bool {
        self.args.iter().any(|arg| names_ident(ty, arg))
    }
}

/// Whether `ty` mentions `ident` anywhere.
fn names_ident(ty: &Type, ident: &Ident) -> bool {
    fn walk(tokens: TokenStream, ident: &Ident) -> bool {
        tokens.into_iter().any(|tree| match tree {
            TokenTree::Ident(found) => found == *ident,
            TokenTree::Group(group) => walk(group.stream(), ident),
            _ => false,
        })
    }
    walk(ty.to_token_stream(), ident)
}

/// Whether `ty` contains `impl Trait`, which a struct field cannot hold.
fn has_impl_trait(ty: &Type) -> bool {
    fn walk(tokens: TokenStream) -> bool {
        tokens.into_iter().any(|tree| match tree {
            TokenTree::Ident(ident) => ident == "impl",
            TokenTree::Group(group) => walk(group.stream()),
            _ => false,
        })
    }
    walk(ty.to_token_stream())
}

/// `#[defaults(name = expr, ..)]` on the component.
fn defaults(attrs: &[syn::Attribute]) -> Result<Vec<(Ident, Expr)>> {
    let mut out = Vec::new();
    for attr in attrs {
        // Docs go on the marker; see `protocol`.
        if attr.path().is_ident("doc") {
            continue;
        }
        if !attr.path().is_ident("defaults") {
            return Err(Error::new(attr.span(), "a component takes `#[defaults(..)]`, and docs"));
        }
        let entries = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated,
        )?;
        for entry in entries {
            let Some(name) = entry.path.get_ident() else {
                return Err(Error::new(entry.path.span(), "a default names a prop"));
            };
            out.push((name.clone(), entry.value));
        }
    }
    Ok(out)
}

/// `xui! { .. }` or `view! { .. }` as the body's tail.
fn tail_view(stmt: Option<Stmt>) -> Result<Element> {
    const MESSAGE: &str = "a component ends in `xui! { .. }` or `view! { .. }`";
    let mac = match stmt {
        Some(Stmt::Macro(mac)) if mac.semi_token.is_none() => mac.mac,
        Some(Stmt::Expr(Expr::Macro(mac), None)) => mac.mac,
        other => {
            let span = other.map_or(Span::call_site(), |s| s.span());
            return Err(Error::new(span, MESSAGE));
        }
    };
    let view: Element = match mac.path.segments.last().map(|s| s.ident.to_string()).as_deref() {
        Some("xui") => syn::parse2(mac.tokens)?,
        Some("view") => syn::parse2::<crate::view::View>(mac.tokens)?.0,
        _ => return Err(Error::new(mac.path.span(), MESSAGE)),
    };
    Ok(view)
}

pub fn expand(function: ItemFn) -> Result<TokenStream> {
    let k = crate::krate::xui()?;
    let ItemFn { attrs, vis, sig, block } = function;
    let mut defaults = defaults(&attrs)?;
    if sig.asyncness.is_some() || !matches!(sig.output, ReturnType::Default) {
        return Err(Error::new(sig.span(), "a component is a plain `fn name(prop: &T, ..)` returning nothing"));
    }
    let mut generics = Generics::new(&sig.generics)?;
    // A component may share a host's name (`xui_components::image::image`):
    // only a qualified path reaches it, since a bare tag of that name is the host.
    if sig.ident == "slot" || sig.ident == "portal" {
        return Err(Error::new(
            sig.ident.span(),
            format!("`{}` is a built-in tag; a component needs a name of its own", sig.ident),
        ));
    }
    let marker = pascal_case(&sig.ident);
    let name = format_ident!("__{}Instance", marker);
    let props_name = format_ident!("{}Props", marker);

    let mut names = Vec::new();
    for input in &sig.inputs {
        prop(input, &mut names, &mut defaults)?;
    }
    if let Some((name, _)) = defaults.first() {
        return Err(Error::new(name.span(), format!("no prop is called `{name}`")));
    }

    let mut stmts = block.stmts;
    let view = tail_view(stmts.pop())?;
    let mut setup = Vec::new();
    for (i, stmt) in stmts.into_iter().enumerate() {
        setup.push(classify(stmt, i + 1, &mut names)?);
    }

    // A field a later untyped `let` shadows moves to a hidden name just
    // before that `let`, which still sees it by reference, so the instance
    // is built from the field and not from the local.
    let mut moves: BTreeMap<usize, Vec<&Ident>> = BTreeMap::new();
    let mut moved = Vec::new();
    for field in names.iter().filter(|n| n.kind == Kind::Field) {
        if let Some(local) = names.iter().find(|n| n.kind == Kind::Local && n.ident == field.ident && n.order > field.order) {
            moves.entry(local.order).or_default().push(&field.ident);
            moved.push(field.ident.clone());
        }
    }
    let kept = |ident: &Ident| hidden(&format!("__field_{ident}"));

    // A memo, and a `#[source]` prop (which may resolve to one), compares
    // values to tell whether it changed: implied, like `'static`, for a type
    // that names a parameter.
    for n in &names {
        let compared = matches!((n.kind, n.handle), (Kind::Prop, Some(Handle::ReadSignal)) | (_, Some(Handle::Memo)));
        if let Some(ty) = n.ty.as_ref().filter(|ty| compared && generics.mentioned_by(ty)) {
            generics.preds.push(quote!(#ty: ::core::cmp::PartialEq));
        }
    }
    let sources = names.iter().filter(|n| n.source.is_some()).count() as u16;
    let mut g = Gen { k: k.clone(), names: &names, sources, fns: Vec::new(), effect_blocks: 0 };
    let mut out = Out::default();
    let mut create = Vec::new();
    let user_cx = Ident::new("cx", Span::call_site());
    for (i, step) in setup.iter().enumerate() {
        let order = i + 1;
        for ident in moves.get(&order).into_iter().flatten() {
            let kept = kept(ident);
            create.push(quote!(let #kept = #ident; let #ident = &#kept;));
        }
        match step {
            Setup::Verbatim(stmt) => create.push(rust(stmt.to_token_stream())?),
            Setup::State { name, init } => {
                let n = &names[*name];
                let (id, ty, init) = (&n.ident, n.field_ty(&k), rust(init.to_token_stream())?);
                create.push(quote!(let #id: #ty = #user_cx.signal(#init);));
            }
            Setup::Memo { name, body } => create.push(g.memo(&mut out, &names[*name], body)?),
            Setup::Effect { closure } => g.effect_block(&mut out, order, closure)?,
            Setup::Ticker { closure, name } => {
                g.ticker(&mut out, order, closure)?;
                create.push(match name {
                    Some(name) => {
                        let (id, ty) = (&names[*name].ident, &names[*name].ty);
                        quote!(let #id: #ty = #user_cx.__ticker();)
                    }
                    None => quote!(#user_cx.__ticker();),
                });
            }
            Setup::Resource { name, future } => {
                let n = &names[*name];
                let (id, ty) = (&n.ident, n.field_ty(&k));
                g.resource(&mut out, n, future)?;
                create.push(quote!(let #id: #ty = #user_cx.signal(#k::reactive::AsyncValue::Pending);));
            }
            Setup::Size { name } => {
                let n = &names[*name];
                let (id, ty) = (&n.ident, n.field_ty(&k));
                create.push(quote!(let #id: #ty = #user_cx.signal(::core::option::Option::None);));
            }
            Setup::Context { name } => {
                let n = &names[*name];
                let (id, ty, value) = (&n.ident, n.field_ty(&k), &n.ty);
                create.push(quote!(let #id: #ty = #user_cx.context::<#value>();));
            }
            Setup::Provide { expr } => {
                // A source by name is provided as itself, anything else as a constant.
                let source = match expr {
                    Expr::Path(path) if path.qself.is_none() => path.path.get_ident().and_then(|ident| {
                        names.iter().rev().find(|n| n.ident == *ident && n.order < order && n.handle.is_some())
                    }),
                    _ => None,
                };
                let code = rust(expr.to_token_stream())?;
                create.push(match source {
                    Some(_) => quote!(#user_cx.provide(#code.read_only());),
                    None => quote!(#user_cx.provide_value(#code);),
                });
            }
        }
    }
    g.element(&Scope::default(), &mut out, &view, None)?;

    let props = hidden("__props");
    let phantom = hidden("__phantom");
    let mut fields = Vec::new();
    let mut prop_fields = Vec::new();
    let mut prop_lets = Vec::new();
    let mut inits = Vec::new();
    for n in names.iter().filter(|n| n.instance()) {
        let (id, ty) = (&n.ident, n.field_ty(&k));
        fields.push(quote!(pub #id: #ty));
        if n.kind == Kind::Prop {
            prop_fields.push(quote!(pub #id: #ty));
            prop_lets.push(quote!(let #id = &#props.#id;));
            inits.push(quote!(#id: #props.#id));
        } else if moved.contains(id) {
            let kept = kept(id);
            inits.push(quote!(#id: #kept));
        } else {
            inits.push(quote!(#id));
        }
    }
    let source_names = names.iter().filter(|n| n.source.is_some()).map(|n| n.ident.to_string());
    let source_arms = names.iter().filter_map(|n| {
        let (index, id) = (n.source?, &n.ident);
        Some(quote!(#index => #k::reactive::Source::id(&self.#id),))
    });
    // Hidden items carry the parameters as phantom data whatever their fields
    // name; the props struct only when no prop names one, since a caller
    // writes its literal.
    if !generics.is_empty() {
        let ty = &generics.phantom;
        fields.push(quote!(#phantom: #ty));
        inits.push(quote!(#phantom: ::core::marker::PhantomData));
    }
    let props_phantom = generics.args.iter().any(|arg| {
        !names
            .iter()
            .filter(|n| n.kind == Kind::Prop)
            .any(|n| names_ident(n.ty.as_ref().expect("a prop has a type"), arg))
    });
    if props_phantom {
        let ty = &generics.phantom;
        prop_fields.push(quote!(pub #phantom: #ty));
    }
    let fns = g.fns;
    let Out { nodes, effects, children } = out;
    let docs: Vec<_> = attrs.iter().filter(|a| a.path().is_ident("doc")).collect();
    let protocol = protocol(&k, &vis, &marker, &sig.ident, &props_name, &name, &names, &docs, &generics, props_phantom);
    let (params, args, where_clause) = (generics.params(&[]), generics.args(), generics.where_clause(&[]));

    Ok(quote! {
        #protocol

        #[doc(hidden)]
        #vis struct #name #params #where_clause { #(#fields,)* }

        #[derive(Clone)]
        #vis struct #props_name #params #where_clause { #(#prop_fields,)* }

        impl #params #name #args #where_clause {
            #(#fns)*
        }

        impl #params #k::reactive::Scope for #name #args #where_clause {
            fn source(&self, index: u16) -> #k::reactive::SourceId {
                match index {
                    #(#source_arms)*
                    _ => ::core::unreachable!("no source {}", index),
                }
            }
        }

        impl #params #k::reactive::ComponentDef for #name #args #where_clause {
            type Props = #props_name #args;

            const TEMPLATE: #k::reactive::Template<Self> = #k::reactive::Template {
                sources: &[#(#source_names),*],
                nodes: &[#(#nodes),*],
                effects: &[#(#effects),*],
                children: &[#(#children),*],
            };

            #[allow(unused_variables)]
            fn create(#props: Self::Props, #user_cx: &mut #k::reactive::MountCx<'_>) -> Self {
                #(#prop_lets)*
                #(#create)*
                Self { #(#inits,)* }
            }
        }
    })
}

/// The component as a node — its marker, its props draft and property
/// table, one property per prop, the requirement state of its required
/// props — so that a parent builds it through the table like a host, and
/// `__mount` resolves the draft into the instance's props.
fn protocol(
    k: &TokenStream,
    vis: &syn::Visibility,
    marker: &Ident,
    tag: &Ident,
    props_name: &Ident,
    instance: &Ident,
    names: &[Name],
    docs: &[&syn::Attribute],
    generics: &Generics,
    props_phantom: bool,
) -> TokenStream {
    let props: Vec<&Name> = names.iter().filter(|n| n.kind == Kind::Prop).collect();
    let draft_props = format_ident!("{}Draft", props_name);
    let table = format_ident!("{}Properties", marker);
    // Flat, not a module: a component in a function body has no path to
    // its own module, and a prop's type is written relative to the component.
    let property = |n: &Name| format_ident!("__xui_{}_{}", tag, n.ident);
    let requirements = format_ident!("{}Requirements", marker);
    let phantom = hidden("__phantom");
    let (params, args, where_clause) = (generics.params(&[]), generics.args(), generics.where_clause(&[]));
    let generic = !generics.is_empty();
    let phantom_ty = &generics.phantom;

    // What a draft holds per prop: how it is delivered, until mount.
    let stored = |n: &Name| {
        let ty = &n.ty;
        if n.special.is_some() {
            return quote!(#ty);
        }
        match n.handle {
            None => quote!(#ty),
            Some(Handle::ReadSignal) => quote!(#k::reactive::Tracked<#ty>),
            Some(_) => quote!(#k::reactive::Signal<#ty>),
        }
    };
    let mut draft_fields: Vec<TokenStream> = props
        .iter()
        .map(|n| {
            let (id, stored) = (&n.ident, stored(n));
            quote!(pub #id: ::core::option::Option<#stored>)
        })
        .collect();
    let mut draft_defaults: Vec<TokenStream> = props
        .iter()
        .map(|n| {
            let id = &n.ident;
            quote!(#id: ::core::option::Option::None)
        })
        .collect();
    if generic {
        draft_fields.push(quote!(#phantom: #phantom_ty));
        draft_defaults.push(quote!(#phantom: ::core::marker::PhantomData));
    }

    let required: Vec<&&Name> = props.iter().filter(|n| n.default.is_none() && n.special.is_none()).collect();
    let state = |n: &Name, what: &str| format_ident!("{}{}{}", marker, pascal_case(&n.ident), what);
    let states: Vec<Ident> = required.iter().map(|n| format_ident!("__{}State", pascal_case(&n.ident))).collect();

    let value = Ident::new("__XuiValue", Span::call_site());
    let handle = Ident::new("__XuiHandle", Span::call_site());
    let properties = props.iter().map(|n| {
        let (field, ty, stored) = (&n.ident, &n.ty, stored(n));
        let id = property(n);
        let requirement = if n.default.is_some() || n.special.is_some() {
            quote!(#k::property::NotRequired)
        } else {
            quote!(Self)
        };
        let item = match &n.special {
            Some(Special::Children) => quote!(#k::reactive::Children),
            Some(Special::Event | Special::Size) | None => quote!(#ty),
        };
        // An `impl` of this property, over the component's parameters and `extra`.
        let header = |extra: &[Ident], bounds: &[TokenStream]| {
            (generics.params(extra), generics.where_clause(bounds))
        };
        let (p, w) = header(&[], &[]);
        let accepts = match (&n.special, n.handle) {
            (Some(Special::Children), _) => quote! {
                impl #p #k::property::Accepts<#ty> for #id #args #w {
                    fn accept(value: #ty) -> #ty {
                        value
                    }
                }
                // Children passed on: `trigger: trigger`.
                impl #p #k::property::Accepts<#k::reactive::Expr<#k::reactive::Children>> for #id #args #w {
                    fn accept(value: #k::reactive::Expr<#k::reactive::Children>) -> #ty {
                        value.once()
                    }
                }
            },
            (Some(_), _) => quote! {
                impl #p #k::property::Accepts<#ty> for #id #args #w {
                    fn accept(value: #ty) -> #ty {
                        value
                    }
                }
                // A parent's closure, or a callback passed on.
                impl #p #k::property::Accepts<#k::reactive::Expr<#ty>> for #id #args #w {
                    fn accept(value: #k::reactive::Expr<#ty>) -> #ty {
                        value.once()
                    }
                }
            },
            (None, None) => {
                let (ph, wh) = header(&[handle.clone()], &[quote!(#handle: #k::reactive::Current<#ty>)]);
                quote! {
                    impl #p #k::property::Accepts<#k::reactive::Expr<#ty>> for #id #args #w {
                        fn accept(value: #k::reactive::Expr<#ty>) -> #ty {
                            value.once()
                        }
                    }
                    impl #ph #k::property::Accepts<#k::reactive::SourceArg<#handle>> for #id #args #wh {
                        fn accept(value: #k::reactive::SourceArg<#handle>) -> #ty {
                            value.0.current()
                        }
                    }
                }
            }
            (None, Some(Handle::ReadSignal)) => {
                let (ph, wh) = header(
                    &[handle.clone()],
                    &[quote!(#handle: ::core::convert::Into<#k::reactive::ReadSignal<#ty>>)],
                );
                quote! {
                    impl #p #k::property::Accepts<#k::reactive::Expr<#ty>> for #id #args #w {
                        fn accept(value: #k::reactive::Expr<#ty>) -> #stored {
                            #k::reactive::Tracked::Derived(value)
                        }
                    }
                    impl #ph #k::property::Accepts<#k::reactive::SourceArg<#handle>> for #id #args #wh {
                        fn accept(value: #k::reactive::SourceArg<#handle>) -> #stored {
                            #k::reactive::Tracked::Handle(value.0.into())
                        }
                    }
                }
            }
            (None, Some(_)) => quote! {
                impl #p #k::property::Accepts<#k::reactive::SourceArg<#k::reactive::Signal<#ty>>> for #id #args #w {
                    fn accept(value: #k::reactive::SourceArg<#k::reactive::Signal<#ty>>) -> #stored {
                        value.0
                    }
                }
            },
        };
        // How a parent's expression becomes the prop: converted into it, or,
        // when its type names a parameter, taken as is so the parameter is
        // inferred from it. A `#[state]` prop takes no expression.
        let from_value = match n.handle {
            Some(Handle::Signal) => quote!(),
            _ if generics.mentioned_by(ty.as_ref().expect("a prop has a type")) => quote! {
                impl #p #k::reactive::FromValue<#item> for #id #args #w {
                    fn from_value(value: #item) -> #item {
                        value
                    }
                }
            },
            _ => {
                let (pv, wv) = header(&[value.clone()], &[quote!(#value: ::core::convert::Into<#item>)]);
                quote! {
                    impl #pv #k::reactive::FromValue<#value> for #id #args #wv {
                        fn from_value(value: #value) -> #item {
                            ::core::convert::Into::into(value)
                        }
                    }
                }
            }
        };
        let doc = format!("The `{field}` prop of `{tag}`.");
        let def = if generic {
            quote! {
                #vis enum #id #params #where_clause {
                    #[doc(hidden)]
                    __Phantom(::core::convert::Infallible, #phantom_ty),
                }
            }
        } else {
            quote!(#vis enum #id {})
        };
        quote! {
            #[doc = #doc]
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            #def

            impl #p #k::property::Property for #id #args #w {
                type Scope = #k::node::OwnProps<#marker #args>;
                type Value = #stored;
                type Requirement = #requirement;

                fn set(props: &mut #draft_props #args, value: #stored) {
                    props.#field = ::core::option::Option::Some(value);
                }
            }

            impl #p #k::reactive::PropKind for #id #args #w {
                type Item = #item;
            }

            #accepts
            #from_value
        }
    });
    let table_methods = props.iter().map(|n| {
        let (id, property) = (&n.ident, property(n));
        quote! {
            #[doc(hidden)]
            #[inline(always)]
            pub fn #id(&self) -> #k::property::PropertyMarker<#property #args> {
                #k::property::PropertyMarker::new()
            }
        }
    });

    // The requirement state: one `Missing`/`Set` parameter per required prop.
    let (requirements_def, initial, complete, state_bounds) = if required.is_empty() {
        (quote!(), quote!(#k::property::Complete), quote!(#k::property::Complete), Vec::new())
    } else {
        let markers = required.iter().map(|n| {
            let (missing, set, is_set) = (state(n, "Missing"), state(n, "Set"), state(n, "IsSet"));
            let message = format!("required prop `{}` of `{tag}` was never set", n.ident);
            let note = format!("set it with `{}: ..`, or give it a default with `#[defaults({} = ..)]`", n.ident, n.ident);
            quote! {
                #[doc(hidden)]
                #[diagnostic::on_unimplemented(message = #message, label = "missing here", note = #note)]
                #vis trait #is_set {}
                #[doc(hidden)]
                #vis struct #missing;
                #[doc(hidden)]
                #vis struct #set;
                impl #is_set for #set {}
            }
        });
        let fulfils = required.iter().enumerate().map(|(i, n)| {
            let id = property(n);
            let output = states.iter().enumerate().map(|(j, p)| if i == j { state(required[j], "Set") } else { p.clone() });
            let (p, w) = (generics.params(&states), generics.where_clause(&[]));
            quote! {
                impl #p #k::property::Fulfil<#id #args> for #requirements<#(#states),*> #w {
                    type Output = #requirements<#(#output),*>;
                }
            }
        });
        let missing = required.iter().map(|n| state(n, "Missing"));
        let bounds = required
            .iter()
            .zip(&states)
            .map(|(n, p)| {
                let is_set = state(n, "IsSet");
                quote!(#p: #is_set)
            })
            .collect();
        (
            quote! {
                #(#markers)*
                #[doc(hidden)]
                #vis struct #requirements<#(#states),*>(::core::marker::PhantomData<fn() -> (#(#states,)*)>);
                #(#fulfils)*
            },
            quote!(#requirements<#(#missing),*>),
            quote!(#requirements<#(#states),*>),
            bounds,
        )
    };
    let (mount_params, mount_where) = (generics.params(&states), generics.where_clause(&state_bounds));

    let mut resolve: Vec<TokenStream> = props
        .iter()
        .map(|n| {
            let id = &n.ident;
            let missing = format!("required prop `{id}` was marked as set by the draft's requirement state");
            let value = match &n.default {
                // Callbacks and children default to none.
                None if n.special.is_some() => quote!(__draft.#id.unwrap_or_default()),
                Some(default) => {
                    // A numeric literal takes the prop's type; anything else converts into it.
                    let ty = &n.ty;
                    let value = match default {
                        Expr::Lit(syn::ExprLit { lit: syn::Lit::Int(_) | syn::Lit::Float(_), .. }) => {
                            quote!({ let __default: #ty = #default; __default })
                        }
                        _ => quote!(::core::convert::Into::<#ty>::into(#default)),
                    };
                    let default = match n.handle {
                        Some(Handle::ReadSignal) => quote!(#k::reactive::Tracked::Constant(#value)),
                        _ => value,
                    };
                    quote!(__draft.#id.unwrap_or_else(|| #default))
                }
                None => quote!(__draft.#id.expect(#missing)),
            };
            match n.handle {
                Some(Handle::ReadSignal) => quote!(#id: #value.resolve(__cx)),
                _ => quote!(#id: #value),
            }
        })
        .collect();
    if props_phantom {
        resolve.push(quote!(#phantom: ::core::marker::PhantomData));
    }

    let marker_doc = format!("The `{tag}` component.");
    let marker_doc = if docs.is_empty() { quote!(#[doc = #marker_doc]) } else { quote!(#(#docs)*) };
    let (marker_def, table_def) = if generic {
        (
            quote!(#vis struct #marker #params (#phantom_ty) #where_clause;),
            quote! {
                #[doc(hidden)]
                #vis struct #table #params (#phantom_ty) #where_clause;

                impl #params ::core::default::Default for #table #args #where_clause {
                    fn default() -> Self {
                        Self(::core::marker::PhantomData)
                    }
                }
            },
        )
    } else {
        (
            quote!(#vis struct #marker;),
            quote! {
                #[doc(hidden)]
                #[derive(Default)]
                #vis struct #table;
            },
        )
    };
    quote! {
        #marker_doc
        #marker_def

        #[doc(hidden)]
        #vis struct #draft_props #params #where_clause { #(#draft_fields,)* }

        impl #params ::core::default::Default for #draft_props #args #where_clause {
            fn default() -> Self {
                Self { #(#draft_defaults,)* }
            }
        }

        #table_def

        impl #params #table #args #where_clause {
            #(#table_methods)*
        }

        impl #params #k::node::NodeDef for #marker #args #where_clause {
            type Kind = #k::node::ComponentKind;
            type Props = #draft_props #args;
            type Style = ();
            type Behavior = ();
            type Properties = #table #args;
        }

        impl #params #k::reactive::Component for #marker #args #where_clause {
            type Instance = #instance #args;
        }

        #(#properties)*

        #requirements_def

        #vis fn #tag #params () -> #k::node::Draft<#marker #args, #initial> #where_clause {
            #k::node::Draft::with_requirements(::core::default::Default::default())
        }

        impl #mount_params #k::reactive::MountDraft<#marker #args> for #k::node::Draft<#marker #args, #complete>
        #mount_where
        {
            fn __mount(self) -> #k::reactive::PendingMount {
                let __draft = self.storage.props;
                #k::reactive::__pending_with::<#instance #args>(move |__cx| #props_name { #(#resolve,)* })
            }
        }
    }
}
