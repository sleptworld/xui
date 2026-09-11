# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`xui` is a retained-mode, declarative GUI framework for Rust (edition 2024), organized as a Cargo workspace of small crates: a JSX-style `xui!` macro, a React-style hook/fiber runtime, taffy flexbox layout, a backend-independent render graph, Skia/wgpu backends, and a deterministic `.xpak` asset pipeline. `README.md`, `docs/architecture.md`, and `docs/components.md` are the authoritative long-form docs; API reference lives in `//!`/`///` doc comments (`cargo doc --no-deps --open`).

## Commands

```sh
cargo build                       # library crates only (default-members)
cargo test                        # all library-crate tests
cargo test -p xui                 # one crate
cargo test -p xui --test dsl      # one integration test file (xui/tests/dsl.rs)
cargo test -p xui some_test_name  # one test by name filter
cargo test -p xui --test ui       # trybuild compile-fail suite (xui/tests/ui/)
cargo test -p xui-components --lib --tests
cargo doc --no-deps --open

cargo xui run                     # build + launch xui-example-app (see below)
cargo xui build --release / check / test
cargo xui assets verify | list    # inspect the generated .xpak
```

`cargo xui` is the `xui-cli` crate installed as a cargo subcommand (`cargo install --path xui-cli`; already installed on this machine). It reads `xui.toml`, packs assets via `xui-pak-build`, sets `XUI_ASSETS_BOOTSTRAP`, then invokes plain cargo.

### Workspace gotchas

- **`xui-example-app` is a workspace member but NOT in `default-members`.** Its `#[xui::main]` expands `xui::include_assets!()`, which `include!`s the file named by `XUI_ASSETS_BOOTSTRAP` — plain `cargo build -p xui-example-app` (or `--workspace`) fails without it. Always use `cargo xui run` / `cargo xui test` for that crate.
- **`xui-table` is NOT a workspace member** (its directory exists but is absent from `Cargo.toml` `members`), so `cargo test -p xui-table` currently errors with "current package believes it's in a workspace when it's not". Add it to `members`/`default-members` before building it, or ask before doing so.
- `xui-text` and `xui-text-engine` are referenced in docs but are not in the workspace; the live text backends are `xui-cosmic` (cosmic-text, default) and `xui-f` (HarfRust/fontique shaping, no rasterization).
- `xui-render-graph` is `#![forbid(unsafe_code)]` — keep it that way.

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
    xui-skia ─► xui, render-graph   skia-safe backend; Metal/D3D12/Vulkan, softbuffer fallback (XUI_SKIA_GPU=0)
    xui-winit ─► xui, skia|wgpu, xui-cosmic, xui-f   window/event loop, AccessKit, runner
    lucide-rs                    embedded Lucide SVG set as IconData (build.rs codegen)
    xui-pak ◄─ xui-pak-build ◄─ xui-cli (cargo xui) / xui-pak-cli (xpak)
```

**Per-frame flow:** winit event → `xui_winit::translate_window_event` → `RawEvent` → `GuiRuntime`/`App` runs the event lane (EventTranslator → semantic events/callbacks) and effect lane (effects, tokio task wakeups) → render phase: dirty components re-render via `HookContext` into `ElementDesc` → fiber reconciler diffs into the retained widget/layout tree → style system merges patches+theme+`WidgetStateMatcher` rules into `ComputedStyle` → taffy layout → scene (`RenderNodeId`/`PictureId`/`PrimitiveId`) → scene compiler → render graph → backend rasterizes; text goes through `TextHost` → configured `TextBackend`.

**Phase guards:** `slot` keeps a thread-local `RenderPhase` (`Render`/`Event`/`Effect`/`Commit`). Hook storage may only be written in `Event`/`Effect`; debug builds assert this. If a test or new code panics on a phase assertion, the fix is usually where the write happens, not the guard.

**Backend feature flags (`xui-winit`):** `skia` (default) → `xui-skia`; `wgpu` → standalone wgpu renderer; `skia-wgpu` → Skia running on a wgpu-owned device so a `CanvasController::with_gpu_painter` canvas can composite its own shader output (enables `xui/wgpu` and `xui-skia/wgpu`). `xui-skia` itself defaults to its `wgpu` feature; `wgpu-hal` is pinned to `>=29.0.4` deliberately (see comment in `xui-skia/Cargo.toml`).

**Components contract** (`docs/components.md`): stateful controls take an optional controlled value + `default_*` + `on_*` callback; controlled values are the source of truth. Layout primitives (`ContainerWidget`, `GridWidget`, portals) stay in `xui`, not `xui-components`. Disabled items are removed from focus navigation; modal overlays trap/restore focus and render in a portal.

## Testing conventions

- Crates that need a text backend in tests use `xui-cosmic` as a dev-dependency (`xui`, `xui-table`, `xui-example-app`).
- `xui/tests/ui.rs` drives trybuild against `xui/tests/ui/*.rs` for macro diagnostics; add a `.stderr` alongside new compile-fail cases.
- `xui-components/tests/virtual_list_bench.rs` and `xui/src/frame_bench.rs` (cfg(test)) are benchmarks-as-tests; don't gate on their timings.
