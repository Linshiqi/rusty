//! Wire types for the embedded workbench.
//!
//! Like `rusty_core::model`, this is compiled unconditionally and must stay
//! free of IO so the Leptos frontend can `use` these types directly.
//!
//! One module per concern, all re-exported here so every existing path —
//! `rusty_embed::model::Chip` and `rusty_embed::Chip` alike — keeps working.
//! It was one file of sixty types across seventeen concerns, which is a file
//! nobody can read and a boundary nothing enforces: the terminal's shell picker
//! had ended up beside the chip catalogue because there was nowhere else to
//! put it. A new type goes in the module it belongs to, or a new module gets
//! added — not on the end.
//!
//! What is *not* here, deliberately: the file formats. `catalog.rs` and
//! `config.rs` parse TOML into their own private structs and convert to these.
//! The file format is a public contract with users who write board definitions
//! and edit `workbench.toml`; these types are an internal contract with the
//! frontend, and tying them together means a UI refactor silently breaking
//! everybody's files (rule 2).

mod command;
mod device;
mod firmware;
mod memory;
mod part;
mod project;
mod registers;
mod repo;
mod sim;
mod toolchain;
mod wizard;
mod workbench;

pub use command::{CommandPlan, LogLevel, LogLine, LogStream};
pub use device::{FlashAction, Probe, SerialPort, Transport, UsbIdentity};
pub use firmware::Firmware;
pub use memory::{CrateSize, MemoryReport, MemoryTotals, SectionKind, SectionSize};
pub use part::{
    Arch, Board, CatalogProblem, CatalogSource, Chip, Flasher, PinAssignment, Runtime,
    ToolchainRequirement, UsbMatch, Vendor,
};
pub use project::{
    CInterop, Edit, EmbeddedProject, FileChange, Migration, PinClaim, PinInfo, PinReport, Problem,
    ScaffoldReport, Severity,
};
pub use registers::{Peripheral, Register, RegisterField, RegisterMap};
pub use repo::{RELEASES_API, REPO, REPO_ISSUES, REPO_RELEASES};
// Only the `.rusty/sim.toml` writer needs it, and that is backend-side; on
// wasm the re-export would be an unused import.
#[cfg(feature = "backend")]
pub(crate) use sim::is_upright;
pub use sim::{
    Drive, PartDef, Placement, SimAnalog, SimBoard, SimButton, SimDebug, SimDisplay, SimLed,
    SimMotor, SimPlan, SimPot, SimRgb, SimSeven, SimTool, UNWIRED_PIN,
};
pub use toolchain::{ToolStatus, Toolchain, ToolchainReport, ToolchainStatus};
pub use wizard::{Explanation, WizardChoice, WizardOption};
pub use workbench::{AssistantChoice, ProjectTabs, RelocateReport, StorageLocation, UpdateStatus};
