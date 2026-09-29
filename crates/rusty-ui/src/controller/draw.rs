//! What a program draws with `rusty-draw`: the `[rusty:draw]` lines
//! `absorb` hands over, gathered into scenes for the Draw tab.
//!
//! Whatever stream they arrive on — a test run from its lens, an example,
//! the simulator's console, a board's port — they are read the same way,
//! because `absorb` is the one reader of every stream.

use leptos::prelude::*;

use rusty_embed::draw::{self, DrawLine, Sketch};

use crate::state::{AppState, DockTab};

/// One line of a drawing. A scene is shown when it ends, not mark by mark:
/// a half-drawn scene is a picture of nothing in particular, and a firmware
/// drawing every loop would redraw the tab once per line.
pub(super) fn drawn(state: AppState, line: DrawLine) {
    if let Some(sketch) = state
        .draw
        .book
        .try_update_value(|book| book.read(line))
        .flatten()
    {
        file_sketch(state, sketch);
    }
}

/// The run ended. A scene it began and never ended is shown as far as it
/// got — a test that panicked mid-scene drew what it drew — and the next
/// run's first scene may bring the tab forward again.
pub(super) fn drawing_ends(state: AppState) {
    if let Some(sketch) = state
        .draw
        .book
        .try_update_value(|book| book.close())
        .flatten()
    {
        file_sketch(state, sketch);
    }
    state.draw.fronted.set_value(false);
}

fn file_sketch(state: AppState, sketch: Sketch) {
    let title = sketch.title.clone();
    state.draw.sketches.update(|sketches| {
        draw::file(sketches, sketch);
    });
    state.draw.latest.set(Some(title));
    // The first scene of a run brings the tab forward — drawing is what the
    // program was run for — and every later one leaves the dock alone.
    if state.draw.fronted.get_value() {
        state.reveal_tab(DockTab::Draw);
    } else {
        state.draw.fronted.set_value(true);
        state.show_dock(DockTab::Draw);
    }
}

/// Show one scene from the list; `None` follows the newest again.
pub fn choose_sketch(state: AppState, title: Option<String>) {
    state.draw.chosen.set(title);
    state.draw.hovered.set(None);
}

/// Forget every scene drawn so far.
pub fn clear_drawings(state: AppState) {
    state.draw.sketches.set(Vec::new());
    state.draw.latest.set(None);
    state.draw.chosen.set(None);
    state.draw.hovered.set(None);
    state.draw.fitted.set_value(None);
}
