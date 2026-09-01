//! Preferences that outlive the window: updates, modal editing, shortcuts.

use leptos::prelude::*;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::AppState,
};

/// Ask GitHub whether there is a newer rusty.
pub fn check_update(state: AppState) {
    state.app.update.set(None);
    track(
        state,
        async move { ipc::get::<rusty_embed::UpdateStatus>(cmd::workbench::UPDATE).await },
        move |status| state.app.update.set(Some(status)),
    );
}

/// Hand a link to the desktop browser.
pub fn open_url(state: AppState, url: String) {
    #[derive(serde::Serialize)]
    struct Args {
        url: String,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::OPEN_URL, &Args { url }).await },
        move |()| {},
    );
}

/// The stored shortcut overrides.
/// Read the modal-editing switch at startup.
///
/// From the file, not the WebView's storage: a second window boots the same
/// frontend, and landing in the wrong mode is not a shrug — the next twenty
/// keystrokes do something else entirely.
pub fn load_vim(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, bool>(cmd::workbench::VIM, &()).await },
        move |on| state.editor.vim_on.set(on),
    );
}

/// Turn it on or off, and remember.
pub fn set_vim(state: AppState, enabled: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        enabled: bool,
    }
    // Back to normal mode either way, so switching never leaves the editor
    // in a mode nobody asked for.
    state.editor.vim.set(crate::vim::Vim::default());
    state.editor.vim_on.set(enabled);

    // And give the editor the keyboard back. Both ways of reaching this — the
    // menu and the palette — take focus to get themselves clicked, and Vim's
    // keys are handled on the textarea, so without this the very next `j`
    // goes nowhere and the feature reads as not working at all.
    if enabled && let Some(element) = editor_element() {
        let _ = element.focus();
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_VIM, &Args { enabled }).await },
        move |()| {},
    );
}

pub fn load_keybinds(state: AppState) {
    track(
        state,
        async move {
            ipc::call::<_, std::collections::HashMap<String, String>>(cmd::workbench::KEYBINDS, &())
                .await
        },
        move |map| state.app.keybinds.set(map),
    );
}

/// Override one shortcut (or clear the override with `None`). Optimistic:
/// the map updates now, the file catches up.
pub fn save_keybind(state: AppState, id: String, chord: Option<String>) {
    state.app.keybinds.update(|map| match &chord {
        Some(chord) => {
            map.insert(id.clone(), chord.clone());
        }
        None => {
            map.remove(&id);
        }
    });

    #[derive(serde::Serialize)]
    struct Args {
        id: String,
        chord: Option<String>,
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_KEYBIND, &Args { id, chord }).await },
        |()| {},
    );
}
