//! A chip's peripherals, as a register view needs them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterMap {
    pub peripherals: Vec<Peripheral>,
    /// How many peripherals, registers or fields the parse could not place —
    /// `derivedFrom` inheritance mostly, and anything whose address or width
    /// would not parse as a number. Shown rather than hidden: a panel silently
    /// missing GPIO2 is a panel that lies about the chip.
    pub dropped: u32,
    /// Why the map may be short of the file: set when the XML ended or broke
    /// before the document did, naming where. A truncated download is the
    /// usual cause, and a register view that showed half a chip without saying
    /// so would send somebody looking for a peripheral the parse never
    /// reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peripheral {
    pub name: String,
    pub description: String,
    pub base: u64,
    pub registers: Vec<Register>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Register {
    pub name: String,
    pub description: String,
    /// From the peripheral's base.
    pub offset: u32,
    pub bits: u32,
    /// False for write-only registers — reading one can wedge the
    /// peripheral, so the panel must not offer to.
    pub readable: bool,
    pub fields: Vec<RegisterField>,
}

/// Registers default to readable and 32 bits: most SVDs say neither, and
/// a register the panel refuses to read because a file omitted `access` is
/// a register the user cannot see.
impl Default for Register {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            offset: 0,
            bits: 32,
            readable: true,
            fields: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterField {
    pub name: String,
    pub description: String,
    /// Bit position of the field's least significant bit.
    pub offset: u32,
    pub width: u32,
}
