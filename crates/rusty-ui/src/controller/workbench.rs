//! Preferences that outlive the window: updates, modal editing, shortcuts.

use leptos::prelude::*;

// The sibling modules, flat: `controller` re-exports every one of them,
// so a call between two of them reads the same as a call from a view.
use super::*;
use crate::{
    ipc::{self, cmd},
    state::{AppState, UpdateStage},
};

/// The launch check: a moment after boot, and only in the shell window.
///
/// A moment after, so it never competes with the project coming back; only
/// the shell, because a torn-off editor or commit window booting the same
/// frontend would otherwise ask again and show a second sheet.
pub fn schedule_update_check(state: AppState) {
    if state.app.detached.get_untracked().is_some()
        || state.git.window_target.get_untracked().is_some()
    {
        return;
    }
    set_timeout(
        move || check_update(state, false),
        std::time::Duration::from_secs(8),
    );
}

/// Ask the release feed whether there is a newer rusty.
///
/// `manual` is the Help menu or Settings asking: the answer opens the sheet
/// whatever it is, because a menu item that sometimes does nothing is one
/// people stop trusting. The automatic check at launch opens it only for a
/// version that is newer and not skipped, and says nothing at all when the
/// feed cannot be reached — no network is the normal state of a bench, and
/// a red banner at every launch would teach people to dismiss banners.
pub fn check_update(state: AppState, manual: bool) {
    // What the last check found, read *before* the answer is cleared below:
    // the first version read it after, found nothing, and a manual re-check
    // put a verified download back to "Download and install".
    let held = state.app.update.get_untracked().and_then(|s| s.latest);
    if manual {
        state.app.update.set(None);
    }
    // Only an idle stage becomes "checking": a download in flight or held
    // is about the version already found, and a re-check must not lose it.
    if state.app.update_stage.get_untracked() == UpdateStage::Idle {
        state.app.update_stage.set(UpdateStage::Checking);
    }

    let apply = move |status: rusty_embed::UpdateStatus| {
        let stage = state.app.update_stage.get_untracked();
        // A verified download is for one version. The backend dropped it
        // if the feed now names another; the stage follows.
        if stage == UpdateStage::Checking || (stage == UpdateStage::Ready && held != status.latest)
        {
            state.app.update_stage.set(UpdateStage::Idle);
        }
        let prompt = status.newer && !status.skipped;
        state.app.update.set(Some(status));
        if manual || prompt {
            state.app.update_open.set(true);
        }
    };
    let future = async move { ipc::get::<rusty_embed::UpdateStatus>(cmd::workbench::UPDATE).await };
    if manual {
        track(state, future, apply);
    } else {
        spawn_local(async move {
            match future.await {
                Ok(status) => apply(status),
                Err(_) => {
                    if state.app.update_stage.get_untracked() == UpdateStage::Checking {
                        state.app.update_stage.set(UpdateStage::Idle);
                    }
                }
            }
        });
    }
}

/// Fetch the update the last check found. Progress arrives on a channel;
/// the answer says whether the installer is held and verified, or whether
/// the download was cancelled — the one failure that is not a failure.
pub fn download_update(state: AppState) {
    use wasm_bindgen::{JsValue, prelude::Closure};

    #[derive(serde::Serialize)]
    struct Args {}

    if matches!(
        state.app.update_stage.get_untracked(),
        UpdateStage::Downloading | UpdateStage::Ready | UpdateStage::Applying
    ) {
        return;
    }
    state.app.update_stage.set(UpdateStage::Downloading);
    state.app.update_progress.set(None);

    let channel = ipc::Channel::new();
    let on_progress = Closure::wrap(Box::new(move |value: JsValue| {
        if let Ok(progress) = serde_wasm_bindgen::from_value::<rusty_embed::UpdateProgress>(value) {
            state.app.update_progress.set(Some(progress));
        }
    }) as Box<dyn FnMut(JsValue)>);
    channel.set_onmessage(&on_progress);
    on_progress.forget();

    track(
        state,
        async move {
            let verified = ipc::call_streaming::<_, bool>(
                cmd::workbench::UPDATE_DOWNLOAD,
                &Args {},
                "onProgress",
                &channel,
            )
            .await;
            if verified.is_err() {
                state.app.update_stage.set(UpdateStage::Idle);
            }
            verified
        },
        move |verified| {
            state.app.update_stage.set(if verified {
                UpdateStage::Ready
            } else {
                UpdateStage::Idle
            });
        },
    );
}

/// Stop the download. The download command answers "cancelled" on its own
/// and the stage follows from there.
pub fn cancel_update(state: AppState) {
    track(
        state,
        async move { ipc::get::<()>(cmd::workbench::UPDATE_CANCEL).await },
        move |()| {},
    );
}

/// Install what was downloaded and restart into it. On Windows the process
/// ends inside the call, so the answer never arrives; a failure puts the
/// download back on offer.
pub fn apply_update(state: AppState) {
    if state.app.update_stage.get_untracked() != UpdateStage::Ready {
        return;
    }
    state.app.update_stage.set(UpdateStage::Applying);
    track(
        state,
        async move {
            let applied = ipc::get::<()>(cmd::workbench::UPDATE_APPLY).await;
            if applied.is_err() {
                state.app.update_stage.set(UpdateStage::Ready);
            }
            applied
        },
        move |()| {},
    );
}

/// Stop prompting about the version on offer, and close the sheet.
pub fn skip_update(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        version: String,
    }
    let Some(version) = state.app.update.get_untracked().and_then(|s| s.latest) else {
        return;
    };
    let args = Args { version };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::UPDATE_SKIP, &args).await },
        move |()| {
            state.app.update.update(|status| {
                if let Some(status) = status {
                    status.skipped = true;
                }
            });
            state.app.update_open.set(false);
        },
    );
}

/// Close the sheet. A download keeps going; the restart waits in Settings.
/// Refused while installing, since there is nothing left to decide.
pub fn dismiss_update(state: AppState) {
    if state.app.update_stage.get_untracked() == UpdateStage::Applying {
        return;
    }
    state.app.update_open.set(false);
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
    if enabled && let Some(element) = editor_area(state.focused().group) {
        let _ = element.focus();
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_VIM, &Args { enabled }).await },
        move |()| {},
    );
}

/// Read the auto-save switch at startup, like the modal-editing one: a
/// second window that did not auto-save would lose work on the assumption
/// that it had.
pub fn load_auto_save(state: AppState) {
    track(
        state,
        async move { ipc::call::<_, bool>(cmd::workbench::AUTO_SAVE, &()).await },
        move |on| state.editor.auto_save.set(on),
    );
}

/// Turn it on or off, and remember.
///
/// Turning it *on* writes what is already there, rather than waiting for the
/// next keystroke: a switch flipped over a dirty buffer that leaves the dot
/// lit reads as a switch that did nothing.
pub fn set_auto_save(state: AppState, enabled: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        enabled: bool,
    }
    state.editor.auto_save.set(enabled);
    if enabled {
        super::autosave_file(state);
    }
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_AUTO_SAVE, &Args { enabled }).await },
        move |()| {},
    );
}

/// Read the named rust-analyzer at startup, beside the other editor
/// settings a second window has to agree about.
pub fn load_rust_analyzer(state: AppState) {
    spawn_local(async move {
        if let Ok(path) = ipc::call::<_, Option<String>>(cmd::workbench::RUST_ANALYZER, &()).await {
            state.editor.rust_analyzer.set(path.unwrap_or_default());
        }
    });
}

/// Name one, or clear the choice with an empty field.
///
/// It takes effect on the next language-server start rather than now:
/// swapping the server under a window mid-edit would drop every diagnostic
/// on screen, and the setting's own footer says so.
pub fn set_rust_analyzer(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: Option<String>,
    }
    let trimmed = path.trim().to_string();
    state.editor.rust_analyzer.set(trimmed.clone());
    let path = (!trimmed.is_empty()).then_some(trimmed);
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::workbench::SET_RUST_ANALYZER, &Args { path }).await },
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
