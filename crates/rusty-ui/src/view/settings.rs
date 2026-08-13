//! Settings.
//!
//! An overlay over the whole window rather than a sidebar entry. Settings are
//! not part of the work loop — giving them a slot next to Flash and Monitor
//! would say they are visited as often, which they are not. Apple keeps
//! preferences out of primary navigation for the same reason.
//!
//! One category at a time, chosen from a list on the left. Stacking every
//! section in one scroll makes the reader do the filtering, and the result is
//! that nobody reads any of it.

use leptos::prelude::*;

use rusty_ai::{ProviderConfig, ProviderKind};

use crate::{
    controller,
    state::AppState,
    theme::{self, Theme},
    view::components::{Button, ButtonKind, Dot, Pill, Tone},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Appearance,
    Keyboard,
    Terminal,
    Language,
    Assistant,
    Catalogue,
    Storage,
    Network,
}

impl Category {
    const ALL: [Category; 8] = [
        Category::Appearance,
        Category::Keyboard,
        Category::Terminal,
        Category::Language,
        Category::Assistant,
        Category::Catalogue,
        Category::Storage,
        Category::Network,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Keyboard => "Keyboard",
            Category::Terminal => "Terminal",
            Category::Language => "Language",
            Category::Assistant => "Assistant",
            Category::Catalogue => "Catalogue",
            Category::Storage => "Storage",
            Category::Network => "Network",
        }
    }

    /// One line under the title in the list, so a category can be chosen
    /// without opening it first.
    fn summary(self) -> &'static str {
        match self {
            Category::Appearance => "Theme",
            Category::Keyboard => "Shortcuts",
            Category::Terminal => "Which shell runs",
            Category::Language => "Interface language",
            Category::Assistant => "Model and credentials",
            Category::Catalogue => "Chips and boards",
            Category::Storage => "Where rusty keeps its data",
            Category::Network => "How downloads reach the internet",
        }
    }
}

#[component]
pub fn Settings() -> impl IntoView {
    let selected = RwSignal::new(Category::Appearance);

    view! {
        <div class="flex min-h-0 flex-1 flex-col bg-content">
                // No Done and no Save: every control here applies the
                // moment it is touched, and leaving is any click in the
                // sidebar. A button that only closes teaches people to
                // wonder what it commits.
                <header class="flex h-10 flex-none items-center gap-3 border-b border-line px-4">
                    <span class="text-strong font-semibold tracking-tight">"Settings"</span>
                </header>

                <div class="flex min-h-0 flex-1">
                    <nav class="w-[168px] flex-none overflow-y-auto border-r border-line bg-sidebar p-2">
                        {Category::ALL
                            .into_iter()
                            .map(|category| {
                                let is_selected = Signal::derive(move || {
                                    selected.get() == category
                                });
                                view! {
                                    <button
                                        type="button"
                                        on:click=move |_| selected.set(category)
                                        class=move || {
                                            let base = "w-full rounded-[6px] px-2 py-1.5 text-left \
                                                        transition-colors";
                                            if is_selected.get() {
                                                format!("{base} bg-selection text-rust")
                                            } else {
                                                format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                                            }
                                        }
                                    >
                                        <div class="text-body font-medium">{category.label()}</div>
                                        <div class="text-footnote text-label-3">
                                            {category.summary()}
                                        </div>
                                    </button>
                                }
                            })
                            .collect_view()}
                    </nav>

                    <div class="min-w-0 flex-1 overflow-y-auto px-6 py-5">
                        {move || match selected.get() {
                            Category::Appearance => view! { <Appearance /> }.into_any(),
                            Category::Keyboard => view! { <Keyboard /> }.into_any(),
                            Category::Terminal => view! { <TerminalShell /> }.into_any(),
                            Category::Language => view! { <Language /> }.into_any(),
                            Category::Assistant => view! { <Assistant /> }.into_any(),
                            Category::Catalogue => view! { <CatalogueSettings /> }.into_any(),
                            Category::Storage => view! { <StorageSettings /> }.into_any(),
                            Category::Network => view! { <NetworkSettings /> }.into_any(),
                        }}
                    </div>
                </div>
        </div>
    }
}

/// A titled block within a category.
#[component]
fn Field(
    #[prop(into)] label: String,
    #[prop(optional, into)] help: Option<String>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="mb-6 max-w-[62ch] last:mb-0">
            <div class="mb-2 text-body font-medium">{label}</div>
            {children()}
            {help.map(|text| {
                view! { <p class="mt-2 text-callout leading-relaxed text-label-2">{text}</p> }
            })}
        </div>
    }
}

#[component]
fn Appearance() -> impl IntoView {
    let state = AppState::expect();
    let theme = RwSignal::new(theme::stored());

    view! {
        <Field
            label="Interface scale"
            help="Browser-style zoom over the whole window, 70% to 160%. Takes effect as you \
                  drag; the editor's own Ctrl+wheel text zoom is separate."
        >
            <div class="flex items-center gap-3">
                <input
                    type="range"
                    min="70"
                    max="160"
                    step="5"
                    prop:value=move || format!("{:.0}", state.ui_zoom.get() * 100.0)
                    on:input=move |event| {
                        if let Ok(percent) = event_target_value(&event).parse::<f64>() {
                            let factor = (percent / 100.0).clamp(0.7, 1.6);
                            state.ui_zoom.set(factor);
                            crate::state::remember_ui_zoom(factor);
                            controller::apply_ui_zoom(state);
                        }
                    }
                    class="w-56 accent-rust"
                />
                <span class="tnum w-[5ch] font-mono text-callout text-label-2">
                    {move || format!("{:.0}%", state.ui_zoom.get() * 100.0)}
                </span>
            </div>
        </Field>
        <Field
            label="Theme"
            help="System follows your desktop, including when you change it while rusty is open."
        >
            <div class="inline-flex rounded-[7px] bg-sunken p-0.5">
                {Theme::ALL
                    .into_iter()
                    .map(|option| {
                        let is_selected = Signal::derive(move || theme.get() == option);
                        view! {
                            <button
                                type="button"
                                on:click=move |_| {
                                    theme::set(option);
                                    theme.set(option);
                                }
                                class=move || {
                                    // A segmented control, as macOS uses for a
                                    // small set of exclusive choices.
                                    let base = "h-[24px] rounded-[5px] px-3 text-callout \
                                                transition-colors";
                                    if is_selected.get() {
                                        format!("{base} bg-content font-medium text-label shadow-sm")
                                    } else {
                                        format!("{base} text-label-2 hover:text-label")
                                    }
                                }
                            >
                                {option.label()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Field>
    }
}

#[component]
fn Keyboard() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <Field
            label="Shortcuts"
            help="Click a key to change it: press the new combination, Esc cancels, Backspace \
                  restores the default. Two commands on one chord — the lower one wins — show \
                  in amber. Esc itself closes whatever is in front and cannot be rebound."
        >
            <div class="grid grid-cols-[max-content_1fr_max-content] items-center gap-x-4 gap-y-1.5">
                {move || {
                    // Read for reactivity: rows re-render as overrides land.
                    let overrides = state.keybinds.get();
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
                                        .keybind_capture
                                        .with(|c| c.as_deref() == Some(id.as_str()))
                                }
                            });
                            view! {
                                <button
                                    type="button"
                                    title="Click, then press the new combination"
                                    on:click=move |_| {
                                        state.keybind_capture.set(Some(capture_id.clone()));
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
                                                    state.keybind_capture.set(None);
                                                }
                                                "Backspace" | "Delete" => {
                                                    controller::save_keybind(
                                                        state,
                                                        id.clone(),
                                                        None,
                                                    );
                                                    state.keybind_capture.set(None);
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
                                                        state.keybind_capture.set(None);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    on:blur=move |_| {
                                        if capturing.get_untracked() {
                                            state.keybind_capture.set(None);
                                        }
                                    }
                                    class=move || {
                                        let base = "justify-self-start rounded-[4px] px-1.5 \
                                                    py-0.5 font-mono text-footnote";
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
                                            "press keys…".to_string()
                                        } else {
                                            chord.clone()
                                        }
                                    }}
                                </button>
                                <span class="text-callout text-label-2">{binding.label}</span>
                                <span>
                                    {overridden
                                        .then(|| {
                                            view! {
                                                <button
                                                    type="button"
                                                    title="Restore the default"
                                                    on:click=move |_| {
                                                        controller::save_keybind(
                                                            state,
                                                            reset_id.clone(),
                                                            None,
                                                        );
                                                    }
                                                    class="rounded-[4px] px-1.5 py-0.5 text-footnote text-label-3 hover:bg-sunken hover:text-label"
                                                >
                                                    "reset"
                                                </button>
                                            }
                                        })}
                                </span>
                            }
                        })
                        .collect_view()
                }}
                <kbd class="justify-self-start rounded-[4px] bg-sunken px-1.5 py-0.5 font-mono text-footnote text-label-3">
                    "Esc"
                </kbd>
                <span class="text-callout text-label-3">"Close what is in front"</span>
                <span />
            </div>
        </Field>
    }
}

#[component]
fn TerminalShell() -> impl IntoView {
    let state = AppState::expect();
    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::load_shell_info(state);
        }
    });
    let custom = RwSignal::new(String::new());

    view! {
        <Field
            label="Shell"
            help="Auto is rusty's own built-in shell: compiled into the app, starts the \
                  instant the terminal opens, and reads the same on every OS. Plain \
                  commands, cd and history — pipes need the system shell. Changing this \
                  restarts the shell."
        >
            {move || {
                let Some(info) = state.shell_info.get() else {
                    return view! {
                        <p class="text-callout text-label-3">"Asking the backend…"</p>
                    }
                        .into_any();
                };
                let preference = info.preference.clone();
                let is_auto = preference.is_none();
                let is_system = preference.as_deref() == Some("system");
                let is_custom = !is_auto && !is_system;
                if is_custom && custom.get_untracked().is_empty() {
                    custom.set(preference.clone().unwrap_or_default());
                }
                let active = info.active.clone();
                view! {
                    <div class="flex flex-col gap-3">
                        <div class="inline-flex self-start rounded-[7px] bg-sunken p-0.5">
                            {[
                                ("Auto", "auto", is_auto),
                                ("System shell", "system", is_system),
                            ]
                                .into_iter()
                                .map(|(label, value, selected)| {
                                    view! {
                                        <button
                                            type="button"
                                            on:click=move |_| {
                                                controller::set_terminal_shell(
                                                    state,
                                                    Some(value.to_string()),
                                                );
                                            }
                                            class=if selected {
                                                "rounded-[6px] bg-content px-3 py-1 text-callout font-medium shadow-sm"
                                            } else {
                                                "rounded-[6px] px-3 py-1 text-callout text-label-2 hover:text-label"
                                            }
                                        >
                                            {label}
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                        <div class="flex items-center gap-2">
                            <input
                                placeholder="or a program: nu, fish, C:\\tools\\zsh.exe"
                                class="w-72 rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                                prop:value=move || custom.get()
                                on:input=move |event| custom.set(event_target_value(&event))
                                on:keydown=move |event: leptos::ev::KeyboardEvent| {
                                    if event.key() == "Enter" {
                                        let value = custom.get_untracked();
                                        let value = value.trim();
                                        if !value.is_empty() {
                                            controller::set_terminal_shell(
                                                state,
                                                Some(value.to_string()),
                                            );
                                        }
                                    }
                                }
                            />
                            {is_custom
                                .then(|| view! { <Pill label="in use" tone=Tone::Rust /> })}
                        </div>
                        <div class="flex items-center gap-2 text-callout text-label-2">
                            <span class="text-label-3">"Next shell:"</span>
                            <code class="rounded-[4px] bg-sunken px-1.5 py-0.5 font-mono text-footnote">
                                {active}
                            </code>
                        </div>
                    </div>
                }
                    .into_any()
            }}
        </Field>
    }
}

#[component]
fn Language() -> impl IntoView {
    view! {
        <Field
            label="Interface language"
            help="Translations will be checked at compile time, so a missing string is a build \
                  error rather than an English word appearing in another language."
        >
            <div class="flex items-center gap-2">
                <Pill label="English" tone=Tone::Rust />
                <Pill label="简体中文" />
                <span class="text-callout text-label-3">"not wired up yet"</span>
            </div>
        </Field>
    }
}

/// Bring-your-own-model configuration.
///
/// The key field is write-only by construction: it is sent to the OS credential
/// store and never read back, so this screen has no way to display a secret and
/// nothing to leak if the WebView is inspected. `ai_key_configured` answers
/// "is one on file" with a boolean, which is all the UI needs to know.
#[component]
fn Assistant() -> impl IntoView {
    let state = AppState::expect();

    // Local to the form: edits are not applied until Save, so abandoning a
    // half-typed base URL cannot leave the assistant pointed at nothing.
    let draft = RwSignal::new(state.ai_config.get_untracked().unwrap_or_else(|| {
        ProviderConfig {
            profile: "default".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: String::new(),
            model: String::new(),
            max_tokens: 4096,
            temperature: None,
            supports_tools: true,
        }
    }));
    let api_key = RwSignal::new(String::new());

    // The stored-key check, refreshed whenever the profile changes: the whole
    // point of the write-only design is that this boolean is the only thing
    // the screen can know.
    Effect::new(move |_| {
        let profile = draft.with(|d| d.profile.clone());
        crate::controller::refresh_key_state(state, profile);
    });
    let verdict = RwSignal::new(None::<String>);
    let models = RwSignal::new(Vec::<String>::new());

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.ai_presets.with(Vec::is_empty) {
            controller::load_assistant(state);
        }
    });

    view! {
        <Field
            label="Start from"
            help="Local runtimes are first-class: nothing leaves the machine and no key is \
                  needed. Picking one only fills the fields below — nothing is sent."
        >
            <div class="flex flex-wrap gap-1.5">
                {move || {
                    state
                        .ai_presets
                        .get()
                        .into_iter()
                        .map(|preset| {
                            let label = preset.label.clone();
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        draft
                                            .update(|d| {
                                                d.kind = preset.kind;
                                                d.base_url = preset.base_url.clone();
                                                d.model = preset.suggested_model.clone();
                                                d.profile = preset.label.to_lowercase();
                                            });
                                        verdict.set(None);
                                        models.set(Vec::new());
                                    }
                                    class="rounded-[6px] px-2 py-1 text-callout text-label-2 ring-1 ring-line transition-colors hover:bg-sunken hover:text-label"
                                >
                                    {label}
                                    {preset
                                        .local
                                        .then(|| {
                                            view! {
                                                <span class="ml-1.5 text-footnote text-patina">
                                                    "local"
                                                </span>
                                            }
                                        })}
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </div>
        </Field>

        <Field label="Endpoint">
            <div class="flex flex-col gap-2">
                <TextRow
                    label="Base URL"
                    value=Signal::derive(move || draft.with(|d| d.base_url.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.base_url = v))
                />
                <TextRow
                    label="Model"
                    value=Signal::derive(move || draft.with(|d| d.model.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.model = v))
                />
                <TextRow
                    label="Profile"
                    value=Signal::derive(move || draft.with(|d| d.profile.clone()))
                    on_input=Callback::new(move |v: String| draft.update(|d| d.profile = v))
                />
            </div>

            <div class="mt-2 flex items-center gap-2">
                <Button
                    label="List models"
                    on_click=Callback::new(move |_| {
                        controller::list_models(state, draft.get_untracked(), models)
                    })
                />
                <span class="text-footnote text-label-3">
                    "Names drift faster than any list rusty could ship."
                </span>
            </div>

            {move || {
                let found = models.get();
                (!found.is_empty())
                    .then(|| {
                        view! {
                            <div class="mt-2 flex max-h-[132px] flex-wrap gap-1.5 overflow-y-auto">
                                {found
                                    .into_iter()
                                    .map(|name| {
                                        let pick = name.clone();
                                        view! {
                                            <button
                                                type="button"
                                                on:click=move |_| {
                                                    draft.update(|d| d.model = pick.clone())
                                                }
                                                class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2 hover:text-label"
                                            >
                                                {name}
                                            </button>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        </Field>

        <Field
            label="API key"
            help="Stored in the operating system's credential store and read only by the Rust \
                  side at the moment of a request. It is never sent back to this window, which is \
                  why there is nothing here to show you once it is saved."
        >
            <div class="flex items-center gap-2">
                <input
                    type="password"
                    placeholder="paste, save, and it leaves this window"
                    class="h-[28px] w-[320px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                    on:input=move |event| api_key.set(event_target_value(&event))
                />
                <Button
                    label="Save key"
                    disabled=Signal::derive(move || api_key.with(|k| k.trim().is_empty()))
                    on_click=Callback::new(move |_| {
                        controller::store_key(
                            state,
                            draft.get_untracked().profile,
                            api_key.get_untracked(),
                        );
                        api_key.set(String::new());
                        crate::controller::refresh_key_state(
                            state,
                            draft.get_untracked().profile,
                        );
                    })
                />
                // The one thing this screen is allowed to know about the key.
                {move || {
                    if state.ai_key_stored.get() {
                        view! {
                            <>
                                <Pill label="stored" tone=Tone::Patina />
                                <Button
                                    label="Remove"
                                    on_click=Callback::new(move |_| {
                                        crate::controller::delete_key(
                                            state,
                                            draft.get_untracked().profile,
                                        )
                                    })
                                />
                            </>
                        }
                            .into_any()
                    } else {
                        view! { <Pill label="none saved" tone=Tone::Neutral /> }.into_any()
                    }
                }}
            </div>
        </Field>

        <Field label="Use it">
            <div class="flex items-center gap-2">
                <Button
                    label="Save"
                    kind=ButtonKind::Primary
                    on_click=Callback::new(move |_| {
                        controller::set_provider(state, draft.get_untracked())
                    })
                />
                <Button
                    label="Test"
                    title="Check the endpoint, the key and the model name before a conversation \
                           depends on them"
                    on_click=Callback::new(move |_| {
                        controller::check_provider(state, draft.get_untracked(), verdict)
                    })
                />
                {move || {
                    verdict
                        .get()
                        .map(|text| {
                            view! {
                                <span class="flex items-center gap-2 text-callout text-label-2">
                                    <Dot tone=Tone::Patina />
                                    {text}
                                </span>
                            }
                        })
                }}
            </div>

            <div class="mt-2 flex items-center gap-2">
                <Dot tone=Signal::derive(move || {
                    if state.ai_config.with(Option::is_some) { Tone::Patina } else { Tone::Neutral }
                })
                    .get() />
                <span class="text-callout text-label-2">
                    {move || {
                        state
                            .ai_config
                            .get()
                            .map(|c| format!("in use: {} at {}", c.model, c.base_url))
                            .unwrap_or_else(|| "None configured".to_string())
                    }}
                </span>
            </div>
        </Field>

        <Field
            label="What it can call"
            help="The assistant does not read your configuration files and theorise; it calls \
                  these and gets the actual answer. Each one reports what it is missing rather \
                  than guessing, so a wrong answer is a bug and not a coin flip."
        >
            <div class="flex flex-wrap gap-1.5">
                {move || {
                    state
                        .ai_tools
                        .get()
                        .into_iter()
                        .map(|tool| {
                            view! {
                                <span
                                    title=tool.description.clone()
                                    class="rounded-full bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2"
                                >
                                    {tool.name}
                                </span>
                            }
                        })
                        .collect_view()
                }}
            </div>
        </Field>
    }
}

/// A labelled single-line field.
#[component]
fn TextRow(
    #[prop(into)] label: String,
    value: Signal<String>,
    on_input: Callback<String>,
) -> impl IntoView {
    view! {
        <label class="flex items-center gap-3">
            <span class="w-[72px] shrink-0 text-callout text-label-2">{label}</span>
            <input
                class="h-[28px] flex-1 rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                prop:value=move || value.get()
                on:input=move |event| on_input.run(event_target_value(&event))
            />
        </label>
    }
}

#[component]
fn CatalogueSettings() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_catalog_problems(state);
        }
    });

    view! {
        <Field
            label="Where definitions come from"
            help="Later layers win, so you can correct a built-in entry without forking anything, \
                  and a team can check its boards into the repository."
        >
            <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1.5 font-mono text-footnote select-text">
                <dt class="text-label-3">"built in"</dt>
                <dd class="m-0 text-label-2">"compiled into rusty"</dd>
                <dt class="text-label-3">"yours"</dt>
                <dd class="m-0">"%APPDATA%\\rusty\\boards\\*.toml"</dd>
                <dt class="text-label-3">"project"</dt>
                <dd class="m-0">".rusty/boards/*.toml"</dd>
            </dl>
        </Field>
        {move || {
            let problems = state.catalog_problems.get();
            (!problems.is_empty())
                .then(|| {
                    view! {
                        <Field
                            label="Files that would not load"
                            help="A malformed file is kept out of the catalogue rather than \
                                  blanking it — so the board simply never appears, which is \
                                  why the reason belongs here."
                        >
                            <div class="flex flex-col gap-1.5">
                                {problems
                                    .into_iter()
                                    .map(|problem| {
                                        view! {
                                            <div class="max-w-[70ch] rounded-[6px] bg-amber-fill px-3 py-2">
                                                <p class="font-mono text-footnote select-text">
                                                    {problem.path}
                                                </p>
                                                <p class="mt-0.5 text-footnote leading-relaxed text-label-2 select-text">
                                                    {problem.detail}
                                                </p>
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        </Field>
                    }
                })
        }}
        <Field label="Loaded">
            <div class="flex items-center gap-2">
                <Dot tone=Tone::Patina />
                <span class="tnum text-callout text-label-2">
                    {move || {
                        format!(
                            "{} chips, {} boards",
                            state.chips.with(Vec::len),
                            state.boards.with(Vec::len),
                        )
                    }}
                </span>
            </div>
        </Field>
    }
}

/// Where the data directory is, and moving it.
///
/// This screen exists so nobody deletes a folder they never knew they had —
/// and so pointing the data at a synced folder is a button, not a wiki page.
#[component]
fn StorageSettings() -> impl IntoView {
    let state = AppState::expect();
    let location = RwSignal::new(None::<rusty_embed::StorageLocation>);
    let note = RwSignal::new(None::<String>);
    // A relocation refused because the target already holds rusty data; kept
    // so the adopt choice is offered as its own deliberate step.
    let blocked = RwSignal::new(None::<String>);

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_storage_location(location);
        }
    });

    view! {
        <Field
            label="Data directory"
            help="Board definitions and the workbench's memory live here, as plain files. \
                  Point it at a synced folder and every machine sees the same boards. API \
                  keys are not in it — they stay in the operating system's credential \
                  store and never sync."
        >
            {move || {
                let Some(here) = location.get() else {
                    return view! { <p class="text-callout text-label-2">"…"</p> }.into_any();
                };
                let (badge, tone) = if here.env_override {
                    ("RUSTY_CONFIG_DIR", Tone::Amber)
                } else if here.is_default {
                    ("default", Tone::Neutral)
                } else {
                    ("custom", Tone::Patina)
                };
                let env_note = here.env_override;
                view! {
                    <div class="flex flex-wrap items-center gap-2">
                        <code class="rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote select-text">
                            {here.path.clone()}
                        </code>
                        <Pill label=badge tone=tone />
                    </div>
                    {env_note
                        .then(|| {
                            view! {
                                <p class="mt-2 text-footnote text-amber">
                                    "Set by the RUSTY_CONFIG_DIR environment variable — a \
                                     move made here would be silently outvoted, so the \
                                     button below is disabled."
                                </p>
                            }
                        })}
                }
                .into_any()
            }}
        </Field>

        <Field
            label="Move it"
            help="Everything is copied to the folder you pick, then rusty switches over. \
                  The old files stay where they were until you delete them yourself — a \
                  migration that deletes its own fallback cannot be undone."
        >
            <Button
                label="Choose a new folder…"
                disabled=Signal::derive(move || {
                    location.get().is_some_and(|here| here.env_override)
                })
                on_click=Callback::new(move |_| {
                    crate::controller::pick_storage_folder(Callback::new(move |picked| {
                        if let Some(target) = picked {
                            crate::controller::relocate_storage(
                                state, target, false, note, blocked, location,
                            );
                        }
                    }));
                })
            />
            {move || {
                blocked
                    .get()
                    .map(|target| {
                        let adopt = target.clone();
                        view! {
                            <div class="mt-2 max-w-[62ch] rounded-[8px] bg-amber-fill px-3 py-2">
                                <p class="text-callout leading-relaxed">
                                    "That folder already holds rusty data. Use what is \
                                     there instead of copying?"
                                </p>
                                <div class="mt-1.5">
                                    <Button
                                        label="Use the folder's existing data"
                                        on_click=Callback::new(move |_| {
                                            crate::controller::relocate_storage(
                                                state,
                                                adopt.clone(),
                                                true,
                                                note,
                                                blocked,
                                                location,
                                            )
                                        })
                                    />
                                </div>
                            </div>
                        }
                    })
            }}
            {move || {
                note.get()
                    .map(|text| {
                        view! {
                            <p class="mt-2 max-w-[62ch] text-footnote leading-relaxed text-label-2 select-text">
                                {text}
                            </p>
                        }
                    })
            }}
        </Field>
    }
}

/// The proxy for tool downloads and crates.io queries.
///
/// Detect is the default and reads the environment, then the OS proxy the
/// browser uses — a Clash on 127.0.0.1:7890 is found without being told.
/// The other two exist for when detection is wrong: force direct, or name
/// the proxy outright.
#[component]
fn NetworkSettings() -> impl IntoView {
    let state = AppState::expect();
    let stored = RwSignal::new(None::<String>);
    let detected = RwSignal::new(None::<String>);
    let saved = RwSignal::new(false);

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_proxy_setting(stored, detected);
        }
    });
    let _ = state;

    let choose = move |value: Option<&'static str>| {
        crate::controller::save_proxy_setting(
            value.map(str::to_string),
            stored,
            detected,
            saved,
        );
    };

    view! {
        <Field
            label="Proxy"
            help="Used by the QEMU installer and the Crates panel. Detect follows the \
                  environment variables, then the system proxy — the one the browser \
                  uses. cargo and rustup keep their own proxy settings."
        >
            <div class="flex flex-col gap-2">
                <div class="flex items-center gap-2">
                    {move || {
                        let current = stored.get();
                        let is_auto = current.is_none();
                        let is_none = current.as_deref() == Some("none");
                        let manual = !is_auto && !is_none;
                        let pick = |on: bool| {
                            if on {
                                "rounded-[6px] bg-selection px-2.5 py-1 text-footnote text-rust"
                            } else {
                                "rounded-[6px] px-2.5 py-1 text-footnote text-label-3 hover:text-label"
                            }
                        };
                        view! {
                            <button
                                type="button"
                                class=pick(is_auto)
                                on:click=move |_| choose(None)
                            >
                                "Detect"
                            </button>
                            <button
                                type="button"
                                class=pick(is_none)
                                on:click=move |_| choose(Some("none"))
                            >
                                "Direct"
                            </button>
                            <span class=pick(manual)>"Manual:"</span>
                        }
                    }}
                    <input
                        type="text"
                        placeholder="http://127.0.0.1:7890"
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || {
                            stored
                                .get()
                                .filter(|v| v != "none")
                                .unwrap_or_default()
                        }
                        on:change=move |event: leptos::ev::Event| {
                            let value = event_target_value(&event);
                            let value = value.trim();
                            if !value.is_empty() {
                                crate::controller::save_proxy_setting(
                                    Some(value.to_string()),
                                    stored,
                                    detected,
                                    saved,
                                );
                            }
                        }
                        class="w-[26ch] rounded-[6px] bg-sunken px-2.5 py-1 font-mono text-footnote text-label placeholder:text-label-4"
                    />
                </div>
                {move || {
                    let line = match (stored.get(), detected.get()) {
                        (None, Some(found)) => {
                            format!("detected: {found} — downloads will use it")
                        }
                        (None, None) => "nothing detected — downloads go direct".to_string(),
                        (Some(v), _) if v == "none" => {
                            "forced direct, whatever the system says".to_string()
                        }
                        (Some(url), _) => format!("using {url}"),
                    };
                    view! {
                        <p class="text-footnote text-label-3 select-text">{line}</p>
                    }
                }}
                {move || {
                    saved
                        .get()
                        .then(|| {
                            view! {
                                <p class="text-footnote text-patina">"saved"</p>
                            }
                        })
                }}
            </div>
        </Field>
    }
}
