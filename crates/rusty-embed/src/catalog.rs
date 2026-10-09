//! The chip and board catalogue, and where it comes from.
//!
//! Deliberately data, not code. The long tail here is hardware — thousands of
//! parts and boards — and no team can enumerate it. Making the catalogue a file
//! format means a user adds their board by writing six lines of TOML instead of
//! forking the project.
//!
//! Three layers, later winning by `id`:
//!
//! | Layer | Where | For |
//! |---|---|---|
//! | built-in | compiled into the binary | the common parts |
//! | user | the platform config directory | boards you own |
//! | project | `.rusty/` in the open project | boards your team owns, checked in |
//!
//! The file format is a **public contract with users**; the types in
//! [`crate::model`] are an internal contract with the frontend. They are kept
//! separate on purpose — coupling them would mean a UI refactor silently
//! breaking everybody's board files.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{
    Arch, Board, CCompiler, CatalogProblem, CatalogSource, Chip, Emulation, EmulatorKind, Flasher,
    Generator, Kit, KitUsb, PinAssignment, Ports, ToolchainRequirement, UsbMatch, Vendor,
};

const BUILTIN_CHIPS: &str = include_str!("../data/chips.toml");
const BUILTIN_BOARDS: &str = include_str!("../data/boards.toml");

/// Everything rusty knows about hardware, after layering.
#[derive(Debug, Clone)]
pub struct Catalog {
    vendors: Vec<Vendor>,
    chips: Vec<Chip>,
    boards: Vec<Board>,
    /// Files that failed to parse, with the reason.
    ///
    /// Kept rather than thrown: one malformed user file must not blank out the
    /// catalogue, and silently ignoring it would leave the user staring at a
    /// board that never appears.
    problems: Vec<CatalogProblem>,
}

impl Catalog {
    /// Only what ships in the binary. Pure and deterministic — this is what the
    /// tests and the free functions in [`crate::chip`] use.
    ///
    /// Parsed once per process and cloned out: `load` starts from this on
    /// every call, and every project open, every device scan and every
    /// wizard step calls `load`. Re-parsing two TOML files each time was
    /// measurable and bought nothing, since the files are compiled in.
    pub fn builtin() -> Self {
        Self::builtin_shared().clone()
    }

    /// The parsed built-ins, shared. What [`crate::chip`] reads through.
    pub(crate) fn builtin_shared() -> &'static Catalog {
        static PARSED: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
        PARSED.get_or_init(Self::parse_builtin)
    }

    fn parse_builtin() -> Self {
        let mut catalog = Catalog {
            vendors: Vec::new(),
            chips: Vec::new(),
            boards: Vec::new(),
            problems: Vec::new(),
        };
        // A malformed built-in file is a build-time mistake, so failing loudly
        // here is right — but only in debug, so a shipped binary degrades to an
        // empty catalogue rather than refusing to start.
        catalog.absorb_chips(BUILTIN_CHIPS, "<builtin>/chips.toml");
        catalog.absorb_boards(
            BUILTIN_BOARDS,
            "<builtin>/boards.toml",
            CatalogSource::Builtin,
        );
        catalog.resolve();
        debug_assert!(
            catalog.problems.is_empty(),
            "built-in catalogue is malformed: {:?}",
            catalog.problems
        );
        catalog
    }

    /// Built-ins plus the user's own files, plus the project's.
    pub fn load(project_root: Option<&Path>) -> Self {
        let mut catalog = Self::builtin();

        if let Some(dir) = user_catalog_dir() {
            catalog.absorb_dir(&dir, CatalogSource::User);
        }
        if let Some(root) = project_root {
            catalog.absorb_dir(&root.join(".rusty"), CatalogSource::Project);
        }
        catalog.resolve();
        catalog
    }

    pub fn vendors(&self) -> &[Vendor] {
        &self.vendors
    }

    pub fn vendor(&self, id: &str) -> Option<&Vendor> {
        self.vendors.iter().find(|v| v.id == id)
    }

    pub fn chips(&self) -> &[Chip] {
        &self.chips
    }

    pub fn boards(&self) -> &[Board] {
        &self.boards
    }

    pub fn problems(&self) -> &[CatalogProblem] {
        &self.problems
    }

    pub fn chip(&self, id: &str) -> Option<&Chip> {
        let wanted = normalize(id);
        self.chips.iter().find(|c| c.id == wanted)
    }

    pub fn board(&self, id: &str) -> Option<&Board> {
        self.boards.iter().find(|b| b.id == id)
    }

    /// Boards that enumerate as this USB device.
    ///
    /// Several boards legitimately share one bridge chip — a CP210x is a
    /// CP210x — so this returns all of them and lets the caller present a
    /// choice rather than picking one and being wrong.
    pub fn boards_for_usb(&self, vendor_id: u16, product_id: u16) -> Vec<&Board> {
        self.boards
            .iter()
            .filter(|b| {
                b.usb
                    .iter()
                    .any(|u| u.vendor_id == vendor_id && u.product_id == product_id)
            })
            .collect()
    }

    /// Boards carrying a given chip, for the wizard.
    pub fn boards_for_chip(&self, chip_id: &str) -> Vec<&Board> {
        let wanted = normalize(chip_id);
        self.boards.iter().filter(|b| b.chip == wanted).collect()
    }

    // ── loading ──────────────────────────────────────────────────────────────

    fn absorb_dir(&mut self, dir: &Path, source: CatalogSource) {
        for subdir in ["chips", "boards"] {
            for file in crate::layers::files_in(&dir.join(subdir), "toml") {
                let label = file.display().to_string();
                match std::fs::read_to_string(&file) {
                    Ok(text) if subdir == "chips" => self.absorb_chips(&text, &label),
                    Ok(text) => self.absorb_boards(&text, &label, source),
                    Err(e) => self.problems.push(CatalogProblem {
                        path: label,
                        detail: e.to_string(),
                    }),
                }
            }
        }
    }

    /// Chips carry no source marker: overriding a die's properties is a rare
    /// and deliberate act, and the UI has nowhere useful to show the provenance.
    /// Boards do, because a user's own board list is the common case.
    fn absorb_chips(&mut self, text: &str, path: &str) {
        let file: ChipFile = match toml::from_str(text) {
            Ok(file) => file,
            Err(e) => {
                self.problems.push(CatalogProblem {
                    path: path.to_string(),
                    detail: e.to_string(),
                });
                return;
            }
        };
        for entry in file.vendor {
            let vendor = entry.build();
            crate::layers::replace_or_push(&mut self.vendors, vendor, |held, vendor| {
                held.id == vendor.id
            });
        }
        for entry in file.chip {
            self.replace_chip(entry.build());
        }
    }

    /// What every chip carries from its vendor, once every layer is in —
    /// a project's file may add a part from a vendor the built-ins name, or
    /// a vendor and its parts together. A chip naming no vendor anybody
    /// declared is said, not dropped: its parts still detect and flash, and
    /// only the vendor's name and HAL are missing.
    fn resolve(&mut self) {
        for chip in &mut self.chips {
            match self.vendors.iter().find(|v| v.id == chip.vendor) {
                Some(vendor) => {
                    chip.vendor_name = vendor.name.clone();
                    chip.hal_label = vendor.hal.clone();
                }
                None => {
                    chip.vendor_name = chip.vendor.clone();
                    chip.hal_label = None;
                    let detail = format!(
                        "chip `{}` names vendor `{}`, which no [[vendor]] table declares",
                        chip.id, chip.vendor
                    );
                    if !self.problems.iter().any(|p| p.detail == detail) {
                        self.problems.push(CatalogProblem {
                            path: "chips".to_string(),
                            detail,
                        });
                    }
                }
            }
        }
    }

    fn absorb_boards(&mut self, text: &str, path: &str, source: CatalogSource) {
        let file: BoardFile = match toml::from_str(text) {
            Ok(file) => file,
            Err(e) => {
                self.problems.push(CatalogProblem {
                    path: path.to_string(),
                    detail: e.to_string(),
                });
                return;
            }
        };
        for entry in file.board {
            let board = entry.build(source);
            self.replace_board(board);
        }
    }

    fn replace_chip(&mut self, chip: Chip) {
        crate::layers::replace_or_push(&mut self.chips, chip, |held, chip| held.id == chip.id);
    }

    fn replace_board(&mut self, board: Board) {
        crate::layers::replace_or_push(&mut self.boards, board, |held, board| held.id == board.id);
    }
}

impl Default for Catalog {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Accept the spellings that appear in the wild — `ESP32-C3`, `esp32_c3`,
/// `esp32c3` — and normalize to the canonical id.
pub fn normalize(id: &str) -> String {
    id.to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Where a user's own catalogue files live: the configurable data
/// directory, so pointing storage at a synced folder carries the board
/// definitions along with everything else.
fn user_catalog_dir() -> Option<PathBuf> {
    crate::config::data_dir()
}

// ─── file format ─────────────────────────────────────────────────────────────
//
// Separate from `model` on purpose. This is what users write; `model` is what
// the frontend renders. Tying them together would mean a UI change breaking
// everyone's board files.

// `deny_unknown_fields` on the file as well as on each entry: `[[boards]]`
// or `[[chips]]` — the plural, the likeliest typo of all — used to parse as
// a file with nothing in it, and the board simply never appeared.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChipFile {
    #[serde(default)]
    vendor: Vec<VendorEntry>,
    #[serde(default)]
    chip: Vec<ChipEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VendorEntry {
    id: String,
    name: String,
    #[serde(default)]
    hal: Option<String>,
    #[serde(default)]
    chip_crates: Vec<String>,
    #[serde(default)]
    bare_metal_crates: Vec<String>,
    #[serde(default)]
    std_crates: Vec<String>,
}

impl VendorEntry {
    fn build(self) -> Vendor {
        Vendor {
            id: self.id,
            name: self.name,
            hal: self.hal,
            chip_crates: self.chip_crates,
            bare_metal_crates: self.bare_metal_crates,
            std_crates: self.std_crates,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChipEntry {
    id: String,
    name: String,
    vendor: String,
    arch: ArchSpec,
    cores: u8,
    sram_bytes: u32,
    #[serde(default)]
    flash_bytes: Option<u32>,
    bare_metal_target: String,
    #[serde(default)]
    std_target: Option<String>,
    toolchain: ToolchainSpec,
    flashers: Vec<FlasherSpec>,
    #[serde(default)]
    probe_rs_target: Option<String>,
    #[serde(default)]
    radios: Vec<String>,
    #[serde(default)]
    gpio: Vec<u32>,
    /// Optional, and its absence is a refusal rather than a gap: a chip with
    /// no `hal` is one rusty will not offer to switch a project to.
    #[serde(default)]
    hal: Option<String>,
    #[serde(default)]
    ports: Option<PortsEntry>,
    #[serde(default)]
    header: Vec<String>,
    #[serde(default)]
    kit: Option<KitEntry>,
    #[serde(default)]
    emulator: Option<EmulatorEntry>,
    #[serde(default)]
    generator: Option<GeneratorSpec>,
    #[serde(default)]
    std_generator: Option<GeneratorSpec>,
    #[serde(default)]
    gdb: Option<String>,
    #[serde(default)]
    c_compiler: Option<CCompilerEntry>,
    #[serde(default)]
    svd: Option<String>,
}

impl ChipEntry {
    fn build(self) -> Chip {
        Chip {
            id: normalize(&self.id),
            name: self.name,
            vendor: self.vendor,
            // Filled from the vendor's table once every layer is in.
            vendor_name: String::new(),
            hal_label: None,
            arch: self.arch.into(),
            cores: self.cores,
            sram_bytes: self.sram_bytes,
            flash_bytes: self.flash_bytes,
            bare_metal_target: self.bare_metal_target,
            std_target: self.std_target,
            toolchain: self.toolchain.into(),
            flashers: self.flashers.into_iter().map(Into::into).collect(),
            probe_rs_target: self.probe_rs_target,
            radios: self.radios,
            gpio: self.gpio,
            hal: self.hal,
            ports: self.ports.map(|p| Ports {
                width: p.width,
                numbered: p.numbered,
            }),
            header: self.header,
            kit: self.kit.map(KitEntry::build),
            emulation: self.emulator.map(EmulatorEntry::build),
            generator: self.generator.map(Into::into),
            std_generator: self.std_generator.map(Into::into),
            gdb: self.gdb,
            c_compiler: self.c_compiler.map(|c| CCompiler {
                binary: c.binary,
                install: c.install,
            }),
            svd: self.svd,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PortsEntry {
    width: u8,
    #[serde(default)]
    numbered: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KitEntry {
    module: String,
    usb: KitUsbSpec,
    #[serde(default = "default_reset")]
    reset: String,
    #[serde(default = "default_boot")]
    boot: String,
    #[serde(default)]
    rgb: bool,
}

fn default_reset() -> String {
    "RST".to_string()
}

fn default_boot() -> String {
    "BOOT".to_string()
}

impl KitEntry {
    fn build(self) -> Kit {
        Kit {
            module: self.module,
            usb: match self.usb {
                KitUsbSpec::MicroB => KitUsb::MicroB,
                KitUsbSpec::TypeC => KitUsb::TypeC,
                KitUsbSpec::DualTypeC => KitUsb::DualTypeC,
            },
            reset: self.reset,
            boot: self.boot,
            rgb: self.rgb,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum KitUsbSpec {
    MicroB,
    TypeC,
    DualTypeC,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmulatorEntry {
    kind: EmulatorKindSpec,
    #[serde(default)]
    binary: Option<String>,
    #[serde(default)]
    limit: Option<String>,
    #[serde(default)]
    limit_outdated: Option<String>,
}

impl EmulatorEntry {
    fn build(self) -> Emulation {
        Emulation {
            kind: match self.kind {
                EmulatorKindSpec::Qemu => EmulatorKind::Qemu,
                EmulatorKindSpec::RustyMcu => EmulatorKind::RustyMcu,
            },
            binary: self.binary,
            limit: self.limit,
            limit_outdated: self.limit_outdated,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EmulatorKindSpec {
    Qemu,
    RustyMcu,
}

/// `"esp-generate"`, `"esp-idf-template"`, or `{ template = "<name>" }`.
#[derive(Deserialize)]
#[serde(untagged)]
enum GeneratorSpec {
    Named(NamedGenerator),
    Template { template: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum NamedGenerator {
    EspGenerate,
    EspIdfTemplate,
}

impl From<GeneratorSpec> for Generator {
    fn from(spec: GeneratorSpec) -> Self {
        match spec {
            GeneratorSpec::Named(NamedGenerator::EspGenerate) => Generator::EspGenerate,
            GeneratorSpec::Named(NamedGenerator::EspIdfTemplate) => Generator::EspIdfTemplate,
            GeneratorSpec::Template { template } => Generator::Template { name: template },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CCompilerEntry {
    binary: String,
    install: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardFile {
    #[serde(default)]
    board: Vec<BoardEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardEntry {
    id: String,
    name: String,
    chip: String,
    #[serde(default)]
    flash_bytes: Option<u32>,
    #[serde(default)]
    psram_bytes: Option<u32>,
    #[serde(default)]
    usb: Vec<UsbEntry>,
    #[serde(default)]
    flash_baud: Option<u32>,
    /// Free-form `name = gpio` pairs, so a board can declare whatever pins
    /// matter to it without the schema growing a field per peripheral.
    #[serde(default)]
    pins: std::collections::BTreeMap<String, u32>,
}

impl BoardEntry {
    fn build(self, source: CatalogSource) -> Board {
        Board {
            id: self.id,
            name: self.name,
            chip: normalize(&self.chip),
            flash_bytes: self.flash_bytes,
            psram_bytes: self.psram_bytes,
            usb: self
                .usb
                .into_iter()
                .map(|u| UsbMatch {
                    vendor_id: u.vendor_id,
                    product_id: u.product_id,
                    note: u.note,
                })
                .collect(),
            flash_baud: self.flash_baud,
            pins: self
                .pins
                .into_iter()
                .map(|(name, gpio)| PinAssignment { name, gpio })
                .collect(),
            source,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsbEntry {
    vendor_id: u16,
    product_id: u16,
    #[serde(default)]
    note: Option<String>,
}

// Kebab-case in files, because that is how these read as configuration; the
// wire enums stay camelCase for the frontend.

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ArchSpec {
    Xtensa,
    RiscV,
    CortexM,
}

impl From<ArchSpec> for Arch {
    fn from(spec: ArchSpec) -> Self {
        match spec {
            ArchSpec::Xtensa => Arch::Xtensa,
            ArchSpec::RiscV => Arch::RiscV,
            ArchSpec::CortexM => Arch::CortexM,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ToolchainSpec {
    Stock,
    EspXtensa,
    NightlyBuildStd,
}

impl From<ToolchainSpec> for ToolchainRequirement {
    fn from(spec: ToolchainSpec) -> Self {
        match spec {
            ToolchainSpec::Stock => ToolchainRequirement::Stock,
            ToolchainSpec::EspXtensa => ToolchainRequirement::EspXtensa,
            ToolchainSpec::NightlyBuildStd => ToolchainRequirement::NightlyBuildStd,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FlasherSpec {
    Espflash,
    ProbeRs,
    Wlink,
}

impl From<FlasherSpec> for Flasher {
    fn from(spec: FlasherSpec) -> Self {
        match spec {
            FlasherSpec::Espflash => Flasher::Espflash,
            FlasherSpec::ProbeRs => Flasher::ProbeRs,
            FlasherSpec::Wlink => Flasher::Wlink,
        }
    }
}
