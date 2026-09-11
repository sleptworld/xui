# XUI Documentation

XUI is a retained-mode, declarative GUI framework for Rust: a JSX-style `xui!`
macro, `#[component]` functions, hook-based state, taffy flexbox layout, and
Skia/wgpu rendering, with a deterministic `.xpak` asset pipeline.

## Guides

- **[Overview & quickstart](../README.md)** — workspace map, how to build/run,
  and a first component.
- **[Architecture](architecture.md)** — crate layering, data flow, the rendering
  and asset pipelines, and state storage.

## API reference

The reference for every crate is generated from the `//!` and `///` doc comments
in the source via `cargo doc`:

```sh
cargo doc --no-deps --open        # opens target/doc/xui/index.html
```

Each crate's landing page is its top-level `//!` module doc; each public item's
page is its `///` doc comment. Keep reference prose in the source so it stays in
sync with the code.

> **`xui-example-app`:** the example binary packs its assets from its own
> build script through `xui-build`, so plain `cargo` builds, tests and documents
> it like every other member. `cargo run -p xui-example-app` launches it.

## Crate map

| Crate | Role |
| --- | --- |
| [`xui`](../xui) | framework façade: `xui!`, `#[component]`, hooks, runtime |
| [`xui-interface`](../xui-interface) | core types: `Bounds`, `Point`, `Size`, color, styling |
| [`xui-components`](../xui-components) | built-in widget library |
| [`xui-animation`](../xui-animation) | `Animation<T>` + spring/easing drivers |
| [`xui-text`](../xui-text) | text layout abstraction |
| [`xui-text-engine`](../xui-text-engine) | `cosmic-text` text backend |
| [`xui-skia`](../xui-skia) | Skia 2D renderer |
| [`xui-winit`](../xui-winit) | wgpu + winit host / runner |
| [`xui-render-graph`](../xui-render-graph) | GPU draw-call batching + transient texture slots |
| [`xui-assets`](../xui-assets) | asset ids + `AssetSource`/manager |
| [`xui-pak`](../xui-pak) | `.xpak` container format + readers |
| [`xui-pak-build`](../xui-pak-build) | archive packing + codegen |
| [`xui-build`](../xui-build) | packs an app's assets from `build.rs` |
| [`xui-cli`](../xui-cli) | `cargo xui` subcommand |
| [`xui-pak-cli`](../xui-pak-cli) | `xpak` archive CLI |
| [`xui-macros`](../xui-macros) | proc macros: `xui!`, `#[component]`, `#[main]` |
| [`xui-slot`](../xui-slot) | `Copy` `GenerationalBox` state (sync/unsync) |
| [`slot`](../slot) | generational slot storage + `RenderPhase` guards |

## Building & running

```sh
# example application (its build script packs the assets)
cargo run -p xui-example-app

# tests for every crate, the example application included
cargo test

# render the API docs (library crates) and open them
cargo doc --no-deps --open
```
