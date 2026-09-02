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

/// The stored shortcut overrides.
///
/// Loaded at boot beside the Vim switch, because it is the same kind of
/// thing: window-level, not project-level. It used to hang off the recents
/// path alone, so a project opened through the picker, a reloaded WebView
/// and a detached editor window all advertised the default chords while the
/// overrides sat unread in the file.
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

/// The stored language, handed to `i18n` to reconcile with what this window
/// booted into.
///
/// Guarded, because this runs at mount: with no backend the shim throws a
/// synchronous TypeError rather than rejecting, and a task that dies that way
/// records nothing — the trunk-only page rendered and then answered no click.
pub fn restore_locale() {
    if !ipc::backend_available() {
        return;
    }
    spawn_local(async move {
        if let Ok(stored) = ipc::get::<Option<String>>(cmd::workbench::LOCALE).await {
            crate::i18n::reconcile(stored);
        }
    });
}

/// What the file says the language is — the *stored* choice, which the
/// settings picker shows rather than the active one, because "follow the
/// system" and "English" look identical on an English machine.
pub fn load_locale(state: AppState, into: RwSignal<Option<Option<String>>>) {
    track(
        state,
        async move { ipc::get::<Option<String>>(cmd::workbench::LOCALE).await },
        move |stored| into.set(Some(stored)),
    );
}

/// Store a language choice, then switch this window into it. `None` means
/// follow the system. The switch waits for the save: a failed save shows as
/// a banner and changes nothing, instead of a window that reloads into a
/// language the file never heard about.
pub fn choose_locale(state: AppState, tag: Option<String>) {
    #[derive(serde::Serialize)]
    struct Args {
        tag: Option<String>,
    }
    let chosen = tag.clone();
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_LOCALE, &Args { tag }).await },
        move |()| crate::i18n::apply_choice(chosen),
    );
}
