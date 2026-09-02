//! Interface language.
//!
//! Its own category rather than a field inside Appearance, because somebody
//! who cannot read this window is looking for exactly one control in it and
//! the sidebar is where they will look. The category keeps its English name
//! *and* carries the endonym beside it, so it is findable from either side.
//!
//! **Every language names itself.** Never "Chinese (Simplified)" in English:
//! the person hunting for their language cannot read the list they are
//! hunting in, and finds it by how it looks.

use leptos::prelude::*;

use rusty_i18n::t;

use super::*;

#[component]
pub(super) fn Language() -> impl IntoView {
    // What is *stored*, as opposed to what is active: "follow the system" and
    // "English" look identical on an English machine, and a picker that could
    // not tell them apart would move the highlight the first time somebody
    // opened it.
    let stored = LocalResource::new(|| async move {
        crate::ipc::get::<Option<String>>(crate::ipc::cmd::workbench::LOCALE)
            .await
            .ok()
            .flatten()
    });

    view! {
        <Field label=t!("settings.language") help=t!("settings.language-hint")>
            <div class="inline-flex rounded-[7px] bg-sunken p-0.5">
                {
                    // The click is answered locally as well as saved, because
                    // the reload it triggers is a moment away and a picker
                    // that does not move under the pointer reads as broken.
                    let chosen = RwSignal::new(None::<Option<String>>);
                    let current = move || {
                        chosen.get().unwrap_or_else(|| stored.get().flatten())
                    };
                    let mut options: Vec<(Option<String>, String)> =
                        vec![(None, t!("settings.language-system"))];
                    options
                        .extend(
                            rusty_i18n::Locale::ALL
                                .iter()
                                .map(|l| {
                                    (Some(l.tag().to_string()), l.endonym().to_string())
                                }),
                        );
                    options
                        .into_iter()
                        .map(|(tag, label)| {
                            let mine = tag.clone();
                            let selected = Signal::derive(move || current() == mine);
                            let pick = tag.clone();
                            view! {
                                <button
                                    type="button"
                                    on:click=move |_| {
                                        chosen.set(Some(pick.clone()));
                                        crate::i18n::choose_locale(pick.clone());
                                    }
                                    class=move || {
                                        let base =
                                            "h-[24px] rounded-[5px] px-3 text-callout transition-colors";
                                        if selected.get() {
                                            format!("{base} bg-content font-medium text-label shadow-sm")
                                        } else {
                                            format!("{base} text-label-2 hover:text-label")
                                        }
                                    }
                                >
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()
                }
            </div>
        </Field>
    }
}
