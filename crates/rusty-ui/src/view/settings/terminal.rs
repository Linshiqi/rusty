//! Which shell the terminal runs.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{Pill, Tone},
};

use super::*;

#[component]
pub(super) fn TerminalShell() -> impl IntoView {
    let state = AppState::expect();
    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::load_shell_info(state);
        }
    });
    let custom = RwSignal::new(String::new());
    // Seed the custom-path field from the stored preference once it arrives.
    // In an effect, not mid-render: writing a signal while rendering is the
    // pattern `plot.rs` warns against, and this view did exactly that.
    Effect::new(move |_| {
        let preference = state
            .term
            .info
            .with(|info| info.as_ref().and_then(|info| info.preference.clone()));
        if let Some(preference) = preference
            && preference != "system"
            && custom.get_untracked().is_empty()
        {
            custom.set(preference);
        }
    });

    view! {
        <Field
            label=t!("settings.terminal.shell")
            help=t!("settings.terminal.shell-help")
        >
            {move || {
                let Some(info) = state.term.info.get() else {
                    return view! {
                        <p class="text-callout text-label-3">{t!("settings.terminal.asking")}</p>
                    }
                        .into_any();
                };
                let preference = info.preference.clone();
                let is_auto = preference.is_none();
                let is_system = preference.as_deref() == Some("system");
                let is_custom = !is_auto && !is_system;
                let active = info.active.clone();
                view! {
                    <div class="flex flex-col gap-3">
                        <div class="inline-flex self-start rounded-[7px] bg-sunken p-0.5">
                            {[
                                (t!("settings.terminal.auto"), "auto", is_auto),
                                (t!("settings.terminal.system"), "system", is_system),
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
                                placeholder=t!("settings.terminal.custom-placeholder")
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
                                .then(|| view! { <Pill label=t!("settings.terminal.in-use") tone=Tone::Rust /> })}
                        </div>
                        <div class="flex items-center gap-2 text-callout text-label-2">
                            <span class="text-label-3">{t!("settings.terminal.next-shell")}</span>
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
