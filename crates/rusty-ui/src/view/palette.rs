//! The command palette, and the keyboard bindings that reach the same actions.
//!
//! `Ctrl K` is the most-used affordance in modern development tools, and its
//! absence is the first thing a keyboard-driven user notices. Everything here
//! delegates to [`crate::command`], so a binding and its palette entry cannot
//! describe different behaviour.

use leptos::{ev, html, prelude::*};

use rusty_i18n::t;

use crate::{
    command::{self, Action, Chrome},
    state::AppState,
};

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
        label: t!("bind.palette"),
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
            label: t!("bind.go-to", panel = panel.title),
            default: format!("Ctrl+{}", index + 1),
            action: Action::ShowPanel(panel.id),
        });
    }
    out.extend([
        Binding {
            id: "project.open".into(),
            label: t!("bind.open-project"),
            default: "Ctrl+O".into(),
            action: Action::OpenProject,
        },
        Binding {
            id: "project.recheck".into(),
            label: t!("bind.recheck"),
            default: "Ctrl+R".into(),
            action: Action::RefreshProject,
        },
        // The project's verbs, on the keys the tools people come from put
        // them: VS Code's build task and its F5 family, Arduino's upload and
        // serial monitor. Flash and Monitor are Arduino's rather than
        // PlatformIO's Ctrl+Alt letters, which on a layout with AltGr type
        // characters — the reason this system leaves Alt letters alone.
        Binding {
            id: "project.build".into(),
            label: t!("menu.project.build"),
            default: "Ctrl+Shift+B".into(),
            action: Action::Build,
        },
        Binding {
            id: "project.run".into(),
            label: t!("menu.project.run"),
            default: "Ctrl+F5".into(),
            action: Action::Run,
        },
        Binding {
            id: "project.debug".into(),
            label: t!("menu.project.debug"),
            default: "F5".into(),
            action: Action::Debug,
        },
        Binding {
            id: "project.stop".into(),
            label: t!("menu.project.stop"),
            default: "Shift+F5".into(),
            action: Action::Stop,
        },
        Binding {
            id: "project.restart".into(),
            label: t!("menu.project.restart"),
            default: "Ctrl+Shift+F5".into(),
            action: Action::Restart,
        },
        // The debugger's, on VS Code's keys. The transport's tooltips said
        // F10, F11 and Shift+F11 from the day it was drawn, and nothing was
        // bound to any of them: a step was a click or nothing.
        Binding {
            id: "debug.pause".into(),
            label: t!("debugger.pause"),
            default: "F6".into(),
            action: Action::Pause,
        },
        Binding {
            id: "debug.step-over".into(),
            label: t!("debugger.step-over"),
            default: "F10".into(),
            action: Action::StepOver,
        },
        Binding {
            id: "debug.step-into".into(),
            label: t!("debugger.step-into"),
            default: "F11".into(),
            action: Action::StepInto,
        },
        Binding {
            id: "debug.step-out".into(),
            label: t!("debugger.step-out"),
            default: "Shift+F11".into(),
            action: Action::StepOut,
        },
        Binding {
            id: "device.flash".into(),
            label: t!("menu.device.flash"),
            default: "Ctrl+U".into(),
            action: Action::Flash,
        },
        Binding {
            id: "device.monitor".into(),
            label: t!("menu.device.monitor"),
            default: "Ctrl+Shift+M".into(),
            action: Action::Monitor,
        },
        Binding {
            id: "editor.comment".into(),
            label: t!("bind.comment"),
            default: "Ctrl+/".into(),
            action: Action::ToggleComment,
        },
        Binding {
            id: "editor.rename".into(),
            label: t!("bind.rename"),
            default: "F2".into(),
            action: Action::Rename,
        },
        Binding {
            id: "nav.back".into(),
            label: t!("bind.back"),
            default: "Alt+ArrowLeft".into(),
            action: Action::NavBack,
        },
        Binding {
            id: "nav.forward".into(),
            label: t!("bind.forward"),
            default: "Alt+ArrowRight".into(),
            action: Action::NavForward,
        },
        // VS Code's pair. Held, the chord walks the list and letting go of
        // its modifiers opens the pick — whatever it is rebound to, which is
        // why `view/switcher.rs` matches these ids rather than the keys.
        Binding {
            id: "editor.switch".into(),
            label: t!("bind.switch-editor"),
            default: "Ctrl+Tab".into(),
            action: Action::SwitchEditor,
        },
        Binding {
            id: "editor.switch-back".into(),
            label: t!("bind.switch-editor-back"),
            default: "Ctrl+Shift+Tab".into(),
            action: Action::SwitchEditorBack,
        },
        Binding {
            id: "search.project".into(),
            label: t!("bind.search"),
            default: "Ctrl+Shift+F".into(),
            action: Action::ShowPanel("search"),
        },
        Binding {
            id: "dock.toggle".into(),
            label: t!("bind.toggle-dock"),
            default: "Ctrl+`".into(),
            action: Action::ToggleDock,
        },
        Binding {
            id: "settings.open".into(),
            label: t!("bind.settings"),
            default: "Ctrl+,".into(),
            action: Action::OpenSettings,
        },
        // VS Code's three, so hands that know them need not learn ours.
        Binding {
            id: "quick.open".into(),
            label: t!("bind.quick-open"),
            default: "Ctrl+P".into(),
            action: Action::QuickOpen,
        },
        Binding {
            id: "quick.line".into(),
            label: t!("bind.go-to-line"),
            default: "Ctrl+G".into(),
            action: Action::GoToLine,
        },
        Binding {
            id: "quick.symbol".into(),
            label: t!("bind.symbol-in-file"),
            default: "Ctrl+Shift+O".into(),
            action: Action::GoToSymbolInFile,
        },
        Binding {
            id: "quick.workspace-symbol".into(),
            label: t!("bind.symbol-in-workspace"),
            default: "Ctrl+T".into(),
            action: Action::GoToSymbolInWorkspace,
        },
        Binding {
            id: "editor.references".into(),
            label: t!("bind.references"),
            default: "Shift+F12".into(),
            action: Action::FindReferences,
        },
        Binding {
            id: "editor.implementations".into(),
            label: t!("bind.implementations"),
            default: "Ctrl+F12".into(),
            action: Action::GoToImplementations,
        },
        Binding {
            id: "tree.toggle".into(),
            label: t!("bind.toggle-tree"),
            default: "Ctrl+B".into(),
            action: Action::ToggleTree,
        },
        Binding {
            id: "editor.split".into(),
            label: t!("bind.split"),
            default: "Ctrl+\\".into(),
            action: Action::SplitEditor,
        },
    ]);
    out
}

/// The bindings with overrides applied: what the keyboard actually does.
pub fn effective(state: AppState) -> Vec<(Binding, String)> {
    let overrides = state.app.keybinds.get_untracked();
    defaults()
        .into_iter()
        .map(|binding| {
            let chord = overrides
                .get(&binding.id)
                .cloned()
                .unwrap_or_else(|| binding.default.clone());
            (binding, chord)
        })
        .collect()
}

/// A control's tooltip with the key that does the same, read off the
/// bindings as they stand — so a rebound key is the key shown, and a tooltip
/// never names a key that does nothing.
pub fn with_chord(state: AppState, action: Action, label: String) -> String {
    chords(state)(action).map_or(label.clone(), |chord| format!("{label} ({chord})"))
}

/// What each action's key actually is right now — overrides included, so
/// nothing advertises a chord that stopped working. Read once and asked
/// many times: a menu of sixty rows does not read the overrides sixty times.
pub fn chords(state: AppState) -> impl Fn(Action) -> Option<String> {
    let bound = effective(state);
    move |action| {
        bound
            .iter()
            .find(|(binding, _)| binding.action == action)
            .map(|(_, chord)| chord.clone())
    }
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
    // A function key is a chord on its own: nothing types F2, and every
    // editor binds them bare — F2 to rename, F5 to run. Requiring Ctrl would
    // make those defaults unreachable.
    let function =
        key.len() >= 2 && key.starts_with('F') && key[1..].chars().all(|c| c.is_ascii_digit());
    if !ctrl && !alt_named && !function {
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

/// Install the global key handler. Called once, from the shell.
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
        if state.app.capturing.get_untracked().is_some() {
            return;
        }

        let Some(chord) = chord_of(
            event.ctrl_key() || event.meta_key(),
            event.shift_key(),
            event.alt_key(),
            &key,
        ) else {
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
            .filter(|c| command::matches(&needle, &c.title) || command::matches(&needle, &c.group))
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
                        placeholder=t!("bind.type-command")
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
                                    <p class="px-4 py-3 text-callout text-label-2">{t!("bind.no-match")}</p>
                                }
                                    .into_any();
                            }
                            let mut previous_group = String::new();
                            commands
                                .into_iter()
                                .enumerate()
                                .map(|(index, command)| {
                                    let heading = (command.group != previous_group)
                                        .then(|| {
                                            previous_group = command.group.clone();
                                            view! {
                                                <div class="px-4 pt-2 pb-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                                    {command.group.clone()}
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
                        <span>{t!("palette.hint-move")}</span>
                        <span>{t!("palette.hint-run")}</span>
                        <span>{t!("palette.hint-close")}</span>
                    </div>
                </div>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;

    fn default_of(action: Action) -> Option<String> {
        defaults()
            .into_iter()
            .find(|b| b.action == action)
            .map(|b| b.default)
    }

    /// The debugger's keys are VS Code's, and bound: the transport's
    /// tooltips said F10, F11 and Shift+F11 for as long as it existed while
    /// nothing answered any of them.
    #[test]
    fn the_debugger_has_its_keys() {
        assert_eq!(default_of(Action::StepOver).as_deref(), Some("F10"));
        assert_eq!(default_of(Action::StepInto).as_deref(), Some("F11"));
        assert_eq!(default_of(Action::StepOut).as_deref(), Some("Shift+F11"));
        assert_eq!(default_of(Action::Pause).as_deref(), Some("F6"));
        // Each one spelled the way a key press is read, or it is a binding
        // that no key can ever reach.
        for (key, shift) in [("F10", false), ("F11", false), ("F11", true), ("F6", false)] {
            let chord = chord_of(false, shift, false, key).expect("a function key is a chord");
            assert!(
                defaults().iter().any(|b| b.default == chord),
                "{chord} reaches no binding"
            );
        }
    }

    /// The handler takes the first binding a key matches, so a second
    /// default on the same key is a command that can never be reached.
    #[test]
    fn no_two_defaults_share_a_key() {
        let all = defaults();
        for (i, one) in all.iter().enumerate() {
            for two in &all[i + 1..] {
                assert_ne!(one.default, two.default, "{} and {}", one.id, two.id);
                assert_ne!(
                    one.id, two.id,
                    "an id is what an override is stored against"
                );
            }
        }
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
    fn function_keys_bind_bare() {
        assert_eq!(chord_of(false, false, false, "F2").as_deref(), Some("F2"));
        assert_eq!(chord_of(false, false, false, "F12").as_deref(), Some("F12"));
        // Not everything starting with F — `f` is a Vim motion and `Find` is
        // a key name on some layouts.
        assert_eq!(chord_of(false, false, false, "f"), None);
        assert_eq!(chord_of(false, false, false, "Find"), None);
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
