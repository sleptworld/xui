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

/// `cargo xui new`: a package that depends on this checkout's `xui` and
/// `xui-build`, with the asset setup and an app to start from.
#[test]
fn new_creates_an_app_on_this_checkout() {
    let temp = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_cargo-xui"))
        .current_dir(temp.path())
        .args([
            "xui",
            "new",
            "hello",
            "--vcs",
            "none",
            "--offline",
            "--quiet",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let app = temp.path().join("hello");
    let manifest = fs::read_to_string(app.join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[dependencies]\nxui = "), "{manifest}");
    assert!(
        manifest.contains("[build-dependencies]\nxui-build = "),
        "{manifest}"
    );
    for file in ["xui.toml", "build.rs", "src/main.rs"] {
        assert!(app.join(file).is_file(), "{file}");
    }
    assert!(app.join("assets").is_dir());
    let main = fs::read_to_string(app.join("src/main.rs")).unwrap();
    assert!(main.contains("Hello from hello"));

    // A second time: the directory is there, and `init` is the way on.
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-xui"))
        .current_dir(temp.path())
        .args(["xui", "--color", "never", "new", "hello"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists") && stderr.contains("cargo xui init"),
        "{stderr}"
    );
}
