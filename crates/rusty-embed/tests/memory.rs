//! Memory analysis, checked against a synthetic ELF with known contents.
//!
//! Building a real firmware image in a test would need a cross toolchain, so
//! instead this writes an ELF whose section sizes and symbol names are chosen
//! by hand. That still exercises the parts that can actually be wrong: reading
//! the section header flags, deciding what costs flash versus RAM, and mapping
//! mangled symbols back to crates.

use std::fs;

use object::{
    Architecture, BinaryFormat, Endianness, SectionKind, SymbolFlags, SymbolKind, SymbolScope,
    write::{Object, Symbol, SymbolSection},
};
use rusty_embed::{memory, model::SectionKindDto};

const CODE_BYTES: usize = 4096;
const RODATA_BYTES: usize = 1024;
const DATA_BYTES: usize = 256;
const BSS_BYTES: u64 = 8192;

/// An ELF with one section of each kind and symbols from two crates.
fn synthetic_elf() -> Vec<u8> {
    let mut obj = Object::new(BinaryFormat::Elf, Architecture::Riscv32, Endianness::Little);

    let text = obj.add_section(Vec::new(), b".text".to_vec(), SectionKind::Text);
    obj.append_section_data(text, &vec![0x13; CODE_BYTES], 4);

    let rodata = obj.add_section(Vec::new(), b".rodata".to_vec(), SectionKind::ReadOnlyData);
    obj.append_section_data(rodata, &vec![0xAA; RODATA_BYTES], 4);

    let data = obj.add_section(Vec::new(), b".data".to_vec(), SectionKind::Data);
    obj.append_section_data(data, &vec![0x01; DATA_BYTES], 4);

    let bss = obj.add_section(
        Vec::new(),
        b".bss".to_vec(),
        SectionKind::UninitializedData,
    );
    obj.append_section_bss(bss, BSS_BYTES, 4);

    // Debug info is in the file but never loaded; if it leaks into the totals
    // every figure on the panel is wrong by however large the debug info is.
    let debug = obj.add_section(Vec::new(), b".debug_info".to_vec(), SectionKind::Debug);
    obj.append_section_data(debug, &vec![0xFF; 100_000], 1);

    let mut symbol = |name: &[u8], size: u64, value: u64, section, kind| {
        obj.add_symbol(Symbol {
            name: name.to_vec(),
            value,
            size,
            kind,
            scope: SymbolScope::Compilation,
            weak: false,
            section: SymbolSection::Section(section),
            flags: SymbolFlags::None,
        });
    };

    // esp_hal: 2 KB of code plus 256 B of constants.
    symbol(b"_ZN7esp_hal4gpio5Input3new17h0000000000000001E", 2048, 0, text, SymbolKind::Text);
    symbol(b"_ZN7esp_hal4gpio9PIN_TABLE17h0000000000000002E", 256, 0, rodata, SymbolKind::Data);
    // core: 1 KB of code.
    symbol(b"_ZN4core3fmt5write17h0000000000000003E", 1024, 2048, text, SymbolKind::Text);
    // A C symbol from the ROM — must not be attributed to any crate.
    symbol(b"esp_rom_printf", 512, 3072, text, SymbolKind::Text);
    // A buffer in .bss, which costs RAM and no flash.
    // The legacy mangling encodes each path segment's byte length, so
    // `RX_BUFFER` must be prefixed with 9 — get it wrong and the symbol simply
    // fails to demangle and silently lands in the unattributed bucket.
    symbol(b"_ZN7esp_hal3dma9RX_BUFFER17h0000000000000004E", 4096, 0, bss, SymbolKind::Data);

    obj.write().expect("write synthetic ELF")
}

#[test]
fn sections_are_classified_from_their_header_flags() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("firmware.elf");
    fs::write(&path, synthetic_elf()).unwrap();

    let report = memory::analyze(&path, Some("esp32c3")).expect("analysis");

    let kind_of = |name: &str| {
        report
            .sections
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {:?}", report.sections))
            .kind
    };
    assert_eq!(kind_of(".text"), SectionKindDto::Code);
    assert_eq!(kind_of(".rodata"), SectionKindDto::ReadOnlyData);
    assert_eq!(kind_of(".data"), SectionKindDto::InitialisedData);
    assert_eq!(kind_of(".bss"), SectionKindDto::ZeroedData);

    assert!(
        !report.sections.iter().any(|s| s.name.starts_with(".debug")),
        "debug sections are not loaded and must not be counted"
    );
}

#[test]
fn flash_and_ram_totals_follow_the_section_budgets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("firmware.elf");
    fs::write(&path, synthetic_elf()).unwrap();

    let report = memory::analyze(&path, Some("esp32c3")).unwrap();

    // Flash stores code, constants, and the initialiser for .data.
    let expected_flash = (CODE_BYTES + RODATA_BYTES + DATA_BYTES) as u64;
    assert_eq!(report.totals.flash_bytes, expected_flash);

    // RAM holds .data (copied there at startup) and .bss.
    assert_eq!(report.totals.ram_bytes, DATA_BYTES as u64 + BSS_BYTES);

    // The chip was named, so capacity and a fraction are available.
    assert_eq!(report.totals.ram_capacity, Some(400 * 1024));
    assert!(report.totals.ram_fraction().unwrap() > 0.0);
}

#[test]
fn an_unknown_chip_leaves_capacity_unset_rather_than_guessing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("firmware.elf");
    fs::write(&path, synthetic_elf()).unwrap();

    let report = memory::analyze(&path, None).unwrap();
    assert_eq!(report.totals.ram_capacity, None);
    assert!(report.totals.ram_fraction().is_none());
    // Sizes are still real; only the comparison is missing.
    assert!(report.totals.flash_bytes > 0);
}

#[test]
fn bytes_are_attributed_to_the_crate_that_emitted_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("firmware.elf");
    fs::write(&path, synthetic_elf()).unwrap();

    let report = memory::analyze(&path, Some("esp32c3")).unwrap();
    let find = |name: &str| {
        report
            .crates
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("{name} missing from {:?}", report.crates))
    };

    let esp_hal = find("esp_hal");
    assert_eq!(esp_hal.code, 2048);
    assert_eq!(esp_hal.read_only_data, 256);
    assert_eq!(esp_hal.bss, 4096);
    assert_eq!(esp_hal.total, 2048 + 256 + 4096);

    assert_eq!(find("core").code, 1024);

    // Largest first, so the panel does not have to sort.
    assert_eq!(report.crates[0].name, "esp_hal");

    // The ROM symbol has no crate; spreading it across the others would make
    // every per-crate number quietly wrong.
    assert_eq!(report.unattributed_bytes, 512);
    assert!(!report.crates.iter().any(|c| c.name == "esp_rom_printf"));
}

#[test]
fn a_missing_or_unreadable_file_says_to_build_first() {
    let dir = tempfile::tempdir().unwrap();

    let missing = dir.path().join("nope.elf");
    let err = memory::analyze(&missing, None).unwrap_err().to_string();
    assert!(err.contains("nope.elf"), "{err}");

    let garbage = dir.path().join("garbage.elf");
    fs::write(&garbage, b"this is not an ELF").unwrap();
    let err = memory::analyze(&garbage, None).unwrap_err().to_string();
    assert!(err.contains("build the project first"), "{err}");
}
