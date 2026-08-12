//! The command palette, and the keyboard bindings that reach the same actions.
//!
//! `Ctrl K` is the most-used affordance in modern development tools, and its
//! absence is the first thing a keyboard-driven user notices. Everything here
//! delegates to [`crate::command`], so a binding and its palette entry cannot
//! describe different behaviour.

use leptos::{ev, html, prelude::*};

use crate::{
    command::{self, Action, Chrome},
    state::AppState,
};

/// Install the global key handler. Called once, from the shell.
pub fn install(state: AppState, chrome: Chrome) {
    let Chrome {
        palette_open,
        settings_open,
    } = chrome;

    let handle = window_event_listener(ev::keydown, move |event| {
        let key = event.key();

        // Escape closes the topmost thing, innermost first.
        if key == "Escape" {
            if palette_open.get_untracked() {
                palette_open.set(false);
                event.prevent_default();
            } else if settings_open.get_untracked() {
                settings_open.set(false);
                event.prevent_default();
            }
            return;
        }

        // Everything else is modified. Without this check, typing "1" into the
        // palette's own search box would switch panels underneath it.
        if !(event.ctrl_key() || event.meta_key()) {
            return;
        }

        let action = match key.as_str() {
            "k" | "K" => {
                palette_open.update(|open| *open = !*open);
                event.prevent_default();
                return;
            }
            "," => Some(Action::OpenSettings),
            "o" | "O" => Some(Action::OpenProject),
            "r" | "R" => Some(Action::RefreshProject),
            "`" => Some(Action::ToggleDock),
            // Ctrl 1..9 goes to the nth panel. Zero is deliberately unbound —
            // there is no zeroth panel, and binding it to the last one is a
            // convention nobody expects here.
            digit => digit
                .parse::<usize>()
                .ok()
                .filter(|d| (1..=9).contains(d))
                .and_then(|d| {
                    crate::view::panels::all()
                        .get(d - 1)
                        .map(|panel| Action::ShowPanel(panel.id))
                }),
        };

        if let Some(action) = action {
            // Only swallow the key once it is known to be ours; Ctrl+A and the
            // rest must keep working in text fields.
            event.prevent_default();
            command::run(action, state, chrome);
        }
    });

    std::mem::forget(handle);
}

#[component]
pub fn Palette(open: RwSignal<bool>, chrome: Chrome) -> impl IntoView {
    let state = AppState::expect();

    let query = RwSignal::new(String::new());
    let highlighted = RwSignal::new(0usize);
    let input: NodeRef<html::Input> = NodeRef::new();

    // Reset and focus each time it opens. A palette that reopens showing the
    // last search is a palette that needs clearing before every use.
    Effect::new(move |_| {
        if open.get() {
            query.set(String::new());
            highlighted.set(0);
            if let Some(element) = input.get() {
                let _ = element.focus();
            }
        }
    });

    let filtered = Signal::derive(move || {
        let needle = query.get();
        command::all(state)
            .into_iter()
            .filter(|c| command::matches(&needle, &c.title) || command::matches(&needle, c.group))
            .collect::<Vec<_>>()
    });

    let run_at = move |index: usize| {
        let commands = filtered.get_untracked();
        if let Some(command) = commands.get(index) {
            open.set(false);
            command::run(command.action, state, chrome);
        }
    };

    view! {
        <Show when=move || open.get()>
            <div
                class="absolute inset-0 z-30 flex justify-center bg-black/25 pt-[12vh]"
                on:click=move |_| open.set(false)
            >
                <div
                    class="flex max-h-[60vh] w-[560px] flex-col overflow-hidden rounded-[12px] bg-raised shadow-2xl ring-1 ring-line-strong"
                    // The backdrop closes the palette; a click inside it must not.
                    on:click=move |event| event.stop_propagation()
                >
                    <input
                        node_ref=input
                        class="h-12 flex-none border-b border-line bg-transparent px-4 text-strong text-label outline-none placeholder:text-label-3"
                        placeholder="Type a command…"
                        on:input=move |event| {
                            query.set(event_target_value(&event));
                            highlighted.set(0);
                        }
                        on:keydown=move |event| {
                            let count = filtered.get_untracked().len();
                            match event.key().as_str() {
                                "ArrowDown" => {
                                    event.prevent_default();
                                    highlighted
                                        .update(|i| *i = if count == 0 { 0 } else { (*i + 1) % count });
                                }
                                "ArrowUp" => {
                                    event.prevent_default();
                                    highlighted
                                        .update(|i| {
                                            *i = if count == 0 { 0 } else { (*i + count - 1) % count }
                                        });
                                }
                                "Enter" => {
                                    event.prevent_default();
                                    run_at(highlighted.get_untracked());
                                }
                                _ => {}
                            }
                        }
                    />

                    <div class="min-h-0 flex-1 overflow-y-auto py-1.5">
                        {move || {
                            let commands = filtered.get();
                            if commands.is_empty() {
                                return view! {
                                    <p class="px-4 py-3 text-callout text-label-2">"No match"</p>
                                }
                                    .into_any();
                            }
                            let mut previous_group = "";
                            commands
                                .into_iter()
                                .enumerate()
                                .map(|(index, command)| {
                                    let heading = (command.group != previous_group)
                                        .then(|| {
                                            previous_group = command.group;
                                            view! {
                                                <div class="px-4 pt-2 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                                    {command.group}
                                                </div>
                                            }
                                        });
                                    let selected = Signal::derive(move || {
                                        highlighted.get() == index
                                    });
                                    view! {
                                        {heading}
                                        <button
                                            type="button"
                                            on:mouseenter=move |_| highlighted.set(index)
                                            on:click=move |_| run_at(index)
                                            class=move || {
                                                let base = "flex w-full items-center gap-3 px-4 py-1.5 \
                                                            text-left text-body transition-colors";
                                                if selected.get() {
                                                    format!("{base} bg-selection text-rust")
                                                } else {
                                                    format!("{base} text-label-2")
                                                }
                                            }
                                        >
                                            <span class="min-w-0 flex-1 truncate">{command.title}</span>
                                            {command
                                                .shortcut
                                                .map(|keys| {
                                                    view! {
                                                        <kbd class="shrink-0 rounded-[4px] bg-sunken px-1.5 py-0.5 font-mono text-footnote text-label-3">
                                                            {keys}
                                                        </kbd>
                                                    }
                                                })}
                                        </button>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }}
                    </div>

                    <div class="flex flex-none items-center gap-3 border-t border-line px-4 py-1.5 text-footnote text-label-3">
                        <span>"↑↓ to move"</span>
                        <span>"↵ to run"</span>
                        <span>"esc to close"</span>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Every binding, for the settings screen to list.
///
/// Beside the handler above so the two cannot disagree about what a key does.
/// A shortcut nobody can discover is a shortcut nobody uses, which is why this
/// is shown in the interface rather than left to documentation.
pub fn bindings() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Ctrl K", "Command palette"),
        ("Ctrl 1…9", "Go to panel"),
        ("Ctrl O", "Open project"),
        ("Ctrl R", "Re-check project"),
        ("Ctrl `", "Toggle the panel below"),
        ("Ctrl ,", "Settings"),
        ("Esc", "Close what is in front"),
    ]
}
