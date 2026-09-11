//! A package that packs its assets from `build.rs`, built by plain Cargo.

use std::{fs, path::Path, process::Command};

/// Runs the package's binary, which asserts that the asset it loads holds
/// `expected`.
fn cargo_run(project: &Path, expected: &str) {
    let status = Command::new(env!("CARGO"))
        .current_dir(project)
        .args(["run", "--quiet", "--offline"])
        .env("EXPECT", expected)
        // Nothing from `cargo xui`: the build script alone has to be enough.
        .env_remove("XUI_ASSETS_BOOTSTRAP")
        .env_remove("XUI_ASSETS_DIR")
        .env_remove("XUI_ASSETS_PAK")
        .status()
        .unwrap();
    assert!(status.success(), "expected the asset to hold {expected:?}");
}

#[test]
fn plain_cargo_builds_rebuilds_and_runs_a_package() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("assets/data")).unwrap();
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"xui-build-smoke\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n\
             [dependencies]\nxui = {{ path = {:?} }}\n\n\
             [build-dependencies]\nxui-build = {{ path = {:?} }}\n",
            workspace.join("xui").to_str().unwrap(),
            workspace.join("xui-build").to_str().unwrap(),
        ),
    )
    .unwrap();
    // The workspace's lockfile pins every dependency, so `--offline` resolves
    // from the local cache.
    fs::copy(workspace.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();
    fs::write(
        project.join("build.rs"),
        "fn main() {\n    xui_build::assets();\n}\n",
    )
    .unwrap();
    fs::write(
        project.join("src/main.rs"),
        r#"
xui::include_assets!();

fn main() {
    let manager = xui_assets::manager().unwrap();
    let data = manager.load(xui_assets::refs::data::APP_BIN).unwrap().unwrap();
    assert_eq!(&*data.bytes, std::env::var("EXPECT").unwrap().as_bytes());
}
"#,
    )
    .unwrap();
    // No development directory: reading the source files directly would
    // hide whether the package itself was rebuilt.
    fs::write(
        project.join("xui.toml"),
        "[assets]\ndev_directory = false\n",
    )
    .unwrap();
    fs::write(project.join("assets/data/app.bin"), "one").unwrap();
    cargo_run(project, "one");

    // Cargo reruns the build script when an asset changes.
    fs::write(project.join("assets/data/app.bin"), "two").unwrap();
    cargo_run(project, "two");

    // And when the configuration does. An external package is not beside a
    // debug executable, so the debug build falls back to the one it packed.
    fs::write(
        project.join("xui.toml"),
        "[assets]\nbundle = \"external\"\ndev_directory = false\n",
    )
    .unwrap();
    cargo_run(project, "two");

    // With the development directory on, a debug build reads the source
    // files themselves: an edit shows without Cargo rebuilding anything.
    fs::write(project.join("xui.toml"), "[assets]\n").unwrap();
    cargo_run(project, "two");
    fs::write(project.join("assets/data/app.bin"), "three").unwrap();
    let status = Command::new(project.join("target/debug/xui-build-smoke"))
        .env("EXPECT", "three")
        .env_remove("XUI_ASSETS_DIR")
        .status()
        .unwrap();
    assert!(
        status.success(),
        "the development directory was not mounted"
    );
}
