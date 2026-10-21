# Architecture overview

This document describes how the `xui` crates layer, how data flows through a
single frame, and how the rendering pipeline is structured. For the public API
of each crate, see the [per-crate docs](./README.md).

## Layering

The workspace is intentionally split so that foundational types have no
dependency on the runtime or backends. Dependencies point downward only.

```text
application crate
    │
    ├──► xui                      core runtime, fiber, hooks, layout, widgets, scene
    │       │
    │       ├──► xui-interface    shared types (geometry, style, events, text traits)
    │       ├──► xui-animation    Tween / Animatable (depends on interface + macros)
    │       ├──► xui-text         optional text layout (depends on interface)
    │       └──► slot             generational slot storage with RenderPhase guards
    │
    ├──► xui-components           built-in widgets (depends on xui)
    │
    ├──► xui-text-engine          cosmic-text impl of interface text traits
    │
    ├──► xui-shell                platform host contract + driver (depends on xui, interface)
    │
    ├──► xui-vello                Vello backend, the default (depends on xui, interface,
    │                            render-graph, shell, wgpu-text)
    │
    ├──► xui-skia                 optional Skia backend (depends on xui, interface,
    │                            render-graph, shell)
    │
    ├──► xui-winit                winit host for xui-shell (depends on xui, interface,
    │                            shell, render-graph; selects vello, skia or wgpu)
    │
    ├──► xui-macos                native AppKit host for xui-shell (depends on xui,
    │                            interface, shell, vello or skia; macOS only)
    │
    └──► xui-windows              native Win32 host for xui-shell (depends on xui,
                                 interface, shell, vello or skia; Windows only)
```

Backends (`xui-vello` by default, `xui-skia`, and the standalone `wgpu`
renderer in `xui-winit`) implement the
`xui_core::render::RenderBackend` and `xui_interface::TextBackend` traits. The core
runtime never names a GPU library directly.

Window hosts are split along `xui-shell`. A host owns the event loop and the
window, translates native events into `ShellEvent`s, and implements
`PlatformWindow`; `Shell` does the rest (input state, window visibility,
first-frame reveal, cursor/IME output). Backends only see a `SurfaceTarget`
(raw handles, size, scale factor), so they never name winit. That is what lets
the native hosts -- `xui-macos` on AppKit, `xui-windows` on Win32 -- stand in
for `xui-winit` without the runtime or the backends changing at all.

## Crate roles by concern

### Types and contracts — `xui-interface`

The shared vocabulary every other crate agrees on:

- **Geometry** — `Color`, `Point`, `Rect`, `Size`, `Sizing`, `EdgeInsets`,
  `Bounds`, `Translation`.
- **Style** — `ComputedStyle`, style values and units, tokens (`ColorToken`,
  `SpacingToken`, `FontSizeToken`, `RadiusToken`), `Theme`, `WidgetStateMatcher`
  for state rules.
- **Events** — raw window/pointer/keyboard events (`RawWindowEvent`,
  `RawPointerButton`, `RawKeyboard`, `RawWheel`, …), semantic requests
  (`EventRequest`, `EventRequests`), shortcuts (`Shortcut`, `ShortcutBinding`).
- **Widget** — `Component`, `NodeId`, `Key`, `WidgetType`, `AccessibilityRole`.
- **Platform** — `TextInputSession`, `PlatformOutput`.
- **Transition** — `Easing`, `Transition`, `AnimationProgress`, and the looping
  keyframe `Animation` (`Iterations`, `AnimationDirection`). The motion capability carries
  timing; typed node layers carry keyframes: a transition animates a change of target and stops; an
  animation moves on its own (a spinner) and keeps the loop asking for frames
  while it runs. They stack per property -- target, then transition sample,
  then the properties the keyframes name -- so a transition shows through
  wherever the animation does not reach. Transform-only animations stay on the
  frame-properties path -- no layout, no repaint.
- **Text traits** — `TextBackend` bundles three parts: a shared, thread-safe
  `FontCollection` (font ids → `FontData`), the mutable `Shaper`, and an
  optional thread-safe `GlyphRasterizer` that renders `GlyphRequest`s into
  `GlyphImage`s. Layout is in logical pixels and independent of the scale
  factor; physical size and subpixel offset are rasterization parameters.
  Plus layout data (`ParagraphLayout`, `LineLayout`, `GlyphRun`, …).
- **Dirty tracking** — `WidgetUpdateFlags`, `StyleDiffFlags`.

### Runtime and reconciler — `xui`

The heart of the framework:

- **`runtime::GuiRuntime`** — drives the event/render loop, owns the
  `TextHost`, `ShortcutManager`, and `EventSource`.
- **`app::App`** — owns the component root, tokio async runtime, scheduler,
  theme, and `TextHost`; renders frames through a `RenderBackend`.
- **`fiber`** — retained fiber tree keyed by `FiberId` (slotmap); reconciles
  `ElementDesc` trees into a widget/layout tree.
- **`state`** — hooks (`HookContext`, `Memo`, `Resource`, `Callback`,
  `AsyncValue`, `TaskContext`), lanes for batched updates, scheduler and async
  dispatcher. Uses `slot` for phase-guarded storage.
- **`context`** — `cx.provide_context(value)` / `cx.use_context::<T>()`:
  values keyed by type that a component hands to its whole subtree, plus
  `App::provide_context` for application-wide ones and a
  `<context_provider value={..}>` element. Resolution walks the *fiber*
  ancestors, so portal content sees the contexts where it was written.
  Values live in a per-runtime `ContextRegistry` (not hook slots, so providing
  may be conditional) that records who read what; a provider whose value
  changes (by `PartialEq`), disappears, or newly shadows an outer one marks
  exactly those readers dirty in the current render lane, after its own
  render and before its children are reconciled, which is what carries the
  change past components that bail out on unchanged props.
- **`lanes`** — update lanes (default, retry, …) for concurrent and batched
  state propagation.
- **`layout`** — `taffy` flexbox over retained spatial nodes.
- **`style`** — resolves capability layers, tokens and themes into `ComputedStyle`, applies
  `WidgetStateMatcher` rules.
- **`render`** — backend-independent scene (`RenderNodeId`, `PictureId`,
  `PrimitiveId`), scene compiler, compiled scene, and `RenderBackend` trait.
- **`widgets`** — primitive widget interface (`WidgetI`) and built-in primitives
  (text, container, scroll, overlay).
- **`event_system`** — translates raw events into semantic events and dispatches
  callbacks; focus manager; shortcut manager.
- **`assets`** — installs the process-global `AssetManager` and exposes
  `load_image_asset` and friends.
- **`element`** — `ElementDesc`, `ComponentDesc`, `FragmentDesc`, `PortalDesc`;
  `Expanded` and `ExpandedHost` are inspection-only output.
- **`size`** — `HookContext::use_size()` creates a stable size subscription;
  `.observe_size(handle)` binds it to one widget without adding a wrapper.
  `handle.get()` returns `None` before layout or while unbound, otherwise the
  final border-box size in logical pixels. Layout publishes changes on the
  ordinary update lane for a subsequent render; no containment or frame-local
  layout/rebuild loop is involved.

### Node descriptions and retained state

`ElementDesc` has four variants: `Template`, `Component`, `Fragment`, and
`Portal`. Every host belongs to a template. Macro-generated templates group
hosts with fixed structure; a manually constructed `Draft` becomes a single-host
template with a fragment slot for its children. There is no separate host-fiber
reconciliation path.

A draft is temporary input. Commit consumes it: generated `scatter` writes
capability values to `UiRuntime`'s `Columns`, and the widget instance retains its
own props and state under a `NodeId`. Reusing a template preserves these IDs;
only hosts with dynamic inputs produce replacement drafts. Their values are
compared at commit to derive dirty flags. Static hosts skip this update. Dynamic
child slots still reconcile keys and element identity; this is node-level
updating, not property-slot patching.

The retained fiber uses `FiberKind` to hold exactly one kind of state. The WIP
tree carries one `Pending` payload, consumed at commit. The root is an ordinary
component. Fragments represent dynamic child lists; portals own a separate
physical subtree and release it even when their logical host ancestor is removed.
`Expanded` is only an inspection tree and cannot be submitted for reconciliation.

### Macros — `xui-macros`

Compile-time DSLs that produce `ElementDesc` / `Style` / components:

- `xui! { <Tag attr={expr}>{children}</Tag> }` → `ElementDesc`.
- `view! { tag(content, attr: expr) { children }.attr(expr) }` → the same
  `ElementDesc`: a second, SwiftUI-style parser over `xui!`'s element tree and
  expansion, adding `if`/`for`/`match`/`let` and `..expr` splices among
  children.
- `style!(padding: 16.0, background: if hovered { ... })` → `Style` with
  state-conditioned rules lowered to `WidgetStateMatcher`.
- `#[component]` / `component_fn!` — generates a props struct, a typed builder,
  a support module, and a `ComponentRender` handle. Generic components are
  supported (see the module docs of `xui-macros/src/component.rs`); each
  instantiation is a distinct `ComponentType`, keyed by name and props `TypeId`.
  Every props field is a `Prop<T>` (`xui-core/src/prop.rs`): inline, or a
  projection into an ancestor's props allocation. `binding.share()` forwards a
  prop to a child without copying it (small values without drop glue, such as
  hook handles, are copied instead); the body still sees plain `&T` bindings.
- `#[main]` — includes the asset bootstrap module and installs the
  `AssetManager` before the user's `main` body runs.
- `#[derive(Animatable)]` — field-wise `Animatable` impl.

### Animation — `xui-animation`

`Animatable` trait + derive, `Tween<T>`, `Timeline`, `AnimationClock`.
Interpolations for `f32`, `NotNan<f32>`, `Point`, `Size`, `EdgeInsets`,
`Color`, `LengthValue`, `ColorValue`, `ColorStyle`, gradients, shadows,
strokes, scrollbars. Discrete variant switches are snap-at-end.

### Components — `xui-components`

Built-in widgets built on top of `xui`: `button`, `input` (wraps
`text_input`), `dropdown`, `tabs`, `image`, `icon` (SVG/path icons),
`virtual_list`, and shared layout helpers (`ComponentLength`, `ComponentSizing`,
`ComponentInsets`, `ComponentColor`).

## Per-frame data flow

```text
platform events (winit)
    │
    ▼
xui-winit WinitRunner               ──►  ShellEvent (xui_shell)
    │
    ▼
xui_shell::Shell                    ──►  RawEvent (xui_interface::events)
    │
    ▼
GuiRuntime  ──►  App
    │
    ├─ event lane:  raw events → EventTranslator → semantic events / callbacks
    │
    ├─ effect lane: run pending effects, async task wakeups (tokio)
    │
    └─ render phase (slot::RenderPhase::Render):
         │
         ├─ re-render dirty components via HookContext → ElementDesc tree
         │    (a provider whose context changed marks its readers dirty in
         │     the same pass, before its children are reconciled)
         │
         ├─ fiber reconciler diffs ElementDesc → retained widget + layout tree
         │
         ├─ style system: capability layers + theme + state → ComputedStyle per node
         │
         ├─ taffy layout pass → node Bounds
         │
         ├─ publish observed sizes → schedule changed owners for a later render
         │
        ── scene built (RenderNodeId / PictureId / PrimitiveId)
         │
         ├─ scene compiler → render graph (xui-render-graph LayerProgram)
         │
        ── backend (xui-vello VelloBackend / xui-skia SkiaBackend) rasterizes frame → surface
         │
        ── TextBackend (xui-text-engine CosmicEngine) lays out & rasterizes glyphs
```

State reads/writes are phase-checked (see `slot`): hook storage may only be
written during `Event`/`Effect` phases and read during any active phase.
Debug builds assert this.

## Rendering pipeline

The render path is split into three stages so backends stay thin:

1. **Scene** (`xui_core::render::scene`) — a retained, backend-independent tree of
   layers, pictures, and primitives. Spatial nodes track transforms/clips.
2. **Render graph** (`xui-render-graph`) — `compile_layer` normalizes static
   style into a reusable `LayerProgram`; `LayerProgram::instantiate` applies
   frame geometry, lowers executable pass IR, and assigns abstract transient
   texture slots. Pure Rust, no GPU dependency, `#![forbid(unsafe_code)]`.
3. **Backend** (`xui-vello` by default, or `xui-skia`) — consumes the graph and
   rasterizes to the window surface. `xui-vello` draws 2D content with Vello
   on a wgpu device it owns, composes layers with its own passes, and draws
   text from glyph bitmaps (`xui-wgpu-text`). `xui-skia` uses `skia-safe` on a
   platform swapchain picked at runtime (Metal on macOS, Direct3D 12 on
   Windows, Vulkan on Linux, and a `softbuffer` CPU blit as the fallback when
   no GPU context can be created — `XUI_SKIA_GPU=0` forces it); caches layers,
   glyphs, and paragraphs; reports frame stats.

## Asset pipeline

```text
xui.toml + assets/  ──►  build.rs: xui_build::assets()  ──►  xui-pak-build
                                              │
                                  writes .xpak + xui_asset_refs.rs into OUT_DIR
                                              │
                                  bootstrap module (xui_assets::refs + manager())
                                              │
                                  cargo::rustc-env=XUI_ASSETS_BOOTSTRAP → xui_core::include_assets!()
                                              │
                                  xui_assets::manager() → AssetManager (xui-assets)
                                              │
                                  mount order: dev directory → EmbeddedPak / PakSource
```

- `xui-pak` defines the `.xpak` format: fixed header, payload blobs (zstd or
  none), postcard index, blake3 content hashes, power-of-two alignment.
- `xui-pak-build` scans a source directory with `ignore`, applies glob rules,
  encodes a deterministic archive, and generates Rust asset-id constants.
- `xui-assets` `AssetManager` searches mounted sources in insertion order
  (first match wins), caches decompressed immutable bytes, and parses via
  `AssetFormat`. `DirectorySource` is `Volatile` (live edits); paks are
  `Immutable` (cached, zero-copy `AssetBytes::Mapped`/`Static`).
- `xui-build` runs the above from an application's build script. Cargo reruns
  it when `xui.toml` or the assets directory changes. The bootstrap decides at
  startup, from `XUI_ASSETS_DIR` / `XUI_ASSETS_PAK`, where the development
  directory and an external package are; debug builds fall back to the paths
  the build produced, release builds carry none.
- `xui-cli` (`cargo xui`) sets packages up (`init`), and for forwarded Cargo
  commands mounts the development directory and copies an external package
  beside the executable. For a package without a build script it generates
  the bootstrap itself and passes `XUI_ASSETS_BOOTSTRAP` to Cargo.

## State storage

Two cooperating primitives back hook state:

- **`slot`** — `Scope`/`Pointer`/`Storage` with a thread-local `RenderPhase`
  (`Render`/`Event`/`Effect`/`Commit`). Debug builds assert reads/writes happen
  in a legal phase. Used directly by `xui_core::state`.
- **`xui-slot`** — `GenerationalBox<T, S>` with `UnsyncStorage`/`SyncStorage`
  and an `Owner` that drops its boxes on drop. Reference-counted variants and
  `Owner::insert_reference` support signal-style sharing.

## Backend selection

Every host (`xui-winit`, `xui-macos`, `xui-windows`) and the `xui` facade
expose the same flags:

```toml
[dependencies]
xui-winit = { path = "../xui-winit" }                          # vello (default): `runner`
# or
xui-winit = { path = "../xui-winit", features = ["skia"] }      # adds `skia_runner`
# or
xui-winit = { path = "../xui-winit", default-features = false, features = ["skia"] }
```

`vello` pulls in `xui-vello` and provides `runner`; `skia` pulls in `xui-skia`,
re-exports `SkiaBackend` and provides `skia_runner`. In `xui-winit`, `wgpu`
pulls in the
optional `wgpu` renderer module (`TexturePool`, `WGPUBackend`, …).
