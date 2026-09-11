# xui

`xui` is a retained-mode, declarative GUI framework for Rust. It combines a
JSX-style element macro, a React-style hook/component runtime, a flexbox layout
engine, a backend-independent render graph, and a deterministic asset packaging
pipeline into a single workspace.

The framework is organized as a Cargo workspace of small, focused crates. Pick
the crates you need: depend on `xui` to build an app, or pull in `xui-interface`
on its own if you only want the shared type vocabulary.

> The example application in [`xui-example-app`](xui-example-app) is the fastest
> way to see everything wired together.

## Highlights

- **Declarative UI** — `xui! { <Column>{ /* children */ }</Column> }` macro and
  `#[component]` functions with typed, builder-style props.
- **Hook runtime** — state, memo, resources, effects, and async tasks with a
  fiber-based reconciler and lane scheduling.
- **Style system** — tokens (color, spacing, font-size, radius), themes,
  computed styles, state rules (`hovered`, `pressed`, `focused`, …), and
  animatable patches.
- **Layout** — flexbox via `taffy`, with retained spatial nodes.
- **Text** — pluggable text backend; `xui-text-engine` ships a `cosmic-text`
  implementation, `xui-text` ships a `swash`/`fontique` shaping stack.
- **Rendering** — backend-independent scene → render graph → frame pipeline.
  `xui-skia` provides a Skia backend; `xui-winit` integrates `winit` windows.
- **Assets** — deterministic, versioned `.xpak` archives built by `cargo xui`
  from `xui.toml`, with embedded or external bundles and live directory mounts.
- **No `unsafe` in the render graph** — `xui-render-graph` is `#![forbid(unsafe_code)]`.

## Workspace layout

| Crate | Role |
| --- | --- |
| [`xui`](xui/src/lib.rs) | Core framework: runtime, fiber, hooks, layout, style, widgets, render scene |
| [`xui-interface`](xui-interface/src/lib.rs) | Shared types: geometry, style, events, widgets, transitions, text traits |
| [`xui-macros`](xui-macros/src/lib.rs) | Procedural macros: `xui!`, `style!`, `#[component]`, `#[main]`, `component_fn!`, `#[derive(Animatable)]` |
| [`xui-animation`](xui-animation/src/lib.rs) | `Animatable`, `Tween`, `Timeline`, `AnimationClock`, field interpolation |
| [`xui-components`](xui-components/src/lib.rs) | Built-in widgets: button, input, dropdown, tabs, image, icon, virtual list |
| [`xui-text`](xui-text/src/lib.rs) | `swash`/`fontique` text shaping, atlas, bidi, layout |
| [`xui-text-engine`](xui-text-engine/src/lib.rs) | `cosmic-text` implementation of the `xui-interface` text traits |
| [`xui-render-graph`](xui-render-graph/src/lib.rs) | Backend-independent layer/backdrop/filter compilation (`#![forbid(unsafe_code)]`) |
| [`xui-skia`](xui-skia/src/lib.rs) | Skia rendering backend (`skia-safe`); Metal on macOS, Direct3D 12 on Windows, Vulkan on Linux, `softbuffer` fallback |
| [`xui-winit`](xui-winit/src/lib.rs) | `winit` window/event loop integration; `skia` (default) or `wgpu` backend |
| [`xui-assets`](xui-assets/src/lib.rs) | `AssetManager`, source mounting, caching, `AssetFormat` |
| [`xui-pak`](xui-pak/src/lib.rs) | Versioned `.xpak` container format and readers |
| [`xui-pak-build`](xui-pak-build/src/lib.rs) | Build-time archive writer + Rust asset-ref codegen |
| [`xui-build`](xui-build/src/lib.rs) | Packs an application's assets from its `build.rs` |
| [`xui-pak-cli`](xui-pak-cli/src/main.rs) | `xpak` binary: pack / list / verify |
| [`xui-cli`](xui-cli/src/lib.rs) | `cargo xui`: package setup, live asset mounting, external packages, inspection |
| [`slot`](slot/src/lib.rs) | Generational slot storage with `RenderPhase` guards |
| [`xui-slot`](xui-slot/src/lib.rs) | Generational box state (`UnsyncStorage` / `SyncStorage`, `Owner`) |
| [`xui-example-app`](xui-example-app/src/main.rs) | End-to-end demo application |

See [`docs/README.md`](docs/README.md) for the documentation index and
[`docs/architecture.md`](docs/architecture.md) for how the crates layer.

## Quick start

### 1. Add dependencies and a build script

```toml
[dependencies]
xui = { path = "../path/to/xui" }
xui-winit = { path = "../path/to/xui-winit", features = ["skia"] }
xui-components = { path = "../path/to/xui-components" }

[build-dependencies]
xui-build = { path = "../path/to/xui-build" }
```

```rust
// build.rs
fn main() {
    xui_build::assets();
}
```

`cargo xui init` does both, and writes an `xui.toml`; install it with
`cargo install --path xui-cli`.

### 2. Add assets (optional)

Put asset files under `assets/` next to your `Cargo.toml`. They are packed and
embedded, named by constants under `xui_assets::refs`, and Cargo repacks them
whenever anything there changes. An `xui.toml` changes the defaults:

```toml
[assets]
source = "assets"          # directory scanned for assets
bundle = "embedded"        # "embedded" or "external"
dev_directory = true       # mount the source dir live in debug builds
output = "assets.xpak"

[[assets.rules]]
glob = "**/*.{png,jpg,ogg,mp4,zip}"
compression = "none"
alignment = 16

[[assets.rules]]
glob = "**/*"
compression = "zstd"
alignment = 1
```

### 3. Write the app

```rust
use xui::prelude::*;
use xui_components::button;

#[xui::main]
fn main() {
    xui::include_assets!(); // generated by the build script

    let app = App::new(root_component);
    xui_winit::runner(app, xui_winit::WinitRunnerOptions::default())
        .expect("failed to run");
}

#[xui::component]
fn root(cx: &mut HookContext) -> ElementDesc {
    xui! {
        <Column style={style!(padding: 16.0, gap: 8.0)}>
            <button on_click={Callback::from(|_| { /* ... */ })}>
                {"Click me"}
            </button>
        </Column>
    }
}
```

### 4. Build and run

```sh
cargo run                # debug builds mount assets/ live
cargo build --release
cargo test
```

Plain Cargo, rust-analyzer and every Cargo subcommand build the app: the build
script hands `xui::include_assets!()` its generated module through
`XUI_ASSETS_BOOTSTRAP`. `xui_assets::manager()` builds the
[`AssetManager`](xui-assets/src/lib.rs).

`cargo xui` is optional, for what a build script cannot do:

```sh
cargo xui init             # set a package up; safe to rerun, migrates older packages
cargo xui run --release    # mounts assets/ live in release builds too
cargo xui build --release  # bundle = "external": copies the package beside the executable
cargo xui assets list      # list entries (or: cargo xui assets list some.xpak)
cargo xui assets verify    # integrity-check the built .xpak, or a given one
```

It forwards any Cargo subcommand, and finds the package the way Cargo does. A
package without a build script still builds through it: `cargo xui` then
generates the bootstrap module itself and passes it to Cargo.

## Architecture at a glance

```text
                 ┌──────────────────────────────────────────────┐
   application →  │ xui  (macros, runtime, fiber, hooks, layout) │
                 └───────────────┬──────────────────────────────┘
                                 │ depends on
            ┌────────────────────┼──────────────────────┐
            ▼                    ▼                      ▼
   xui-interface        xui-animation             xui-components
   (types, style,       (tweens, Animatable)      (button, input, …)
    events, text)              │
            │                   └─ xui-macros (derive Animatable)
            │
   ┌────────┴─────────┬───────────────────┬───────────────────┐
   ▼                  ▼                   ▼                   ▼
 xui-text       xui-text-engine     xui-render-graph     xui-skia
 (swash stack)  (cosmic-text impl)  (layer/filter IR)    (Skia backend)
   │                  │                   │                   │
   └──────────────────┴───────────────────┴───────────────────┘
                                   │
                            xui-winit (window/event loop)
                                   │
                            slot / xui-slot (generational state)

   assets: xui-pak ← xui-pak-build ← xui-build (build.rs) ← xui-cli (cargo xui)
                          ↖ xui-pak-cli (xpak)
                          │
                     xui-assets (AssetManager)
```

Data flow each frame:

1. `winit` produces raw window/pointer/keyboard events.
2. `xui-winit` translates them into `xui_interface::events::RawEvent`s.
3. The `xui` runtime feeds events through the event lane, runs effects, and
   re-renders dirty components into an `ElementDesc` tree.
4. The fiber reconciler diffs elements, updates the retained widget/layout tree
   (taffy flexbox), and produces a backend-independent scene.
5. The scene compiler lowers the scene into a render graph; `xui-skia` (or
   `wgpu`) rasterizes a frame to the window surface.
6. Text is laid out by the configured `TextBackend`
   (`xui-text-engine::CosmicEngine` by default).

## License

TBD — see repository metadata.
