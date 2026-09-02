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
//! Attribution is best-effort by nature: assembly, C from ESP-IDF, and linker
//! fill have no crate. Those bytes are reported as unattributed rather than
//! spread across crates, because a number that is quietly wrong is worse than
//! one that is visibly incomplete.

use std::{cmp::Reverse, collections::BTreeMap, path::Path};

use object::{Object, ObjectSection, ObjectSymbol};

use crate::{
    chip,
    error::{Error, Result},
    model::{CrateSize, MemoryReport, MemoryTotals, SectionKind, SectionSize},
};

/// Analyse a linked ELF.
pub fn analyze(elf_path: &Path, chip_id: Option<&str>) -> Result<MemoryReport> {
    let bytes = std::fs::read(elf_path).map_err(|source| Error::Read {
        path: elf_path.display().to_string(),
        source,
    })?;
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

    for symbol in file.symbols() {
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
        let Some(krate) = crate_of(name) else {
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

/// The crate a mangled Rust symbol came from.
///
/// `rustc-demangle` handles both the legacy `_ZN` scheme and v0 `_R`. What
/// comes back is a path like `core::fmt::write`, whose first segment is the
/// crate. Symbols that do not demangle are C or assembly and get no crate.
fn crate_of(symbol: &str) -> Option<String> {
    let demangled = rustc_demangle::try_demangle(symbol).ok()?.to_string();

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

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        assert!(unknown.ram_fraction().is_none());

        let c3 = MemoryTotals {
            ram_capacity: Some(400 * 1024),
            ..unknown
        };
        let fraction = c3.ram_fraction().unwrap();
        assert!((fraction - 0.0977).abs() < 0.001, "{fraction}");
    }
}
