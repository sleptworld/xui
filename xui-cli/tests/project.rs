use std::{fs, process::Command};

#[test]
fn embedded_and_external_projects_compile_and_load_assets() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path();
    // The workspace root is where its lockfile is, however deep this crate sits.
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join("Cargo.lock").is_file())
        .expect("xui-cli is built inside the xui workspace");
    let xui_core_path = workspace.join("crates/xui-core");
    fs::create_dir_all(project.join("src")).unwrap();
    fs::create_dir_all(project.join("assets/data")).unwrap();
    fs::write(project.join("assets/data/app.bin"), b"hello from xpak").unwrap();
    fs::write(
        project.join("Cargo.toml"),
        format!(
            "[package]\nname = \"xui-assets-smoke\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\nxui-core = {{ path = {:?} }}\n",
            xui_core_path.to_str().unwrap()
        ),
    )
    .unwrap();
    fs::write(
        project.join("src/main.rs"),
        r#"
xui_core::include_assets!();

fn main() {
    let manager = xui_assets::manager().unwrap();
    let data = manager.load(xui_assets::refs::data::APP_BIN).unwrap().unwrap();
    assert_eq!(&*data.bytes, b"hello from xpak");
}
"#,
    )
    .unwrap();

    // The workspace's lockfile pins every dependency `xui` pulls in, so an
    // `--offline` build resolves from the local cache instead of needing a
    // registry.
    fs::copy(workspace.join("Cargo.lock"), project.join("Cargo.lock")).unwrap();

    // (config, directory run from): embedded from a subdirectory, as Cargo
    // allows; external from the package root; and no xui.toml at all.
    let cases = [
        (Some("embedded"), "src"),
        (Some("external"), "."),
        (None, "."),
    ];
    for (mode, directory) in cases {
        match mode {
            Some(mode) => fs::write(
                project.join("xui.toml"),
                format!(
                    "[assets]\nsource = \"assets\"\nbundle = \"{mode}\"\ndev_directory = false\n"
                ),
            )
            .unwrap(),
            None => fs::remove_file(project.join("xui.toml")).unwrap(),
        }
        let status = Command::new(env!("CARGO_BIN_EXE_cargo-xui"))
            .current_dir(project.join(directory))
            .args(["xui", "run", "--quiet", "--offline"])
            .status()
            .unwrap();
        assert!(status.success(), "{mode:?} from {directory} failed");
    }
}
