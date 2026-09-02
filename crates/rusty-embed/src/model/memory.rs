//! Where a firmware's bytes go.

use serde::{Deserialize, Serialize};

/// What a loaded ELF section costs the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionKind {
    /// Executable code. Lives in flash.
    Code,
    /// Constants and string literals. Lives in flash.
    ReadOnlyData,
    /// Mutable data with a non-zero initial value. Costs flash *and* RAM: the
    /// initialiser is stored in the image and copied into RAM at startup.
    InitialisedData,
    /// Mutable data that starts zeroed (`.bss`). Costs RAM only.
    ZeroedData,
}

impl SectionKind {
    /// Whether one byte of this kind occupies (flash, RAM).
    pub fn budget(self) -> (bool, bool) {
        match self {
            SectionKind::Code | SectionKind::ReadOnlyData => (true, false),
            SectionKind::InitialisedData => (true, true),
            SectionKind::ZeroedData => (false, true),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SectionKind::Code => "code",
            SectionKind::ReadOnlyData => "read-only",
            SectionKind::InitialisedData => "data",
            SectionKind::ZeroedData => "bss",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSize {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: SectionKind,
}

/// One crate's contribution to the image.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrateSize {
    /// Crate name as it appears in symbols, so underscores rather than hyphens.
    pub name: String,
    pub code: u64,
    pub read_only_data: u64,
    pub data: u64,
    pub bss: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTotals {
    /// Bytes stored in the flash image.
    pub flash_bytes: u64,
    /// Bytes resident in RAM once running.
    pub ram_bytes: u64,
    /// Nominal on-chip SRAM, when the part is known.
    ///
    /// Headline capacity, not what the linker will grant — some is reserved by
    /// the ROM bootloader and by the cache configuration. Treat a reading close
    /// to this number as trouble well before it reaches it.
    pub ram_capacity: Option<u32>,
}

impl MemoryTotals {
    /// Static RAM use as a fraction of nominal capacity.
    ///
    /// Static only: the stack and any heap grow on top of this at runtime,
    /// which is exactly why a number that looks comfortable here can still
    /// overflow in the field.
    pub fn ram_fraction(&self) -> Option<f32> {
        let capacity = self.ram_capacity?;
        (capacity > 0).then(|| self.ram_bytes as f32 / capacity as f32)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryReport {
    pub elf_path: String,
    pub chip: Option<String>,
    /// Loaded sections, largest first.
    pub sections: Vec<SectionSize>,
    pub totals: MemoryTotals,
    /// Per-crate attribution, largest first.
    pub crates: Vec<CrateSize>,
    /// Bytes belonging to symbols with no identifiable crate — assembly, C from
    /// ESP-IDF, ROM stubs. Reported separately rather than distributed, so the
    /// per-crate figures stay honest.
    pub unattributed_bytes: u64,
}
