use std::{env, error::Error, path::Path};

use xui_pak::PakSource;

fn main() {
    if let Err(error) = run() {
        eprintln!("xpak: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
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
        _ => return Err("usage: xpak <pack|list|verify> ...".into()),
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
