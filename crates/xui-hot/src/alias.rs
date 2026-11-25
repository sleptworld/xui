//! Statics of the binary crate that a patch shares with the running app.
//!
//! A thin build's objects define the binary crate's statics again, and
//! reach them PC-relative (`adrp` + `add`/`ldr`), as their own: linked as
//! they are, a patch would have fresh copies, and its code would not see
//! what the app's had stored. Nor can such an access reach an address in
//! the app, which is outside the patch and may be farther than `adrp`
//! reaches.
//!
//! So each static the app has, in writable data, is taken out of the
//! objects ([`rewrite`]): its definition is renamed out of the way, and
//! every reference to it goes to an undefined symbol of its name instead.
//! The stub then defines those names in a zero-fill section of the patch,
//! the *mirror*, each at its offset in the app's pages
//! ([`crate::symbols::stub`]); as the patch loads, the app maps its own
//! pages over the mirror, shared (`xui-core`'s `hot.rs`), so that both
//! addresses are the same memory.
//!
//! Mach-O, arm64 only: x86_64 objects reach a local symbol by section, not
//! by symbol, so a reference cannot be told whose it is.

use std::collections::HashMap;

use anyhow::{Result, bail};

const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_OBJECT: u32 = 1;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
/// Load commands whose data is in the file: `linkedit_data_command`s.
const LINKEDIT_DATA: [u32; 4] = [
    0x26, // LC_FUNCTION_STARTS
    0x29, // LC_DATA_IN_CODE
    0x2e, // LC_LINKER_OPTIMIZATION_HINT
    0x1e, // LC_SEGMENT_SPLIT_INFO
];
const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_SECT: u8 = 0x0e;
const N_EXT: u8 = 0x01;
const NLIST_SIZE: usize = 16;
const R_EXTERN: u32 = 1 << 27;
const R_SCATTERED: u32 = 0x8000_0000;
/// What a renamed definition is called: its name, then this.
pub const SHADOW_SUFFIX: &str = "$xui_shadow";

/// One Mach-O object, rewritten: its bytes, and the statics taken out.
pub struct Rewritten {
    pub bytes: Vec<u8>,
    pub statics: Vec<String>,
}

/// Takes out of the arm64 Mach-O object `bytes` every static it defines in
/// writable data that `shared` takes (by its symbol name, underscore
/// included). `None` if it defines none.
///
/// The symbol and string tables are rebuilt at the end of the file, where
/// a compiler puts them; an object laid out otherwise is an error.
pub fn rewrite(bytes: &[u8], shared: impl Fn(&str) -> bool) -> Result<Option<Rewritten>> {
    let object = Object::parse(bytes)?;
    let symbols = object.symbols(bytes)?;
    let taken: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            symbol.n_type & N_STAB == 0
                && symbol.n_type & N_TYPE == N_SECT
                && object
                    .sections
                    .get(usize::from(symbol.n_sect).wrapping_sub(1))
                    .is_some_and(|section| {
                        crate::symbols::is_writable_data(
                            Some(section.segment.as_str()),
                            Some(section.name.as_str()),
                        )
                    })
                // Not an assembler's temporary label (`ltmp0`, `l_anon..`).
                && !symbol.name.starts_with('l')
                && !symbol.name.starts_with('L')
                && shared(&symbol.name)
        })
        .map(|(index, _)| index)
        .collect();
    if taken.is_empty() {
        return Ok(None);
    }
    object.check_layout(bytes)?;

    let (symoff, nsyms) = (object.symtab.symoff as usize, object.symtab.nsyms as usize);
    let strings = &bytes[object.symtab.stroff as usize..][..object.symtab.strsize as usize];
    let mut new_strings = strings.to_vec();
    let mut table = bytes[symoff..symoff + nsyms * NLIST_SIZE].to_vec();
    // Old index → the undefined symbol that stands for it.
    let mut moved: HashMap<u32, u32> = HashMap::new();
    let mut statics = Vec::new();
    for (k, &index) in taken.iter().enumerate() {
        let symbol = &symbols[index];
        // The definition, renamed: nothing binds to it any more.
        let shadow = new_strings.len() as u32;
        new_strings.extend_from_slice(symbol.name.as_bytes());
        new_strings.extend_from_slice(SHADOW_SUFFIX.as_bytes());
        new_strings.push(0);
        put_u32(&mut table, index * NLIST_SIZE, shadow);
        // An undefined symbol of its name, after every other one: undefined
        // symbols are the table's last group.
        let mut undefined = [0u8; NLIST_SIZE];
        undefined[..4].copy_from_slice(&symbol.n_strx.to_le_bytes());
        undefined[4] = N_EXT;
        table.extend_from_slice(&undefined);
        moved.insert(index as u32, (nsyms + k) as u32);
        statics.push(symbol.name.clone());
    }

    let mut out = bytes[..symoff].to_vec();
    // Every reference by symbol to one taken: to its undefined symbol.
    for section in &object.sections {
        for at in 0..section.nreloc as usize {
            let offset = section.reloff as usize + at * 8;
            let address = get_u32(&out, offset);
            if address & R_SCATTERED != 0 {
                bail!("a scattered relocation in an arm64 object");
            }
            let info = get_u32(&out, offset + 4);
            if info & R_EXTERN == 0 {
                continue;
            }
            if let Some(&to) = moved.get(&(info & 0x00ff_ffff)) {
                put_u32(&mut out, offset + 4, (info & !0x00ff_ffff) | to);
            }
        }
    }
    out.extend_from_slice(&table);
    out.resize(out.len().next_multiple_of(8), 0);
    let stroff = out.len();
    out.extend_from_slice(&new_strings);
    out.resize(out.len().next_multiple_of(8), 0);
    let strsize = out.len() - stroff;

    let symtab = object.symtab.command;
    put_u32(&mut out, symtab + 12, (nsyms + taken.len()) as u32);
    put_u32(&mut out, symtab + 16, stroff as u32);
    put_u32(&mut out, symtab + 20, strsize as u32);
    if let Some(dysymtab) = object.dysymtab {
        // nundefsym
        let count = get_u32(&out, dysymtab + 28);
        put_u32(&mut out, dysymtab + 28, count + taken.len() as u32);
    }
    Ok(Some(Rewritten {
        bytes: out,
        statics,
    }))
}

struct Section {
    segment: String,
    name: String,
    /// File range of its contents; `None` for zero-fill.
    contents: Option<(u64, u64)>,
    reloff: u32,
    nreloc: u32,
}

struct Symtab {
    /// Where the command is in the file.
    command: usize,
    symoff: u32,
    nsyms: u32,
    stroff: u32,
    strsize: u32,
}

struct Symbol {
    n_strx: u32,
    n_type: u8,
    n_sect: u8,
    name: String,
}

struct Object {
    sections: Vec<Section>,
    symtab: Symtab,
    /// Where the `LC_DYSYMTAB` command is in the file.
    dysymtab: Option<usize>,
    /// File ranges of other load commands' data.
    linkedit: Vec<(u64, u64)>,
}

impl Object {
    fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 32 || get_u32(bytes, 0) != MH_MAGIC_64 {
            bail!("not a 64-bit Mach-O file");
        }
        if get_u32(bytes, 4) != CPU_TYPE_ARM64 {
            bail!("statics are shared with the app on arm64 only");
        }
        if get_u32(bytes, 12) != MH_OBJECT {
            bail!("not an object file");
        }
        let ncmds = get_u32(bytes, 16);
        let mut sections = Vec::new();
        let mut symtab = None;
        let mut dysymtab = None;
        let mut linkedit = Vec::new();
        let mut at = 32;
        for _ in 0..ncmds {
            let cmd = get_u32(bytes, at);
            let size = get_u32(bytes, at + 4) as usize;
            if size < 8 || at + size > bytes.len() {
                bail!("a load command runs past the file");
            }
            match cmd {
                LC_SEGMENT_64 => {
                    let nsects = get_u32(bytes, at + 64) as usize;
                    for s in 0..nsects {
                        let sect = at + 72 + s * 80;
                        let flags = get_u32(bytes, sect + 64);
                        // S_ZEROFILL, S_GB_ZEROFILL, S_THREAD_LOCAL_ZEROFILL
                        let zerofill = matches!(flags & 0xff, 0x1 | 0xc | 0x12);
                        sections.push(Section {
                            name: fixed_str(&bytes[sect..sect + 16]),
                            segment: fixed_str(&bytes[sect + 16..sect + 32]),
                            contents: (!zerofill).then(|| {
                                (
                                    u64::from(get_u32(bytes, sect + 48)),
                                    get_u64(bytes, sect + 40),
                                )
                            }),
                            reloff: get_u32(bytes, sect + 56),
                            nreloc: get_u32(bytes, sect + 60),
                        });
                    }
                }
                LC_SYMTAB => {
                    symtab = Some(Symtab {
                        command: at,
                        symoff: get_u32(bytes, at + 8),
                        nsyms: get_u32(bytes, at + 12),
                        stroff: get_u32(bytes, at + 16),
                        strsize: get_u32(bytes, at + 20),
                    });
                }
                LC_DYSYMTAB => dysymtab = Some(at),
                cmd if LINKEDIT_DATA.contains(&cmd) => linkedit.push((
                    u64::from(get_u32(bytes, at + 8)),
                    u64::from(get_u32(bytes, at + 12)),
                )),
                _ => {}
            }
            at += size;
        }
        let Some(symtab) = symtab else {
            bail!("no symbol table");
        };
        let end = |offset: u32, size: u64| u64::from(offset) + size;
        if end(symtab.symoff, u64::from(symtab.nsyms) * NLIST_SIZE as u64) > bytes.len() as u64
            || end(symtab.stroff, u64::from(symtab.strsize)) > bytes.len() as u64
        {
            bail!("the symbol table runs past the file");
        }
        Ok(Self {
            sections,
            symtab,
            dysymtab,
            linkedit,
        })
    }

    fn symbols(&self, bytes: &[u8]) -> Result<Vec<Symbol>> {
        let strings = &bytes[self.symtab.stroff as usize..][..self.symtab.strsize as usize];
        (0..self.symtab.nsyms as usize)
            .map(|index| {
                let at = self.symtab.symoff as usize + index * NLIST_SIZE;
                let n_strx = get_u32(bytes, at);
                let name = strings
                    .get(n_strx as usize..)
                    .and_then(|rest| rest.split(|&b| b == 0).next())
                    .map(|name| String::from_utf8_lossy(name).into_owned())
                    .unwrap_or_default();
                Ok(Symbol {
                    n_strx,
                    n_type: bytes[at + 4],
                    n_sect: bytes[at + 5],
                    name,
                })
            })
            .collect()
    }

    /// The tables are rebuilt from the symbol table's offset on: nothing
    /// else may be there, and no undefined symbol may come after.
    fn check_layout(&self, bytes: &[u8]) -> Result<()> {
        let symoff = u64::from(self.symtab.symoff);
        let symend = symoff + u64::from(self.symtab.nsyms) * NLIST_SIZE as u64;
        if u64::from(self.symtab.stroff) < symend {
            bail!("the string table is not after the symbol table");
        }
        let mut ranges: Vec<(u64, u64)> = self.linkedit.clone();
        for section in &self.sections {
            ranges.extend(section.contents);
            ranges.push((u64::from(section.reloff), u64::from(section.nreloc) * 8));
        }
        if ranges
            .iter()
            .any(|&(offset, size)| size > 0 && offset + size > symoff)
        {
            bail!("data after the symbol table");
        }
        if let Some(at) = self.dysymtab {
            let (iundef, nundef) = (get_u32(bytes, at + 24), get_u32(bytes, at + 28));
            if iundef + nundef != self.symtab.nsyms {
                bail!("undefined symbols are not the symbol table's last");
            }
            // The counts of ntoc .. nlocrel (fields 7, 9, .., 17): an object
            // has none of these tables.
            if (6..18).any(|field| field % 2 == 1 && get_u32(bytes, at + 8 + field * 4) != 0) {
                bail!("an object with a dynamic symbol table's other tables");
            }
        }
        Ok(())
    }
}

fn fixed_str(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn get_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("4 bytes"))
}

fn get_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("8 bytes"))
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
mod tests {
    use super::*;
    use object::{Object as _, ObjectSection, ObjectSymbol, RelocationTarget};

    /// An object rustc makes of `source`, with codegen units as a thin
    /// build has them.
    fn compile(source: &str) -> Vec<u8> {
        let dir = std::env::temp_dir().join(format!(
            "xui-hot-alias-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("s.rs"), source).unwrap();
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
        let status = std::process::Command::new(rustc)
            .current_dir(&dir)
            .args(["--edition", "2024", "--crate-type", "bin", "--emit", "obj"])
            .args(["-C", "debuginfo=2", "-o", "s.o", "s.rs"])
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = std::fs::read(dir.join("s.o")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        bytes
    }

    const SOURCE: &str = "
        use std::sync::atomic::{AtomicUsize, Ordering};
        static HITS: AtomicUsize = AtomicUsize::new(0);
        static mut RAW: u8 = 3;
        static NAME: &str = \"app\";
        #[inline(never)]
        fn bump() -> usize { HITS.fetch_add(1, Ordering::Relaxed) }
        fn main() {
            unsafe { RAW += 1; }
            println!(\"{} {}\", bump(), NAME);
        }
    ";

    #[test]
    fn takes_the_shared_statics_out() {
        let bytes = compile(SOURCE);
        let rewritten = rewrite(&bytes, |name| name.contains("HITS") || name.contains("RAW"))
            .unwrap()
            .unwrap();
        let mut statics = rewritten.statics.clone();
        statics.sort();
        assert_eq!(statics.len(), 2);
        assert!(statics.iter().any(|name| name.contains("4HITS")));
        assert!(statics.iter().any(|name| name.contains("3RAW")));

        let file = object::File::parse(rewritten.bytes.as_slice()).unwrap();
        for name in &statics {
            // Its name is undefined now: the stub defines it.
            let symbol = file.symbol_by_name(name).unwrap();
            assert!(symbol.is_undefined(), "{name}");
            let shadow = file
                .symbol_by_name(&format!("{name}{SHADOW_SUFFIX}"))
                .unwrap();
            assert!(!shadow.is_undefined());
            // The code refers to the undefined one only.
            let mut to_undefined = 0;
            for section in file.sections() {
                if section.segment_name().ok().flatten() == Some("__DWARF") {
                    continue;
                }
                for (_, relocation) in section.relocations() {
                    if let RelocationTarget::Symbol(index) = relocation.target() {
                        assert_ne!(index, shadow.index(), "{name} still referenced");
                        to_undefined += usize::from(index == symbol.index());
                    }
                }
            }
            assert!(to_undefined > 0, "{name} referenced by nothing");
        }
        // A static in read-only data stays.
        let name = file
            .symbols()
            .find(|s| s.name().is_ok_and(|n| n.contains("4NAME")))
            .unwrap();
        assert!(!name.is_undefined());
    }

    #[test]
    fn leaves_an_object_without_them_alone() {
        let bytes = compile(SOURCE);
        assert!(rewrite(&bytes, |_| false).unwrap().is_none());
    }
}
