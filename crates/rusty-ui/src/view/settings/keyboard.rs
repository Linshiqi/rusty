//! Shortcuts, and rebinding them.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{controller, state::AppState};

use super::*;

#[component]
pub(super) fn Keyboard() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <Group title=t!("settings.keyboard.shortcuts") footer=t!("settings.keyboard.note")>
            {move || {
                // Read for reactivity: rows re-render as overrides land.
                let overrides = state.app.keybinds.get();
                let rows = crate::view::palette::effective(state);
                // A chord bound twice is a surprise worth surfacing.
                let mut seen = std::collections::HashMap::new();
                for (_, chord) in &rows {
                    *seen.entry(chord.clone()).or_insert(0) += 1;
                }
                rows.into_iter()
                    .map(|(binding, chord)| {
                        let id = binding.id.clone();
                        let overridden = overrides.contains_key(&id);
                        let duplicate = seen.get(&chord).copied().unwrap_or(0) > 1;
                        let capture_id = id.clone();
                        let reset_id = id.clone();
                        let capturing = Signal::derive({
                            let id = id.clone();
                            move || {
                                state
                                    .app.capturing
                                    .with(|c| c.as_deref() == Some(id.as_str()))
                            }
                        });
                        view! {
                            <Row label=binding.label>
                                {overridden
                                    .then(|| {
                                        view! {
                                            <button
                                                type="button"
                                                title=t!("settings.keyboard.restore")
                                                on:click=move |_| {
                                                    controller::save_keybind(
                                                        state,
                                                        reset_id.clone(),
                                                        None,
                                                    );
                                                }
                                                class="rounded-[4px] px-1.5 py-0.5 text-footnote text-label-3 hover:bg-sunken hover:text-label"
                                            >
                                                {t!("settings.keyboard.reset")}
                                            </button>
                                        }
                                    })}
                                <button
                                    type="button"
                                    title=t!("settings.keyboard.rebind")
                                    on:click=move |_| {
                                        state.app.capturing.set(Some(capture_id.clone()));
                                    }
                                    on:keydown={
                                        let id = id.clone();
                                        move |event: leptos::ev::KeyboardEvent| {
                                            if !capturing.get_untracked() {
                                                return;
                                            }
                                            event.prevent_default();
                                            event.stop_propagation();
                                            match event.key().as_str() {
                                                "Escape" => {
                                                    state.app.capturing.set(None);
                                                }
                                                "Backspace" | "Delete" => {
                                                    controller::save_keybind(
                                                        state,
                                                        id.clone(),
                                                        None,
                                                    );
                                                    state.app.capturing.set(None);
                                                }
                                                key => {
                                                    if let Some(chord) =
                                                        crate::view::palette::chord_of(
                                                            event.ctrl_key()
                                                                || event.meta_key(),
                                                            event.shift_key(),
                                                            event.alt_key(),
                                                            key,
                                                        )
                                                    {
                                                        controller::save_keybind(
                                                            state,
                                                            id.clone(),
                                                            Some(chord),
                                                        );
                                                        state.app.capturing.set(None);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    on:blur=move |_| {
                                        if capturing.get_untracked() {
                                            state.app.capturing.set(None);
                                        }
                                    }
                                    class=move || {
                                        let base = "min-w-[6ch] rounded-[5px] px-2 py-0.5 \
                                                    font-mono text-footnote";
                                        if capturing.get() {
                                            format!("{base} bg-selection text-rust ring-1 ring-rust")
                                        } else if duplicate {
                                            format!("{base} bg-sunken text-amber")
                                        } else {
                                            format!("{base} bg-sunken text-label-2 hover:text-label")
                                        }
                                    }
                                >
                                    {move || {
                                        if capturing.get() {
                                            t!("settings.keyboard.press-keys")
                                        } else {
                                            chord.clone()
                                        }
                                    }}
                                </button>
                            </Row>
                        }
                    })
                    .collect_view()
            }}
            <Row label=t!("misc.close-front")>
                <kbd class="rounded-[5px] bg-sunken px-2 py-0.5 font-mono text-footnote text-label-3">
                    "Esc"
                </kbd>
            </Row>
        </Group>
    }
}
