# cargo-xui

`cargo-xui` reads `xui.toml` next to an application's `Cargo.toml`, builds its arbitrary binary
assets, and then invokes Cargo.

```toml
[assets]
source = "assets"
bundle = "embedded" # or "external"
dev_directory = true
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

Invoke `xui::include_assets!();` once at the application crate root, then use
`xui_assets::refs` and `xui_assets::manager()`.

```sh
cargo install --path xui-cli
cargo xui run
cargo xui build --release
cargo xui check
cargo xui test
cargo xui assets verify
```
