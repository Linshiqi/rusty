//! Ctrl+Tab: the focused group's files, most recently used first.
//!
//! VS Code's editor history in a group. Hold Ctrl and press Tab to walk down
//! the list, Shift+Tab to walk back up, and let go of Ctrl to open the file
//! the list is on; Escape or a click outside puts it away. A tap — Ctrl+Tab
//! and straight off — opens the file before this one without the list ever
//! being drawn, so two files can be flipped between as fast as the key goes.
//!
//! The order is `RecentEditors`, and the list and its steps are `Switcher`,
//! both pure and tested in `state`. What is here is only what a browser
//! forces: which key events are the chord, and a list to look at.

use leptos::{ev, html, prelude::*};
use wasm_bindgen::{JsCast, closure::Closure};

use rusty_i18n::t;

use crate::{
    command::{Action, Chrome},
    controller,
    state::AppState,
    view::palette,
};

/// Which way the key event walks the list, if it is one of the switcher's
/// chords — as rebound in Settings, so the ids decide, not the keys.
fn switch_chord(state: AppState, event: &web_sys::KeyboardEvent) -> Option<bool> {
    let chord = palette::chord_of(
        event.ctrl_key() || event.meta_key(),
        event.shift_key(),
        event.alt_key(),
        &event.key(),
    )?;
    palette::effective(state)
        .into_iter()
        .find(|(_, bound)| *bound == chord)
        .and_then(|(binding, _)| match binding.action {
            Action::SwitchEditor => Some(false),
            Action::SwitchEditorBack => Some(true),
            _ => None,
        })
}

/// Install the switcher's keys. Called once, from the shell, beside the
/// palette's.
///
/// **In the capture phase**, where every other binding listens in the bubble
/// phase: the chord has to reach the switcher before whatever has focus acts
/// on a Tab — the editor indents, completion accepts, the terminal sends a
/// tab to the shell, and the terminal and Vim both stop the events they
/// take, so a bubbling listener would never hear Ctrl+Tab from either.
/// Letting go is a `keyup` of the chord's last modifier, and the window
/// losing focus with Ctrl still down puts the list away, since that `keyup`
/// will never come.
pub fn install(state: AppState, chrome: Chrome) {
    let Some(window) = web_sys::window() else {
        return;
    };

    let keydown =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            // Composing belongs to the input method; recording a new chord in
            // Settings belongs to the box doing the recording.
            if event.is_composing() || state.app.capturing.get_untracked().is_some() {
                return;
            }
            let open = state.layout.switcher.with_untracked(Option::is_some);
            if let Some(back) = switch_chord(state, &event) {
                // Taken wherever focus is, whether or not there is anything
                // to switch to — it means the same key everywhere.
                event.prevent_default();
                event.stop_propagation();
                // A dialog over the workbench has the keyboard; the files
                // behind it are not what anybody is looking at.
                let covered = chrome.palette_open.get_untracked()
                    || chrome.settings_open.get_untracked()
                    || state.layout.quick_open.get_untracked();
                if open || (!covered && state.has_project_now()) {
                    controller::switch_editor(state, back);
                }
                return;
            }
            if !open {
                return;
            }
            let taken = match event.key().as_str() {
                "ArrowDown" => {
                    controller::switch_editor(state, false);
                    true
                }
                "ArrowUp" => {
                    controller::switch_editor(state, true);
                    true
                }
                "Enter" => {
                    controller::commit_switch(state);
                    true
                }
                "Escape" => {
                    controller::cancel_switch(state);
                    true
                }
                _ => false,
            };
            if taken {
                event.prevent_default();
                event.stop_propagation();
            }
        });
    // Letting go: no modifier left down. A Tab or Shift let go of with Ctrl
    // still held keeps the list open; the last modifier opens the pick —
    // and a chord rebound to a bare function key opens it at once, a tap.
    let keyup =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |event: web_sys::KeyboardEvent| {
            let held = event.ctrl_key() || event.meta_key() || event.alt_key();
            if !held && state.layout.switcher.with_untracked(Option::is_some) {
                controller::commit_switch(state);
            }
        });
    // The window's own blur only: an element's does not bubble, so focus
    // moving inside the page never reaches this.
    let blur = Closure::<dyn Fn(web_sys::Event)>::new(move |_: web_sys::Event| {
        controller::cancel_switch(state);
    });

    let _ = window.add_event_listener_with_callback_and_bool(
        "keydown",
        keydown.as_ref().unchecked_ref(),
        true,
    );
    let _ = window.add_event_listener_with_callback_and_bool(
        "keyup",
        keyup.as_ref().unchecked_ref(),
        true,
    );
    let _ = window.add_event_listener_with_callback("blur", blur.as_ref().unchecked_ref());
    // For the life of the window, like the palette's listener.
    keydown.forget();
    keyup.forget();
    blur.forget();
}

/// The list, once Ctrl has been held long enough for it to be worth reading.
#[component]
pub fn EditorSwitcher() -> impl IntoView {
    let state = AppState::expect();
    let switcher = state.layout.switcher;
    let list: NodeRef<html::Div> = NodeRef::new();

    let paths = Memo::new(move |_| {
        switcher.with(|switcher| {
            switcher
                .as_ref()
                .map(|switcher| switcher.paths.clone())
                .unwrap_or_default()
        })
    });
    let at = Memo::new(move |_| switcher.with(|switcher| switcher.as_ref().map_or(0, |s| s.at)));
    let shown =
        Memo::new(move |_| switcher.with(|switcher| switcher.as_ref().is_some_and(|s| s.shown)));

    // A strip longer than the list is tall: keep the pick in view as the key
    // walks past the bottom edge, or round from the bottom to the top.
    Effect::new(move |_| {
        let at = at.get();
        let (Some(list), true) = (list.get(), shown.get()) else {
            return;
        };
        let Some(row) = list
            .query_selector(&format!("[data-row='{at}']"))
            .ok()
            .flatten()
            .and_then(|row| row.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return;
        };
        let (top, bottom) = (row.offset_top(), row.offset_top() + row.offset_height());
        if top < list.scroll_top() {
            list.set_scroll_top(top);
        } else if bottom > list.scroll_top() + list.client_height() {
            list.set_scroll_top(bottom - list.client_height());
        }
    });

    view! {
        <Show when=move || shown.get()>
            // Not dimmed, unlike the finder: this is a glance while a key is
            // held, not a place to type. A press outside puts it away, and no
            // press moves focus off the editor underneath.
            // `items-start`: as tall as its rows, up to the cap. Stretched, a
            // strip of four was a box two-thirds of the window high.
            <div
                class="absolute inset-0 z-30 flex items-start justify-center pt-2"
                on:mousedown=move |event: ev::MouseEvent| {
                    event.prevent_default();
                    controller::cancel_switch(state);
                }
            >
                <div
                    class="flex max-h-[60vh] w-[560px] flex-col overflow-hidden rounded-[12px] bg-raised shadow-2xl ring-1 ring-line-strong"
                    on:mousedown=move |event: ev::MouseEvent| {
                        event.prevent_default();
                        event.stop_propagation();
                    }
                >
                    <div class="border-b border-line px-4 py-2 text-caption text-label-3">
                        {t!("switcher.title")}
                    </div>
                    <div node_ref=list class="relative min-h-0 flex-1 overflow-y-auto py-1.5">
                        {move || {
                            paths
                                .get()
                                .into_iter()
                                .enumerate()
                                .map(|(index, path)| {
                                    let (dir, name) = match path.rfind('/') {
                                        Some(slash) => {
                                            (path[..=slash].to_string(), path[slash + 1..].to_string())
                                        }
                                        None => (String::new(), path.clone()),
                                    };
                                    let dirty = {
                                        let path = path.clone();
                                        move || state.is_dirty(&path)
                                    };
                                    view! {
                                        <button
                                            type="button"
                                            data-row=index.to_string()
                                            title=path.clone()
                                            on:click=move |_| controller::pick_switch(state, index)
                                            class=move || {
                                                let base = "flex w-full items-baseline gap-2 px-4 py-1.5 \
                                                            text-left font-mono text-footnote";
                                                if at.get() == index {
                                                    format!("{base} bg-selection text-rust")
                                                } else {
                                                    format!("{base} text-label-2 hover:bg-sunken")
                                                }
                                            }
                                        >
                                            <span class="shrink-0 text-label">{name}</span>
                                            <span class="min-w-0 flex-1 truncate text-label-3">{dir}</span>
                                            // The strip's own dot, so an unsaved
                                            // file looks the same in both.
                                            {move || {
                                                dirty()
                                                    .then(|| {
                                                        view! {
                                                            <span
                                                                class="size-1.5 shrink-0 self-center rounded-full bg-rust"
                                                                title=t!("files.unsaved")
                                                            />
                                                        }
                                                    })
                                            }}
                                        </button>
                                    }
                                })
                                .collect_view()
                        }}
                    </div>
                    <div class="border-t border-line px-4 py-1.5 text-caption text-label-3">
                        {t!("switcher.hint")}
                    </div>
                </div>
            </div>
        </Show>
    }
}
