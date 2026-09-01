//! Wire types for the embedded workbench.
//!
//! Like `rusty_core::model`, this is compiled unconditionally and must stay
//! free of IO so the Leptos frontend can `use` these types directly.

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Where users go
// ─────────────────────────────────────────────────────────────────────────────

/// The repository: source, downloads, and somewhere to report a fault.
///
/// One place, since the source went public. It was two for a while — a
/// private repository that built, and a public one that published — because
/// GitHub answers 404 rather than 403 for a private repository, so an update
/// check pointed at the source failed for every user and said "not found",
/// which reads as "there is no release" rather than "you cannot see this".
///
/// Named here, in the one module both sides compile: `update.rs` is
/// backend-only and the Help menu is wasm. Written out in full rather than
/// concatenated — `concat!` takes literals and not constants, and building
/// them with `format!` would make runtime strings that no longer fit in a
/// `Copy` action.
pub const REPO: &str = "https://github.com/Linshiqi/rusty";
pub const REPO_RELEASES: &str = "https://github.com/Linshiqi/rusty/releases";
pub const REPO_ISSUES: &str = "https://github.com/Linshiqi/rusty/issues/new/choose";

/// The releases API for [`REPO`]. Anonymous calls work because it is public.
pub const RELEASES_API: &str = "https://api.github.com/repos/Linshiqi/rusty/releases/latest";

// ─────────────────────────────────────────────────────────────────────────────
// Chips
// ─────────────────────────────────────────────────────────────────────────────

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

/// The assistant profile, as the file records it.
///
/// A separate type from `rusty_ai::ProviderConfig` on purpose — the same rule
/// that keeps `catalog.rs` from serialising `model` types. This is a contract
/// with a file somebody may edit; that one is a contract with the frontend,
/// and coupling them means a refactor rewriting everybody's workbench.toml.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantChoice {
    pub profile: String,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
}

/// One project's open editors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTabs {
    /// The project root as the user spelled it when it was recorded. Matching
    /// is by [`same_dir`], so a different spelling of the same directory finds
    /// it — the trap `recent_projects` already learned.
    pub root: String,
    pub tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
}

/// The part's pins, and what the project's own source says about them.
///
/// Two independent halves on purpose. The claims come from the source text and
/// are always available; the capabilities come from the HAL's own device
/// description and are available only when it can be found. A pin map that
/// showed one as if it were the other would be the confident wrong answer this
/// workbench is written against, so [`Self::source`] says which is which.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinReport {
    pub chip: String,
    /// Every pin the part has, in numeric order. Empty when the device
    /// description could not be read — [`Self::note`] then says why, and the
    /// claims below are still worth showing.
    pub pins: Vec<PinInfo>,
    /// Where the capabilities came from, for a user who wants to check them.
    pub source: Option<String>,
    /// Why there are no capabilities, in terms the caller can act on.
    pub note: Option<String>,
    /// Pins the source names that the part does not have. After a chip switch
    /// this is the whole of the work left, so it is not buried in `pins`.
    pub unknown: Vec<PinClaim>,
}

/// One pin, as the vendor describes it and as the project uses it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinInfo {
    pub gpio: u32,
    /// No output driver at all — ESP32's 34..39. Assigning an LED here
    /// compiles and does nothing.
    pub input_only: bool,
    /// `ADC1_CH4`, `DAC1`, `TOUCH7` — what an analog use needs.
    pub analog: Vec<String>,
    /// What the pin is wired to on essentially every module, from its
    /// function at mux level 0: the SPI flash, the USB pair, the console.
    /// Using one of these is a board that stops booting, not a compile error.
    pub reserved: Option<String>,
    /// Where the project's own source names this pin.
    pub claims: Vec<PinClaim>,
}

/// One place the source text names a pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinClaim {
    pub gpio: u32,
    /// Project-relative, `/`-separated.
    pub file: String,
    /// Zero-based, like every line number that crosses this boundary.
    pub line: u32,
    /// The line itself, trimmed — enough to recognise the site without
    /// opening it.
    pub text: String,
}

/// What switching a project from one chip to another would change.
///
/// The plan crosses the wire and comes back to be applied, so what runs is
/// exactly what the user was shown — the same contract the flash plans have.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Migration {
    pub from: String,
    pub to: String,
    /// Files that would be edited, in the order they would be written.
    pub files: Vec<FileChange>,
    /// What this deliberately does not do. Never empty for a plan that can
    /// run: a chip switch that said nothing about pins would be exactly the
    /// plausible-looking answer that costs somebody an afternoon.
    pub notes: Vec<String>,
    /// Why it will not run at all, when it will not.
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Project-relative, `/`-separated.
    pub path: String,
    pub edits: Vec<Edit>,
}

/// One substitution. An empty `before` appends; an empty `after` deletes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub before: String,
    pub after: String,
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

// ─────────────────────────────────────────────────────────────────────────────
// Boards
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Project detection
// ─────────────────────────────────────────────────────────────────────────────

/// What rusty could work out about an opened project.
///
/// Every field is optional because a half-configured project is the normal
/// state of the world — and saying "I could not tell, here is why" is more
/// useful than guessing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedProject {
    pub root: String,
    /// Chip id, if it could be determined.
    pub chip: Option<String>,
    /// How the chip was determined, so the user can correct a wrong guess.
    pub chip_source: Option<String>,
    pub runtime: Option<Runtime>,
    /// Target triple from `.cargo/config.toml`, if set.
    pub configured_target: Option<String>,
    /// Toolchain channel from `rust-toolchain.toml`, if set.
    pub configured_toolchain: Option<String>,
    /// HAL and framework crates found in the manifest.
    pub frameworks: Vec<String>,
    /// Whether `defmt` is a dependency — decides whether the monitor should
    /// decode logs or show them raw.
    pub uses_defmt: bool,
    /// Whether `embassy-executor` is present.
    pub uses_embassy: bool,
    /// What C this project already speaks, if any.
    #[serde(default)]
    pub c_interop: CInterop,
    /// Files that informed the detection, relative to the root.
    pub evidence: Vec<String>,
    /// Things that will stop a build, in the order worth fixing them.
    pub problems: Vec<Problem>,
}

/// Something wrong with the project or the machine, stated in terms of what to
/// do about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub severity: Severity,
    /// One line, what is wrong.
    pub title: String,
    /// Why it matters, in the user's terms.
    pub detail: String,
    /// A command that fixes it, when one exists. Shown as a copyable button
    /// rather than run automatically — installing toolchains is the user's
    /// call, not ours.
    pub fix_command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    /// The build cannot succeed until this is fixed.
    Blocking,
    /// The build works but something is inconsistent or suboptimal.
    Warning,
    /// Worth knowing, no action required.
    Info,
}

// ─────────────────────────────────────────────────────────────────────────────
// New project
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardChoice {
    pub chip: String,
    pub runtime: Runtime,
    /// Crate name for the new project.
    pub name: String,
    /// Generator option ids, e.g. `embassy`, `wifi`, `alloc`.
    #[serde(default)]
    pub options: Vec<String>,
}

/// A generator option, with what turning it on costs.
///
/// A model type rather than a DTO in the Tauri layer: the frontend renders
/// these, and rule 1 is that it `use`s model types directly. A struct declared
/// beside the command would have to be mirrored by hand in the frontend, which
/// is the drift the shared types exist to make impossible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardOption {
    /// What `esp-generate -o` expects.
    pub id: String,
    pub label: String,
    /// What it commits the project to, in the user's terms.
    pub detail: String,
    /// Options this one cannot work without.
    ///
    /// `esp-generate` enforces these and rejects the entire run when they are
    /// missing, so the wizard needs them to avoid offering a combination that
    /// cannot succeed.
    #[serde(default)]
    pub requires: Vec<String>,
}

/// What one choice in the wizard commits the user to.
///
/// The reason the wizard exists. A list of chip names tells a beginner nothing
/// about the fact that half of them require downloading a forked compiler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub topic: String,
    pub detail: String,
    /// A concrete follow-on — a command to run, a target that gets used.
    pub consequence: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Devices
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialPort {
    /// OS name: `COM3`, `/dev/ttyUSB0`, `/dev/cu.usbserial-0001`.
    pub name: String,
    /// The USB-to-UART bridge, named as it is printed on the board — `CP210x`,
    /// `CH340`. The fallback when no board in the catalogue matches.
    pub bridge: Option<String>,
    /// Boards whose USB identity matches this port.
    ///
    /// Usually zero or one. More than one means several boards share a bridge
    /// chip — very common, since a CP210x is a CP210x — and the UI has to let
    /// the user pick rather than guessing.
    pub boards: Vec<String>,
    /// True when this looks like a development board rather than a modem or a
    /// virtual port, which would only waste the user's time.
    pub likely_board: bool,
    pub usb: Option<UsbIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    /// What `probe-rs --probe` expects.
    pub identifier: String,
    pub description: String,
}

/// How to reach the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Transport {
    /// Through the ROM serial bootloader. No extra hardware; Espressif only.
    Serial { port: String },
    /// Through a JTAG/SWD probe. Adds breakpoints and RTT, and is the only way
    /// onto a part with no serial bootloader.
    Probe { identifier: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlashAction {
    /// Write the image and stop.
    Flash,
    /// Attach to a board already running, without rewriting flash.
    Monitor,
    /// Write, then stay attached for logs. The usual inner loop.
    FlashAndMonitor,
}

/// What a scaffolding run wrote, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldReport {
    pub written: Vec<String>,
    /// The dependency to add, as a command the user watches run.
    pub command: Option<CommandPlan>,
    /// The step scaffolding cannot do for you, in one sentence.
    pub next: String,
}

/// How a Rust firmware project meets C.
///
/// Reported, not guessed: every entry names the file that proves it. A
/// workbench that says "this project uses bindgen" without being able to
/// point at the line is a workbench that will eventually be wrong about it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CInterop {
    /// Crates that pull a C toolchain into the build, each with what it
    /// does — `cc`, `bindgen`, `esp-idf-sys`.
    pub via: Vec<String>,
    /// C and C++ sources carried in the project itself.
    pub sources: u32,
    /// True when the crate is built as a library C can link against —
    /// `staticlib` or `cdylib` in the manifest.
    pub exports_to_c: bool,
    /// The files that prove the above.
    pub evidence: Vec<String>,
}

impl CInterop {
    /// Whether anything at all was found — the panel shows nothing rather
    /// than an empty heading for a pure-Rust project.
    pub fn is_empty(&self) -> bool {
        self.via.is_empty() && self.sources == 0 && !self.exports_to_c
    }
}

/// A chip's peripherals, as a register view needs them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterMap {
    pub peripherals: Vec<Peripheral>,
    /// How many peripherals or registers the parse could not place —
    /// `derivedFrom` inheritance, mostly. Shown rather than hidden: a panel
    /// silently missing GPIO2 is a panel that lies about the chip.
    pub dropped: u32,
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

/// What an update check found.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The running build.
    pub current: String,
    /// The newest published version, when the check reached GitHub.
    pub latest: Option<String>,
    /// Where to get it.
    pub url: Option<String>,
    /// True only when `latest` is genuinely ahead of `current`.
    pub newer: bool,
    /// Why the check could not answer — no network is the normal state of a
    /// workbench on a bench, so this is a note rather than an error.
    pub note: Option<String>,
}

/// A command that is about to be run, in full.
///
/// Produced without spawning anything so it can be tested without hardware —
/// and shown to the user verbatim before it runs. Embedded developers reach for
/// the terminal constantly; hiding the command behind a button is how a tool
/// becomes something to work around rather than with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandPlan {
    pub program: String,
    pub args: Vec<String>,
    /// The whole thing as one copy-pasteable line.
    pub display: String,
    /// Why this tool and these flags, in one sentence.
    pub rationale: String,
    /// Read this before running it. Absent for the ordinary case; present
    /// when the plan is defensible but something about the situation says
    /// it will not do what the user expects — a device that cannot be the
    /// chip this project builds for, for instance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogStream {
    Stdout,
    Stderr,
}

/// What shell the terminal will start, and what choices exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellChoice {
    /// Human name for the picker: "PowerShell 7", "rusty bash (built-in)".
    pub label: String,
    /// What set_terminal_shell stores: "auto", "system", or a program.
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    /// The program the next shell start will actually run.
    pub active: String,
    /// The stored preference: absent = auto (the built-in shell),
    /// "system" = the OS shell, anything else = a custom program.
    pub preference: Option<String>,
}

/// One line of output from a flash or monitor session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
    /// Severity parsed out of a defmt or ESP-IDF log line, when present.
    pub level: Option<LogLevel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

// ─────────────────────────────────────────────────────────────────────────────
// Storage
// ─────────────────────────────────────────────────────────────────────────────

/// A tool the simulator needs and cannot find, with the way to get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimTool {
    pub name: String,
    pub install: String,
}

/// A display pin nobody has wired yet.
///
/// One value, named once: the file format and the wire model both need it,
/// and two spellings of 255 is how one of them ends up meaning something else.
pub const UNWIRED_PIN: u8 = 255;

fn unwired_pin() -> u8 {
    UNWIRED_PIN
}

/// Serde's skip test for the common case: most parts are never turned.
pub(crate) fn is_upright(rot: &u16) -> bool {
    *rot == 0
}

/// Where a part sits on the sheet, how it is turned, and how its wires run.
///
/// One struct rather than the same five fields on every part. They were copied
/// six times, comments and all, and the copies drifted the moment one of them
/// gained a field: `flip` was added to all six wire types and to *neither*
/// half of the file format, so mirroring a part survived until the project was
/// reopened and then silently was not there. A field added here reaches every
/// part or none of them.
///
/// A nested field rather than `#[serde(flatten)]`: flatten routes the whole
/// struct through serde's buffering path, and the frontend decodes this from a
/// JS value, where a buffered number is not reliably the integer `rot` needs.
/// The JSON shape is internal — both sides `use` this same type — so nesting
/// costs nothing and cannot misdecode.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    /// Canvas position, when the editor has placed it. Absent means "lay it
    /// out automatically", which is what hand-written files get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// User-drawn waypoints per wire, world coordinates. Empty means "route
    /// automatically". routes[0] belongs to pins[0] (or the only pin).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<Vec<(f64, f64)>>,
    /// Quarter turns on the sheet: 0, 90, 180 or 270 degrees. A schematic
    /// nobody can rotate is a diagram that fights its own wiring.
    #[serde(default, skip_serializing_if = "is_upright")]
    pub rot: u16,
    /// Mirrored left-to-right — what a part on the chip's right wants.
    /// Mirrored rather than turned because a 180° turn also reverses the stub
    /// order, and seven wires to a seven-segment then cross on the way in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// One LED on the simulated board view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimLed {
    pub pin: u8,
    /// `green`, `blue`, `red`, `yellow` — the stylesheet's palette names.
    pub color: String,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A push button on the board. Pressing it sends `B<pin>=1` (and release
/// `=0`) into the firmware's UART through the simulator's stdin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimButton {
    pub pin: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// An RGB LED: three pins, one lens. The lit colour is the additive mix of
/// whichever channels the firmware reports high.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A seven-segment digit: seven GPIO pins, one per segment a..g. Lit
/// segment by segment from the same gpio report channel as every lamp —
/// the most honest display there is, because it is not a display at all,
/// just seven LEDs in a font.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSeven {
    /// Segments a, b, c, d, e, f, g in order.
    pub pins: [u8; 7],
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// A small text screen fed by the `[rusty:disp]` serial channel — the
/// firmware prints what the screen shows. Stands in for OLED/LCD until a
/// protocol decoder exists; the caption on the panel says whose word it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDisplay {
    pub label: String,
    /// The I2C pins the module hangs on. 255 = not wired yet — old board
    /// files carry no pins at all, and an unwired screen still shows text.
    #[serde(default = "unwired_pin")]
    pub sda: u8,
    #[serde(default = "unwired_pin")]
    pub scl: u8,
    /// `routes` here is (sda, scl), in that order.
    #[serde(default)]
    pub place: Placement,
}

/// A potentiometer: a slider in the UI that sends `P<pin>=<0..255>` into
/// the firmware's UART as it moves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimPot {
    pub pin: u8,
    pub label: String,
    #[serde(default)]
    pub place: Placement,
}

/// An analog source on a pin: a battery through a divider, a thermistor, any
/// voltage the firmware reads with its ADC.
///
/// Distinct from [`SimPot`], which is a *knob a person turns* and sends 8
/// bits. This is a *voltage that is there*, at the resolution the chip's ADC
/// actually has — a 1S cell sagging from 4.2 V to 3.3 V under throttle is
/// four counts of an 8-bit range and eighty of a 12-bit one, and a low-battery
/// cutoff cannot be tested against four.
///
/// **Counts, not volts.** rusty does not know the divider on your board, so
/// it does not claim a voltage. [`Self::note`] is where you write what the
/// count means to *you*, and it is shown verbatim as your words, not rusty's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimAnalog {
    pub pin: u8,
    pub label: String,
    /// Full scale for this source. 4095 is the ESP32's 12-bit ADC; a part
    /// wired to something else can say so.
    #[serde(default = "full_scale")]
    pub max: u16,
    /// Where the slider sits when the board loads.
    #[serde(default)]
    pub start: u16,
    /// What the count means on this board — "4095 = 4.2 V through 100k/27k".
    /// Yours to write and yours to be right about; rusty only repeats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub place: Placement,
}

fn full_scale() -> u16 {
    4095
}

/// A motor: a toy car's drive through an H-bridge, or a fan.
///
/// One part rather than two, because a fan *is* a motor with one direction —
/// and which one you have is a property of what you wired, not a mode to
/// pick from a menu. Wire only [`Self::pwm`] and it turns one way; wire the
/// two direction pins as well and it is an H-bridge.
///
/// The direction pins are ordinary GPIO and arrive on the boolean channel.
/// The speed cannot: a duty cycle is not a level, which is why
/// [`crate::protocol::parse_pwm_report`] exists at all.
///
/// **This shows commanded drive, never a measured shaft speed.** There is no
/// inertia here, no load, no back-EMF — a motor that has been told 40% shows
/// 40% the instant it is told, and a real one takes time to get there under
/// a load rusty knows nothing about. The panel says so rather than implying
/// a physics it does not have.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimMotor {
    /// The pin carrying the duty cycle — the enable leg of an H-bridge, or
    /// the fan's single control wire.
    #[serde(default = "unwired_pin")]
    pub pwm: u8,
    /// The H-bridge's two direction inputs. Both [`UNWIRED_PIN`] for a fan.
    #[serde(default = "unwired_pin")]
    pub in1: u8,
    #[serde(default = "unwired_pin")]
    pub in2: u8,
    pub label: String,
    /// `routes` here is (pwm, in1, in2), in that order.
    #[serde(default)]
    pub place: Placement,
}

/// What an H-bridge is doing, from its two direction inputs.
///
/// Worth naming rather than leaving as two booleans in the view, because the
/// table is the thing people get wrong: `1,1` is not "full speed", it is a
/// brake — both low-side transistors on, the winding shorted, the motor
/// fighting its own momentum. Someone who reaches for it expecting speed
/// gets a stop, and nothing in a datasheet page of timing diagrams says so
/// as plainly as a board that shows BRAKE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Drive {
    Forward,
    Reverse,
    /// Both inputs low: the bridge is open and the motor freewheels.
    Coast,
    /// Both inputs high: the winding is shorted and the motor is held.
    Brake,
}

impl Drive {
    /// The H-bridge truth table, and the whole reason this type exists.
    pub fn from_inputs(in1: bool, in2: bool) -> Self {
        match (in1, in2) {
            (true, false) => Drive::Forward,
            (false, true) => Drive::Reverse,
            (false, false) => Drive::Coast,
            (true, true) => Drive::Brake,
        }
    }

    /// What the panel writes beside the rotor.
    pub fn label(self) -> &'static str {
        match self {
            Drive::Forward => "FWD",
            Drive::Reverse => "REV",
            Drive::Coast => "COAST",
            Drive::Brake => "BRAKE",
        }
    }

    /// Whether the shaft turns at all — a duty of 90% into a braked bridge
    /// still goes nowhere, and a rotor that spun anyway would be teaching
    /// the wrong thing.
    pub fn turns(self) -> bool {
        matches!(self, Drive::Forward | Drive::Reverse)
    }
}

/// A user-defined part from `.rusty/parts/*.toml` — how a device rusty never
/// heard of still gets drawn and driven. v1 parts behave as lamps on the
/// gpio report channel; richer behaviours grow on this same record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartDef {
    pub name: String,
    /// Glow hue, one of the palette names.
    pub color: String,
}

/// The board view beside the serial output, when the project describes one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBoard {
    pub chip: String,
    /// Where the devkit itself sits on the canvas — it is a part like any
    /// other, and a schematic whose chip cannot move is a poster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kit_y: Option<f64>,
    pub leds: Vec<SimLed>,
    #[serde(default)]
    pub buttons: Vec<SimButton>,
    #[serde(default)]
    pub rgbs: Vec<SimRgb>,
    #[serde(default)]
    pub sevens: Vec<SimSeven>,
    #[serde(default)]
    pub displays: Vec<SimDisplay>,
    #[serde(default)]
    pub pots: Vec<SimPot>,
    #[serde(default)]
    pub motors: Vec<SimMotor>,
    #[serde(default)]
    pub analogs: Vec<SimAnalog>,
}

/// Everything the frontend needs to attach a debugger to a frozen boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimDebug {
    /// The full command line to type into the terminal: gdb, the ELF, and
    /// `target remote` — composed here so the frontend never builds paths.
    /// Kept for the terminal path, which is still the way to reach gdb's
    /// own REPL for anything the panel does not model.
    pub gdb_command: String,
    /// The image with the symbols in it — what the in-app debugger loads.
    #[serde(default)]
    pub elf: String,
    /// Where QEMU's gdbstub listens.
    #[serde(default = "gdbstub_port")]
    pub port: u16,
}

fn gdbstub_port() -> u16 {
    1234
}

/// How this project would be simulated, or exactly why it cannot be.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimPlan {
    pub supported: bool,
    /// Set when `supported` is false — the refusal, in actionable terms.
    pub reason: Option<String>,
    /// Tools to install before the steps can run.
    pub missing: Vec<SimTool>,
    /// build → image → boot, each inspectable before anything runs.
    pub steps: Vec<CommandPlan>,
    /// Drawn beside the serial output when `.rusty/sim.toml` describes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<SimBoard>,
    /// User-defined parts from `.rusty/parts/`, offered in the library.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartDef>,
    /// Present when the right gdb is installed; the Debug button needs it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug: Option<SimDebug>,
    /// The gdb to install when `debug` is absent — same card, same one-click
    /// installer as every other missing tool, but it only gates Debug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_tool: Option<SimTool>,
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

/// Where rusty keeps its data, for the settings screen to show.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLocation {
    pub path: String,
    /// True when no pointer and no env override is in play.
    pub is_default: bool,
    /// True when `RUSTY_CONFIG_DIR` decided — relocating from the UI would be
    /// silently outvoted, so the UI disables it and says why.
    pub env_override: bool,
}

/// What a relocation did, so the user can verify before deleting the old copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateReport {
    pub from: String,
    pub to: String,
    pub copied_files: usize,
    pub adopted: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Built firmware
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Memory
// ─────────────────────────────────────────────────────────────────────────────

/// What a loaded ELF section costs the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionKindDto {
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

impl SectionKindDto {
    /// Whether one byte of this kind occupies (flash, RAM).
    pub fn budget(self) -> (bool, bool) {
        match self {
            SectionKindDto::Code | SectionKindDto::ReadOnlyData => (true, false),
            SectionKindDto::InitialisedData => (true, true),
            SectionKindDto::ZeroedData => (false, true),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SectionKindDto::Code => "code",
            SectionKindDto::ReadOnlyData => "read-only",
            SectionKindDto::InitialisedData => "data",
            SectionKindDto::ZeroedData => "bss",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionSize {
    pub name: String,
    pub address: u64,
    pub size: u64,
    pub kind: SectionKindDto,
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

// ─────────────────────────────────────────────────────────────────────────────
// Toolchain
// ─────────────────────────────────────────────────────────────────────────────

/// The state of the machine's Rust and Espressif tooling.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainStatus {
    /// `rustup toolchain list`, normalized to channel names.
    pub toolchains: Vec<Toolchain>,
    /// Targets installed for the active toolchain.
    pub installed_targets: Vec<String>,
    /// Espressif and probe tooling found on PATH.
    pub tools: Vec<ToolStatus>,
    /// True when a toolchain named `esp` is present — the Xtensa one espup
    /// installs.
    pub has_esp_toolchain: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    pub name: String,
    pub is_default: bool,
    /// True for the espup-installed Xtensa toolchain.
    pub is_esp: bool,
}

/// One external binary the workbench can drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    /// Executable name, e.g. `espflash`.
    pub name: String,
    /// What it is for, shown when it is missing so the user can decide whether
    /// they need it at all.
    pub purpose: String,
    /// What `<tool> --version` said, when it says anything. Decoration, not
    /// evidence: `ldproxy` is a linker shim with no CLI at all and panics on
    /// the flag, and treating that as "not installed" is what it used to do.
    pub version: Option<String>,
    /// Where the binary actually is. `None` is what "not installed" means,
    /// and showing it answers the question every one of these raises — which
    /// copy is being used, and on which disk it sits.
    pub path: Option<String>,
    /// How to install it, if absent.
    pub install_command: String,
    /// Whether rusty can install it itself. False means the panel offers no
    /// button: one that always fails is worse than the instructions it hides,
    /// which is the rule the chip picker already follows.
    #[serde(default)]
    pub installable: bool,
    /// False when this tool is only needed for some projects.
    pub required: bool,
}

impl ToolStatus {
    /// Presence on PATH, not a successful `--version`.
    pub fn is_installed(&self) -> bool {
        self.path.is_some()
    }
}

/// Everything the toolchain panel shows: machine state plus what this
/// particular project needs from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolchainReport {
    pub status: ToolchainStatus,
    /// Target triple this project needs, when it is known.
    pub required_target: Option<String>,
    /// Whether that target is installed.
    pub required_target_installed: bool,
    /// Whether this project needs the Xtensa toolchain.
    pub needs_esp_toolchain: bool,
    pub problems: Vec<Problem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The H-bridge table, which is the one piece of real hardware knowledge
    /// this type carries. `1,1` is the entry worth having a test for: it is
    /// the one people reach for expecting full speed, and it is a brake.
    #[test]
    fn both_inputs_high_is_a_brake_and_not_full_speed() {
        assert_eq!(Drive::from_inputs(true, true), Drive::Brake);
        assert!(!Drive::Brake.turns(), "a braked bridge holds the shaft");

        assert_eq!(Drive::from_inputs(false, false), Drive::Coast);
        assert!(!Drive::Coast.turns(), "an open bridge freewheels");

        assert_eq!(Drive::from_inputs(true, false), Drive::Forward);
        assert_eq!(Drive::from_inputs(false, true), Drive::Reverse);
        assert!(Drive::Forward.turns() && Drive::Reverse.turns());
    }

    /// Reversing is swapping the two inputs, and nothing else. Firmware that
    /// drives one pin and leaves the other alone gets brake or coast rather
    /// than the reverse it wanted, which is exactly the mistake the board is
    /// meant to make visible.
    #[test]
    fn reverse_is_the_mirror_of_forward() {
        for (a, b) in [(true, false), (false, true)] {
            let one = Drive::from_inputs(a, b);
            let other = Drive::from_inputs(b, a);
            assert_ne!(one, other);
            assert!(one.turns() && other.turns());
        }
    }
}
