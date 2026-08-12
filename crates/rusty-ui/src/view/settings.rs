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
    Language,
    Assistant,
    Catalogue,
}

impl Category {
    const ALL: [Category; 5] = [
        Category::Appearance,
        Category::Keyboard,
        Category::Language,
        Category::Assistant,
        Category::Catalogue,
    ];

    fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Keyboard => "Keyboard",
            Category::Language => "Language",
            Category::Assistant => "Assistant",
            Category::Catalogue => "Catalogue",
        }
    }

    /// One line under the title in the list, so a category can be chosen
    /// without opening it first.
    fn summary(self) -> &'static str {
        match self {
            Category::Appearance => "Theme",
            Category::Keyboard => "Shortcuts",
            Category::Language => "Interface language",
            Category::Assistant => "Model and credentials",
            Category::Catalogue => "Chips and boards",
        }
    }
}

#[component]
pub fn Settings(open: RwSignal<bool>) -> impl IntoView {
    let selected = RwSignal::new(Category::Appearance);

    view! {
        <Show when=move || open.get()>
            <div class="absolute inset-0 z-20 flex flex-col bg-content">
                // No drag region needed: this covers the working area only, so
                // the menu bar above it stays visible and draggable — and, more
                // to the point, this header's own Done button stays on screen.
                <header class="flex h-10 flex-none items-center gap-3 border-b border-line px-4">
                    <span class="text-strong font-semibold tracking-tight">"Settings"</span>
                    <span class="flex-1" />
                    <Button
                        label="Done"
                        kind=ButtonKind::Primary
                        on_click=Callback::new(move |_| open.set(false))
                    />
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
                            Category::Language => view! { <Language /> }.into_any(),
                            Category::Assistant => view! { <Assistant /> }.into_any(),
                            Category::Catalogue => view! { <CatalogueSettings /> }.into_any(),
                        }}
                    </div>
                </div>
            </div>
        </Show>
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
    let theme = RwSignal::new(theme::stored());

    view! {
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
    view! {
        <Field
            label="Shortcuts"
            help="Not editable yet. Listed here because a shortcut nobody can discover is a \
                  shortcut nobody uses."
        >
            <dl class="grid grid-cols-[max-content_1fr] items-center gap-x-4 gap-y-1.5">
                {crate::view::palette::bindings()
                    .into_iter()
                    .map(|(keys, what)| {
                        view! {
                            <dt>
                                <kbd class="rounded-[4px] bg-sunken px-1.5 py-0.5 font-mono text-footnote text-label-2">
                                    {keys}
                                </kbd>
                            </dt>
                            <dd class="m-0 text-callout text-label-2">{what}</dd>
                        }
                    })
                    .collect_view()}
            </dl>
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
                    })
                />
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
