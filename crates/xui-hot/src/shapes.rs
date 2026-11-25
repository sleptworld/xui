//! Whether a change to the binary crate's sources alters what the running
//! app already holds in memory, which a patch cannot carry.
//!
//! A patch replaces code, not data. Values the app made keep the layout
//! they were made with: a struct given a field, read by the new code, is
//! read wrong. The runtime mounts a changed component again, so what a
//! component keeps is safe; a type defined outside one is not, nor is a
//! static's type or initial value (the app's is the one in use), a trait's method list (the
//! vtables of trait objects the app made), or a `const` a type's size
//! depends on (`[u8; N]`).
//!
//! So each source's *shapes* — those items, as tokens — are compared with
//! the running version's; a change to one is reported, and the app is
//! rebuilt instead of patched. This is conservative: a struct changed that
//! nothing alive holds still restarts.
//!
//! # Code a kept component calls
//!
//! A component whose own code did not change is kept, and its bindings run
//! again, through the newest code. But a prop that reads no signal was
//! evaluated once, when its host was built, and a handler is a closure the
//! host holds: both keep calling the old version of what they call. So the
//! code that changed — functions, methods, consts, statics, macros — is
//! followed, by the identifiers that name it, to every component that names
//! it, directly or through other code; those are mounted again too. A
//! name shared by unrelated items mounts more than needed, never fewer.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

/// One shape-giving item.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shape {
    /// `struct`, `static`, `trait`, …, for messages.
    kind: &'static str,
    tokens: String,
}

/// The shapes of one file, by the item's path inside it
/// (`model::Todo`, `main::Local`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Shapes {
    items: BTreeMap<String, Shape>,
    /// `const`s, apart: one matters only if a type names it.
    consts: BTreeMap<String, String>,
    /// Every identifier the file's type definitions use.
    type_idents: HashSet<String>,
    /// Code a component may call, by the item's path.
    code: BTreeMap<String, Code>,
    /// Each component, by function name: the identifiers its code uses.
    components: BTreeMap<String, HashSet<String>>,
    /// The statics that hold state, and whether each is a thread-local.
    mutable_statics: Vec<(String, bool)>,
}

/// A function, method, const, static or macro.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Code {
    tokens: String,
    /// How code that uses it names it: any one of these identifier sets
    /// (`label`; `Todo` and `new` for `Todo::new()`).
    known_as: Vec<Vec<String>>,
    /// The identifiers it uses.
    refs: HashSet<String>,
}

/// What a change to the sources takes.
pub struct Check {
    /// What changed shape: a patch cannot carry it.
    pub reshaped: Vec<Reshaped>,
    /// Components, by function name, that call code which changed.
    pub remount: Vec<String>,
    /// The sources as they are now.
    pub candidate: Candidate,
}

/// The running version of each of the binary crate's sources.
#[derive(Debug, Default)]
pub struct Snapshot {
    files: HashMap<PathBuf, (String, Option<Shapes>)>,
}

/// A change a patch cannot carry: `struct Todo` in `src/model.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reshaped {
    pub kind: &'static str,
    pub name: String,
    pub file: PathBuf,
}

/// The sources as they are now, to [`Snapshot::commit`] once a patch of
/// them is in.
pub struct Candidate(Vec<(PathBuf, String, Option<Shapes>)>);

impl Snapshot {
    /// Reads `files` as they are: what the app was just built from.
    pub fn take<'a>(files: impl IntoIterator<Item = &'a PathBuf>) -> Self {
        let files = files
            .into_iter()
            .filter_map(|path| {
                let text = std::fs::read_to_string(path).ok()?;
                let shapes = shapes(&text);
                Some((path.clone(), (text, shapes)))
            })
            .collect();
        Self { files }
    }

    /// Compares every source (a patch compiles them all, not only the one
    /// just saved) with its running version: what changed shape, which
    /// components call code that changed, and the sources as they are now.
    ///
    /// A source that does not parse is left out: it does not compile
    /// either, and is looked at again when it does.
    pub fn check(&self) -> Check {
        let mut reshaped = Vec::new();
        let mut candidate = Vec::new();
        let mut now = HashMap::new();
        for (path, (text, _)) in &self.files {
            let Ok(new_text) = std::fs::read_to_string(path) else {
                continue;
            };
            if &new_text == text {
                continue;
            }
            let new = shapes(&new_text);
            now.insert(path.clone(), new.clone());
            candidate.push((path.clone(), new_text, new));
        }
        // A const matters if a type anywhere names it, before or after.
        let mut type_idents: HashSet<&str> = HashSet::new();
        for (path, (_, old)) in &self.files {
            for shapes in [old.as_ref(), now.get(path).and_then(Option::as_ref)]
                .into_iter()
                .flatten()
            {
                type_idents.extend(shapes.type_idents.iter().map(String::as_str));
            }
        }
        for (path, new) in &now {
            let (Some(old), Some(new)) = (self.files[path].1.as_ref(), new.as_ref()) else {
                continue;
            };
            for (name, shape) in &old.items {
                if new.items.get(name).is_some_and(|now| now != shape) {
                    reshaped.push(Reshaped {
                        kind: shape.kind,
                        name: name.clone(),
                        file: path.clone(),
                    });
                }
            }
            for (name, tokens) in &old.consts {
                let ident = name.rsplit("::").next().unwrap_or(name);
                if new.consts.get(name).is_some_and(|now| now != tokens)
                    && type_idents.contains(ident)
                {
                    reshaped.push(Reshaped {
                        kind: "const",
                        name: name.clone(),
                        file: path.clone(),
                    });
                }
            }
        }
        reshaped.sort_by(|a, b| (&a.file, &a.name).cmp(&(&b.file, &b.name)));
        let remount = self.callers(&now);
        Check {
            reshaped,
            remount,
            candidate: Candidate(candidate),
        }
    }

    /// The components that call code changed in `now`, directly or through
    /// other code.
    fn callers(&self, now: &HashMap<PathBuf, Option<Shapes>>) -> Vec<String> {
        let mut changed: HashSet<&[String]> = HashSet::new();
        for (path, new) in now {
            let (Some(old), Some(new)) = (self.files[path].1.as_ref(), new.as_ref()) else {
                continue;
            };
            for (key, code) in &new.code {
                if old
                    .code
                    .get(key)
                    .is_some_and(|old| old.tokens != code.tokens)
                {
                    changed.extend(code.known_as.iter().map(Vec::as_slice));
                }
            }
        }
        if changed.is_empty() {
            return Vec::new();
        }
        let current: Vec<&Shapes> = self
            .files
            .iter()
            .filter_map(|(path, (_, old))| match now.get(path) {
                Some(Some(new)) => Some(new),
                _ => old.as_ref(),
            })
            .collect();
        let names = |refs: &HashSet<String>, changed: &HashSet<&[String]>| {
            changed
                .iter()
                .any(|name| name.iter().all(|ident| refs.contains(ident)))
        };
        // Code that calls changed code behaves differently too.
        loop {
            let mut grew = false;
            for shapes in &current {
                for code in shapes.code.values() {
                    let known = code.known_as.iter().map(Vec::as_slice);
                    if known.clone().all(|name| changed.contains(name)) {
                        continue;
                    }
                    if names(&code.refs, &changed) {
                        changed.extend(known);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let mut remount: Vec<String> = current
            .iter()
            .flat_map(|shapes| &shapes.components)
            .filter(|(_, refs)| names(refs, &changed))
            .map(|(name, _)| name.clone())
            .collect();
        remount.sort();
        remount.dedup();
        remount
    }

    /// The binary crate's statics that hold state: by name, file, and
    /// whether a thread-local. A patch shares the others with the app on
    /// arm64 ([`crate::alias`]); elsewhere, and a thread-local anywhere,
    /// each patch has a fresh copy of its own, which the code it brings
    /// uses, while the code it keeps goes on with the app's.
    pub fn mutable_statics(&self) -> Vec<(String, PathBuf, bool)> {
        let mut statics: Vec<(String, PathBuf, bool)> = self
            .files
            .iter()
            .filter_map(|(path, (_, shapes))| Some((path, shapes.as_ref()?)))
            .flat_map(|(path, shapes)| {
                shapes
                    .mutable_statics
                    .iter()
                    .map(move |(name, local)| (name.clone(), path.clone(), *local))
            })
            .collect();
        statics.sort();
        statics
    }

    /// Reads the files of `files` it has no version of: modules a patch
    /// just compiled for the first time.
    pub fn track<'a>(&mut self, files: impl IntoIterator<Item = &'a PathBuf>) {
        for path in files {
            if self.files.contains_key(path) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(path) {
                let shapes = shapes(&text);
                self.files.insert(path.clone(), (text, shapes));
            }
        }
    }

    /// Makes `candidate` the running version: its patch is in. A source
    /// that did not parse keeps its older shapes, to compare with later.
    pub fn commit(&mut self, candidate: Candidate) {
        for (path, text, shapes) in candidate.0 {
            let entry = self.files.entry(path).or_insert((String::new(), None));
            entry.0 = text;
            if shapes.is_some() {
                entry.1 = shapes;
            }
        }
    }
}

/// Says what changed shape, for a message: the first few.
pub fn describe(reshaped: &[Reshaped], show: impl Fn(&Path) -> String) -> String {
    const SHOWN: usize = 3;
    let mut parts: Vec<String> = reshaped
        .iter()
        .take(SHOWN)
        .map(|r| format!("{} `{}` ({})", r.kind, r.name, show(&r.file)))
        .collect();
    if reshaped.len() > SHOWN {
        parts.push(format!("{} more", reshaped.len() - SHOWN));
    }
    format!(
        "{} changed shape: the running app holds values of the old",
        parts.join(", ")
    )
}

/// The shapes of a source; `None` if it does not parse.
fn shapes(text: &str) -> Option<Shapes> {
    let file = syn::parse_file(text).ok()?;
    let mut collector = Collector {
        path: Vec::new(),
        impls: Vec::new(),
        shapes: Shapes::default(),
    };
    collector.visit_file(&file);
    Some(collector.shapes)
}

struct Collector {
    path: Vec<String>,
    /// The `impl` blocks it is in: the type's name, and the trait's as
    /// written.
    impls: Vec<(String, Option<String>)>,
    shapes: Shapes,
}

impl Collector {
    fn name(&self, ident: &syn::Ident) -> String {
        let mut path = self.path.clone();
        path.push(ident.to_string());
        path.join("::")
    }

    fn item(&mut self, kind: &'static str, ident: &syn::Ident, item: &impl ToTokens) {
        let tokens = normalize(item.to_token_stream());
        if matches!(kind, "struct" | "enum" | "union" | "type" | "trait") {
            idents(&tokens, &mut self.shapes.type_idents);
        }
        let name = self.name(ident);
        self.shapes.items.insert(
            name,
            Shape {
                kind,
                tokens: tokens.to_string(),
            },
        );
    }

    fn code(&mut self, key: String, tokens: &TokenStream, known_as: Vec<Vec<String>>) {
        let mut refs = HashSet::new();
        idents(tokens, &mut refs);
        // `Self::helper()` names the type.
        if let Some((ty, _)) = self.impls.last() {
            refs.insert(ty.clone());
        }
        self.shapes.code.insert(
            key,
            Code {
                tokens: tokens.to_string(),
                known_as,
                refs,
            },
        );
    }

    fn nested(&mut self, segment: String, visit: impl FnOnce(&mut Self)) {
        self.path.push(segment);
        visit(self);
        self.path.pop();
    }
}

impl<'ast> Visit<'ast> for Collector {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.item("struct", &item.ident, item);
    }
    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.item("enum", &item.ident, item);
    }
    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.item("union", &item.ident, item);
    }
    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.item("type", &item.ident, item);
    }
    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        // Its methods' bodies are code, which a patch carries; what they
        // are, and in what order, is the vtable.
        let mut signature = item.clone();
        for member in &mut signature.items {
            if let syn::TraitItem::Fn(method) = member {
                method.default = None;
            }
        }
        self.item("trait", &item.ident, &signature);
    }
    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        self.item("static", &item.ident, item);
        let mut used = HashSet::new();
        idents(&item.ty.to_token_stream(), &mut used);
        if matches!(item.mutability, syn::StaticMutability::Mut(_))
            || used.iter().any(|ident| holds_state(ident))
        {
            self.shapes
                .mutable_statics
                .push((self.name(&item.ident), false));
        }
        let tokens = normalize(item.to_token_stream());
        self.code(
            self.name(&item.ident),
            &tokens,
            vec![vec![item.ident.to_string()]],
        );
    }
    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        let name = self.name(&item.ident);
        let tokens = normalize(item.to_token_stream());
        self.shapes.consts.insert(name.clone(), tokens.to_string());
        self.code(name, &tokens, vec![vec![item.ident.to_string()]]);
    }
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if let Some(ident) = &item.ident {
            // `macro_rules! name`.
            let tokens = normalize(item.to_token_stream());
            self.code(
                format!("{}!", self.name(ident)),
                &tokens,
                vec![vec![ident.to_string()]],
            );
            return;
        }
        let macro_name = item.mac.path.segments.last().map(|s| s.ident.to_string());
        if !matches!(macro_name.as_deref(), Some("thread_local" | "lazy_static")) {
            return;
        }
        // Each `static` it declares, by name, the whole macro its shape.
        let tokens = normalize(item.mac.tokens.clone());
        let shape = tokens.to_string();
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let statics = trees.iter().enumerate().filter_map(|(at, tree)| {
            let TokenTree::Ident(keyword) = tree else {
                return None;
            };
            if keyword != "static" {
                return None;
            }
            // `static NAME`, `static mut NAME`, `static ref NAME`.
            trees[at + 1..]
                .iter()
                .map_while(|tree| match tree {
                    TokenTree::Ident(ident) => Some(ident.clone()),
                    _ => None,
                })
                .find(|ident| ident != "ref" && ident != "mut")
        });
        let statics: Vec<syn::Ident> = statics.collect();
        let whole = normalize(item.to_token_stream());
        for ident in statics {
            let name = self.name(&ident);
            let local = macro_name.as_deref() == Some("thread_local");
            self.shapes.mutable_statics.push((name.clone(), local));
            self.code(name.clone(), &whole, vec![vec![ident.to_string()]]);
            self.shapes.items.insert(
                name,
                Shape {
                    kind: "static",
                    tokens: shape.clone(),
                },
            );
        }
    }
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        self.nested(item.ident.to_string(), |this| {
            syn::visit::visit_item_mod(this, item);
        });
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        let tokens = normalize(item.to_token_stream());
        // What a component defines inside it is the component's: it is
        // mounted again when its tokens change.
        if is_component(&item.attrs) {
            let mut refs = HashSet::new();
            idents(&tokens, &mut refs);
            self.shapes
                .components
                .insert(item.sig.ident.to_string(), refs);
            return;
        }
        let ident = item.sig.ident.to_string();
        self.code(
            self.name(&item.sig.ident),
            &tokens,
            vec![vec![ident.clone()]],
        );
        self.nested(ident, |this| {
            syn::visit::visit_item_fn(this, item);
        });
    }
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if is_cfg_test(&item.attrs) {
            return;
        }
        let ty = match &*item.self_ty {
            syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        }
        .unwrap_or_default();
        let of = item
            .trait_
            .as_ref()
            .map(|(_, path, _)| path.to_token_stream().to_string());
        self.impls.push((ty, of));
        syn::visit::visit_item_impl(self, item);
        self.impls.pop();
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        let method = item.sig.ident.to_string();
        let (ty, of_trait) = self.impls.last().cloned().unwrap_or_default();
        // `value.method()` names only the method; `Type::function()` both;
        // a trait's method is often called by nothing that names it
        // (`format!` and `Display::fmt`): the type, then.
        let known_as = if of_trait.is_some() {
            vec![vec![method.clone()], vec![ty.clone()]]
        } else if item.sig.receiver().is_some() {
            vec![vec![method.clone()]]
        } else {
            vec![vec![ty.clone(), method.clone()]]
        };
        let key = match &of_trait {
            Some(of) => format!("impl {of} for {ty}::{method}"),
            None => format!("impl {ty}::{method}"),
        };
        let tokens = normalize(item.to_token_stream());
        self.code(key, &tokens, known_as);
        self.nested(method, |this| {
            syn::visit::visit_impl_item_fn(this, item);
        });
    }
}

/// Whether a type named `ident` lets a static change: the standard
/// library's cells, locks and atomics, and the usual crates' alike.
fn holds_state(ident: &str) -> bool {
    ident.starts_with("Atomic")
        || matches!(
            ident,
            "Cell"
                | "RefCell"
                | "UnsafeCell"
                | "OnceCell"
                | "OnceLock"
                | "LazyLock"
                | "LazyCell"
                | "Lazy"
                | "Mutex"
                | "RwLock"
                | "ReentrantMutex"
                | "ArcSwap"
        )
}

fn is_component(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "component")
    })
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

/// `tokens` without doc comments and derives, which change no layout, and
/// without trailing commas.
fn normalize(tokens: TokenStream) -> TokenStream {
    let mut out = Vec::new();
    let mut trees = tokens.into_iter().peekable();
    while let Some(tree) = trees.next() {
        match tree {
            TokenTree::Punct(punct) if punct.as_char() == '#' => {
                // `#[doc = ".."]`, `#![doc ..]`, `#[derive(..)]`.
                let mut lookahead = trees.clone();
                let bang =
                    matches!(lookahead.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if bang {
                    lookahead.next();
                }
                if let Some(TokenTree::Group(group)) = lookahead.peek()
                    && group.delimiter() == proc_macro2::Delimiter::Bracket
                    && matches!(
                        group.stream().into_iter().next(),
                        Some(TokenTree::Ident(ident)) if ident == "doc" || ident == "derive"
                    )
                {
                    if bang {
                        trees.next();
                    }
                    trees.next();
                    continue;
                }
                out.push(TokenTree::Punct(punct));
            }
            TokenTree::Group(group) => {
                let mut inner =
                    proc_macro2::Group::new(group.delimiter(), normalize(group.stream()));
                inner.set_span(group.span());
                out.push(TokenTree::Group(inner));
            }
            other => out.push(other),
        }
    }
    // A trailing comma is formatting.
    if matches!(out.last(), Some(TokenTree::Punct(p)) if p.as_char() == ',') {
        out.pop();
    }
    out.into_iter().collect()
}

fn idents(tokens: &TokenStream, into: &mut HashSet<String>) {
    for tree in tokens.clone() {
        match tree {
            TokenTree::Ident(ident) => {
                into.insert(ident.to_string());
            }
            TokenTree::Group(group) => idents(&group.stream(), into),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What changed shape, and what is mounted again, when the one
    /// source goes from `before` to `after`.
    fn check(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let dir = std::env::temp_dir().join(format!(
            "xui-hot-shapes-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, before).unwrap();
        let mut snapshot = Snapshot::take([&file]);
        std::fs::write(&file, after).unwrap();
        let Check {
            reshaped,
            remount,
            candidate,
        } = snapshot.check();
        snapshot.commit(candidate);
        // Committed: no change any more.
        let again = snapshot.check();
        assert!(again.reshaped.is_empty() && again.remount.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        let reshaped = reshaped
            .into_iter()
            .map(|r| format!("{} {}", r.kind, r.name))
            .collect();
        (reshaped, remount)
    }

    fn reshaped(before: &str, after: &str) -> Vec<String> {
        check(before, after).0
    }

    fn remounted(before: &str, after: &str) -> Vec<String> {
        let (reshaped, remount) = check(before, after);
        assert!(reshaped.is_empty(), "{reshaped:?}");
        remount
    }

    const APP: &str = "
        #[component]
        fn greeting() { view! { text(label()) } }
        #[component]
        fn counter() { let count: i32 = state!(0); view! { text(format!(\"{count}\")) } }
        #[component]
        fn card() { view! { text(Todo::title_of(GAP)) container(on_click: |cx| todo.toggle()) {} } }
        #[component]
        fn badge() { view! { text(format!(\"{}\", Kind::Done)) } }
        fn label() -> String { shout(\"hi\") }
        fn shout(text: &str) -> String { text.to_uppercase() }
        const GAP: f32 = 8.0;
        struct Todo;
        impl Todo {
            fn title_of(gap: f32) -> String { String::new() }
            fn toggle(&mut self) {}
        }
        enum Kind { Done }
        impl std::fmt::Display for Kind {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(\"done\") }
        }
    ";

    #[test]
    fn code_and_formatting_are_no_change_of_shape() {
        let before = "
            struct Todo { done: bool }
            fn label(todo: &Todo) -> &str { if todo.done { \"done\" } else { \"open\" } }
            trait Shape { fn area(&self) -> f32 { 0.0 } }
            const GAP: f32 = 8.0;
        ";
        let after = "
            /// A thing to do.
            #[derive(Clone)]
            struct Todo {
                /// Whether it is.
                done: bool,
            }
            fn label(todo: &Todo) -> &str { if todo.done { \"finished\" } else { \"open\" } }
            trait Shape { fn area(&self) -> f32 { 1.0 } }
            const GAP: f32 = 12.0;
            struct Added(u8);
        ";
        assert!(reshaped(before, after).is_empty());
    }

    #[test]
    fn types_statics_and_traits_change_shape() {
        let before = "
            struct Todo { done: bool }
            enum Tab { Home, Settings }
            static HITS: AtomicUsize = AtomicUsize::new(0);
            thread_local! { static CACHE: RefCell<Vec<u8>> = RefCell::new(Vec::new()); }
            trait Shape { fn area(&self) -> f32; }
            mod model { pub type Id = u32; }
        ";
        let after = "
            struct Todo { done: bool, title: String }
            enum Tab { Home, Settings, About }
            static HITS: AtomicU64 = AtomicU64::new(0);
            thread_local! { static CACHE: RefCell<Vec<u16>> = RefCell::new(Vec::new()); }
            trait Shape { fn area(&self) -> f32; fn name(&self) -> String; }
            mod model { pub type Id = u64; }
        ";
        assert_eq!(
            reshaped(before, after),
            [
                "static CACHE",
                "static HITS",
                "trait Shape",
                "enum Tab",
                "struct Todo",
                "type model::Id"
            ]
        );
    }

    #[test]
    fn a_const_changes_shape_when_a_type_names_it() {
        let before = "const N: usize = 4; const GAP: f32 = 8.0; struct Grid { cells: [u8; N] }";
        let after = "const N: usize = 5; const GAP: f32 = 9.0; struct Grid { cells: [u8; N] }";
        assert_eq!(reshaped(before, after), ["const N"]);
    }

    #[test]
    fn what_a_component_defines_is_its_own() {
        let before = "
            #[component]
            fn counter() { struct Local(u8); }
            fn helper() { struct Inner(u8); }
            #[cfg(test)]
            mod tests { struct Fixture(u8); }
        ";
        let after = "
            #[component]
            fn counter() { struct Local(u16); }
            fn helper() { struct Inner(u16); }
            #[cfg(test)]
            mod tests { struct Fixture(u16); }
        ";
        assert_eq!(reshaped(before, after), ["struct helper::Inner"]);
    }

    #[test]
    fn a_source_that_does_not_parse_is_looked_at_once_it_does() {
        let dir = std::env::temp_dir().join(format!("xui-hot-shapes-parse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, "struct A(u8);").unwrap();
        let mut snapshot = Snapshot::take([&file]);
        std::fs::write(&file, "struct A(u16").unwrap();
        let check = snapshot.check();
        assert!(check.reshaped.is_empty());
        snapshot.commit(check.candidate);
        std::fs::write(&file, "struct A(u16);").unwrap();
        assert_eq!(snapshot.check().reshaped.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_component_calling_changed_code_is_mounted_again() {
        let edit = |from: &str, to: &str| {
            assert!(APP.contains(from), "{from}");
            remounted(APP, &APP.replace(from, to))
        };
        // Through another function.
        assert_eq!(
            edit("text.to_uppercase()", "text.to_lowercase()"),
            ["greeting"]
        );
        // A const, an associated function, a method.
        assert_eq!(edit("GAP: f32 = 8.0", "GAP: f32 = 9.0"), ["card"]);
        assert_eq!(
            edit(
                "fn title_of(gap: f32) -> String { String::new() }",
                "fn title_of(gap: f32) -> String { gap.to_string() }"
            ),
            ["card"]
        );
        assert_eq!(
            edit(
                "fn toggle(&mut self) {}",
                "fn toggle(&mut self) { let _ = 1; }"
            ),
            ["card"]
        );
        // A trait's method, which nothing names: by its type.
        assert_eq!(
            edit("f.write_str(\"done\")", "f.write_str(\"finished\")"),
            ["badge"]
        );
        // A component's own change is the runtime's to see; nothing more.
        assert!(edit("text(label())", "text(label() + \"!\")").is_empty());
        assert!(remounted(APP, APP).is_empty());
    }

    #[test]
    fn finds_the_statics_that_hold_state() {
        let dir = std::env::temp_dir().join(format!("xui-hot-statics-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(
            &file,
            "
            static NAME: &str = \"app\";
            static HITS: AtomicUsize = AtomicUsize::new(0);
            static mut RAW: u8 = 0;
            static CACHE: Mutex<Vec<u8>> = Mutex::new(Vec::new());
            thread_local! { static LOCAL: Cell<u8> = Cell::new(0); }
            ",
        )
        .unwrap();
        let names: Vec<String> = Snapshot::take([&file])
            .mutable_statics()
            .into_iter()
            .map(|(name, _, local)| format!("{name}{}", if local { " (local)" } else { "" }))
            .collect();
        assert_eq!(names, ["CACHE", "HITS", "LOCAL (local)", "RAW"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
