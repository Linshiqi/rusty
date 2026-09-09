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

#[cfg(test)]
mod tests {
    use super::*;

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
