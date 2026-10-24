//! The embedded workbench's data types.
//!
//! Like `rusty_core::model`, free of IO: what the window, the CLI and the
//! assistant's tools all read.
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
//! window and the CLI, and tying them together means a UI refactor silently
//! breaking everybody's files (rule 2).

mod command;
mod device;
mod firmware;
mod memory;
mod part;
mod playground;
mod project;
mod registers;
mod repo;
mod sheet;
mod sim;
mod symbol;
mod toolchain;
mod wizard;
mod workbench;

pub use command::{CommandPlan, LogLevel, LogLine, LogStream};
pub use device::{
    BootDevice, BootKind, FlashAction, Probe, SerialPort, Transport, UsbIdentity, boot_transport,
    only_candidate, serial_capable, usable_boot,
};
pub use firmware::Firmware;
pub use memory::{CrateSize, MemoryReport, MemoryTotals, SectionKind, SectionSize};
pub use part::{
    Arch, Board, CCompiler, CatalogProblem, CatalogSource, Chip, DfuBoot, Emulation, EmulatorKind,
    Flasher, Generator, Kit, KitUsb, PinAssignment, Ports, Runtime, ToolchainRequirement, Uf2Boot,
    UsbMatch, Vendor,
};
pub use playground::{
    PLAYGROUND_DRAW, PLAYGROUND_MAIN, PLAYGROUNDS, playground_has_board, playground_main,
};
pub use project::{
    BuildSetup, BuildSystem, CInterop, CmakeSdk, Edit, EmbeddedProject, FileChange, Migration,
    PinClaim, PinInfo, PinReport, Problem, ScaffoldReport, Severity,
};
pub use registers::{Peripheral, Register, RegisterField, RegisterMap};
pub use repo::{REPO, REPO_ISSUES, REPO_RELEASES};
pub use sheet::{Instance, KIT_REFERENCE, PinRef, Sheet, Wire};
// Crate-private: only the `.rusty/sim.toml` writer needs it.
pub(crate) use sim::is_upright;
pub use sim::{Drive, Emulator, SimDebug, SimLimit, SimPlan, SimTool};
pub use symbol::{Fill, Graphic, MM_PX, Pin, PinKind, Symbol};
pub use toolchain::{
    ToolStatus, Toolchain, ToolchainReport, ToolchainStatus, cargo_loses_dependencies,
};
pub use wizard::{
    CrateNameProblem, Explanation, Language, WizardChoice, WizardLayout, WizardOption,
    crate_name_problem,
};
pub use workbench::{
    AssistantChoice, DockLayout, EditorView, GitLayout, HistoryLayout, PagePlace, ProjectTabs,
    RelocateReport, StorageLocation, TabPlace, UpdateProgress, WindowLayout,
};
