//! Interface language.
//!
//! Its own category rather than a row inside Appearance, because somebody
//! who cannot read this window is looking for exactly one control in it and
//! the sidebar is where they will look.
//!
//! **Every language names itself.** Never "Chinese (Simplified)" in English:
//! the person hunting for their language cannot read the list they are
//! hunting in, and finds it by how it looks.

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;
use crate::{controller, state::AppState};

#[component]
pub(super) fn Language() -> impl IntoView {
    let state = AppState::expect();
    // What is *stored*, as opposed to what is active: "follow the system" and
    // "English" look identical on an English machine, and a picker that could
    // not tell them apart would move the highlight the first time somebody
    // opened it. `None` until the answer arrives, then `Some(the choice)`.
    let stored = RwSignal::new(None::<Option<String>>);
    controller::load_locale(state, stored);
    // The click is answered locally as well as saved, because the reload it
    // triggers is a moment away and a picker that does not move under the
    // pointer reads as broken.
    let chosen = RwSignal::new(None::<Option<String>>);
    let current = move || chosen.get().unwrap_or_else(|| stored.get().flatten());

    let mut options: Vec<(Option<String>, String)> = vec![(None, t!("settings.language-system"))];
    options.extend(
        rusty_i18n::Locale::ALL
            .iter()
            .map(|l| (Some(l.tag().to_string()), l.endonym().to_string())),
    );

    view! {
        <Group footer=t!("settings.language-note")>
            <Row label=t!("settings.language")>
                <Segmented>
                    {options
                        .into_iter()
                        .map(|(tag, label)| {
                            let mine = tag.clone();
                            view! {
                                <Segment
                                    label=label
                                    selected=Signal::derive(move || current() == mine)
                                    on_click=Callback::new(move |_| {
                                        chosen.set(Some(tag.clone()));
                                        controller::choose_locale(state, tag.clone());
                                    })
                                />
                            }
                        })
                        .collect_view()}
                </Segmented>
            </Row>
        </Group>
    }
}
