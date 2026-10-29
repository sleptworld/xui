# Reactive components (replacing the fiber runtime)

Status: in progress. This document is the plan of record for replacing
`xui-core`'s hook/fiber component runtime with the compile-time-analysed
reactive runtime prototyped in `crates/xui-dev` (since removed). It is written so a later
session can resume from it; keep the checklist at the end current.

## Model

A component is a function run **once**, at mount. Its body declares sources
and effects; its tail is a view. Nothing re-renders: `#[component]` works out
at expansion time what each dynamic part of the view reads, and the runtime
reruns exactly those parts when a source changes.

```rust
#[component]
pub fn counter(step: &i32, #[source] total: &i32, #[event] on_change: &Callback<i32>) {
    let count: i32 = state!(0);               // Signal<i32> on the instance
    let doubled: i32 = memo!(count * 2);      // Memo<i32>; `count` is `&i32` here
    effect!(|cx| on_change.call(cx, count.get()));
    view! {
        column {
            text(format!("{count} / {total}"))
            button(on_click: |cx| count.update(cx, |c| *c += *step)) { text("+") }
            display(value: doubled)             // a `#[source]` prop of `display`
        }
    }
}
```

Rules (the long form, with compile-fail examples, is the `xui_core::reactive`
module doc):

- Sources are declared by statements at the top of the body: `let x: T =
  state!(..)`, `let x: T = memo!(..)`, `effect!(|cx| ..)`. Props are sources
  when marked `#[source]` (read-only) or `#[state]` (shared writable).
- Tracked code (memo bodies, `{content}`, attribute values, block
  conditions/scrutinees/iterators, derived props) sees a source as its
  value `&T` and depends on every source it mentions, except under
  `untrack!(..)`.
- Untracked code (effects, handlers, setup statements) sees handles. Only
  effects and handlers get a `Cx`, so only they write. An effect depends on
  what it mentions too, so it writes through `untrack!(x)`.
- Names resolve as Rust resolves them (syn visitor, scope-aware).

## Where things live

| Prototype (`xui-dev`)     | `xui-core`                                  |
|---------------------------|---------------------------------------------|
| `graph.rs`, `signal.rs`   | `reactive/graph.rs`, `reactive/signal.rs`   |
| `runtime.rs`              | `reactive/runtime.rs` (owned by `App`, replaces `ComponentRuntime`) |
| `block.rs`                | `reactive/block.rs`                         |
| `template.rs`             | `reactive/template.rs` (replaces `template.rs`, `fiber.rs`, `element.rs`) |
| `host.rs` (`HostTree`)    | gone: `UiRuntime` is the host               |
| `macros/*`                | `xui-macros`: `component.rs`, `view.rs`, `analyze.rs` replace the hook-based ones |

Removed: `fiber.rs`, `component.rs`, `state.rs` (hooks), `lanes.rs`,
`element.rs`/`template.rs` (`ElementDesc`, `TemplateDesc`), `prop.rs`
(`Prop<T>`/`share()`), `dsl.rs` children plumbing, the `slot` crate's phase
guards if nothing else uses them, `xui-dev`.

## Host integration

- A template node is a host: mount builds its `Draft` with the static
  attributes (`tag().__with(table.attr(), value)`), `UiRuntime::create_node`,
  `place(parent, child, before)`. A scope's roots are placed into the
  enclosing host (or fragment position) at mount; `remove_subtree` on unmount.
- Fragments (block rows, slots) are not hosts: a block keeps the ordered
  host roots of its rows and places them before the block's *anchor* — the
  next host sibling after the block in the parent, or last. No fragment
  widget is inserted into the layout tree.
- A render binding writes a property: `ui.patch_node(node, |widgets, id,
  columns, cx| HostPatcher::new(..).__patch(marker, value))`, i.e. the path
  `TemplateHosts::patch` already uses, returning `WidgetUpdateFlags`. The
  property's `same` keeps an unchanged value from dirtying anything.
- Handlers are handler properties (`__with_on(table.on_click(), closure)`).
  The closure captures `Rc<Scope>` and the shared `Notifier`, builds a `Cx`
  and runs the generated handler fn. Writes push source ids into the
  notifier; `App` flushes the reactive runtime after event dispatch, after
  async messages, and at the start of a frame (where `rebuild_sync_if_needed`
  is called today).
- Portals: a `portal { .. }` block mounts its rows' roots under the overlay
  entry instead of the enclosing host (`mount_overlay_entry`).

## Writes and `Cx`

`Signal::set(&self, cx: &mut Cx, v)` keeps the type-level rule. `Cx` holds
the notifier (an `Rc<RefCell<Vec<SourceId>>>`); async code gets a
`SendSignal<T>` (posts through `App`'s async channel) — replaces
`State`'s `Send` variant and `TaskContext`.

## Hooks → sources

| Hook                        | Replacement |
|-----------------------------|-------------|
| `use_state`                 | `state!` |
| `use_memo`, `use_memo_with` | `memo!` (deps found, not listed) |
| `use_callback`              | a `&Callback<A>` prop, set by the parent with a closure `|cx, value| ..`; the child calls `on_x.call(cx, v)` |
| `use_ref`                   | a typed `let` (a field, not tracked); `Cell`/`RefCell` inside for mutation |
| `use_context`               | `let t: T = context!();` — nearest ancestor's `provide!(source_or_value)`, else `App::provide_context` |
| `use_size`                  | `let s: Option<Size<f32>> = size!();` passed by name to a host's `size_observer` |
| `use_ticker`                | `let t: Ticker = ticker!(|cx, frame| -> Tick { .. });` (or as a statement) |
| `use_resource`, `use_task`  | `resource!(deps => async ..)`: a `Signal<AsyncValue<T>>` written from the async channel |
| `use_asset`                 | `memo!` over `assets::get(..)` |

## Props protocol

Per component the macro emits a ZST `Name` implementing `Component` (`Props`,
`Properties` table, `Instance`, `TEMPLATE`, `create`), one `Property` ZST per
prop with xui-core's `Requirement` typestate (`#[defaults]` makes a prop
optional), and `Deliver::Mode` per prop:

| Declaration          | Mode          | source name passed | expression passed |
|----------------------|---------------|--------------------|-------------------|
| `x: &T`              | `Snapshot<T>` | value at mount     | evaluated at mount |
| `#[source] x: &T`    | `Track<T>`    | the handle         | a derived source |
| `#[state] x: &T`     | `Shared<T>`   | the `Signal`       | compile error |
| `x: &Callback<A>`    | callback (optional) | —            | closure `|cx, value| ..` |
| `children: &Children`| children (optional) | —            | trailing block, mounted by `slot(children)` |

The parent builds `tag()` drafts through the table, so a misspelt prop is a
missing method and a missing required prop fails the typestate.

## Checklist

The new attribute is registered as `#[xui_macros::reactive]` until the old
`#[component]` is deleted, then renamed. Tests: `cargo test -p xui-core --lib
reactive` (`crates/xui-core/src/reactive/tests.rs`).

- [x] `reactive/` core moved from `xui-dev` (graph, signal, scheduling, blocks)
- [x] host layer on `UiRuntime` (mount/unmount, block rows placed by anchor, `Cx::__attr` patch-or-rebuild)
- [x] macro codegen on xui-macros' `Element` tree (`xui-macros/src/reactive.rs`, `analyze.rs`)
- [x] handlers via handler properties + notifier (end-to-end click test)
- [x] `App` owns `reactive::Runtime`, flushed in `rebuild_sync_if_needed` (before each dispatch and each frame); `App::mount::<C>(props)`
- [x] handler closures taking the event and returning a `Flow`: `|cx, event| ..`
- [x] props protocol (`reactive/props.rs`): marker ZST implementing `NodeDef` + `Component`, hidden `__NameInstance`, draft props, one `Property` per prop (`__xui_{tag}_{prop}`) with mode-specific `Accepts<Expr<T>>` / `Accepts<SourceArg<H>>`, required-prop typestate, `#[defaults(..)]`, `Name::__mount(draft)`
- [x] derived `#[source]` props: `Tracked::Derived` resolved at mount into a memo owned by the child, bound to the parent's `SourceId`s (`Mounted::derived`, effect slots after the template's)
- [x] ~~`App` async writes (`SendSignal`)~~ superseded by `resource!`: `drain_async_messages` now delivers finished resource futures
- [x] callbacks (`reactive/pass.rs`: `Callback`, `__callback`), `children` + `slot(children)` (a row of the parent's scope, `NodeKind::Slot`)
- [x] portals: `portal(z_index:, modal:, hit_test:, scope:, anchor:, on_dismiss: |cx, reason| ..) { .. }` → `NodeKind::Portal`, content under the overlayer as an overlay entry
- [x] `context!()` / `provide!(..)`; app contexts are signals (`Runtime::provide_context`)
- [x] `size!()` (`SizeObserver::reactive`) — known gap: a host rebuilt whole (dynamic `style`) gets a fresh observer, so the size reads `None` until the next layout
- [x] `ticker!(..)` (`Lane::Tick`, handle registered at create, callback bound at mount)
- [x] `resource!(future)` (`reactive/tasks.rs`): tracked expression yielding a `Send` future of `Result<T, E>`, field `Signal<AsyncValue<T, E>>`; futures run on `App`'s tokio runtime, results come back over `App`'s channel as `(ticket, Box<dyn Any + Send>)` and are written on the main thread; a newer start drops an older result
- [ ] export `state!`/`memo!`/`effect!`/`untrack!`/`context!`/`provide!`/`ticker!`/`size!` as `compile_error!` placeholders from `xui-core` (as `xui-dev` did) for docs
- [x] deleted `fiber`, `component`, `state` (hooks), `lanes`, `element`, `template`, `prop`, `dsl`, `context`, `component_bench`, `frame_bench`; `HostPatcher`/`PatchScope`/`HostTag` moved to `reactive/host.rs`. `App::new(root: AppRoot)` with `AppRoot = fn(&mut App)` (mount the root there), `App::mount`, `App::update(|cx| ..)`, `App::provide_context`; `mark_needs_rebuild` now only relayouts
- [x] xui-macros: `#[component]` is the reactive macro; old `component.rs`/`template.rs`/`errors.rs` and `component_fn!` deleted; `xui!`/`view!` alone are a compile error (`hosts.rs` keeps the tag list); a component named like a built-in tag is rejected
- [x] xui-core tests ported (unit tests use `test_tree::StaticTree` for static host trees; trybuild cases rewritten as components; `memory.rs` benchmarks on components). Dropped: tests of removed features (prop sharing, generic components, memo components, `ElementDesc` lowering)
- [x] hosts (`xui-winit`, `xui-macos`, `xui-windows`) take `AppRoot`; `xui` facade re-exports
- [x] port tests of `xui-shell`, `xui-vello`, `xui-skia` and `xui-macos` examples: each root is a `#[component]` mounted by an `AppRoot`; scenes that changed between frames through a thread-local and `mark_needs_rebuild` now take a `#[source] step` prop fed by `App::signal` and written with `App::update`. Added for that: `App::signal(value)` (app-owned state for a root) and `From<ReadSignal<T>>` / `From<&Signal<T>>` for `Tracked<T>`
- [x] generic components: type and const parameters (lifetimes and `impl Trait` props rejected) go on every generated item — marker, instance, props, draft, table, each prop's property, `MountDraft` — with the user's `where` clause, `T: 'static` per type parameter, and `PartialEq` implied for `#[source]` props and `memo!`s whose type names a parameter. A parent's expression now reaches a child prop through the prop's `FromValue<V>` (converted inside the expression's closure by `__convert`): `Into` for most props, the value as is for one whose type names a parameter, so the parameter is inferred from it. A parameter no prop names takes a turbofish, `typed::<u8>(..)`, and is carried by the props as `__phantom`; a closure for a parameter-typed prop goes in parentheses, since a bare closure is a callback. A `for` block over a parameter's values still needs `T: PartialEq` written out (row data compares)
- [x] port `xui-components` (all 36 components, its tests and doc examples). What the port added:
  - `Children` content built anywhere: `Children::component::<C>(props)`, `From<&str>`/`From<String>`, `From<&Children>`; any prop typed `Children` takes `name: view! { .. }` (or `xui! { <..> }`) and an expression yielding `Children`; `slot(expr)` reads its expression once, with sources as values
  - items carrying content (`TabItem`, `RadioItem`, `AccordionItem`, `TableRow`) hold `Children`; `Children` and `Callback` compare by identity, so items stay `PartialEq` row data
  - callback props: any prop whose type is a callback takes a closure `|cx, value| ..` (`CallbackArg`); spelled `Callback<A>` it is also optional
  - host handlers take `|cx, event, event_cx| ..` and may return `EventResult`/`Flow`
  - `draft: |d| d.animate(..)` on a host applies any draft method; the host is rebuilt when what it reads changes
  - children mount through `MountDraft<C>` on the completed draft, so a parent needs the tag function only, not the marker
  - a `#[source]` prop given an expression that reads nothing is a constant (no derived effect); `ReadSignal::constant(v)` for root mounts
  - unsuffixed numeric literals take the prop's type; `#[defaults]` likewise
  - a component may share a host's name when reached by path (`xui_components::image::image`); a view's root may be any element
  - list rows by index: `VirtualItemRenderer::new(|index| Children)`
- [x] port `xui-example-app`: Gallery (helpers and every page as components, the page switch a `match` block) and the flight-icing dashboard (`dashboard_root`); its smoke and Skia snapshot tests pass
- [x] remove `crates/xui-dev` (its crate docs moved to `reactive/mod.rs`); workspace builds; `cargo test --workspace --features xui-core/strict-invariants` green except `xui-build`'s `build_script` and `xui-cli`'s `project`, which look for `xui`/`Cargo.lock` under `crates/` and fail the same way without the feature (they predate this work)
- [x] dead code the old system left in `xui-core` removed (`assets::generation`, `ErasedDraft::{clone_box, as_any}`, unused damage/scene/query helpers, test imports)
- [ ] portal options (`z_index`, `modal`, `scope`, `anchor`, ..) are read once, at mount; a portal under an `if` remounts with new ones, but one whose options change in place keeps the old. `UiRuntime::update_overlay_entry` is kept for that
- [ ] update `README.md`, `docs/architecture.md`, `docs/components.md`, `CLAUDE.md`
