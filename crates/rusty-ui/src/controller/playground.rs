//! The playground: a project rusty keeps per chip for trying things —
//! Wokwi's new project, with no folder to choose, no generator to run and no
//! account. The backend writes it (`rusty_embed::playground`); this is the
//! window's half: opening one with its code beside its board, the other
//! chip's, the example back, and keeping what was written as a project of
//! its own.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_i18n::t;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Open the playground for `chip`: its code in the editor, its board beside
/// it, Run a key away.
///
/// Leaving a playground for the other chip's writes what was typed first,
/// the code and the board: a playground is where somebody's code lives
/// until they keep it, and switching chips is no reason to lose it. And a
/// simulation still running stops, since the board it lights is about to
/// be another chip's.
pub fn open_playground(state: AppState, chip: &'static str) {
    if state.playground_now().is_some() {
        save_all_then(state, move || {
            save_sheet_then(state, move || {
                if simulating(state) {
                    stop_anything(state);
                }
                open_playground_now(state, chip);
            })
        });
    } else {
        open_playground_now(state, chip);
    }
}

fn open_playground_now(state: AppState, chip: &'static str) {
    #[derive(serde::Serialize)]
    struct Args {
        chip: &'static str,
    }
    track(
        state,
        async move { ipc::call::<_, OpenResult>(cmd::playground::OPEN, &Args { chip }).await },
        move |result| project_opened(state, result),
    );
}

/// Put the example back, code and board, after asking — what was written
/// over it goes. Then the playground opens again from the disk, so no draft
/// of the old code is left on screen to be saved back over the example.
pub fn reset_playground(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        chip: &'static str,
    }
    let Some(chip) = playground_chip(state) else {
        return;
    };
    spawn_local(async move {
        if !ipc::confirm(&t!("playground.reset-question")).await {
            return;
        }
        if simulating(state) {
            stop_anything(state);
        }
        track(
            state,
            async move { ipc::call::<_, ()>(cmd::playground::RESET, &Args { chip }).await },
            move |()| open_playground_now(state, chip),
        );
    });
}

/// Keep what is in the playground as a project of its own: a folder picked,
/// everything but the build copied into it, and that project opened — in
/// the recents list, where the playground never is — with the board still
/// beside the code, since it is the same work carrying on. What is on
/// screen is written first, so the copy is what the user sees.
pub fn keep_playground(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        chip: &'static str,
        dest: String,
    }
    let Some(chip) = playground_chip(state) else {
        return;
    };
    save_all_then(state, move || {
        save_sheet_then(state, move || {
            spawn_local(async move {
                match ipc::pick_folder(&t!("playground.keep-title")).await {
                    Ok(Some(dest)) => track(
                        state,
                        async move {
                            ipc::call::<_, String>(cmd::playground::KEEP, &Args { chip, dest })
                                .await
                        },
                        move |kept| {
                            open_project_then(state, kept, move || {
                                state.layout.board_beside.set(true);
                                state.layout.panel.set("files".to_string());
                                // A folder that was never a project has no
                                // strip to put back; it opens on its code,
                                // as the playground did.
                                open_file(state, rusty_embed::PLAYGROUND_MAIN.to_string());
                            })
                        },
                    ),
                    Ok(None) => {}
                    Err(error) => state.app.error.set(Some(error)),
                }
            });
        })
    });
}

/// The board beside the code, or not — the editor strip's toggle, the View
/// menu and the palette.
pub fn toggle_board(state: AppState) {
    state.layout.board_beside.update(|on| *on = !*on);
    if state.layout.board_beside.get_untracked() {
        state.layout.panel.set("files".to_string());
    }
}

/// The open playground's chip as the list spells it, which is what the
/// backend takes — `None` for any other project.
fn playground_chip(state: AppState) -> Option<&'static str> {
    let open = state.playground_now()?;
    rusty_embed::PLAYGROUND_CHIPS
        .into_iter()
        .find(|chip| *chip == open)
}
