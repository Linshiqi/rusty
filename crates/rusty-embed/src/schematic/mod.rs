//! Schematic symbols: the drawing of a part with its pins, read from KiCad's
//! own library format and from EasyEDA's answer for an LCSC part number.
//!
//! `docs/schematic.md` is the design. The model (`Symbol`, `Pin`,
//! `Graphic`) lives in `model::symbol` so the frontend can draw one; this
//! module is the reading, which is backend-only because it touches files
//! and the network.
//!
//! Three layers, later ones winning by id: the built-in library
//! (`data/symbols/*.kicad_sym`, compiled in), the data directory's
//! `symbols/` (where imported LCSC parts are cached), and the project's
//! `.rusty/symbols/`. A file that does not parse is named in `warnings` and
//! skipped — refuse rather than guess, applied to a library: a half-read
//! symbol would draw and never wire.

pub mod easyeda;
pub mod kicad_out;
pub mod kicad_sch;
pub mod kicad_sym;
pub mod place;
pub(crate) mod sexpr;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::Symbol;

/// The built-in library, one file per KiCad library name.
const BUILTIN: &[(&str, &str)] = &[
    (
        "Device",
        include_str!("../../data/symbols/Device.kicad_sym"),
    ),
    // The parts the simulator gives a behaviour of its own: a pot, an analog
    // source, a text display, an RGB lens, a digit, a motor.
    ("rusty", include_str!("../../data/symbols/rusty.kicad_sym")),
];

/// Every symbol the sheet may place, with what could not be read.
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub symbols: Vec<Symbol>,
    /// One line per file that could not be read, naming the file and why.
    pub warnings: Vec<String>,
}

impl Library {
    /// A symbol by `library:name`, or by bare name when the caller does not
    /// say which library — the first match in layer order.
    pub fn find(&self, id: &str) -> Option<&Symbol> {
        match id.split_once(':') {
            Some((library, name)) => self
                .symbols
                .iter()
                .find(|s| s.library == library && s.name == name),
            None => self.symbols.iter().find(|s| s.name == id),
        }
    }

    /// Add `symbols`, each replacing an earlier one with the same id.
    fn extend(&mut self, symbols: Vec<Symbol>) {
        for symbol in symbols {
            let id = symbol.id();
            match self.symbols.iter_mut().find(|s| s.id() == id) {
                Some(slot) => *slot = symbol,
                None => self.symbols.push(symbol),
            }
        }
    }

    /// Read every `*.kicad_sym` in `dir`, the file stem naming the library.
    fn read_dir(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "kicad_sym"))
            .collect();
        files.sort();
        for file in files {
            let library = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("local")
                .to_string();
            match std::fs::read_to_string(&file) {
                Ok(text) => match kicad_sym::parse(&library, &text) {
                    Ok(symbols) => self.extend(symbols),
                    Err(error) => self.warnings.push(format!("{}: {error}", file.display())),
                },
                Err(error) => self.warnings.push(format!("{}: {error}", file.display())),
            }
        }
    }
}

/// The compiled-in symbols alone.
pub fn builtin() -> Library {
    let mut library = Library::default();
    for (name, text) in BUILTIN {
        match kicad_sym::parse(name, text) {
            Ok(symbols) => library.extend(symbols),
            // The built-in file is checked by a test; this is the message
            // for the release that shipped one anyway.
            Err(error) => library
                .warnings
                .push(format!("built-in library {name}: {error}")),
        }
    }
    library
}

/// The directory imported parts are cached in.
pub fn cache_dir() -> Option<PathBuf> {
    crate::config::data_dir().map(|d| d.join("symbols"))
}

/// Built-in, then the data directory's cache, then the project's own.
pub fn load(project: Option<&Path>) -> Library {
    let mut library = builtin();
    if let Some(cache) = cache_dir() {
        library.read_dir(&cache);
    }
    if let Some(root) = project {
        library.read_dir(&root.join(".rusty").join("symbols"));
    }
    library
}

/// Read a `.kicad_sch` into a project.
///
/// **The symbols travel with it.** A schematic carries a copy of every
/// symbol it places, and those copies are the only place some of them
/// exist — a part from a library the user has and rusty does not. So they
/// are written into the project's own `.rusty/symbols/`, one file per KiCad
/// library, which is where the loader looks and which wins by
/// `library:name` rather than by file: a `Device.kicad_sym` holding one
/// imported lamp shadows `Device:LED` and leaves every other built-in
/// alone. Without this the sheet would draw once and come back as a row of
/// unknown boxes the next time the project was opened.
pub fn import(root: &Path, file: &Path, chip: &str) -> crate::error::Result<crate::model::Sheet> {
    let text = read_file(file)?;
    let read = kicad_sch::parse(&text, chip).map_err(|error| crate::error::Error::Refused {
        detail: format!(
            "{} is not a schematic rusty can read: {error}",
            file.display()
        ),
    })?;
    // Everything the crossing could not carry goes in the sheet's own
    // `notes`, which is already the channel for "what loading wanted read"
    // and is already shown. A second type saying the same thing would be a
    // second place to forget to look.
    let mut sheet = read.sheet;
    let mut notes = read.notes;

    let mut by_library: BTreeMap<String, Vec<Symbol>> = BTreeMap::new();
    for symbol in &sheet.symbols {
        by_library
            .entry(symbol.library.clone())
            .or_default()
            .push(symbol.clone());
    }
    if !by_library.is_empty() {
        let dir = root.join(".rusty").join("symbols");
        make_dir(&dir)?;
        for (library, symbols) in &by_library {
            let name = if library.is_empty() {
                "imported"
            } else {
                library
            };
            write_file(
                &dir.join(format!("{name}.kicad_sym")),
                &kicad_sym::write(symbols),
            )?;
        }
        notes.push(format!(
            "{} symbol(s) were written into .rusty/symbols/ so the sheet still \
             draws when the project is reopened",
            sheet.symbols.len()
        ));
    }

    // The one thing a KiCad file cannot bring with it, said plainly rather
    // than discovered when Run does nothing: rusty simulates through the
    // devkit, whose header rows *are* the GPIOs, and a schematic drawn
    // elsewhere has a microcontroller of its own instead.
    let touches_kit = sheet.wires.iter().any(|w| {
        w.from.part == crate::model::KIT_REFERENCE || w.to.part == crate::model::KIT_REFERENCE
    });
    if !touches_kit {
        notes.push(
            "nothing here is wired to the devkit U1, so this sheet can be drawn \
             and checked but not simulated: the emulator drives pins through \
             U1's rows, and a part reaches a GPIO by being wired to one"
                .to_string(),
        );
    }
    sheet.notes.append(&mut notes);
    Ok(sheet)
}

/// Write a sheet out as `.kicad_sch`, patching the file that is there.
///
/// The original is re-read rather than remembered, which is what keeps this
/// stateless and what makes the promise hold at the only moment it matters:
/// what is on disk now is what gets patched, whoever last wrote it.
pub fn export(file: &Path, sheet: &crate::model::Sheet) -> crate::error::Result<Vec<String>> {
    let original = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| kicad_sch::parse(&text, &sheet.chip).ok());
    let written = kicad_out::write(sheet, original.as_ref());
    if let Some(dir) = file.parent() {
        make_dir(dir)?;
    }
    write_file(file, &written.text)?;
    Ok(written.notes)
}

fn read_file(path: &Path) -> crate::error::Result<String> {
    std::fs::read_to_string(path).map_err(|source| crate::error::Error::Read {
        path: path.display().to_string(),
        source,
    })
}

fn write_file(path: &Path, text: &str) -> crate::error::Result<()> {
    std::fs::write(path, text).map_err(|source| crate::error::Error::Write {
        path: path.display().to_string(),
        source,
    })
}

fn make_dir(path: &Path) -> crate::error::Result<()> {
    std::fs::create_dir_all(path).map_err(|source| crate::error::Error::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole crossing, both ways, on a real project directory: read a
    /// file, put it back, and find the bytes unchanged — and find the
    /// symbols where the next open will look for them.
    #[test]
    fn a_schematic_comes_in_with_its_symbols_and_goes_back_out_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("board.kicad_sch");
        let source = include_str!("../../tests/fixtures/kicad/lamp.kicad_sch");
        std::fs::write(&file, source).unwrap();

        let brought = import(dir.path(), &file, "esp32c3").expect("imported");
        assert_eq!(brought.parts.len(), 3);
        assert!(
            brought.notes.iter().any(|n| n.contains("devkit U1")),
            "the sheet cannot be simulated and says so: {:?}",
            brought.notes
        );

        // The symbols are where the loader looks, under their own library
        // names, so reopening the project draws the same parts.
        let symbols = dir.path().join(".rusty").join("symbols");
        assert!(symbols.join("Device.kicad_sym").is_file());
        assert!(symbols.join("power.kicad_sym").is_file());
        let library = load(Some(dir.path()));
        assert!(
            library.find("power:GND").is_some(),
            "{:?}",
            library.warnings
        );
        assert!(
            library.find("Device:R").is_some(),
            "and the built-ins the imported file did not mention are still there"
        );

        let notes = export(&file, &brought).expect("exported");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            source,
            "a round trip nobody edited is the identity"
        );
    }

    #[test]
    fn exporting_where_there_is_no_file_writes_a_whole_one() {
        let dir = tempfile::tempdir().unwrap();
        let source = include_str!("../../tests/fixtures/kicad/lamp.kicad_sch");
        let read = kicad_sch::parse(source, "esp32c3").expect("parsed");
        let file = dir.path().join("new").join("board.kicad_sch");

        let notes = export(&file, &read.sheet).expect("exported");
        assert!(
            notes.iter().any(|n| n.contains("from scratch")),
            "{notes:?}"
        );
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.starts_with("(kicad_sch"));
        assert_eq!(
            kicad_sch::parse(&written, "esp32c3")
                .expect("what we wrote parses")
                .sheet
                .parts
                .len(),
            3
        );
    }

    #[test]
    fn the_builtin_library_reads_clean_and_answers_by_id_or_name() {
        let library = builtin();
        assert!(library.warnings.is_empty(), "{:?}", library.warnings);
        assert_eq!(
            library.find("Device:LED").map(|s| s.reference.as_str()),
            Some("D")
        );
        assert_eq!(library.find("SW_Push").map(|s| s.pins.len()), Some(2));
        assert!(library.find("Device:NPN").is_none());
        assert!(
            library.find("Other:R").is_none(),
            "a library name is not ignored"
        );
    }

    #[test]
    fn a_project_library_overrides_by_id_and_a_broken_file_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let symbols = dir.path().join(".rusty").join("symbols");
        std::fs::create_dir_all(&symbols).unwrap();
        std::fs::write(
            symbols.join("Device.kicad_sym"),
            r#"(kicad_symbol_lib (symbol "R" (property "Reference" "R" (at 0 0 0)) (property "Value" "R_mine" (at 0 0 0))
                (symbol "R_1_1" (pin passive line (at 0 2.54 270) (length 2.54) (name "~") (number "1"))
                                (pin passive line (at 0 -2.54 90) (length 2.54) (name "~") (number "2")))))"#,
        )
        .unwrap();
        std::fs::write(
            symbols.join("Broken.kicad_sym"),
            "(kicad_symbol_lib (symbol",
        )
        .unwrap();

        let mut library = builtin();
        library.read_dir(&symbols);
        assert_eq!(
            library.find("Device:R").map(|s| s.value.as_str()),
            Some("R_mine")
        );
        assert_eq!(
            library.symbols.iter().filter(|s| s.name == "R").count(),
            1,
            "replaced, not added beside"
        );
        assert_eq!(library.warnings.len(), 1);
        assert!(
            library.warnings[0].contains("Broken.kicad_sym"),
            "{}",
            library.warnings[0]
        );
        assert!(
            library.find("Device:LED").is_some(),
            "the rest of the built-ins stay"
        );
    }
}
