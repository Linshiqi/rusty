//! Chips and boards, as the frontend renders them.
//!
//! The catalogue *files* are a different set of types in `catalog.rs`; these
//! are what comes out of it.

use serde::{Deserialize, Serialize};

/// Who makes the part, and how a Rust project for one of their parts reads.
///
/// Data, from the catalogue's `[[vendor]]` tables: it was a closed enum, and
/// a vendor added to the catalogue then needed code in six places — the
/// enum, its file-format mirror, the label, the crates detection reads, the
/// order detection reads them in, and the runtime's label — before a single
/// part of theirs could be detected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vendor {
    /// `espressif`, `wch` — what a chip's `vendor` names.
    pub id: String,
    /// `Espressif`, `STMicroelectronics`.
    pub name: String,
    /// The HAL a bare-metal project for their parts is written against, as
    /// the runtime's label names it — `no_std (esp-hal)`. `None` where there
    /// are several and none is *the* one, as for ST's.
    #[serde(default)]
    pub hal: Option<String>,
    /// Crates that carry the part number as a cargo feature, most
    /// authoritative first.
    ///
    /// This is the main thing that differs per vendor: `esp-hal` names the
    /// chip `esp32c3`, while `embassy-stm32` names it `stm32f411ce`.
    /// Detection reads the same shape from both, but has to know where to
    /// look — and which crate it *cites*: half a dozen `esp-*` crates take
    /// the same chip feature, and a user told their chip came from
    /// `esp-backtrace` would go and edit the wrong line.
    #[serde(default)]
    pub chip_crates: Vec<String>,
    /// Crates whose presence means a project runs bare metal on these parts.
    #[serde(default)]
    pub bare_metal_crates: Vec<String>,
    /// Crates whose presence means it runs on the vendor's `std` framework —
    /// ESP-IDF's, for Espressif.
    #[serde(default)]
    pub std_crates: Vec<String>,
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
    /// A nightly rustup toolchain with `rust-src`: the part's target is a
    /// JSON spec rustup has no standard library for, so cargo builds `core`
    /// itself (`-Zbuild-std`). The CH32V003's RV32EC is one — the built-in
    /// `riscv32e*` targets are RV32E without C, or with M it lacks.
    NightlyBuildStd,
}

impl ToolchainRequirement {
    pub fn install_command(self) -> Option<&'static str> {
        match self {
            ToolchainRequirement::Stock => None,
            ToolchainRequirement::EspXtensa => Some("espup install"),
            ToolchainRequirement::NightlyBuildStd => {
                Some("rustup toolchain install nightly --component rust-src")
            }
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
    /// WCH's own probe, WCH-LinkE, through ch32-rs's `wlink`: flashes over
    /// the one-wire debug pin and relays SDI print to its serial port. What
    /// ch32-hal's examples run.
    Wlink,
}

impl Flasher {
    /// The program, as the tool ladder and the Environment page name it.
    pub fn tool(self) -> &'static str {
        match self {
            Flasher::Espflash => "espflash",
            Flasher::ProbeRs => "probe-rs",
            Flasher::Wlink => "wlink",
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

    /// The label for a part whose bare metal is `hal`'s — the vendor's
    /// ([`Chip::hal_label`]) — since a CH32 called "esp-hal" is a promise
    /// about the wrong crate.
    pub fn label_on(self, hal: Option<&str>) -> String {
        match (self, hal) {
            (Runtime::BareMetal, Some(hal)) => format!("no_std ({hal})"),
            (Runtime::BareMetal, None) => "no_std".to_string(),
            (Runtime::EspIdf, _) => self.label().to_string(),
        }
    }
}

/// How a project for a part is started: a generator somebody else
/// maintains, or one of rusty's own proven templates written out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Generator {
    /// `esp-generate`, the esp-rs project's bare-metal generator, with its
    /// own options.
    EspGenerate,
    /// `cargo generate esp-rs/esp-idf-template`, for a `std` project on
    /// ESP-IDF.
    EspIdfTemplate,
    /// One of rusty's templates (`data/templates/<name>/`), renamed for the
    /// project and moved onto the chosen part — a project rusty writes
    /// itself, with no process to run.
    Template { name: String },
}

/// The connector a devkit carries at its bottom edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KitUsb {
    MicroB,
    TypeC,
    /// One socket for the USB-UART bridge and one native, as on the S3 and
    /// C6 devkits.
    DualTypeC,
}

/// What a part's devkit looks like beyond its pins, for the board view: the
/// module soldered on it, its connector, its two buttons and whether it has
/// an RGB LED. A part without one is drawn as a bare chip, which is the
/// honest drawing of a die rusty knows no module for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Kit {
    /// The module's printed name, `ESP32-C3-MINI-1`.
    pub module: String,
    pub usb: KitUsb,
    /// The reset button's silkscreen — `EN` on the classic ESP32 devkit,
    /// `RST` on the rest — and the boot button's.
    pub reset: String,
    pub boot: String,
    pub rgb: bool,
}

/// Which emulator runs a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmulatorKind {
    /// A QEMU system emulator booting the merged image espflash would burn —
    /// rusty's build of Espressif's, or Espressif's own.
    Qemu,
    /// rusty-mcu, rusty's own instruction-level model of the part, run in
    /// process.
    RustyMcu,
}

/// How a part is simulated, and what the simulation is known not to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emulation {
    pub kind: EmulatorKind,
    /// The QEMU binary the machine needs, `qemu-system-riscv32`; QEMU's
    /// machine is the chip's id.
    #[serde(default)]
    pub binary: Option<String>,
    /// A [`SimLimit`](super::SimLimit) kind said before every run.
    #[serde(default)]
    pub limit: Option<String>,
    /// A limit kind said only when the emulator found is not rusty's current
    /// build.
    #[serde(default)]
    pub limit_outdated: Option<String>,
}

/// A part whose pins are named by port: how many pins a port is in rusty's
/// numbering — the width of the part's GPIO registers, which is how pins
/// travel on the pin channel (`PC4` is 20 at eight) — and whether ports are
/// lettered (`PC4`, ST's and WCH's) or numbered (`P0.13`, Nordic's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ports {
    pub width: u8,
    #[serde(default)]
    pub numbered: bool,
}

impl Ports {
    /// What pin `gpio` is called: `PC4`, or `P1.05`.
    pub fn name(self, gpio: u8) -> String {
        let (port, pin) = (gpio / self.width, gpio % self.width);
        if self.numbered {
            format!("P{port}.{pin:02}")
        } else {
            format!("P{}{pin}", (b'A' + port) as char)
        }
    }

    /// The pin a name says — the inverse of [`Ports::name`], in the vendor's
    /// spellings and the HALs': `PC13`, `P1.05`, and the `P0_13` embassy-nrf
    /// writes. `None` for anything else, and for a pin past the port's width:
    /// `PA16` on a part sixteen to a port is not `PB0`.
    pub fn number(self, name: &str) -> Option<u8> {
        let rest = name.strip_prefix('P')?;
        let (port, pin) = if self.numbered {
            let (port, pin) = rest.split_once(['.', '_'])?;
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (port.parse::<u8>().ok()?, pin)
        } else {
            let letter = *rest.as_bytes().first()?;
            if !letter.is_ascii_uppercase() {
                return None;
            }
            (letter - b'A', &rest[1..])
        };
        if pin.is_empty() || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let pin: u8 = pin.parse().ok()?;
        if pin >= self.width {
            return None;
        }
        port.checked_mul(self.width)?.checked_add(pin)
    }
}

/// A cross C compiler for a part, and how to get it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CCompiler {
    /// `riscv32-esp-elf-gcc`.
    pub binary: String,
    /// One line saying how to install it.
    pub install: String,
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
    /// The vendor's id, as its `[[vendor]]` table names it.
    pub vendor: String,
    /// The vendor's name, `Espressif`, carried here so nothing on the wire
    /// has to look a second table up to say who makes a part.
    #[serde(default)]
    pub vendor_name: String,
    /// The HAL a bare-metal project for this part is written against, from
    /// the vendor — what the runtime's label names.
    #[serde(default)]
    pub hal_label: Option<String>,
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
    /// How rusty numbers and names the part's pins: `None` names them by
    /// number (`GPIO4`); [`Ports`] names them by port, so many to a port.
    #[serde(default)]
    pub ports: Option<Ports>,
    /// The header of the one module whose row order rusty knows, top to
    /// bottom, left then right: a number is that GPIO, `RX:3` is GPIO3
    /// printed `RX`, anything else a rail or a plain row. Empty draws the
    /// die's pins in numeric order instead.
    #[serde(default)]
    pub header: Vec<String>,
    /// The devkit drawn around the pins, or `None` for a bare chip.
    #[serde(default)]
    pub kit: Option<Kit>,
    /// How the part is simulated, or `None` for one nothing here emulates —
    /// which the plan then refuses by name.
    #[serde(default)]
    pub emulation: Option<Emulation>,
    /// How a bare-metal project for the part is started, or `None` where
    /// rusty knows no way: the wizard refuses that rather than handing the
    /// part to a generator for somebody else's.
    #[serde(default)]
    pub generator: Option<Generator>,
    /// How a `std` project is started, where the part has a `std` target.
    #[serde(default)]
    pub std_generator: Option<Generator>,
    /// The gdb that debugs the part's images, by the name the tool ladder
    /// looks for.
    #[serde(default)]
    pub gdb: Option<String>,
    /// The cross C compiler a `cc` build script reaches for on this part.
    #[serde(default)]
    pub c_compiler: Option<CCompiler>,
    /// Where the vendor publishes the part's SVD, when they do.
    #[serde(default)]
    pub svd: Option<String>,
    /// Other names the part goes by where a project writes it down — the
    /// full ordering code PlatformIO's `board_build.mcu` and STM32CubeMX's
    /// `.ioc` carry (`stm32f411ceu6`), normalised as ids are. A name is
    /// matched whole, never by prefix: `esp32` is a prefix of a dozen parts
    /// it is not.
    #[serde(default)]
    pub aliases: Vec<String>,
}

impl Chip {
    /// The target triple for a given runtime, if that combination is supported.
    pub fn target_for(&self, runtime: Runtime) -> Option<&str> {
        match runtime {
            Runtime::BareMetal => Some(&self.bare_metal_target),
            Runtime::EspIdf => self.std_target.as_deref(),
        }
    }

    /// How a project with `runtime` is started on this part, if rusty knows.
    pub fn generator_for(&self, runtime: Runtime) -> Option<&Generator> {
        match runtime {
            Runtime::BareMetal => self.generator.as_ref(),
            Runtime::EspIdf => self.std_generator.as_ref(),
        }
    }

    /// Whether rusty writes a `runtime` project for this part itself, from a
    /// template, rather than running somebody's generator.
    pub fn writes_itself(&self, runtime: Runtime) -> bool {
        matches!(
            self.generator_for(runtime),
            Some(Generator::Template { .. })
        )
    }

    /// The runtime's label on this part: `no_std (ch32-hal)` on a CH32.
    pub fn runtime_label(&self, runtime: Runtime) -> String {
        runtime.label_on(self.hal_label.as_deref())
    }

    /// Whether rusty simulates this part with `kind`.
    pub fn emulated_by(&self, kind: EmulatorKind) -> bool {
        self.emulation.as_ref().is_some_and(|e| e.kind == kind)
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
    /// The board's id in PlatformIO (`esp32dev`), which is how a
    /// `platformio.ini` names it and how rusty finds its chip.
    #[serde(default)]
    pub platformio: Option<String>,
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
