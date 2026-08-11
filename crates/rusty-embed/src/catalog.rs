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
    Arch, Board, CatalogSource, Chip, Flasher, PinAssignment, ToolchainRequirement, UsbMatch,
    Vendor,
};

const BUILTIN_CHIPS: &str = include_str!("../data/chips.toml");
const BUILTIN_BOARDS: &str = include_str!("../data/boards.toml");

/// Everything rusty knows about hardware, after layering.
#[derive(Debug, Clone)]
pub struct Catalog {
    chips: Vec<Chip>,
    boards: Vec<Board>,
    /// Files that failed to parse, with the reason.
    ///
    /// Kept rather than thrown: one malformed user file must not blank out the
    /// catalogue, and silently ignoring it would leave the user staring at a
    /// board that never appears.
    problems: Vec<CatalogProblem>,
}

#[derive(Debug, Clone)]
pub struct CatalogProblem {
    pub path: String,
    pub detail: String,
}

impl Catalog {
    /// Only what ships in the binary. Pure and deterministic — this is what the
    /// tests and the free functions in [`crate::chip`] use.
    pub fn builtin() -> Self {
        let mut catalog = Catalog {
            chips: Vec::new(),
            boards: Vec::new(),
            problems: Vec::new(),
        };
        // A malformed built-in file is a build-time mistake, so failing loudly
        // here is right — but only in debug, so a shipped binary degrades to an
        // empty catalogue rather than refusing to start.
        catalog.absorb_chips(BUILTIN_CHIPS, "<builtin>/chips.toml");
        catalog.absorb_boards(BUILTIN_BOARDS, "<builtin>/boards.toml", CatalogSource::Builtin);
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
        catalog
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
            let path = dir.join(subdir);
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .collect();
            // Sorted so a directory of files layers deterministically; without
            // this, which of two conflicting definitions wins depends on the
            // filesystem.
            files.sort();

            for file in files {
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
        for entry in file.chip {
            self.replace_chip(entry.build());
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
        match self.chips.iter_mut().find(|c| c.id == chip.id) {
            Some(existing) => *existing = chip,
            None => self.chips.push(chip),
        }
    }

    fn replace_board(&mut self, board: Board) {
        match self.boards.iter_mut().find(|b| b.id == board.id) {
            Some(existing) => *existing = board,
            None => self.boards.push(board),
        }
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

/// Where a user's own catalogue files live.
///
/// Resolved from the environment rather than a crate, to keep the dependency
/// list short: this is two lookups, and a wrong answer degrades to "no user
/// files" rather than breaking anything.
fn user_catalog_dir() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("RUSTY_CONFIG_DIR") {
        return Some(PathBuf::from(explicit));
    }
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("rusty"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(base).join("rusty"));
        }
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".config").join("rusty"))
    }
}

// ─── file format ─────────────────────────────────────────────────────────────
//
// Separate from `model` on purpose. This is what users write; `model` is what
// the frontend renders. Tying them together would mean a UI change breaking
// everyone's board files.

#[derive(Deserialize)]
struct ChipFile {
    #[serde(default)]
    chip: Vec<ChipEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChipEntry {
    id: String,
    name: String,
    vendor: VendorSpec,
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
}

impl ChipEntry {
    fn build(self) -> Chip {
        Chip {
            id: normalize(&self.id),
            name: self.name,
            vendor: self.vendor.into(),
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
        }
    }
}

#[derive(Deserialize)]
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
enum VendorSpec {
    Espressif,
    St,
}

impl From<VendorSpec> for Vendor {
    fn from(spec: VendorSpec) -> Self {
        match spec {
            VendorSpec::Espressif => Vendor::Espressif,
            VendorSpec::St => Vendor::St,
        }
    }
}

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
}

impl From<ToolchainSpec> for ToolchainRequirement {
    fn from(spec: ToolchainSpec) -> Self {
        match spec {
            ToolchainSpec::Stock => ToolchainRequirement::Stock,
            ToolchainSpec::EspXtensa => ToolchainRequirement::EspXtensa,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum FlasherSpec {
    Espflash,
    ProbeRs,
}

impl From<FlasherSpec> for Flasher {
    fn from(spec: FlasherSpec) -> Self {
        match spec {
            FlasherSpec::Espflash => Flasher::Espflash,
            FlasherSpec::ProbeRs => Flasher::ProbeRs,
        }
    }
}
