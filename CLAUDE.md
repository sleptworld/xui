# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`xui` is a retained-mode, declarative GUI framework for Rust (edition 2024), organized as a Cargo workspace of small crates: a JSX-style `xui!` macro, a React-style hook/fiber runtime, taffy flexbox layout, a backend-independent render graph, Vello (default) and Skia backends, and a deterministic `.xpak` asset pipeline. `README.md`, `docs/architecture.md`, and `docs/components.md` are the authoritative long-form docs; API reference lives in `//!`/`///` doc comments (`cargo doc --no-deps --open`).

## Commands

```sh
cargo build                       # every default member, xui-example-app included
cargo test                        # all tests
cargo test -p xui                 # one crate
cargo test -p xui --test dsl      # one integration test file (xui/tests/dsl.rs)
cargo test -p xui some_test_name  # one test by name filter
cargo test -p xui --test ui       # trybuild compile-fail suite (xui/tests/ui/)
cargo test -p xui-components --lib --tests
cargo doc --no-deps --open

cargo run -p xui-example-app      # launch the example app (Component Gallery; XUI_THEME=dark)
cargo run -p xui-example-app --bin flight-icing   # the aircraft icing dashboard demo

cargo xui init                    # set a package up for assets (xui.toml, assets/, build.rs, xui-build dep)
cargo xui run --release           # forwards to cargo; mounts assets/ live, copies external packages
cargo xui assets verify | list    # inspect an .xpak
```

Assets are packed by each app's `build.rs` calling `xui_build::assets()`, so plain cargo works everywhere. `cargo xui` is the optional `xui-cli` crate installed as a cargo subcommand (`cargo install --path xui-cli`); the installed binary does not track this checkout, so reinstall it after changing `xui-cli` or `xui-build`.

### Workspace gotchas

- **A crate using `#[xui::main]` (or `#[xui_macros::main]`) / `xui_core::include_assets!()` needs a build script calling `xui_build::assets()`** (with `xui-build` in `[build-dependencies]`, as `xui-example-app` has). It hands the generated bootstrap to the compiler via `cargo::rustc-env=XUI_ASSETS_BOOTSTRAP`; without it the crate only compiles through `cargo xui`. The bootstrap's generator lives in `xui-build` and its runtime half in `xui_core::assets::bootstrap` — change them together.
- **`xui-table` is NOT a workspace member** (its directory exists but is absent from `Cargo.toml` `members`), so `cargo test -p xui-table` currently errors with "current package believes it's in a workspace when it's not". Add it to `members`/`default-members` before building it, or ask before doing so.
- `xui-text` and `xui-text-engine` are referenced in docs but are not in the workspace; the live text backends are `xui-cosmic` (cosmic-text, default) and `xui-f` (HarfRust/fontique shaping); both rasterize through `xui-swash`.
- `xui-render-graph` is `#![forbid(unsafe_code)]` — keep it that way.
- **`xui-core` depends on `xui-macros` only to put `#[widget]` on its built-in widgets; it does not re-export the macros, and `xui-animation` does not depend on them.** The macros (`xui!`, `style!`, `#[component]`, `#[widget]`, `#[defaults]`, `#[main]`, `derive(Animatable)`) are re-exported by the `xui` facade (`xui::prelude::*`); workspace crates on `xui-core` import them from `xui_macros` directly. Generated code names the runtime as `xui_core` when that is a direct dependency (`crate` inside `xui-core`'s own library) and falls back to `xui` otherwise — `krate.rs` in `xui-macros` for the macros, `runtime_crate` in `xui-build` for the asset bootstrap — so anything a macro expands to must stay reachable through `xui`'s `pub use xui_core::*`. Items generated code needs from `xui-interface` go through `xui_core::__interface`, which re-exports only those.

## Architecture (big picture)

Dependencies point strictly downward:

```
app ──► xui (runtime, fiber, hooks, layout, style, widgets, render scene)
          ├─► xui-interface   shared vocabulary: geometry, Style/StylePatch/ComputedStyle,
          │                   tokens/themes, RawEvent*, text traits (TextBackend, Shaper, …)
          ├─► xui-macros      xui!, style!, #[component], #[main], #[derive(Animatable)]
          ├─► xui-animation   Animatable/Tween/Timeline
          ├─► xui-render-graph  compile_layer → LayerProgram → instantiate per frame (pure Rust)
          ├─► xui-assets      AssetManager (mount order: DirectorySource dev → EmbeddedPak/PakSource)
          └─► slot            phase-guarded generational storage backing hook state
    xui-components ─► xui        built-in widgets (button, input, dialog, data_table, …)
    xui-table ─► xui             advanced_table + pure, window-free TableModel
    xui-shell ─► xui             host contract: PlatformWindow/SurfaceTarget/WindowOptions + Shell driver
    xui-vello ─► xui, render-graph, shell, xui-wgpu-text   the default backend: vello + wgpu, composes layers itself, text from glyph bitmaps
    xui-skia ─► xui, render-graph, shell   optional skia-safe backend; Metal/D3D12/Vulkan, softbuffer fallback (XUI_SKIA_GPU=0)
    xui-swash ─► xui-interface   swash GlyphRasterizer over any FontCollection, with a bitmap cache; used by xui-f, xui-cosmic
    xui-wgpu-text ─► xui-interface, wgpu   draws glyph runs (and carets/underlines) with wgpu: GlyphAtlas (etagere allocation and eviction over the page textures, no CPU copy), text.wgsl; xui-vello draws its text with it
    xui-winit ─► xui, shell, vello|skia|wgpu, xui-cosmic, xui-f   winit host for xui-shell, runner
    xui-macos ─► xui, shell, vello|skia, xui-f   native AppKit host for xui-shell (objc2 0.6); empty off macOS; xui-example-app runs on it
    xui-windows ─► xui, shell, vello|skia, xui-f   native Win32 host for xui-shell (windows 0.62, IMM32 IME); empty off Windows; never built or run — see its lib.rs
    lucide-rs                    embedded Lucide SVG set as IconData (build.rs codegen)
    xui-pak ◄─ xui-pak-build ◄─ xui-build (build.rs) ◄─ xui-cli (cargo xui); xui-pak-build ◄─ xui-pak-cli (xpak)
```

**Window hosts:** a host (`xui-winit`, or the native `xui-macos`) owns the event loop and window, translates native events into `xui_shell::ShellEvent`, and implements `PlatformWindow`; `xui_shell::Shell` holds all platform-independent host state (modifiers, buttons, visibility, first-frame reveal, cursor/IME sync). Render backends take `Arc<dyn SurfaceTarget>`, never a winit type — keep winit out of `xui-shell`, `xui-vello` and `xui-skia`.

**Host ↔ runtime contract** (design and status in `docs/shell-api.md`): besides events in and diffed `PlatformOutput` out, the application leaves one-shot `PlatformCommand`s and `PlatformRequest`s (clipboard, dialogs, window commands) on `App::platform()` (a `PlatformHandle`); `Shell` drains them into `PlatformWindow::perform`, and replies come back through the event route, never from inside `perform`. Everything the runtime produces or consumes here lives in `xui-interface::platform`, because `xui-shell` depends on `xui-core`. Hosts implement only what they can — every new `PlatformWindow` method defaults to "unsupported". A host should hold a `ShellHandle` (re-entrant `submit`) rather than keep its own event queue, and pass native event timestamps to `dispatch_event`/`submit`. Phases 0 (types and plumbing) and 1 (`xui-macos` on it) are done; `xui-winit` and `xui-windows` do not use it yet, and of the widgets only `TextInputWidget` does (copy, cut, paste, select all, undo, redo, by key or as commands; a widget reaches the host through `EventContext::send_platform` / `request_platform`, and the answer comes back to it as `SemanticEvent::PlatformReply`). `cargo run -p xui-macos --example smoke` drives a real window and asserts the macOS host's behaviour (needs the screen on and unlocked, else it stops at its first check; it takes the clipboard briefly and restores it).

**Per-frame flow:** winit event → `WinitRunner` → `ShellEvent` → `Shell` → `RawEvent` → `GuiRuntime`/`App` runs the event lane (EventTranslator → semantic events/callbacks) and effect lane (effects, tokio task wakeups) → render phase: dirty components re-render via `HookContext` into `ElementDesc` → fiber reconciler diffs into the retained widget/layout tree → style system merges patches+theme+`WidgetStateMatcher` rules into `ComputedStyle` → taffy layout → scene (`RenderNodeId`/`PictureId`/`PrimitiveId`) → scene compiler → render graph → backend rasterizes; text goes through `TextHost` → configured `TextBackend`.

**UiRuntime contract** (`crates/xui-core/src/ui_runtime/`, design in `docs/ui-runtime-api.md`): files are split by caller — `commit.rs` (fiber commit; the only code that changes tree shape, via `place`/`remove_subtree`), `query.rs`/`input.rs` (event system), `frame.rs`/`config.rs` (`App`), `scroll.rs`; `work.rs`/`style_pass.rs`/`layout_pass.rs`/`paint_pass.rs` are private stages. Every `NodeId` taken is a live-id precondition (`debug_assert!`, dense subsystems index directly and return no `Option`); ids that can go stale (dirty queues, controller requests, overlay order) are filtered once where they are drained (`live()`), nowhere else. Don't add `contains_key` guards to internal functions — fix the caller. Paint transforms are never set directly either: mark `HostWorkFlags::SYNC_TRANSFORM` and the render sync derives it once, after layout.

**Phase guards:** `slot` keeps a thread-local `RenderPhase` (`Render`/`Event`/`Effect`/`Commit`). Hook storage may only be written in `Event`/`Effect`; debug builds assert this. If a test or new code panics on a phase assertion, the fix is usually where the write happens, not the guard.

**Text backend contract** (`xui-interface/src/text.rs`): `TextBackend` is a bundle of associated types — `Fonts: FontCollection` (shared `Arc`, `&self`, `Send + Sync`; every font id a layout uses must resolve through it until its `epoch` moves), `Shaper` (the `&mut` layout context; most backends use `Shaper = Self`), and `Rasterizer: GlyphRasterizer` (`&self`, `Send + Sync`, `NoRasterizer` when absent). Layouts are scale-independent; renderers build `GlyphRequest`s (font, glyph, physical ppem, subpixel offset, `RasterOptions`) themselves. There is no per-glyph key and no text-backend scale factor. `GlyphImage` pixels: `Mask` = 1 byte/px, `SubpixelMask` = RGBA, `Color` = premultiplied RGBA.

**Backend feature flags (every host, and the `xui` facade):** `vello` (default) → `xui-vello`, the host's `runner`; `skia` → `xui-skia`, the host's `skia_runner` (independent of `vello`); `skia-wgpu` → Skia running on a wgpu-owned device so a `CanvasController::with_gpu_painter` canvas can composite its own shader output (enables `xui-core/wgpu` and `xui-skia/wgpu`; Vello always runs on its own wgpu device, so it needs no such flag). `xui-winit` also has `wgpu` → the standalone wgpu renderer (`wgpu_runner`), which does not compile against the current wgpu. `xui-example-app` runs on Vello; `--features skia` runs it on Skia, and its snapshot tests render headless through Skia either way. `xui-skia` itself defaults to its `wgpu` feature; `wgpu-hal` is pinned to `>=29.0.4` deliberately (see comment in `xui-skia/Cargo.toml`).

**Components contract** (`docs/components.md`): stateful controls take an optional controlled value + `default_*` + `on_*` callback; controlled values are the source of truth. Layout primitives (`ContainerWidget`, `GridWidget`, portals) stay in `xui`, not `xui-components`. Disabled items are removed from focus navigation; modal overlays trap/restore focus and render in a portal.

## Testing conventions

- Crates that need a text backend in tests use `xui-cosmic` as a dev-dependency (`xui`, `xui-table`, `xui-example-app`).
- `xui/tests/ui.rs` drives trybuild against `xui/tests/ui/*.rs` for macro diagnostics; add a `.stderr` alongside new compile-fail cases.
- `cargo test -p xui --features strict-invariants` turns every `invariant!` report in the runtime into a panic instead of a `log::error!`, so a host/layout/style/render desync fails the suite instead of degrading silently. It is expected to stay green; applications must not enable it.
- `xui-components/tests/virtual_list_bench.rs` and `xui/src/frame_bench.rs` (cfg(test)) are benchmarks-as-tests; don't gate on their timings.
