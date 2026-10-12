//! The math toolbox's sheet: read from the project, saved as it is typed,
//! and fed what a running simulation says.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_embed::spatial::sheet::{EXAMPLES, Live, MathSheet};
use rusty_embed::spatial::{Frame, Quat};

use crate::{
    ipc::{self, cmd},
    scene::{Camera, Preset},
    state::{AppState, after_removal, insert_row, reads_live, remove_row},
};

/// How long typing has to pause before the sheet is written.
const SAVE_AFTER_MS: u64 = 600;

/// The open project's sheet, read once per project. With no project the
/// sheet in the window is kept as it is; a project with no sheet yet keeps
/// it too, and it is written there the first time it is edited. One that
/// does not read is said and never saved over.
pub fn load_math_sheet(state: AppState) {
    let root = state
        .project
        .detected
        .with_untracked(|p| p.as_ref().map(|p| p.root.clone()));
    if root == state.math.home.get_untracked() {
        return;
    }
    let Some(root) = root else {
        state.math.home.set(None);
        state.math.unreadable.set(None);
        return;
    };
    if !ipc::backend_available() {
        return;
    }
    spawn_local(async move {
        match ipc::get::<Option<MathSheet>>(cmd::math::LOAD).await {
            Ok(found) => {
                // Moved on to another project while this was read.
                if state
                    .project
                    .detected
                    .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
                    .as_deref()
                    != Some(root.as_str())
                {
                    return;
                }
                if let Some(sheet) = found {
                    show(state, sheet);
                }
                state.math.unreadable.set(None);
                state.math.home.set(Some(root));
            }
            Err(error) => {
                state.math.home.set(None);
                state.math.unreadable.set(Some(error.message));
            }
        }
    });
}

/// Put a sheet on screen: its rows all shown, nothing selected, the camera
/// where its frame's chase view is.
fn show(state: AppState, sheet: MathSheet) {
    state.math.hidden.set(vec![false; sheet.rows.len()]);
    state.math.selected.set(None);
    state.math.step.set(None);
    state.math.progress.set(1.0);
    state
        .math
        .camera
        .set(Camera::preset(Preset::Chase, sheet.frame));
    state.math.sheet.set(sheet);
}

/// One row's text changed.
pub fn set_math_row(state: AppState, index: usize, text: String) {
    let changed = state
        .math
        .sheet
        .try_update(|sheet| match sheet.rows.get_mut(index) {
            Some(row) if *row != text => {
                *row = text;
                true
            }
            _ => false,
        });
    if changed == Some(true) {
        edited(state);
    }
}

/// A new, empty row after `after` (at the end for `None`); answers where.
pub fn add_math_row(state: AppState, after: Option<usize>) -> usize {
    let mut at = 0;
    state.math.sheet.update(|sheet| {
        at = after.map_or(sheet.rows.len(), |i| i + 1);
        state
            .math
            .hidden
            .update(|hidden| insert_row(&mut sheet.rows, hidden, at, String::new()));
        at = at.min(sheet.rows.len() - 1);
    });
    state.math.selected.set(Some(at));
    state.math.step.set(None);
    edited(state);
    at
}

pub fn remove_math_row(state: AppState, index: usize) {
    let mut left = 0;
    state.math.sheet.update(|sheet| {
        state
            .math
            .hidden
            .update(|hidden| remove_row(&mut sheet.rows, hidden, index));
        left = sheet.rows.len();
    });
    state
        .math
        .selected
        .update(|selected| *selected = after_removal(*selected, index, left));
    state.math.step.set(None);
    edited(state);
}

pub fn toggle_math_row(state: AppState, index: usize) {
    state.math.hidden.update(|hidden| {
        if hidden.len() <= index {
            hidden.resize(index + 1, false);
        }
        hidden[index] = !hidden[index];
    });
}

pub fn select_math_row(state: AppState, index: Option<usize>) {
    if state.math.selected.get_untracked() != index {
        state.math.selected.set(index);
        state.math.step.set(None);
        state.math.playing.set(false);
        state.math.progress.set(1.0);
    }
}

/// Which way the world's Z points, for this sheet — and the camera goes
/// where that frame's chase view is, since up has changed ends.
pub fn set_math_frame(state: AppState, frame: Frame) {
    if state.math.sheet.with_untracked(|s| s.frame) == frame {
        return;
    }
    state.math.sheet.update(|sheet| sheet.frame = frame);
    state.math.camera.set(Camera::preset(Preset::Chase, frame));
    edited(state);
}

/// Replace the sheet with an example, asking first when that would lose
/// rows nobody else has.
pub fn open_math_example(state: AppState, id: &'static str) {
    let Some(example) = EXAMPLES.iter().find(|e| e.id == id) else {
        return;
    };
    let current = state.math.sheet.get_untracked();
    let own_work = current.rows.iter().any(|r| !r.trim().is_empty())
        && !EXAMPLES.iter().any(|e| e.sheet().rows == current.rows);
    spawn_local(async move {
        if own_work && !ipc::confirm(&rusty_i18n::t!("math.replace-sheet")).await {
            return;
        }
        show(state, example.sheet());
        edited(state);
    });
}

/// Save once the typing stops — only to a project whose sheet was read, so
/// a file that does not read is never written over.
fn edited(state: AppState) {
    state.math.edits.update(|n| *n += 1);
    let edits = state.math.edits.get_untracked();
    let Some(_) = state.math.home.get_untracked() else {
        return;
    };
    set_timeout(
        move || {
            if state.math.edits.try_get_untracked() != Some(edits) {
                return;
            }
            save(state);
        },
        std::time::Duration::from_millis(SAVE_AFTER_MS),
    );
}

fn save(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        sheet: MathSheet,
    }
    if state.math.home.get_untracked().is_none() || !ipc::backend_available() {
        return;
    }
    let args = Args {
        sheet: state.math.sheet.get_untracked(),
    };
    spawn_local(async move {
        if let Err(error) = ipc::call::<_, ()>(cmd::math::SAVE, &args).await {
            state.app.error.set(Some(error));
        }
    });
}

/// What the firmware last printed on every channel, and the plant while
/// the Flight tab closes the loop — for a sheet that reads either.
pub fn refresh_math_live(state: AppState) {
    if !state.math.sheet.with_untracked(reads_live) {
        return;
    }
    let live = math_live(state);
    if state.math.live.with_untracked(|old| *old != live) {
        state.math.live.set(live);
    }
}

/// What a running simulation offers a sheet right now: the newest value on
/// every telemetry channel, and the plant's attitude and rates while the
/// Flight tab closes the loop. The panel's live rows read it, and so does
/// the assistant's `math_sheet`, sent with the question.
pub fn math_live(state: AppState) -> Live {
    let channels = state.sim.plot.with_untracked(|plot| {
        plot.channels
            .iter()
            .filter_map(|(name, samples)| {
                samples.last().map(|(_, v)| (name.clone(), f64::from(*v)))
            })
            .collect()
    });
    let closed = state.sim.plant_closed.get_untracked();
    let (truth, truth_rate) = if closed {
        state.sim.plant.with_untracked(|plant| {
            let q = plant.orientation();
            let r = plant.rate();
            (
                Some(Quat::new(
                    f64::from(q.w),
                    f64::from(q.x),
                    f64::from(q.y),
                    f64::from(q.z),
                )),
                Some(rusty_embed::spatial::Vec3::new(
                    f64::from(r[0]),
                    f64::from(r[1]),
                    f64::from(r[2]),
                )),
            )
        })
    } else {
        (None, None)
    };
    Live {
        channels,
        truth,
        truth_rate,
    }
}
