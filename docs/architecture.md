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
    ├──► xui-skia                 Skia backend (depends on xui, interface, render-graph)
    │
    └──► xui-winit                winit window/event loop (depends on xui, interface,
                                 render-graph, text-engine; selects skia or wgpu)
```

Backends (`xui-skia`, optional `wgpu` in `xui-winit`) implement the
`xui::render::RenderBackend` and `xui_interface::TextBackend` traits. The core
runtime never names a GPU library directly.

## Crate roles by concern

### Types and contracts — `xui-interface`

The shared vocabulary every other crate agrees on:

- **Geometry** — `Color`, `Point`, `Rect`, `Size`, `Sizing`, `EdgeInsets`,
  `Bounds`, `Translation`.
- **Style** — `Style`, `StylePatch`, `ComputedStyle`, `*StylePatch` per layer
  (layout, paint, text, scroll, transform, effect), tokens (`ColorToken`,
  `SpacingToken`, `FontSizeToken`, `RadiusToken`), `Theme`, `WidgetStateMatcher`
  for state rules.
- **Events** — raw window/pointer/keyboard events (`RawWindowEvent`,
  `RawPointerButton`, `RawKeyboard`, `RawWheel`, …), semantic requests
  (`EventRequest`, `EventRequests`), shortcuts (`Shortcut`, `ShortcutBinding`).
- **Widget** — `Component`, `NodeId`, `Key`, `WidgetType`, `AccessibilityRole`.
- **Platform** — `TextInputSession`, `PlatformOutput`.
- **Transition** — `Easing`, `Transition`, `AnimationProgress`.
- **Text traits** — `TextBackend`, `Shaper`, `FontDatabase`, `GlyphRasterizer`,
  plus layout data (`ParagraphLayout`, `LineLayout`, `GlyphRun`, …).
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
- **`lanes`** — update lanes (default, retry, …) for concurrent and batched
  state propagation.
- **`layout`** — `taffy` flexbox over retained spatial nodes.
- **`style`** — merges patches/tokens/themes into `ComputedStyle`, applies
  `WidgetStateMatcher` rules.
- **`render`** — backend-independent scene (`RenderNodeId`, `PictureId`,
  `PrimitiveId`), scene compiler, compiled scene, and `RenderBackend` trait.
- **`widgets`** — primitive widget interface (`WidgetI`) and built-in primitives
  (text, container, scroll, overlay).
- **`event_system`** — translates raw events into semantic events and dispatches
  callbacks; focus manager; shortcut manager.
- **`assets`** — installs the process-global `AssetManager` and exposes
  `load_image_asset` and friends.
- **`element`** — `ElementDesc`, `ComponentDesc`, `WidgetDesc`, `portal`.

### Macros — `xui-macros`

Compile-time DSLs that produce `ElementDesc` / `Style` / components:

- `xui! { <Tag attr={expr}>{children}</Tag> }` → `ElementDesc`.
- `style!(padding: 16.0, background: if hovered { ... })` → `Style` with
  state-conditioned rules lowered to `WidgetStateMatcher`.
- `#[component]` / `component_fn!` — generates a props struct, a typed builder,
  a support module, and a `ComponentRender` handle.
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
winit events
    │
    ▼
xui_winit::translate_window_event   ──►  RawEvent (xui_interface::events)
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
         │
         ├─ fiber reconciler diffs ElementDesc → retained widget + layout tree
         │
         ├─ style system: patches + theme + state → ComputedStyle per node
         │
         ├─ taffy layout pass → node Bounds
         │
        ── scene built (RenderNodeId / PictureId / PrimitiveId)
         │
         ├─ scene compiler → render graph (xui-render-graph LayerProgram)
         │
        ── backend (xui-skia SkiaBackend / wgpu) rasterizes frame → surface
         │
        ── TextBackend (xui-text-engine CosmicEngine) lays out & rasterizes glyphs
```

State reads/writes are phase-checked (see `slot`): hook storage may only be
written during `Event`/`Effect` phases and read during any active phase.
Debug builds assert this.

## Rendering pipeline

The render path is split into three stages so backends stay thin:

1. **Scene** (`xui::render::scene`) — a retained, backend-independent tree of
   layers, pictures, and primitives. Spatial nodes track transforms/clips.
2. **Render graph** (`xui-render-graph`) — `compile_layer` normalizes static
   style into a reusable `LayerProgram`; `LayerProgram::instantiate` applies
   frame geometry, lowers executable pass IR, and assigns abstract transient
   texture slots. Pure Rust, no GPU dependency, `#![forbid(unsafe_code)]`.
3. **Backend** (`xui-skia`, or `wgpu` via `xui-winit`) — consumes the graph and
   rasterizes to the window surface. `xui-skia` uses `skia-safe` with Metal on
   macOS; caches layers, glyphs, and paragraphs; reports frame stats.

## Asset pipeline

```text
xui.toml  ──►  xui-cli (cargo xui)  ──►  xui-pak-build
                                              │
                                  writes .xpak + xui_asset_refs.rs
                                              │
                                  bootstrap module (xui_assets::refs + manager())
                                              │
                                  XUI_ASSETS_BOOTSTRAP env → xui::include_assets!()
                                              │
                                  xui_assets::manager() → AssetManager (xui-assets)
                                              │
                                  mount order: DirectorySource (dev) → EmbeddedPak / PakSource
```

- `xui-pak` defines the `.xpak` format: fixed header, payload blobs (zstd or
  none), postcard index, blake3 content hashes, power-of-two alignment.
- `xui-pak-build` scans a source directory with `ignore`, applies glob rules,
  encodes a deterministic archive, and generates Rust asset-id constants.
- `xui-assets` `AssetManager` searches mounted sources in insertion order
  (first match wins), caches decompressed immutable bytes, and parses via
  `AssetFormat`. `DirectorySource` is `Volatile` (live edits); paks are
  `Immutable` (cached, zero-copy `AssetBytes::Mapped`/`Static`).
- `xui-cli` orchestrates all of the above from `xui.toml` and invokes Cargo.

## State storage

Two cooperating primitives back hook state:

- **`slot`** — `Scope`/`Pointer`/`Storage` with a thread-local `RenderPhase`
  (`Render`/`Event`/`Effect`/`Commit`). Debug builds assert reads/writes happen
  in a legal phase. Used directly by `xui::state`.
- **`xui-slot`** — `GenerationalBox<T, S>` with `UnsyncStorage`/`SyncStorage`
  and an `Owner` that drops its boxes on drop. Reference-counted variants and
  `Owner::insert_reference` support signal-style sharing.

## Backend selection

`xui-winit` exposes feature flags:

```toml
[dependencies]
xui-winit = { path = "../xui-winit", features = ["skia"] }   # default
# or
xui-winit = { path = "../xui-winit", default-features = false, features = ["wgpu"] }
```

`skia` pulls in `xui-skia` and re-exports `SkiaBackend`; `wgpu` pulls in the
optional `wgpu` renderer module (`TexturePool`, `WGPUBackend`, …).
