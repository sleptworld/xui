//! The app's symbols, the stub a patch links with, the jump table.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use object::write::{Object as WriteObject, StandardSection, Symbol, SymbolSection};
use object::{
    Architecture, BinaryFormat, Endianness, Object, ObjectSection, ObjectSymbol, SymbolFlags,
    SymbolKind, SymbolScope,
};
use subsecond_types::{AddressMap, JumpTable};

use anyhow::{Context, Result, bail};

/// One symbol of the app's binary, as it is on disk (not slid).
#[derive(Debug, Clone)]
struct Cached {
    address: u64,
    kind: SymbolKind,
    undefined: bool,
    weak: bool,
    size: u64,
    macho_desc: Option<u16>,
}

/// The app's binary, read once after the fat build.
pub struct SymbolCache {
    symbols: HashMap<String, Cached>,
    architecture: Architecture,
    format: BinaryFormat,
    /// The `main` sentinel's address on disk.
    main: u64,
    /// Initial values of the binary's thread-locals, and each one's range.
    tls_data: Vec<u8>,
    tls_ranges: HashMap<String, (u64, u64)>,
}

fn main_sentinel(format: BinaryFormat) -> &'static str {
    // The symbol table spells C names with a leading underscore on Mach-O.
    match format {
        BinaryFormat::MachO => "_main",
        _ => "main",
    }
}

impl SymbolCache {
    pub fn load(binary: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(binary).with_context(|| format!("reading {}", binary.display()))?;
        let file = object::File::parse(bytes.as_slice())?;
        let mut symbols: HashMap<String, Cached> = HashMap::new();
        for symbol in file.symbols() {
            let Ok(name) = symbol.name() else { continue };
            let cached = Cached {
                address: symbol.address(),
                kind: symbol.kind(),
                undefined: symbol.is_undefined(),
                weak: symbol.is_weak(),
                size: symbol.size(),
                macho_desc: match symbol.flags() {
                    SymbolFlags::MachO { n_desc } => Some(n_desc),
                    _ => None,
                },
            };
            // A definition wins over an import of the same name.
            match symbols.get(name) {
                Some(existing) if !existing.undefined && cached.undefined => {}
                _ => {
                    symbols.insert(name.to_owned(), cached);
                }
            }
        }
        let format = file.format();
        let main = symbols
            .get(main_sentinel(format))
            .filter(|main| !main.undefined)
            .with_context(|| format!("no `main` in {}: are symbols stripped?", binary.display()))?
            .address;

        let tls = file
            .sections()
            .find(|section| matches!(section.name(), Ok("__thread_data" | ".tdata")));
        let tls_data = tls
            .as_ref()
            .and_then(|section| section.data().ok())
            .unwrap_or_default()
            .to_vec();
        let (tls_start, tls_size, tls_index) = tls
            .as_ref()
            .map_or((0, 0, None), |s| (s.address(), s.size(), Some(s.index())));
        let mut starts: Vec<(u64, String)> = file
            .symbols()
            .filter(|symbol| tls_index.is_some() && symbol.section_index() == tls_index)
            .filter_map(|symbol| {
                Some((
                    symbol.address().saturating_sub(tls_start),
                    symbol.name().ok()?.to_owned(),
                ))
            })
            .collect();
        starts.sort();
        starts.dedup_by_key(|(offset, _)| *offset);
        let tls_ranges = starts
            .iter()
            .enumerate()
            .map(|(i, (offset, name))| {
                let end = starts.get(i + 1).map_or(tls_size, |(next, _)| *next);
                (name.clone(), (*offset, end.saturating_sub(*offset)))
            })
            .collect();

        Ok(Self {
            symbols,
            architecture: file.architecture(),
            format,
            main,
            tls_data,
            tls_ranges,
        })
    }
}

/// The object that defines, for a patch, what its objects use and the app
/// has: each function as a jump to its address in the running app, each
/// static as that address, each thread-local as one of its own (a patch
/// cannot reach the app's thread-local storage; the app's binary crate's
/// thread-locals start over in a patch).
///
/// `aslr_reference` is where the app's `main` is loaded.
pub fn stub(cache: &SymbolCache, objects: &[PathBuf], aslr_reference: u64) -> Result<Vec<u8>> {
    if cache.format != BinaryFormat::MachO {
        bail!("hot patching supports macOS only for now");
    }
    let mut undefined = HashSet::new();
    let mut defined = HashSet::new();
    for path in objects {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let file = object::File::parse(bytes.as_slice())?;
        for symbol in file.symbols() {
            let Ok(name) = symbol.name() else { continue };
            if symbol.is_undefined() {
                undefined.insert(name.to_owned());
            } else if symbol.is_global() {
                defined.insert(name.to_owned());
            }
        }
    }
    let Some(slide) = aslr_reference.checked_sub(cache.main) else {
        bail!(
            "the app's `main` ({aslr_reference:#x}) is below the binary's ({:#x})",
            cache.main
        );
    };

    let mut obj = WriteObject::new(BinaryFormat::MachO, cache.architecture, Endianness::Little);
    obj.set_macho_build_version({
        let mut version = object::write::MachOBuildVersion::default();
        version.platform = object::macho::PLATFORM_MACOS;
        version.minos = 11 << 16;
        version.sdk = 11 << 16;
        version
    });
    let text = obj.section_id(StandardSection::Text);
    let mut names: Vec<&String> = undefined.difference(&defined).collect();
    names.sort();
    for name in names {
        let Some(symbol) = cache.symbols.get(name.as_str()) else {
            continue;
        };
        if symbol.undefined {
            // The app imports it too (from a system library): the dynamic
            // linker finds it for the patch as it did for the app.
            continue;
        }
        // The writer adds Mach-O's underscore itself.
        let bare = name.strip_prefix('_').unwrap_or(name).as_bytes().to_vec();
        let address = symbol.address + slide;
        match symbol.kind {
            SymbolKind::Text => {
                let code = jump(cache.architecture, address)?;
                let offset = obj.append_section_data(text, &code, 8);
                obj.add_symbol(Symbol {
                    name: bare,
                    value: offset,
                    size: code.len() as u64,
                    scope: SymbolScope::Linkage,
                    kind: SymbolKind::Text,
                    weak: false,
                    section: SymbolSection::Section(text),
                    flags: SymbolFlags::None,
                });
            }
            SymbolKind::Tls => {
                let tls = obj.section_id(StandardSection::Tls);
                let (offset, size) = cache
                    .tls_ranges
                    .get(&format!("{name}$tlv$init"))
                    .copied()
                    .unwrap_or((0, symbol.size.max(8)));
                let (start, end) = (offset as usize, (offset + size) as usize);
                let init = cache
                    .tls_data
                    .get(start..end)
                    .map_or_else(|| vec![0; size as usize], <[u8]>::to_vec);
                let id = obj.add_symbol(Symbol {
                    name: bare,
                    value: 0,
                    size: 0,
                    scope: SymbolScope::Linkage,
                    kind: SymbolKind::Tls,
                    weak: false,
                    section: SymbolSection::Undefined,
                    flags: SymbolFlags::None,
                });
                // Through `add_symbol_data`, so the writer makes the TLV
                // descriptor.
                obj.add_symbol_data(id, tls, &init, size.clamp(1, 8).next_power_of_two());
            }
            // Statics and the rest: the app's own, at its address.
            kind => {
                obj.add_symbol(Symbol {
                    name: bare,
                    value: address,
                    size: 0,
                    scope: SymbolScope::Linkage,
                    kind: if kind == SymbolKind::Unknown {
                        SymbolKind::Data
                    } else {
                        kind
                    },
                    weak: symbol.weak,
                    section: SymbolSection::Absolute,
                    flags: symbol
                        .macho_desc
                        .map_or(SymbolFlags::None, |n_desc| SymbolFlags::MachO { n_desc }),
                });
            }
        }
    }
    Ok(obj.write()?)
}

/// Machine code that jumps to `address`.
fn jump(architecture: Architecture, address: u64) -> Result<Vec<u8>> {
    let mut code = match architecture {
        // ldr x16, #8; br x16
        Architecture::Aarch64 => vec![0x50, 0x00, 0x00, 0x58, 0x00, 0x02, 0x1f, 0xd6],
        // jmp [rip + 0]
        Architecture::X86_64 => vec![0xff, 0x25, 0x00, 0x00, 0x00, 0x00],
        other => bail!("hot patching does not support {other:?}"),
    };
    code.extend_from_slice(&address.to_le_bytes());
    Ok(code)
}

/// Maps each of the app's functions (by its address on disk) to its
/// version in the patch, matched by name.
pub fn jump_table(cache: &SymbolCache, patch: &Path) -> Result<JumpTable> {
    let bytes = std::fs::read(patch).with_context(|| format!("reading {}", patch.display()))?;
    let file = object::File::parse(bytes.as_slice())?;
    let mut map = AddressMap::default();
    let mut new_main = None;
    for symbol in file.symbol_map().symbols() {
        let name = symbol.name();
        if name == main_sentinel(cache.format) {
            new_main = Some(symbol.address());
        }
        if let Some(old) = cache.symbols.get(name).filter(|old| !old.undefined) {
            map.insert(old.address, symbol.address());
        }
    }
    Ok(JumpTable {
        lib: patch.to_path_buf(),
        map,
        aslr_reference: cache.main,
        new_base_address: new_main.with_context(|| "the patch has no `main`".to_owned())?,
        ifunc_count: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumps_load_the_address_after_the_instructions() {
        let arm = jump(Architecture::Aarch64, 0x1122_3344_5566_7788).unwrap();
        // ldr x16, #8; br x16; then the address.
        assert_eq!(&arm[..8], &[0x50, 0x00, 0x00, 0x58, 0x00, 0x02, 0x1f, 0xd6]);
        assert_eq!(&arm[8..], &0x1122_3344_5566_7788u64.to_le_bytes());
        let x86 = jump(Architecture::X86_64, 0x42).unwrap();
        assert_eq!(&x86[..6], &[0xff, 0x25, 0, 0, 0, 0]);
        assert_eq!(&x86[6..], &0x42u64.to_le_bytes());
        assert!(jump(Architecture::Riscv64, 0).is_err());
    }
}
