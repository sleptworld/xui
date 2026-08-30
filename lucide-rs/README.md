# lucide-rs

`lucide-rs` embeds the complete Lucide 1.27.0 SVG set and exposes each icon as
XUI `IconData`.

```rust
let icon = lucide_rs::icons::circle_check();
let dynamic_icon = lucide_rs::get("circle-check").unwrap();
```

SVG files are embedded in the binary at compile time. Each icon is parsed on
its first use and cached independently in a `OnceLock`, so unused icons have no
startup parsing cost.

Names that are Rust keywords gain a trailing underscore (`move_`, `type_`),
and names beginning with a digit gain an `icon_` prefix (`icon_3d_glasses`).

The vendored icons come from Lucide 1.27.0. See `LICENSE` for the upstream ISC
license and the MIT terms covering icons derived from Feather.
