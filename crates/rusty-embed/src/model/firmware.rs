//! Built firmware.

use serde::{Deserialize, Serialize};

/// An ELF found in the project's target directory.
///
/// Every screen that does anything with a device needs a path to a binary, and
/// until this existed each one had to be handed one. That made the memory panel
/// a file picker and left the assistant's `memory_report` tool unreachable —
/// it could only run *after* a human had already browsed to the file.
///
/// Discovered rather than constructed. `target/<triple>/release/<crate>` is
/// predictable enough to be tempting, and wrong often enough — renamed binaries,
/// a `[[bin]]` section, a custom `CARGO_TARGET_DIR` — that guessing it would
/// produce a file-not-found where the honest answer is "you have not built yet".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Firmware {
    pub path: String,
    /// File stem, which for a normal project is the crate name.
    pub name: String,
    /// `debug` or `release`.
    pub profile: String,
    /// Target triple, taken from the directory rather than from the ELF header:
    /// it is what cargo actually built for, which is the thing that has to match.
    pub target: String,
    pub bytes: u64,
    /// Seconds since the Unix epoch, when the filesystem reports it.
    pub modified: Option<u64>,
    /// Whether this was built for the triple the project is configured to use.
    ///
    /// A stale binary from a previous chip is the classic embedded trap: it
    /// flashes, it runs, and it behaves like a hardware fault.
    pub matches_configured_target: bool,
}
