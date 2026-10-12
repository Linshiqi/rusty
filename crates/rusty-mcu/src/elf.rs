//! What a flasher writes, read out of the ELF the build left: every loadable
//! segment's file bytes at its *physical* address.
//!
//! The physical address, not the virtual one: qingke-rt links its vector
//! table and trap code to run from RAM at 0x2000_0000 and stores them in
//! flash after the reset jump, and its startup copies them across. Loading
//! by virtual address would put them in RAM already and leave a hole in
//! flash where the startup copies from — a program that works here and
//! nowhere else. wlink loads `p_paddr` too.

/// One run of bytes for one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub addr: u32,
    pub bytes: Vec<u8>,
}

/// An image, or why the file is not one this emulator can boot.
pub fn segments(file: &[u8]) -> Result<Vec<Segment>, String> {
    if file.len() < 52 || &file[..4] != b"\x7fELF" {
        return Err("not an ELF file".to_string());
    }
    if file[4] != 1 {
        return Err("a 64-bit ELF; the CH32V003 runs 32-bit code".to_string());
    }
    if file[5] != 1 {
        return Err("a big-endian ELF; RISC-V is little-endian".to_string());
    }
    let half = |at: usize| u16::from_le_bytes([file[at], file[at + 1]]);
    let word = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap_or([0; 4]));
    // EM_RISCV.
    if half(18) != 243 {
        return Err(format!(
            "an ELF for machine {}, not RISC-V — was this built for the right target?",
            half(18)
        ));
    }
    let phoff = word(28) as usize;
    let phentsize = usize::from(half(42));
    let phnum = usize::from(half(44));
    if phentsize < 32 || phoff + phentsize * phnum > file.len() {
        return Err("the ELF's program headers run past the end of the file".to_string());
    }
    let mut out = Vec::new();
    for i in 0..phnum {
        let at = phoff + i * phentsize;
        // PT_LOAD with something in the file; a .bss is zero by the startup.
        if word(at) != 1 {
            continue;
        }
        let offset = word(at + 4) as usize;
        let paddr = word(at + 12);
        let filesz = word(at + 16) as usize;
        if filesz == 0 {
            continue;
        }
        let bytes = file
            .get(offset..offset + filesz)
            .ok_or_else(|| format!("segment {i} names bytes past the end of the file"))?;
        out.push(Segment {
            addr: paddr,
            bytes: bytes.to_vec(),
        });
    }
    if out.is_empty() {
        return Err("the ELF has nothing to load".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest ELF that says something: one PT_LOAD of four bytes whose
    /// physical address differs from its virtual one.
    fn tiny(paddr: u32, vaddr: u32) -> Vec<u8> {
        let mut f = vec![0u8; 0x60];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 1;
        f[5] = 1;
        f[6] = 1;
        f[18..20].copy_from_slice(&243u16.to_le_bytes());
        f[28..32].copy_from_slice(&52u32.to_le_bytes());
        f[42..44].copy_from_slice(&32u16.to_le_bytes());
        f[44..46].copy_from_slice(&1u16.to_le_bytes());
        let ph = 52;
        f[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes());
        f[ph + 4..ph + 8].copy_from_slice(&0x54u32.to_le_bytes());
        f[ph + 8..ph + 12].copy_from_slice(&vaddr.to_le_bytes());
        f[ph + 12..ph + 16].copy_from_slice(&paddr.to_le_bytes());
        f[ph + 16..ph + 20].copy_from_slice(&4u32.to_le_bytes());
        f[0x54..0x58].copy_from_slice(&[1, 2, 3, 4]);
        f
    }

    #[test]
    fn a_segment_loads_at_its_physical_address() {
        let got = segments(&tiny(0x0000_0004, 0x2000_0000)).unwrap();
        assert_eq!(
            got,
            vec![Segment {
                addr: 4,
                bytes: vec![1, 2, 3, 4]
            }]
        );
    }

    #[test]
    fn a_file_for_another_machine_is_refused_by_name() {
        let mut file = tiny(0, 0);
        file[18] = 94; // Xtensa
        let refused = segments(&file).unwrap_err();
        assert!(refused.contains("machine 94"), "{refused}");
        assert!(segments(b"not an elf at all, just text that is long enough to read").is_err());
    }
}
