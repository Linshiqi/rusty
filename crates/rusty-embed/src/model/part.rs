//! Chips and boards, as the frontend renders them.
//!
//! The catalogue *files* are a different set of types in `catalog.rs`; these
//! are what comes out of it.

use serde::{Deserialize, Serialize};

/// Who makes the part.
///
/// Present from the start even though only Espressif is populated: vendor is
/// what decides *how* to detect a chip, which toolchain to demand, and which
/// flasher to reach for. Threading it through later would mean touching every
/// one of those paths at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vendor {
    Espressif,
    St,
}

impl Vendor {
    pub fn label(self) -> &'static str {
        match self {
            Vendor::Espressif => "Espressif",
            Vendor::St => "STMicroelectronics",
        }
    }

    /// Crates that carry the part number as a cargo feature, most
    /// authoritative first.
    ///
    /// This is the main thing that differs per vendor: `esp-hal` names the chip
    /// `esp32c3`, while `embassy-stm32` names it `stm32f411ce`. Detection reads
    /// the same shape from both, but has to know where to look.
    pub fn chip_feature_crates(self) -> &'static [&'static str] {
        match self {
            Vendor::Espressif => &["esp-hal", "esp-idf-svc", "esp-idf-hal", "esp-wifi"],
            Vendor::St => &[
                "embassy-stm32",
                "stm32f4xx-hal",
                "stm32f1xx-hal",
                "stm32h7xx-hal",
            ],
        }
    }
}

/// Instruction set the chip's main cores run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Arch {
    Xtensa,
    RiscV,
    CortexM,
}

impl Arch {
    pub fn label(self) -> &'static str {
        match self {
            Arch::Xtensa => "Xtensa",
            Arch::RiscV => "RISC-V",
            Arch::CortexM => "Arm Cortex-M",
        }
    }
}

/// What has to be installed before this part can be built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolchainRequirement {
    /// A stock rustup toolchain plus `rustup target add`.
    Stock,
    /// Espressif's forked LLVM, installed by espup as the `esp` toolchain.
    ///
    /// Upstream rustc cannot emit Xtensa code at all, and the error it gives
    /// says nothing about espup — which is why this is modelled rather than
    /// inferred from the triple at each call site.
    EspXtensa,
}

impl ToolchainRequirement {
    pub fn install_command(self) -> Option<&'static str> {
        match self {
            ToolchainRequirement::Stock => None,
            ToolchainRequirement::EspXtensa => Some("espup install"),
        }
    }
}

/// A tool that can put a binary on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Flasher {
    /// Espressif's serial flasher. Needs only the USB cable.
    Espflash,
    /// Flashes and debugs through a JTAG/SWD probe, and decodes defmt over RTT.
    /// The only option for parts with no serial bootloader.
    ProbeRs,
}

impl Flasher {
    pub fn binary(self) -> &'static str {
        match self {
            Flasher::Espflash => "espflash",
            Flasher::ProbeRs => "probe-rs",
        }
    }

    pub fn install_command(self) -> &'static str {
        match self {
            Flasher::Espflash => "cargo install espflash",
            Flasher::ProbeRs => "cargo install probe-rs-tools",
        }
    }
}

/// Whether the project links the ESP-IDF C framework and gets `std`, or runs
/// bare-metal against `esp-hal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Runtime {
    /// `no_std` on `esp-hal`. Smaller, faster to build, no C toolchain.
    BareMetal,
    /// `std` on `esp-idf-hal` / `esp-idf-svc`. Threads, sockets, filesystem —
    /// at the cost of pulling in the whole ESP-IDF build.
    EspIdf,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Runtime::BareMetal => "no_std (esp-hal)",
            Runtime::EspIdf => "std (esp-idf)",
        }
    }
}

/// A supported microcontroller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chip {
    /// Canonical lowercase id, e.g. `esp32c3`. Matches the HAL's feature name
    /// and what the flasher expects on the command line.
    pub id: String,
    /// Marketing name, e.g. `ESP32-C3`.
    pub name: String,
    pub vendor: Vendor,
    pub arch: Arch,
    pub cores: u8,
    /// Nominal on-chip SRAM in bytes, from the datasheet.
    ///
    /// The usable figure is always lower — the linker script and the ROM
    /// bootloader both take a share — so the memory dashboard reports regions
    /// read from the ELF and treats this only as headline context.
    pub sram_bytes: u32,
    /// On-chip flash in bytes, when the part has any. Espressif modules pair
    /// with an external chip whose real size is only knowable once connected;
    /// most STM32 parts have it on die.
    pub flash_bytes: Option<u32>,
    /// Rust target for a bare-metal build.
    pub bare_metal_target: String,
    /// Rust target for a `std` build, where one exists. Espressif provides
    /// these through ESP-IDF; no STM32 part has one.
    pub std_target: Option<String>,
    pub toolchain: ToolchainRequirement,
    /// Ways to put a binary on this part, preferred first.
    pub flashers: Vec<Flasher>,
    /// What `probe-rs --chip` expects for this part.
    ///
    /// `None` where the name depends on package and flash size rather than on
    /// the die — most of the STM32 range — in which case the user has to pick
    /// from `probe-rs chip list`. Guessing would produce a plausible name that
    /// flashes the wrong memory map.
    pub probe_rs_target: Option<String>,
    /// Radios the part provides, for the wizard to explain what it is choosing.
    pub radios: Vec<String>,
    /// Every GPIO the die actually has, ascending — transcribed from the
    /// vendor's own device description rather than typed from a datasheet.
    ///
    /// Empty means rusty does not know, and the board view then draws no pin
    /// rows rather than someone else's: it used to draw the classic 30-pin
    /// ESP32 devkit for every part, so a C3 board showed GPIO36/39/34/35,
    /// none of which exist on it.
    #[serde(default)]
    pub gpio: Vec<u32>,
    /// The crate a project selects this part through, when selecting it means
    /// putting [`Self::id`] in that crate's feature list — `esp-hal` for every
    /// Espressif part.
    ///
    /// This is what makes switching chips mechanical, and its absence is what
    /// makes it impossible: two parts behind one HAL differ by a feature name,
    /// a target triple and a toolchain, all of which are rewriteable. Two
    /// parts behind *different* HALs differ by every API the firmware calls.
    ///
    /// `None` means rusty does not know how a project names this part, so it
    /// refuses to migrate to or from it rather than rewriting four files into
    /// a project that cannot build. A chip added to the catalogue is therefore
    /// safe by default: it works everywhere else and offers no switch until
    /// someone states this.
    #[serde(default)]
    pub hal: Option<String>,
}

impl Chip {
    /// The target triple for a given runtime, if that combination is supported.
    pub fn target_for(&self, runtime: Runtime) -> Option<&str> {
        match runtime {
            Runtime::BareMetal => Some(&self.bare_metal_target),
            Runtime::EspIdf => self.std_target.as_deref(),
        }
    }

    pub fn needs_esp_toolchain(&self) -> bool {
        self.toolchain == ToolchainRequirement::EspXtensa
    }
}

/// A development board: a chip plus everything the chip cannot tell you.
///
/// This is what is actually on the desk. `ESP32-C3` is a die;
/// `ESP32-C3-DevKitM-1` is the thing with a USB socket, 4 MB of flash, and an
/// LED on a particular pin. Flash size, USB identity, and pin names are all
/// board facts, and without them the port list can only say "COM3 (CP210x)".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: String,
    pub name: String,
    /// Chip id this board carries.
    pub chip: String,
    /// Flash fitted on this board.
    pub flash_bytes: Option<u32>,
    /// External PSRAM, where the module has it.
    pub psram_bytes: Option<u32>,
    /// USB devices this board can enumerate as.
    ///
    /// More than one is normal: an S3 devkit has both a UART bridge and the
    /// chip's own USB peripheral, on separate sockets, and they look like
    /// different devices to the OS.
    pub usb: Vec<UsbMatch>,
    /// Flashing baud this board is known to tolerate.
    pub flash_baud: Option<u32>,
    /// Named pins, e.g. `led = 8`.
    pub pins: Vec<PinAssignment>,
    /// Which layer this definition came from, so the UI can distinguish a
    /// built-in entry from one the user or their team wrote.
    pub source: CatalogSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbMatch {
    pub vendor_id: u16,
    pub product_id: u16,
    /// What this particular enumeration is, e.g. `CP210x bridge`.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinAssignment {
    pub name: String,
    pub gpio: u32,
}

/// Where a catalogue entry came from.
///
/// Layered so a team can correct or extend the built-ins without forking:
/// built-in loses to the user's own files, which lose to the project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CatalogSource {
    /// Shipped inside the binary.
    Builtin,
    /// From the user's config directory.
    User,
    /// From `.rusty/` in the open project — checked in, so the whole team gets
    /// it.
    Project,
}

impl CatalogSource {
    pub fn label(self) -> &'static str {
        match self {
            CatalogSource::Builtin => "built in",
            CatalogSource::User => "your config",
            CatalogSource::Project => "this project",
        }
    }
}

/// A catalogue file that would not load, and why.
///
/// A wire type rather than a backend one: the app has to be able to say "your
/// board file did not parse" in the window, not only in the CLI. It was a
/// duplicate DTO in `rusty-app` until this housekeeping — exactly the
/// generated-binding drift rule 1 exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProblem {
    pub path: String,
    pub detail: String,
}
