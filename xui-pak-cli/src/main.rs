//! `xpak` — minimal CLI for inspecting and building `.xpak` archives.
//!
//! The lower-level counterpart to `cargo xui`; applications usually use
//! `cargo xui` instead. Commands: `pack`, `list`, `verify`.

use std::{env, error::Error, path::Path};

use xui_pak::PakSource;

const USAGE: &str = "\
Usage: xpak <COMMAND>

Commands:
  pack [CONFIG] [OUTPUT]  Build an archive from a config (default: xui-pak.toml)
  list <PAK>              List an archive's entries
  verify <PAK>            Check every entry against its content hash

Applications usually want `cargo xui` instead, which drives this from xui.toml.";

fn main() {
    if let Err(error) = run() {
        eprintln!("xpak: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("-h" | "--help" | "help") => println!("{USAGE}"),
        None => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        Some("pack") => {
            let config = args.next().unwrap_or_else(|| "xui-pak.toml".into());
            let output = args.next();
            reject_extra(args)?;
            let result =
                xui_pak_build::build_from_config_to(&config, output.as_deref().map(Path::new))?;
            println!(
                "packed {} assets into {}",
                result.asset_count,
                result.pak_path.display()
            );
        }
        Some("list") => {
            let pak = required_arg(args.next(), "usage: xpak list <pak>")?;
            reject_extra(args)?;
            let pak = PakSource::open(pak)?;
            for entry in pak.entries() {
                println!(
                    "{}\t{}\t{:?}\t{}",
                    entry.original_len, entry.stored_len, entry.compression, entry.path
                );
            }
        }
        Some("verify") => {
            let path = required_arg(args.next(), "usage: xpak verify <pak>")?;
            reject_extra(args)?;
            let pak = PakSource::open(&path)?;
            pak.verify_all()?;
            println!("verified {}", path);
        }
        Some(other) => return Err(format!("unknown command `{other}`\n\n{USAGE}").into()),
    }
    Ok(())
}

fn required_arg(value: Option<String>, usage: &'static str) -> Result<String, Box<dyn Error>> {
    value.ok_or_else(|| usage.into())
}

fn reject_extra(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    if args.next().is_some() {
        Err("too many arguments".into())
    } else {
        Ok(())
    }
}
