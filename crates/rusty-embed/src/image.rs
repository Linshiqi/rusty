//! A firmware image as a bootloader takes it, made from the ELF a build
//! leaves.
//!
//! Two bootloaders need something other than an ELF. A Raspberry Pi RP2040
//! or RP2350 held in BOOTSEL appears as a USB drive and takes a **UF2** file
//! copied onto it; an STM32 in its system bootloader speaks **USB DFU** and
//! takes a raw binary through `dfu-util`. Both are the bytes the ELF stores in
//! flash, at the addresses it stores them at — each `PT_LOAD` segment's
//! *physical* address, which is where `.data`'s initialiser lives rather than
//! the RAM it is copied to.
//!
//! The UF2 rules are picotool's, read off its source (`elf2uf2/elf2uf2.cpp`)
//! rather than the format's documentation, because the boot ROM is what
//! decides and picotool is what the boot ROM has been tested against:
//!
//! - payloads are whole 256-byte pages, zero-filled around what the ELF
//!   stores, each block carrying the family ID the ROM checks;
//! - every 4 KB erase sector the image touches is filled out with empty pages
//!   up to the image's last page, because the ROM works out which sector to
//!   erase from the block's *number*;
//! - nothing is added for the RP2350's erratum E10 — picotool adds its
//!   absolute block only when asked (`--abs-block`), and a device with no
//!   partition table does not need it.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use object::read::elf::{ElfFile32, ProgramHeader};

/// One stretch of bytes the image stores, at the address it stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub address: u32,
    pub bytes: Vec<u8>,
}

/// The bytes a 32-bit ELF puts in memory from the file, at their load
/// addresses: every `PT_LOAD` with something in the file. Zero-initialised
/// memory — `.bss` — has nothing to write and is left out, as picotool
/// leaves it.
pub fn loaded(elf: &[u8]) -> Result<Vec<Segment>, String> {
    let file = ElfFile32::<object::Endianness>::parse(elf)
        .map_err(|e| format!("not a 32-bit ELF image ({e})"))?;
    let endian = file.endian();
    let mut out = Vec::new();
    for header in file.elf_program_headers() {
        if header.p_type(endian) != object::elf::PT_LOAD || header.p_filesz(endian) == 0 {
            continue;
        }
        let bytes = header
            .data(endian, elf)
            .map_err(|()| "a segment runs past the end of the file".to_string())?;
        out.push(Segment {
            address: header.p_paddr(endian),
            bytes: bytes.to_vec(),
        });
    }
    if out.is_empty() {
        return Err("the image stores nothing to write".to_string());
    }
    out.sort_by_key(|segment| segment.address);
    Ok(out)
}

/// Only what lies in `window` — the flash a bootloader writes — refusing a
/// segment that stores bytes anywhere else: written at the wrong address it
/// would be a board that does not start, and said nowhere.
fn in_window(segments: &[Segment], window: &Range<u32>) -> Result<(), String> {
    for segment in segments {
        let end = segment.address as u64 + segment.bytes.len() as u64;
        if segment.address < window.start || end > window.end as u64 {
            return Err(format!(
                "the image stores {} bytes at 0x{:08x}, outside the flash this bootloader \
                 writes (0x{:08x}..0x{:08x})",
                segment.bytes.len(),
                segment.address,
                window.start,
                window.end
            ));
        }
    }
    Ok(())
}

const UF2_MAGIC_START0: u32 = 0x0A32_4655;
const UF2_MAGIC_START1: u32 = 0x9E5D_5157;
const UF2_MAGIC_END: u32 = 0x0AB1_6F30;
const UF2_FLAG_FAMILY_ID_PRESENT: u32 = 0x0000_2000;
const UF2_PAGE: u32 = 256;
const FLASH_SECTOR: u32 = 4096;

/// The segments as a UF2 file for a boot ROM that takes `family`, every byte
/// inside `window` — picotool's layout, block for block.
pub fn uf2(segments: &[Segment], family: u32, window: Range<u32>) -> Result<Vec<u8>, String> {
    in_window(segments, &window)?;
    let mut pages: BTreeMap<u32, [u8; UF2_PAGE as usize]> = BTreeMap::new();
    for segment in segments {
        for (offset, byte) in segment.bytes.iter().enumerate() {
            let address = segment.address + offset as u32;
            let page = address & !(UF2_PAGE - 1);
            pages.entry(page).or_insert([0; UF2_PAGE as usize])[(address - page) as usize] = *byte;
        }
    }
    let Some(&last) = pages.keys().next_back() else {
        return Err("the image stores nothing to write".to_string());
    };
    // Every sector touched, filled with empty pages up to the last one.
    let sectors: BTreeSet<u32> = pages.keys().map(|page| page / FLASH_SECTOR).collect();
    for sector in sectors {
        let mut page = sector * FLASH_SECTOR;
        while page < (sector + 1) * FLASH_SECTOR && page < last {
            pages.entry(page).or_insert([0; UF2_PAGE as usize]);
            page += UF2_PAGE;
        }
    }

    let count = pages.len() as u32;
    let mut out = Vec::with_capacity(pages.len() * 512);
    for (number, (address, data)) in pages.iter().enumerate() {
        for word in [
            UF2_MAGIC_START0,
            UF2_MAGIC_START1,
            UF2_FLAG_FAMILY_ID_PRESENT,
            *address,
            UF2_PAGE,
            number as u32,
            count,
            family,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 476 - UF2_PAGE as usize]);
        out.extend_from_slice(&UF2_MAGIC_END.to_le_bytes());
    }
    Ok(out)
}

/// The segments as one raw binary starting at `window.start`, gaps filled
/// with 0xFF — erased flash — as `objcopy -O binary` lays it out and `dfu-util
/// -s <address>` writes it.
pub fn bin(segments: &[Segment], window: Range<u32>) -> Result<Vec<u8>, String> {
    in_window(segments, &window)?;
    let end = segments
        .iter()
        .map(|segment| segment.address + segment.bytes.len() as u32)
        .max()
        .ok_or_else(|| "the image stores nothing to write".to_string())?;
    let mut out = vec![0xFF; (end - window.start) as usize];
    for segment in segments {
        let at = (segment.address - window.start) as usize;
        out[at..at + segment.bytes.len()].copy_from_slice(&segment.bytes);
    }
    Ok(out)
}

/// A UF2 file read back into the pages it writes and the family it names —
/// for the tests, and for anybody checking what was copied.
pub fn read_uf2(file: &[u8]) -> Result<(u32, Vec<Segment>), String> {
    if !file.len().is_multiple_of(512) {
        return Err("a UF2 file is whole 512-byte blocks".to_string());
    }
    let word = |block: &[u8], at: usize| u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
    let mut family = None;
    let mut pages = Vec::new();
    for (index, block) in file.chunks(512).enumerate() {
        if word(block, 0) != UF2_MAGIC_START0
            || word(block, 4) != UF2_MAGIC_START1
            || word(block, 508) != UF2_MAGIC_END
        {
            return Err(format!("block {index} is not a UF2 block"));
        }
        if word(block, 20) != index as u32 {
            return Err(format!("block {index} calls itself {}", word(block, 20)));
        }
        family = Some(word(block, 28));
        let size = word(block, 16) as usize;
        pages.push(Segment {
            address: word(block, 12),
            bytes: block[32..32 + size].to_vec(),
        });
    }
    Ok((family.unwrap_or_default(), pages))
}

/// A little-endian ELF32 with one `PT_LOAD` per `(paddr, vaddr, bytes,
/// memsz)` — what a linker writes, reduced to what [`loaded`] reads.
#[cfg(test)]
pub(crate) fn test_elf(segments: &[(u32, u32, &[u8], u32)]) -> Vec<u8> {
    tests::elf(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn elf(segments: &[(u32, u32, &[u8], u32)]) -> Vec<u8> {
        let headers = 52 + 32 * segments.len();
        let mut out = vec![0u8; 52];
        out[..4].copy_from_slice(b"\x7fELF");
        out[4] = 1; // 32-bit
        out[5] = 1; // little-endian
        out[6] = 1;
        out[16..18].copy_from_slice(&2u16.to_le_bytes()); // executable
        out[18..20].copy_from_slice(&40u16.to_le_bytes()); // Arm
        out[20..24].copy_from_slice(&1u32.to_le_bytes());
        out[28..32].copy_from_slice(&52u32.to_le_bytes()); // e_phoff
        out[40..42].copy_from_slice(&52u16.to_le_bytes()); // e_ehsize
        out[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
        out[44..46].copy_from_slice(&(segments.len() as u16).to_le_bytes());
        let mut offset = headers as u32;
        for &(paddr, vaddr, bytes, memsz) in segments {
            for word in [1, offset, vaddr, paddr, bytes.len() as u32, memsz, 5, 4] {
                out.extend_from_slice(&u32::to_le_bytes(word));
            }
            offset += bytes.len() as u32;
        }
        for &(_, _, bytes, _) in segments {
            out.extend_from_slice(bytes);
        }
        out
    }

    const RP2040: u32 = 0xe48b_ff56;
    const XIP: Range<u32> = 0x1000_0000..0x1100_0000;

    /// The load address, not the run address: `.data` runs in RAM and is
    /// stored in flash, and the flash copy is what a bootloader writes.
    /// `.bss` stores nothing and is not written at all.
    #[test]
    fn what_is_written_is_what_the_elf_stores_where_it_stores_it() {
        let image = elf(&[
            (0x1000_0000, 0x1000_0000, &[1, 2, 3, 4], 4),
            (0x1000_0400, 0x2000_0000, &[9, 9], 2),
            (0x2000_0100, 0x2000_0100, &[], 64),
        ]);
        let segments = loaded(&image).unwrap();
        assert_eq!(
            segments,
            [
                Segment {
                    address: 0x1000_0000,
                    bytes: vec![1, 2, 3, 4]
                },
                Segment {
                    address: 0x1000_0400,
                    bytes: vec![9, 9]
                },
            ]
        );
    }

    /// picotool's blocks: magic, the family flag, whole zero-filled 256-byte
    /// pages numbered in order, and the empty pages that fill the touched
    /// sector up to the last page — the boot ROM erases by block number.
    #[test]
    fn a_uf2_is_picotools_block_for_block() {
        let image = elf(&[
            (0x1000_0000, 0x1000_0000, &[0xAA; 300], 300),
            (0x1000_0800, 0x1000_0800, &[0xBB; 4], 4),
        ]);
        let file = uf2(&loaded(&image).unwrap(), RP2040, XIP).unwrap();
        // 0x000 and 0x100 hold the first segment, 0x200..0x700 fill the
        // sector, 0x800 holds the second: nine blocks.
        assert_eq!(file.len(), 9 * 512);
        let (family, pages) = read_uf2(&file).unwrap();
        assert_eq!(family, RP2040);
        let addresses: Vec<u32> = pages.iter().map(|p| p.address).collect();
        assert_eq!(
            addresses,
            (0..=8).map(|n| 0x1000_0000 + n * 0x100).collect::<Vec<_>>()
        );
        assert!(pages.iter().all(|p| p.bytes.len() == 256));
        assert_eq!(&pages[1].bytes[..44], &[0xAA; 44][..]);
        assert!(
            pages[1].bytes[44..].iter().all(|&b| b == 0),
            "zero-filled, as picotool"
        );
        assert!(
            pages[4].bytes.iter().all(|&b| b == 0),
            "a padding page is empty"
        );
        // Every block names the same count and the family flag.
        for block in file.chunks(512) {
            assert_eq!(&block[8..12], &0x2000u32.to_le_bytes());
            assert_eq!(&block[24..28], &9u32.to_le_bytes());
        }
    }

    /// No padding past the last page: picotool chooses not to make every
    /// image's last sector whole.
    #[test]
    fn the_last_sector_is_not_padded_past_the_image() {
        let image = elf(&[(0x1000_0000, 0x1000_0000, &[1; 10], 10)]);
        let file = uf2(&loaded(&image).unwrap(), RP2040, XIP).unwrap();
        assert_eq!(file.len(), 512);
    }

    /// Bytes stored outside the flash the bootloader writes are refused by
    /// address rather than written somewhere they do not belong.
    #[test]
    fn a_segment_outside_flash_is_refused_by_its_address() {
        let image = elf(&[(0x2000_0000, 0x2000_0000, &[1; 8], 8)]);
        let error = uf2(&loaded(&image).unwrap(), RP2040, XIP).unwrap_err();
        assert!(error.contains("0x20000000"), "{error}");
    }

    /// A DFU binary starts where `dfu-util -s` writes it, and the gaps are
    /// erased flash.
    #[test]
    fn a_binary_starts_at_the_flash_and_fills_gaps_as_erased() {
        let image = elf(&[
            (0x0800_0000, 0x0800_0000, &[1, 2], 2),
            (0x0800_0010, 0x2000_0000, &[3], 1),
        ]);
        let out = bin(&loaded(&image).unwrap(), 0x0800_0000..0x0808_0000).unwrap();
        assert_eq!(out.len(), 0x11);
        assert_eq!(&out[..2], &[1, 2]);
        assert!(out[2..0x10].iter().all(|&b| b == 0xFF));
        assert_eq!(out[0x10], 3);
    }

    #[test]
    fn something_that_is_not_an_elf_says_so() {
        assert!(loaded(b"not an elf at all").unwrap_err().contains("ELF"));
    }
}
