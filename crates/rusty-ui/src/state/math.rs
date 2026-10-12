//! The math toolbox: its sheet, what is selected in it, and the camera it is
//! looked at through (`view/panels/math/`).

use leptos::prelude::*;

use rusty_embed::spatial::sheet::{EXAMPLES, Live, MathSheet};

use crate::scene::{Camera, Preset};

#[derive(Clone, Copy)]
pub struct Math {
    /// The rows as typed, and the frame they are written in.
    pub sheet: RwSignal<MathSheet>,
    /// The project root the sheet was read from and is saved to. `None`
    /// with no project open, when the sheet lives only in the window.
    pub home: RwSignal<Option<String>>,
    /// The project's sheet is there and does not read: said above the rows,
    /// and nothing is saved over it until it is mended.
    pub unreadable: RwSignal<Option<String>>,
    /// Rows left out of the drawing, by position.
    pub hidden: RwSignal<Vec<bool>>,
    pub selected: RwSignal<Option<usize>>,
    /// The step of the selected row being read: its shapes drawn in full,
    /// the others' dimmed.
    pub step: RwSignal<Option<usize>>,
    pub camera: RwSignal<Camera>,
    /// The plane's view rather than space's; `None` follows the selected
    /// row.
    pub flat: RwSignal<Option<bool>>,
    /// How far through the selected row's turns the aircraft is drawn, from
    /// 0 (where they start) to 1 (where they end).
    pub progress: RwSignal<f64>,
    pub playing: RwSignal<bool>,
    /// What a running simulation offers the sheet, refreshed on a timer
    /// while a row reads it.
    pub live: RwSignal<Live>,
    /// Counts edits, so a save waits for the typing to stop.
    pub edits: RwSignal<u64>,
    /// The function reference, open over the rows.
    pub help: RwSignal<bool>,
}

impl Math {
    pub fn fresh() -> Self {
        let first = EXAMPLES[0].sheet();
        Math {
            hidden: RwSignal::new(vec![false; first.rows.len()]),
            camera: RwSignal::new(Camera::preset(Preset::Chase, first.frame)),
            sheet: RwSignal::new(first),
            home: RwSignal::new(None),
            unreadable: RwSignal::new(None),
            selected: RwSignal::new(None),
            step: RwSignal::new(None),
            flat: RwSignal::new(None),
            progress: RwSignal::new(1.0),
            playing: RwSignal::new(false),
            live: RwSignal::new(Live::default()),
            edits: RwSignal::new(0),
            help: RwSignal::new(false),
        }
    }
}

/// A row put in at `at`, its visibility beside it, so a flag never
/// describes the row that used to be there.
pub fn insert_row(rows: &mut Vec<String>, hidden: &mut Vec<bool>, at: usize, text: String) {
    hidden.resize(rows.len(), false);
    let at = at.min(rows.len());
    rows.insert(at, text);
    hidden.insert(at, false);
}

/// A row taken out, and its flag with it.
pub fn remove_row(rows: &mut Vec<String>, hidden: &mut Vec<bool>, at: usize) {
    hidden.resize(rows.len(), false);
    if at < rows.len() {
        rows.remove(at);
        hidden.remove(at);
    }
}

/// The selection once the row at `removed` has gone: the row that was
/// selected wherever it moved to, the one above a removed selection, or
/// none when the sheet emptied.
pub fn after_removal(selected: Option<usize>, removed: usize, rows_left: usize) -> Option<usize> {
    let selected = selected?;
    if rows_left == 0 {
        return None;
    }
    Some(if selected > removed {
        selected - 1
    } else if selected == removed {
        removed.saturating_sub(1).min(rows_left - 1)
    } else {
        selected
    })
}

/// Whether any row reads the firmware or the plant — asked of the text, so
/// the live values are refreshed only for a sheet that will look at them.
pub fn reads_live(sheet: &MathSheet) -> bool {
    sheet
        .rows
        .iter()
        .any(|row| row.contains("tel(") || row.contains("truth"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_visibility_moves_with_it() {
        let mut rows = vec!["a = 1".to_string(), "b = 2".to_string()];
        let mut hidden = vec![false, true];
        insert_row(&mut rows, &mut hidden, 1, "c = 3".into());
        assert_eq!(rows, ["a = 1", "c = 3", "b = 2"]);
        assert_eq!(hidden, [false, false, true]);
        remove_row(&mut rows, &mut hidden, 0);
        assert_eq!(rows, ["c = 3", "b = 2"]);
        assert_eq!(hidden, [false, true]);
        // Past the end is the end; a flag list that fell short is mended.
        let mut short = vec![];
        insert_row(&mut rows, &mut short, 99, "d = 4".into());
        assert_eq!(rows.last().unwrap(), "d = 4");
        assert_eq!(short.len(), rows.len());
    }

    #[test]
    fn the_selection_follows_its_row_through_a_removal() {
        assert_eq!(after_removal(Some(3), 1, 4), Some(2));
        assert_eq!(after_removal(Some(1), 3, 4), Some(1));
        assert_eq!(after_removal(Some(2), 2, 4), Some(1));
        assert_eq!(after_removal(Some(0), 0, 3), Some(0));
        assert_eq!(after_removal(Some(0), 0, 0), None);
        assert_eq!(after_removal(None, 0, 3), None);
    }

    #[test]
    fn a_sheet_reads_live_when_a_row_names_the_firmware_or_the_plant() {
        let sheet = |rows: &[&str]| MathSheet {
            frame: Default::default(),
            rows: rows.iter().map(|r| r.to_string()).collect(),
        };
        assert!(reads_live(&sheet(&["q = euler(tel(\"roll\"), 0, 0)"])));
        assert!(reads_live(&sheet(&["p = truth()"])));
        assert!(!reads_live(&sheet(&["q = euler(10°, 0, 0)"])));
    }
}
