//! Where the binary's bytes went.
//!
//! On a microcontroller this is not a curiosity, it is a hard constraint: a
//! build that overflows flash fails at link time with a message that names a
//! region and a byte count and nothing about *what* filled it. `cargo size`
//! gives per-section totals, which tells you the situation but not the cause.
//!
//! So this attributes bytes to crates, by demangling every symbol and taking
//! the first path segment. That answers the question people actually have —
//! "what is costing me 40 KB" — which section totals never can.
//!
//! Attribution is best-effort by nature. C and C++ have no crate, so their
//! bytes go to the source file that compiled them: the file the image's DWARF
//! says covers the symbol's address, or — with no debug information, as a
//! CMake Release build has — the `STT_FILE` symbol a file's own `static`s sit
//! under. What neither says, and linker fill, is reported as unattributed
//! rather than spread across crates, because a number that is quietly wrong
//! is worse than one that is visibly incomplete.

use std::{cmp::Reverse, collections::BTreeMap, path::Path};

use object::{Object, ObjectSection, ObjectSymbol, SymbolKind};

use crate::{
    chip,
    error::{Error, Result},
    model::{CrateSize, MemoryReport, MemoryTotals, SectionKind, SectionSize},
};

/// Analyse a linked ELF.
pub fn analyze(elf_path: &Path, chip_id: Option<&str>) -> Result<MemoryReport> {
    let bytes = std::fs::read(elf_path).map_err(Error::reading(elf_path))?;
    let file = object::File::parse(&*bytes).map_err(|e| Error::Elf {
        path: elf_path.display().to_string(),
        detail: e.to_string(),
    })?;

    let mut sections = Vec::new();
    let mut flash_bytes = 0u64;
    let mut ram_bytes = 0u64;

    for section in file.sections() {
        // Only allocated sections exist on the device. Debug info, symbol
        // tables, and .comment live in the ELF but never reach the chip, and
        // counting them would inflate every number on this screen.
        let Ok(name) = section.name() else { continue };
        let size = section.size();
        if size == 0 || !is_allocated(&section) {
            continue;
        }

        let kind = classify(&section);
        let (in_flash, in_ram) = kind.budget();
        if in_flash {
            flash_bytes += size;
        }
        if in_ram {
            ram_bytes += size;
        }

        sections.push(SectionSize {
            name: name.to_string(),
            address: section.address(),
            size,
            kind,
        });
    }

    // Largest first: the panel is read to find what to cut.
    sections.sort_by_key(|s| Reverse(s.size));

    let (crates, unattributed_bytes) = attribute_to_crates(&file);
    let chip_info = chip_id.and_then(chip::by_id);

    Ok(MemoryReport {
        elf_path: elf_path.display().to_string(),
        chip: chip_id.map(str::to_string),
        sections,
        totals: MemoryTotals {
            flash_bytes,
            ram_bytes,
            ram_capacity: chip_info.as_ref().map(|c| c.sram_bytes),
            flash_capacity: chip_info.as_ref().and_then(|c| c.flash_bytes),
        },
        crates,
        unattributed_bytes,
    })
}

// Read straight off the ELF header rather than through `object`'s own section
// classification: linker scripts for these chips invent section names
// (`.rwtext`, `.rodata_wifi`, `.dram2_uninit`) that no heuristic classifies
// correctly, and the flags are unambiguous.
const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;
/// Occupies address space but stores no bytes in the file — `.bss`.
const SHT_NOBITS: u32 = 8;

fn elf_header(section: &object::Section<'_, '_>) -> (u64, u32) {
    match section.flags() {
        // `object` 0.40 wraps both in newtypes; `.0` is the raw header field.
        object::SectionFlags::Elf { sh_flags, sh_type } => (sh_flags.0, sh_type.0),
        _ => (0, 0),
    }
}

/// Whether this section is loaded onto the device at all.
///
/// Debug info, symbol tables, and `.comment` live in the ELF and never reach
/// the chip; counting them would inflate every figure on this screen — a
/// debug-heavy build would appear not to fit when it fits fine.
fn is_allocated(section: &object::Section<'_, '_>) -> bool {
    elf_header(section).0 & SHF_ALLOC != 0
}

fn classify(section: &object::Section<'_, '_>) -> SectionKind {
    let (flags, section_type) = elf_header(section);
    if flags & SHF_EXECINSTR != 0 {
        SectionKind::Code
    } else if section_type == SHT_NOBITS {
        SectionKind::ZeroedData
    } else if flags & SHF_WRITE != 0 {
        SectionKind::InitialisedData
    } else {
        SectionKind::ReadOnlyData
    }
}

/// Sum symbol sizes per originating crate.
fn attribute_to_crates(file: &object::File<'_>) -> (Vec<CrateSize>, u64) {
    let mut totals: BTreeMap<String, CrateSize> = BTreeMap::new();
    let mut unattributed = 0u64;
    let units = foreign_units(file);
    // A symbol table lists each object file's locals after its `STT_FILE`
    // symbol, and every global after all the locals.
    let mut file_of_locals: Option<String> = None;

    for symbol in file.symbols() {
        if symbol.kind() == SymbolKind::File {
            file_of_locals = symbol.name().ok().and_then(foreign_source);
            continue;
        }
        if symbol.is_global() {
            file_of_locals = None;
        }
        let size = symbol.size();
        if size == 0 {
            continue;
        }
        // A symbol only occupies space if its section is loaded onto the chip.
        let Some(index) = symbol.section_index() else {
            continue;
        };
        let Ok(section) = file.section_by_index(index) else {
            continue;
        };
        if !is_allocated(&section) {
            continue;
        }

        let Ok(name) = symbol.name() else {
            unattributed += size;
            continue;
        };
        let address = symbol.address();
        let Some(krate) = crate_of(name)
            .or_else(|| {
                units
                    .iter()
                    .find(|(begin, end, _)| (*begin..*end).contains(&address))
                    .map(|(_, _, source)| source.clone())
            })
            .or_else(|| file_of_locals.clone())
        else {
            unattributed += size;
            continue;
        };

        let entry = totals.entry(krate.clone()).or_insert_with(|| CrateSize {
            name: krate,
            code: 0,
            read_only_data: 0,
            data: 0,
            bss: 0,
            total: 0,
        });
        match classify(&section) {
            SectionKind::Code => entry.code += size,
            SectionKind::ReadOnlyData => entry.read_only_data += size,
            SectionKind::InitialisedData => entry.data += size,
            SectionKind::ZeroedData => entry.bss += size,
        }
        entry.total += size;
    }

    let mut crates: Vec<CrateSize> = totals.into_values().collect();
    crates.sort_by_key(|c| Reverse(c.total));
    (crates, unattributed)
}

/// The file name a C, C++ or assembly source goes by in the report —
/// `main.c`, `vendor.cpp` — or `None` for anything else an `STT_FILE`
/// symbol names, Rust's codegen units among them.
fn foreign_source(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let extension = base.rsplit_once('.')?.1;
    ["c", "cc", "cpp", "cxx", "s", "S"]
        .contains(&extension)
        .then(|| base.to_string())
}

/// The address ranges of every compile unit that is not Rust, by the file
/// it compiled, from the image's DWARF — empty for an image without it.
fn foreign_units(file: &object::File<'_>) -> Vec<(u64, u64, String)> {
    let endian = if file.is_little_endian() {
        gimli::RunTimeEndian::Little
    } else {
        gimli::RunTimeEndian::Big
    };
    let load =
        |id: gimli::SectionId| -> std::result::Result<std::borrow::Cow<'_, [u8]>, gimli::Error> {
            Ok(file
                .section_by_name(id.name())
                .and_then(|section| section.uncompressed_data().ok())
                .unwrap_or(std::borrow::Cow::Borrowed(&[])))
        };
    let Ok(sections) = gimli::DwarfSections::load(load) else {
        return Vec::new();
    };
    let dwarf = sections.borrow(|section| gimli::EndianSlice::new(section, endian));

    let mut out = Vec::new();
    let mut headers = dwarf.units();
    while let Ok(Some(header)) = headers.next() {
        let Ok(unit) = dwarf.unit(header) else {
            continue;
        };
        let source = {
            let mut entries = unit.entries();
            let Ok(Some(root)) = entries.next_dfs() else {
                continue;
            };
            if root.attr_value(gimli::DW_AT_language)
                == Some(gimli::AttributeValue::Language(gimli::DW_LANG_Rust))
            {
                continue;
            }
            let Some(name) = root
                .attr_value(gimli::DW_AT_name)
                .and_then(|value| dwarf.attr_string(&unit, value).ok())
            else {
                continue;
            };
            let name = name.to_string_lossy().into_owned();
            name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string()
        };
        let Ok(mut ranges) = dwarf.unit_ranges(&unit) else {
            continue;
        };
        while let Ok(Some(range)) = ranges.next() {
            if range.end > range.begin {
                out.push((range.begin, range.end, source.clone()));
            }
        }
    }
    out
}

/// The crate a mangled Rust symbol came from.
///
/// `rustc-demangle` handles both the legacy `_ZN` scheme and v0 `_R`. What
/// comes back is a path like `core::fmt::write`, whose first segment is the
/// crate. Symbols that do not demangle are C or assembly and get no crate.
fn crate_of(symbol: &str) -> Option<String> {
    if let Some(krate) = v0_crate(symbol) {
        return Some(krate);
    }
    // The alternate form drops the hash: v0 symbols otherwise name the crate
    // as `embassy_stm32[26b6afaf38d9ac34]`, which is no name anybody wrote.
    let demangled = format!("{:#}", rustc_demangle::try_demangle(symbol).ok()?);

    // Strip the trailing hash the legacy scheme appends, e.g.
    // `core::fmt::write::h9f3a...`, before splitting.
    let path = demangled
        .split_once('<')
        .map_or(demangled.as_str(), |(head, _)| head);
    let first = path.split("::").next()?.trim();

    if first.is_empty() || first.contains(' ') {
        return None;
    }
    // Generic instantiations can begin with a type rather than a crate; those
    // start with a sigil that a crate name never does.
    if first.starts_with(['&', '*', '[', '(']) {
        return None;
    }
    Some(first.to_string())
}

/// The crate a v0-mangled symbol's item is defined in, read off the mangled
/// name itself.
///
/// Demangled, an impl's method reads `<u64 as core::fmt::Display>::fmt` or
/// `<embassy_executor::raw::Executor>::spawn`: the first names no crate at
/// all, and both begin with `<`, so a reading of the demangled text left
/// every method of every impl unattributed — most of a firmware's code. The
/// mangling says where the impl *is* (`X`/`M` carry the impl's own path
/// before its type), and every path is written outermost tag first, so the
/// first crate root reached is the defining crate.
fn v0_crate(symbol: &str) -> Option<String> {
    let mut rest = symbol.strip_prefix("_R")?.as_bytes();
    // An optional encoding version, decimal.
    while rest.first()?.is_ascii_digit() {
        rest = &rest[1..];
    }
    fn skip_disambiguator(rest: &[u8]) -> Option<&[u8]> {
        match rest.strip_prefix(b"s") {
            Some(after) => Some(&after[after.iter().position(|&b| b == b'_')? + 1..]),
            None => Some(rest),
        }
    }
    loop {
        let (&tag, after) = rest.split_first()?;
        rest = match tag {
            // A nested path: its namespace, then the path it is nested in.
            b'N' => after.get(1..)?,
            // An inherent or trait impl: the impl's own path comes first.
            b'M' | b'X' => skip_disambiguator(after)?,
            // A generic instantiation of a path.
            b'I' => after,
            b'C' => {
                let after = skip_disambiguator(after)?;
                // A punycode name is no crate name cargo would accept.
                let digits = after.iter().take_while(|b| b.is_ascii_digit()).count();
                let len: usize = std::str::from_utf8(&after[..digits]).ok()?.parse().ok()?;
                let mut name = &after[digits..];
                if name.first() == Some(&b'_') {
                    name = &name[1..];
                }
                return std::str::from_utf8(name.get(..len)?)
                    .ok()
                    .map(str::to_string);
            }
            // A back-reference or a type at the head (`Y`): the demangled
            // reading decides.
            _ => return None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impls_methods_belong_to_the_crate_the_impl_is_in() {
        // `<u64 as core::fmt::Display>::fmt`, demangled: no crate in it.
        assert_eq!(
            crate_of("_RNvXs8_NtNtNtCs3j8ABkEzctN_4core3fmt3num3impmNtB9_7Display3fmt").as_deref(),
            Some("core")
        );
        // `<embassy_executor::raw::SyncExecutor>::spawn`.
        assert_eq!(
            crate_of("_RNvMs7_NtCsf1M66eZtxQZ_16embassy_executor3rawNtB5_12SyncExecutor5spawn")
                .as_deref(),
            Some("embassy_executor")
        );
        // A task's poll, generic over a closure in the binary, with LLVM's
        // suffix: the executor's code, instantiated.
        assert_eq!(
            crate_of(
                "_RNvMs1_NtCsf1M66eZtxQZ_16embassy_executor3rawINtB5_11TaskStorageNCNvNvCsNkKeOc8Q0S_7withcpp21_____embassy_main_task36_____embassy_main_task_inner_function0E4pollB16_.llvm.9605919393304200671"
            )
            .as_deref(),
            Some("embassy_executor")
        );
        // A plain function, with no hash in the name.
        assert_eq!(
            crate_of("_RNvCs3k4rxGXqJFM_13embassy_stm324init").as_deref(),
            Some("embassy_stm32")
        );
    }

    #[test]
    fn a_c_source_is_named_by_its_file_and_nothing_else_is() {
        assert_eq!(
            foreign_source("csrc/vendor.cpp").as_deref(),
            Some("vendor.cpp")
        );
        assert_eq!(
            foreign_source(r"C:\sdk\startup.S").as_deref(),
            Some("startup.S")
        );
        assert_eq!(foreign_source("main.c").as_deref(), Some("main.c"));
        // A Rust codegen unit, and a name with no extension.
        assert_eq!(foreign_source("embassy_stm32.a1b2c3-cgu.0"), None);
        assert_eq!(foreign_source("crtstuff"), None);
    }

    #[test]
    fn crate_names_come_off_the_front_of_a_demangled_path() {
        // Legacy scheme, with the trailing hash rustc appends.
        assert_eq!(
            crate_of("_ZN4core3fmt5write17h9f3a2b1c4d5e6f70E").as_deref(),
            Some("core")
        );
        assert_eq!(
            crate_of("_ZN7esp_hal4gpio5Input3new17habcdef0123456789E").as_deref(),
            Some("esp_hal")
        );
    }

    #[test]
    fn c_and_assembly_symbols_are_left_unattributed() {
        // Attributing these to a made-up crate would silently distort the
        // per-crate totals, which is the whole point of the panel.
        for symbol in ["memcpy", "esp_rom_printf", "__udivdi3", ""] {
            assert_eq!(crate_of(symbol), None, "{symbol} should not attribute");
        }
    }

    #[test]
    fn initialised_data_is_counted_against_both_budgets() {
        // A `static mut FOO: [u8; 1024] = [1; 1024]` costs 1 KB of flash to
        // store the initialiser *and* 1 KB of RAM to live in. Counting it once
        // in either direction understates the real cost, and this is the case
        // people are surprised by.
        assert_eq!(SectionKind::InitialisedData.budget(), (true, true));

        assert_eq!(SectionKind::Code.budget(), (true, false));
        assert_eq!(SectionKind::ReadOnlyData.budget(), (true, false));
        // .bss is zeroed by startup code, so nothing is stored for it.
        assert_eq!(SectionKind::ZeroedData.budget(), (false, true));
    }

    #[test]
    fn ram_fraction_needs_a_known_chip() {
        let unknown = MemoryTotals {
            flash_bytes: 100_000,
            ram_bytes: 40_000,
            ram_capacity: None,
            flash_capacity: None,
        };
        assert!(unknown.ram_fraction().is_none());
        assert!(
            unknown.flash_fraction().is_none(),
            "external flash has no wall"
        );

        let c3 = MemoryTotals {
            ram_capacity: Some(400 * 1024),
            ..unknown
        };
        let fraction = c3.ram_fraction().unwrap();
        assert!((fraction - 0.0977).abs() < 0.001, "{fraction}");

        let ch32 = MemoryTotals {
            flash_bytes: 5_222,
            flash_capacity: Some(16 * 1024),
            ..unknown
        };
        let flash = ch32.flash_fraction().unwrap();
        assert!((flash - 0.3187).abs() < 0.001, "{flash}");
    }
}
