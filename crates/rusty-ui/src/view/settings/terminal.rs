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
    let apply_custom = move || {
        let value = custom.get_untracked();
        let value = value.trim();
        if !value.is_empty() {
            controller::set_terminal_shell(state, Some(value.to_string()));
        }
    };

    view! {
        <Group footer=t!("settings.terminal.note")>
            {move || {
                let Some(info) = state.term.info.get() else {
                    return view! { <NoteRow text=t!("settings.terminal.asking") /> }.into_any();
                };
                let preference = info.preference.clone();
                let is_auto = preference.is_none();
                let is_system = preference.as_deref() == Some("system");
                let is_custom = !is_auto && !is_system;
                let active = info.active.clone();
                view! {
                    <Row label=t!("settings.terminal.shell")>
                        <Segmented>
                            <Segment
                                label=t!("settings.terminal.auto")
                                selected=Signal::derive(move || is_auto)
                                on_click=Callback::new(move |_| {
                                    controller::set_terminal_shell(state, Some("auto".to_string()))
                                })
                            />
                            <Segment
                                label=t!("settings.terminal.system")
                                selected=Signal::derive(move || is_system)
                                on_click=Callback::new(move |_| {
                                    controller::set_terminal_shell(state, Some("system".to_string()))
                                })
                            />
                        </Segmented>
                    </Row>
                    <Row label=t!("settings.terminal.custom")>
                        <input
                            placeholder=t!("settings.terminal.custom-placeholder")
                            autocomplete="off"
                            spellcheck="false"
                            class="h-[26px] w-[260px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote text-label outline-none ring-1 ring-line placeholder:text-label-4 focus:ring-rust"
                            prop:value=move || custom.get()
                            on:input=move |event| custom.set(event_target_value(&event))
                            on:keydown=move |event: leptos::ev::KeyboardEvent| {
                                if event.key() == "Enter" {
                                    apply_custom();
                                }
                            }
                        />
                        {is_custom
                            .then(|| view! { <Pill label=t!("settings.terminal.in-use") tone=Tone::Rust /> })}
                    </Row>
                    <Row label=t!("settings.terminal.next-shell")>
                        <code class="rounded-[5px] bg-sunken px-2 py-0.5 font-mono text-footnote text-label-2">
                            {active}
                        </code>
                    </Row>
                }
                    .into_any()
            }}
        </Group>
    }
}
