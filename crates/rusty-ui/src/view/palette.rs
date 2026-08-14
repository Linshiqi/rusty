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
/// One customisable shortcut: a stable id (what workbench.toml stores
/// overrides against), the label Settings shows, the factory default, and
/// what it runs.
pub struct Binding {
    pub id: String,
    pub label: String,
    pub default: String,
    pub action: Action,
}

/// Every bindable command, in the order Settings lists them. Panels come
/// from the registry, so a contributed panel is bindable without anyone
/// remembering to add it.
pub fn defaults() -> Vec<Binding> {
    let mut out = vec![Binding {
        id: "palette.open".into(),
        label: "Command palette".into(),
        default: "Ctrl+K".into(),
        action: Action::OpenPalette,
    }];
    for (index, panel) in crate::view::panels::all()
        .into_iter()
        .filter(|p| !p.hidden)
        .enumerate()
        .take(9)
    {
        out.push(Binding {
            id: format!("panel.{}", panel.id),
            label: format!("Go to {}", panel.title),
            default: format!("Ctrl+{}", index + 1),
            action: Action::ShowPanel(panel.id),
        });
    }
    out.extend([
        Binding {
            id: "project.open".into(),
            label: "Open project".into(),
            default: "Ctrl+O".into(),
            action: Action::OpenProject,
        },
        Binding {
            id: "project.recheck".into(),
            label: "Re-check project".into(),
            default: "Ctrl+R".into(),
            action: Action::RefreshProject,
        },
        Binding {
            id: "nav.back".into(),
            label: "Back".into(),
            default: "Alt+ArrowLeft".into(),
            action: Action::NavBack,
        },
        Binding {
            id: "nav.forward".into(),
            label: "Forward".into(),
            default: "Alt+ArrowRight".into(),
            action: Action::NavForward,
        },
        Binding {
            id: "search.project".into(),
            label: "Search in project".into(),
            default: "Ctrl+Shift+F".into(),
            action: Action::ShowPanel("search"),
        },
        Binding {
            id: "dock.toggle".into(),
            label: "Toggle the panel below".into(),
            default: "Ctrl+`".into(),
            action: Action::ToggleDock,
        },
        Binding {
            id: "settings.open".into(),
            label: "Settings".into(),
            default: "Ctrl+,".into(),
            action: Action::OpenSettings,
        },
    ]);
    out
}

/// The bindings with overrides applied: what the keyboard actually does.
pub fn effective(state: AppState) -> Vec<(Binding, String)> {
    let overrides = state.keybinds.get_untracked();
    defaults()
        .into_iter()
        .map(|binding| {
            let chord = overrides.get(&binding.id).cloned().unwrap_or_else(|| {
                binding.default.clone()
            });
            (binding, chord)
        })
        .collect()
}

/// A key event as a canonical chord string, or None for anything that is
/// not a chord this system binds (unmodified keys, bare modifiers). Pure so
/// the canonical form is pinned by tests.
pub fn chord_of(ctrl: bool, shift: bool, alt: bool, key: &str) -> Option<String> {
    // Alt on its own counts only for *named* keys — Alt+ArrowLeft is Back in
    // every editor, while Alt+letter is how a menu mnemonic is reached and
    // how AltGr types on layouts that need it. Binding those would swallow
    // both. Everything else still requires Ctrl, which is what keeps plain
    // typing out of the binding system entirely.
    let alt_named = alt && key.chars().count() > 1;
    if !ctrl && !alt_named {
        return None;
    }
    if matches!(key, "Control" | "Shift" | "Alt" | "Meta") {
        return None;
    }
    let key = match key {
        // The shifted spellings arrive pre-shifted; store the base key so
        // "Ctrl+Shift+F" reads the way people write it.
        k if k.chars().count() == 1 => k.to_uppercase(),
        other => other.to_string(),
    };
    let mut chord = String::new();
    if ctrl {
        chord.push_str("Ctrl+");
    }
    if shift {
        chord.push_str("Shift+");
    }
    if alt {
        chord.push_str("Alt+");
    }
    chord.push_str(&key);
    Some(chord)
}

pub fn install(state: AppState, chrome: Chrome) {
    let Chrome {
        palette_open,
        settings_open,
    } = chrome;

    let handle = window_event_listener(ev::keydown, move |event| {
        let key = event.key();

        // Escape closes the topmost thing, innermost first. Not bindable:
        // an escape key that stopped escaping would strand people.
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

        // While Settings is capturing a new chord, the keyboard belongs to
        // the capture box, not to the bindings being edited.
        if state.keybind_capture.get_untracked().is_some() {
            return;
        }

        let Some(chord) =
            chord_of(event.ctrl_key() || event.meta_key(), event.shift_key(), event.alt_key(), &key)
        else {
            return;
        };
        let Some((binding, _)) = effective(state)
            .into_iter()
            .find(|(_, bound)| *bound == chord)
        else {
            return;
        };

        // Only swallow the key once it is known to be ours; Ctrl+A and the
        // rest must keep working in text fields.
        event.prevent_default();
        if binding.id == "palette.open" {
            // The palette key toggles — pressing it inside the palette is
            // how people close it.
            palette_open.update(|open| *open = !*open);
        } else {
            command::run(binding.action, state, chrome);
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


#[cfg(test)]
mod chord_tests {
    use super::chord_of;

    #[test]
    fn alt_binds_named_keys_and_leaves_typing_alone() {
        // Alt+arrow is Back and Forward in every editor.
        assert_eq!(
            chord_of(false, false, true, "ArrowLeft").as_deref(),
            Some("Alt+ArrowLeft"),
        );
        // Alt+letter is a menu mnemonic, and on some layouts AltGr is how a
        // character is typed at all. Binding either would swallow it.
        assert_eq!(chord_of(false, false, true, "f"), None);
        assert_eq!(chord_of(false, false, true, "3"), None);
    }

    #[test]
    fn nothing_without_a_modifier_is_ever_a_chord() {
        // The guard that keeps plain typing — and every Vim key — out of the
        // binding system entirely.
        assert_eq!(chord_of(false, false, false, "d"), None);
        assert_eq!(chord_of(false, true, false, "D"), None);
        assert_eq!(chord_of(false, false, false, "ArrowLeft"), None);
    }

    #[test]
    fn ctrl_chords_read_the_way_people_write_them() {
        assert_eq!(chord_of(true, false, false, "k").as_deref(), Some("Ctrl+K"));
        assert_eq!(
            chord_of(true, true, false, "F").as_deref(),
            Some("Ctrl+Shift+F"),
        );
        assert_eq!(chord_of(true, false, false, "Control"), None);
    }
}
