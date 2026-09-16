use std::{fs, process::Command};

#[test]
fn pack_list_and_verify() {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("hello.txt"), b"hello xpak").unwrap();
    let config = temp.path().join("xui-pak.toml");
    fs::write(
        &config,
        "[package]\nsource = \"assets\"\noutput = \"assets.xpak\"\n",
    )
    .unwrap();
    let binary = env!("CARGO_BIN_EXE_xpak");
    assert!(
        Command::new(binary)
            .args(["pack", config.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
    let pak = temp.path().join("assets.xpak");
    let list = Command::new(binary)
        .args(["list", pak.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(
        String::from_utf8(list.stdout)
            .unwrap()
            .contains("hello.txt")
    );
    assert!(
        Command::new(binary)
            .args(["verify", pak.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );

    let mut corrupted = fs::read(&pak).unwrap();
    corrupted[xui_pak::HEADER_LEN] ^= 0xff;
    let bad = temp.path().join("bad.xpak");
    fs::write(&bad, corrupted).unwrap();
    assert!(
        !Command::new(binary)
            .args(["verify", bad.to_str().unwrap()])
            .status()
            .unwrap()
            .success()
    );
}
