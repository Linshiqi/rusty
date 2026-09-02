//! Picking the terminal's shell, from the dock's own tab strip.

use rusty_i18n::t;

use leptos::prelude::*;

use crate::{controller, state::AppState};

#[component]
pub(super) fn ShellPicker() -> impl IntoView {
    let state = AppState::expect();
    controller::load_shell_choices(state);
    controller::load_shell_info(state);

    view! {
        <select
            title=t!("misc.which-shell")
            class="h-6 rounded-[5px] bg-sunken px-1.5 text-footnote text-label-2 outline-none"
            prop:value=move || {
                state
                    .term.info
                    .get()
                    .and_then(|info| info.preference)
                    .unwrap_or_else(|| "auto".to_string())
            }
            on:change=move |event| {
                controller::set_terminal_shell(state, Some(event_target_value(&event)));
            }
        >
            {move || {
                let mut choices = state.term.choices.get();
                // A stored preference the list does not carry (an uninstalled
                // shell, an old bare-name value) still has to be visible —
                // a select whose value matches nothing renders blank.
                if let Some(preference) =
                    state.term.info.get().and_then(|info| info.preference)
                    && !choices.iter().any(|c| c.value == preference)
                {
                    let short = preference
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&preference)
                        .to_string();
                    choices.push(rusty_embed::ShellChoice {
                        label: format!("{short} (current)"),
                        value: preference,
                    });
                }
                choices
                    .into_iter()
                    .map(|choice| {
                        view! { <option value=choice.value>{choice.label}</option> }
                    })
                    .collect_view()
            }}
        </select>
    }
}
