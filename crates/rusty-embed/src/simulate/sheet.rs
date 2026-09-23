//! The board a project's `.rusty/sim.toml` describes, with its symbols
//! resolved and the devkit's header read from the catalogue.

use std::path::Path;

use crate::model::Sheet;
use crate::nets::Row;
use crate::schematic::Library;

/// The sheet a project's `.rusty/sim.toml` describes, with its symbols
/// attached — the board as the Simulate panel gets it, for a test outside
/// this module that has to read one.
#[cfg(test)]
pub(crate) fn load_board_for_test(root: &Path, chip: &str) -> Option<Sheet> {
    let library = crate::schematic::load(Some(root));
    super::board_file::load(root, chip).map(|loaded| {
        let mut sheet = loaded.sheet;
        resolve_symbols(&mut sheet, &library);
        sheet
    })
}

/// Attach to the sheet every symbol its parts use, so the frontend draws
/// without a second lookup. A part whose symbol no library has is kept —
/// deleting somebody's part because a library file went missing is a loss,
/// not a repair — and named in the sheet's notes; the editor draws it as
/// the unknown it is.
pub fn resolve_symbols(sheet: &mut Sheet, library: &Library) {
    let mut missing: Vec<String> = Vec::new();
    for part in &sheet.parts {
        if sheet.symbols.iter().any(|s| s.id() == part.symbol) {
            continue;
        }
        match library.find(&part.symbol) {
            Some(symbol) => sheet.symbols.push(symbol.clone()),
            None => missing.push(format!("{} ({})", part.reference, part.symbol)),
        }
    }
    if !missing.is_empty() {
        sheet.notes.push(format!(
            "no symbol library has: {} — import the part from LCSC in the library panel, put a \
             .kicad_sym with it under .rusty/symbols/, or change the part's symbol",
            missing.join(", ")
        ));
    }
}

/// The devkit's header rows for the project's chip, from the catalogue —
/// the same rows the frontend draws, so a button's GPIO is read off the
/// same header on both sides.
pub fn kit_rows_for(root: &Path, chip: &str) -> Vec<Row> {
    let gpio = crate::catalog::Catalog::load(Some(root))
        .chips()
        .iter()
        .find(|c| c.id == chip)
        .map(|c| c.gpio.clone())
        .unwrap_or_default();
    crate::nets::kit_rows(chip, &gpio)
}
