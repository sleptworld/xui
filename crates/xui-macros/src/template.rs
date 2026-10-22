//! Lowering an element tree to a static template (see `xui_core::template`).
//!
//! The tree is split at compile time:
//!
//! - **host nodes** — elements whose tag names a host widget — make up the
//!   template's structure. Each attribute is a *constant* (built only from
//!   literals and paths, re-evaluated only when the template mounts) or an
//!   *input* (evaluated where the macro is written, every render, and
//!   converted to its property's value right there);
//! - a **component** — any other tag — is a slot, built now as an element;
//! - a host's child list holding anything but elements is one **fragment**
//!   slot, built now as a list, its elements templates of their own.
//!
//! The generated code is a `static` `TemplateMeta`, the inputs as a tuple,
//! and a non-capturing build function that turns the inputs into the hosts'
//! drafts — every host's on mount, the dynamic ones' on an update — and the
//! slots' contents.

use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Error, Expr, Ident, Path};

use crate::element::{Attribute, Body, Build, Child, Element, Setter, expand_statements};
use crate::errors::Errors;

/// What a host tag's body can be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostKind {
    /// Takes children.
    Parent,
    /// Takes no body.
    Leaf,
    /// `<text>{content}</text>`: its body is its content.
    Text,
}

/// The host widgets, by tag. The set is closed, so the macro can tell a host
/// from a component by name; the generated code asserts each one
/// (`xui_core::template::host`).
fn host_kind(tag: &Path) -> Option<HostKind> {
    // Qualified application components may share a built-in tag's name.
    // Only canonical runtime paths are eligible for host-template lowering.
    if tag.segments.len() > 1 {
        let names: Vec<_> = tag.segments.iter().map(|s| s.ident.to_string()).collect();
        let runtime = matches!(names[0].as_str(), "xui" | "xui_core");
        if !runtime || !(names.len() == 2 || names.get(1).is_some_and(|s| s == "prelude")) {
            return None;
        }
    }
    let segment = tag.segments.last()?;
    if !segment.arguments.is_none() {
        return None;
    }
    Some(match segment.ident.to_string().as_str() {
        "container" | "row" | "column" | "center" | "grid" | "z_stack" | "image" => {
            HostKind::Parent
        }
        "text_input" | "canvas" | "icon" => HostKind::Leaf,
        "text" => HostKind::Text,
        _ => return None,
    })
}

/// Whether `element` is a host node a template can hold: a host tag with a
/// body of the kind it takes. Anything else is built as an element, which
/// is where a wrong body is reported.
fn is_template_host(element: &Element) -> bool {
    match (host_kind(&element.tag), &element.body) {
        (Some(HostKind::Parent), _) => true,
        (Some(HostKind::Leaf), Body::None) => true,
        (Some(HostKind::Text), Body::None | Body::Content(_)) => true,
        _ => false,
    }
}

/// An attribute built only from literals, paths and pure constructors —
/// `Color::BLACK`, `EdgeInsets::all(4.0)`, `Some(2.0)` — is constant, and
/// evaluated once, when the template mounts. A path of one lowercase segment
/// may be a local, so it is not; neither is a method call or a macro.
///
/// A call is only constant when it cannot be anything but pure, since a
/// constant is never evaluated again: `Instant::now()` or `Id::new()` would
/// otherwise keep the value they had at mount. See [`constant_call`].
fn is_constant(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(_) => true,
        Expr::Path(path) => path.qself.is_none() && constant_path(&path.path),
        Expr::Paren(inner) => is_constant(&inner.expr),
        Expr::Group(inner) => is_constant(&inner.expr),
        Expr::Unary(unary) => is_constant(&unary.expr),
        Expr::Binary(binary) => is_constant(&binary.left) && is_constant(&binary.right),
        Expr::Cast(cast) => is_constant(&cast.expr),
        Expr::Tuple(tuple) => tuple.elems.iter().all(is_constant),
        Expr::Array(array) => array.elems.iter().all(is_constant),
        Expr::Call(call) => {
            matches!(call.func.as_ref(), Expr::Path(path) if path.qself.is_none()
                && constant_call(&path.path, call.args.len()))
                && call.args.iter().all(is_constant)
        }
        Expr::Struct(literal) => {
            literal.rest.is_none() && literal.fields.iter().all(|field| is_constant(&field.expr))
        }
        _ => false,
    }
}

/// The xui value types whose associated functions are all pure: any call
/// `Type::f(..)` on one of them, given constants, is a constant.
const PURE_TYPES: &[&str] = &[
    "Color",
    "Duration",
    "EdgeInsets",
    "Easing",
    "GridTracks",
    "Point",
    "Size",
    "Sizing",
    "StyleOf",
    "Transition",
];

/// Constructors that are pure on any type when given something to build
/// from. Without arguments they are not trusted: `Id::new()` is as likely to
/// hand out a fresh id as to build an empty value.
const PURE_CONSTRUCTORS: &[&str] = &["new", "from", "default"];

/// Whether calling `path` with `arity` constant arguments is a constant:
/// a tuple struct or variant (`Some`, `Sizing::Fixed`), any function of a
/// [`PURE_TYPES`] type, or a [`PURE_CONSTRUCTORS`] constructor given at least
/// one argument (`String::from("a")`).
fn constant_call(path: &Path, arity: usize) -> bool {
    let mut segments = path.segments.iter().rev().map(|segment| segment.ident.to_string());
    let Some(function) = segments.next() else {
        return false;
    };
    if function.chars().next().is_some_and(char::is_uppercase) {
        return true;
    }
    let Some(owner) = segments.next() else {
        return false;
    };
    PURE_TYPES.contains(&owner.as_str())
        || (arity > 0
            && owner.chars().next().is_some_and(char::is_uppercase)
            && PURE_CONSTRUCTORS.contains(&function.as_str()))
}

/// Whether everything `element` is given is a constant: its attributes, and
/// its body. Such a component is built once, when the template mounts.
fn element_is_constant(element: &Element) -> bool {
    let attrs = element.attrs.iter().all(|Attribute { name, value }| {
        Setter::of(name, value) == Setter::Value
            && syn::parse2::<Expr>(value.clone()).is_ok_and(|expr| is_constant(&expr))
    });
    attrs
        && match &element.body {
            Body::None => true,
            Body::Content(expr) => is_constant(expr),
            Body::Children(children) => children.iter().all(|child| match child {
                Child::Element(child) => element_is_constant(child),
                _ => false,
            }),
        }
}

fn constant_path(path: &Path) -> bool {
    let last = path
        .segments
        .last()
        .map(|segment| segment.ident.to_string());
    path.segments.len() > 1
        || last.is_some_and(|name| name.chars().next().is_some_and(char::is_uppercase))
}

struct Ir<'a> {
    xui: &'a TokenStream2,
    /// One per node, in preorder: its `TemplateNode`.
    meta: Vec<TokenStream2>,
    /// `let __xui_vN = ..;`, in evaluation order.
    inputs: Vec<TokenStream2>,
    input_names: Vec<Ident>,
    /// The build function's body, in preorder.
    build: Vec<TokenStream2>,
    /// The commit function's body: the dynamic values written into hosts
    /// that an update patches, in preorder.
    commit: Vec<TokenStream2>,
    /// One `ValueSlot` per dynamic host attribute, in preorder.
    values: Vec<TokenStream2>,
    root_key: Option<Ident>,
}

impl Ir<'_> {
    fn input(&mut self, span: Span, value: TokenStream2) -> Ident {
        let name = Ident::new(
            &format!("__xui_v{}", self.input_names.len()),
            Span::call_site(),
        );
        self.inputs.push(quote_spanned!(span=> let #name = #value;));
        self.input_names.push(name.clone());
        name
    }

    /// Input `name`, taken out of the inputs where the build or commit
    /// function uses it.
    fn take(&self, name: &Ident) -> TokenStream2 {
        let xui = self.xui;
        quote!(#xui::template::take(#name))
    }

    fn index(&self, span: Span, errors: &mut Errors) -> u16 {
        u16::try_from(self.meta.len()).unwrap_or_else(|_| {
            errors.push(Error::new(span, "a template holds at most 65535 nodes"));
            0
        })
    }

    fn parent_tokens(parent: Option<u16>) -> TokenStream2 {
        match parent {
            Some(parent) => quote!(::core::option::Option::Some(#parent)),
            None => quote!(::core::option::Option::None),
        }
    }

    /// Adds `element`, a host node, and everything under it.
    fn host(&mut self, element: &Element, parent: Option<u16>, errors: &mut Errors) {
        let xui = self.xui;
        let tag = &element.tag;
        let index = self.index(tag.span(), errors);
        let meta_slot = self.meta.len();
        self.meta.push(TokenStream2::new());

        let mut dynamic = false;
        let mut sets = Vec::new();
        // An update writes a host's dynamic attributes into it alone, unless
        // one of them cannot be (`patchable`), or the host has an attribute
        // a patch cannot follow: `style`, which layers over everything set
        // before it, or a key of its own.
        let mut patches = Vec::new();
        let mut patchable = Vec::new();
        let mut patch_eligible = true;
        let host_tag = quote_spanned!(tag.span()=> #xui::template::host(#tag));
        let mut attrs: Vec<(Ident, TokenStream2, Setter)> = element
            .attrs
            .iter()
            .map(|Attribute { name, value }| (name.clone(), value.clone(), Setter::of(name, value)))
            .collect();
        if let (Some(HostKind::Text), Body::Content(content)) = (host_kind(tag), &element.body) {
            let name = Ident::new("content", content.span());
            attrs.push((name, quote!(#content), Setter::Value));
        }
        for (name, value, setter) in attrs {
            if name == "key" {
                let key = self.input(
                    name.span(),
                    quote!(::core::convert::Into::<#xui::fiber::Key>::into(#value)),
                );
                if parent.is_none() {
                    self.root_key = Some(key.clone());
                    sets.push(quote! {
                        let __xui_node = __xui_node
                            .key(::core::clone::Clone::clone(#xui::template::input(#key)));
                    });
                } else {
                    dynamic = true;
                    patch_eligible = false;
                    let key = self.take(&key);
                    sets.push(quote!(let __xui_node = __xui_node.key(#key);));
                }
                continue;
            }
            if name == "style" {
                patch_eligible = false;
            }
            let constant = setter == Setter::Value
                && syn::parse2::<Expr>(value.clone()).is_ok_and(|expr| is_constant(&expr));
            if constant {
                sets.push(quote_spanned! {name.span()=>
                    let __xui_property = __xui_node.__properties().#name();
                    let __xui_node = __xui_node.__with(__xui_property, #value);
                });
                continue;
            }
            dynamic = true;
            let convert = match setter {
                Setter::Value => quote_spanned!(name.span()=> value),
                Setter::Handler => quote_spanned!(name.span()=> handler),
                Setter::Shared => quote_spanned!(name.span()=> shared),
            };
            let convert_value = |value| {
                quote_spanned! {name.span()=>
                    #xui::template::#convert(#host_tag, |__xui_table| __xui_table.#name(), #value)
                }
            };
            let expression = if setter == Setter::Handler {
                crate::element::map_handler(value, convert_value)
            } else {
                convert_value(value)
            };
            let input = self.input(name.span(), expression);
            let input = self.take(&input);
            let attribute = name.to_string();
            self.values.push(quote! {
                #xui::template::ValueSlot { node: #index, attribute: #attribute }
            });
            sets.push(quote_spanned! {name.span()=>
                let __xui_property = __xui_node.__properties().#name();
                let __xui_node = __xui_node.__set(__xui_property, #input);
            });
            patches.push(quote_spanned! {name.span()=>
                __xui_target.__patch(__xui_target.__properties().#name(), #input)
            });
            patchable.push(quote_spanned! {name.span()=>
                #xui::template::patchable(#host_tag, |__xui_table| __xui_table.#name())
            });
        }

        let tag_name = tag
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        let parent_tokens = Self::parent_tokens(parent);
        self.meta[meta_slot] = quote! {
            #xui::template::TemplateNode {
                kind: #xui::template::TemplateNodeKind::Host { tag: #tag_name },
                parent: #parent_tokens,
            }
        };
        let whole = quote! {
            let __xui_node = (#host_tag)();
            #(#sets)*
            __xui_out.host(#index, __xui_node);
        };
        // Which hosts an update builds is known here: a host without a
        // dynamic attribute only ever mounts.
        if !dynamic {
            self.build.push(quote! {
                if __xui_out.mounting() {
                    #whole
                }
            });
        } else if patch_eligible && !patches.is_empty() {
            // On an update its dynamic values are left in the inputs, and
            // the commit function writes them into the mounted host.
            let patchable = quote!(#(#patchable)&&*);
            self.build.push(quote! {
                if __xui_out.mounting() || !(#patchable) {
                    #whole
                }
            });
            self.commit.push(quote! {
                if #patchable {
                    __xui_hosts.patch(#index, #host_tag, |__xui_target| {
                        #(#patches)|*
                    });
                }
            });
        } else {
            self.build.push(quote!({ #whole }));
        }

        match &element.body {
            Body::None => {}
            Body::Content(expr) => {
                if host_kind(tag) != Some(HostKind::Text) {
                    let list = quote_spanned! {expr.span()=>
                        {
                            let mut __xui_children = ::std::vec::Vec::new();
                            #xui::IntoChildren::append_children(#expr, &mut __xui_children);
                            __xui_children
                        }
                    };
                    self.fragment(index, list, errors);
                }
            }
            Body::Children(children) => {
                // Each child in its own place: an element in the template, and
                // each `if`, `for`, `match` or `{expr}` a list slot of its own,
                // so the elements around one stay in the template.
                let inputs_start = self.inputs.len();
                let names_start = self.input_names.len();
                for child in children {
                    match child {
                        Child::Element(child) if is_template_host(child) => {
                            self.host(child, Some(index), errors);
                        }
                        Child::Element(child) => self.component(child, index, errors),
                        // In scope for the siblings after it, which are
                        // evaluated with it, in order, among the inputs.
                        Child::Let(stmt) => self.inputs.push(stmt.to_token_stream()),
                        control => {
                            let statements =
                                expand_statements(std::slice::from_ref(control), xui, errors);
                            let list = quote! {
                                {
                                    let mut __xui_children = ::std::vec::Vec::new();
                                    #statements
                                    __xui_children
                                }
                            };
                            self.fragment(index, list, errors);
                        }
                    }
                }
                // A `let` is in scope for its siblings only: what this child
                // list evaluated goes in a block of its own, out of which only
                // the inputs come.
                if children.iter().any(|child| matches!(child, Child::Let(_))) {
                    let statements = self.inputs.split_off(inputs_start);
                    let names = &self.input_names[names_start..];
                    self.inputs.push(quote! {
                        let (#(#names,)*) = {
                            #(#statements)*
                            (#(#names,)*)
                        };
                    });
                }
            }
        }
    }

    fn component(&mut self, element: &Element, parent: u16, errors: &mut Errors) {
        let xui = self.xui;
        self.index(element.tag.span(), errors);
        // Kept typed: a component's props type names it, and becomes part
        // of the template's identity through its inputs.
        let built = element.expand_eager(xui, Build::Slot, errors);
        self.meta.push(quote! {
            #xui::template::TemplateNode {
                kind: #xui::template::TemplateNodeKind::Component,
                parent: ::core::option::Option::Some(#parent),
            }
        });
        if element_is_constant(element) {
            // Built once, in the build function, and kept on every update:
            // the same constant props cannot render anything new. Its type
            // is an input all the same: in a generic function the same
            // constant tag can name another component.
            self.input(
                element.tag.span(),
                quote!(#xui::template::slot_kind(|| #built)),
            );
            self.build.push(quote! {
                if __xui_out.mounting() {
                    __xui_out.slot(#built);
                } else {
                    __xui_out.keep();
                }
            });
        } else {
            let input = self.input(element.tag.span(), built);
            let input = self.take(&input);
            self.build.push(quote!(__xui_out.slot(#input);));
        }
    }

    fn fragment(&mut self, parent: u16, list: TokenStream2, errors: &mut Errors) {
        let xui = self.xui;
        self.index(Span::call_site(), errors);
        let input = self.input(
            Span::call_site(),
            quote!({ let __xui_list: ::std::vec::Vec<#xui::ElementDesc> = #list; __xui_list }),
        );
        self.meta.push(quote! {
            #xui::template::TemplateNode {
                kind: #xui::template::TemplateNodeKind::Fragment,
                parent: ::core::option::Option::Some(#parent),
            }
        });
        let input = self.take(&input);
        self.build.push(quote!(__xui_out.fragment(#input);));
    }
}

/// `element` as an expression: a template when its tag is a host it can
/// hold, the element built now otherwise.
pub(crate) fn lower(element: &Element, xui: &TokenStream2, errors: &mut Errors) -> TokenStream2 {
    if !is_template_host(element) {
        return element.expand_eager(xui, Build::Element, errors);
    }
    element.check_duplicate_attrs(errors);
    let mut ir = Ir {
        xui,
        meta: Vec::new(),
        inputs: Vec::new(),
        input_names: Vec::new(),
        build: Vec::new(),
        commit: Vec::new(),
        values: Vec::new(),
        root_key: None,
    };
    ir.host(element, None, errors);

    let Ir {
        meta,
        inputs,
        input_names,
        build,
        commit,
        values,
        root_key,
        ..
    } = ir;
    let tag = &element.tag;
    let key = match root_key {
        Some(key) => quote!(::core::option::Option::Some(::core::clone::Clone::clone(&#key))),
        None => quote!(::core::option::Option::None),
    };
    quote! {
        {
            #[allow(unused_imports)]
            use #xui::caps::tables::*;
            static __XUI_TEMPLATE: #xui::template::TemplateMeta = #xui::template::TemplateMeta {
                location: ::core::concat!(::core::file!(), ":", ::core::line!(), ":", ::core::column!()),
                nodes: &[#(#meta),*],
                values: &[#(#values),*],
            };
            #(#inputs)*
            #xui::ElementDesc::from(#xui::template::TemplateDesc::new(
                &__XUI_TEMPLATE,
                #key,
                #xui::template::host_type(#tag),
                (#(::core::option::Option::Some(#input_names),)*),
                |__xui_in, __xui_out: &mut #xui::template::TemplateBuild| {
                    #[allow(unused_imports)]
                    use #xui::caps::tables::*;
                    #[allow(unused_variables)]
                    let (#(#input_names,)*) = __xui_in;
                    #(#build)*
                },
                |__xui_in, __xui_hosts: &mut #xui::template::TemplateHosts<'_>| {
                    #[allow(unused_imports)]
                    use #xui::caps::tables::*;
                    #[allow(unused_variables)]
                    let (#(#input_names,)*) = __xui_in;
                    #(#commit)*
                },
            ))
        }
    }
}
