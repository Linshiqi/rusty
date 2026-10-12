//! The clone dialog: a URL, a folder, and a project at the end of it.
//!
//! The one way a project arrives that is not "open a folder": paste the
//! address GitHub shows, pick where it should live, and the clone runs in the
//! dock like every other command — its progress readable, its failure a
//! paragraph rather than a banner. When git exits zero the new checkout opens
//! as the project, remote and all, since that is what a clone is for.
//!
//! The directory it creates is named the way `git clone` itself would name
//! it (`rusty_git::repo_name`), and the dialog says so before anything runs:
//! a folder appearing somewhere unexpected is the kind of surprise that
//! makes people distrust a tool with their disk.

use leptos::{ev, prelude::*};

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{Button, ButtonKind},
};

#[component]
pub fn CloneSheet() -> impl IntoView {
    let state = AppState::expect();
    move || {
        let draft = state.git.clone.get()?;
        let name = rusty_git::repo_name(&draft.url);
        let creates = match (&draft.into, &name) {
            (Some(folder), Some(name)) => {
                let separator = if folder.contains('\\') && !folder.contains('/') {
                    '\\'
                } else {
                    '/'
                };
                Some(t!(
                    "git.clone-creates",
                    path = format!("{}{separator}{name}", folder.trim_end_matches(['/', '\\']))
                ))
            }
            _ => None,
        };
        let ready = creates.is_some() && !draft.running;
        let running = draft.running;
        let close = move |_| {
            if !state
                .git
                .clone
                .with_untracked(|d| d.as_ref().is_some_and(|d| d.running))
            {
                state.git.clone.set(None);
            }
        };
        Some(view! {
            <div
                class="absolute inset-0 z-40 flex items-center justify-center bg-canvas/80 p-8"
                on:keydown=move |event: ev::KeyboardEvent| {
                    if event.key() == "Escape" {
                        close(());
                    }
                }
            >
                <div class="flex w-[560px] max-w-full flex-col gap-4 rounded-[10px] border border-line bg-content p-5 shadow-2xl">
                    <div class="text-title font-semibold">{t!("git.clone-title")}</div>
                    <label class="flex flex-col gap-1.5">
                        <span class="text-footnote text-label-3">{t!("git.clone-url")}</span>
                        <input
                            type="text"
                            autofocus
                            spellcheck="false"
                            placeholder=t!("git.clone-url-placeholder")
                            prop:value=draft.url.clone()
                            disabled=running
                            class="h-[30px] rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                            on:input=move |event| {
                                let url = event_target_value(&event);
                                state.git.clone.update(|d| {
                                    if let Some(d) = d {
                                        d.url = url;
                                    }
                                });
                            }
                            on:keydown=move |event: ev::KeyboardEvent| {
                                if event.key() == "Enter" {
                                    controller::clone_repository(state);
                                }
                            }
                        />
                    </label>
                    <div class="flex flex-col gap-1.5">
                        <span class="text-footnote text-label-3">{t!("git.clone-into")}</span>
                        <div class="flex items-center gap-2">
                            // Typed or chosen: a path pasted from elsewhere is
                            // as good as one picked in a dialog.
                            <input
                                type="text"
                                spellcheck="false"
                                placeholder=t!("git.clone-into-placeholder")
                                prop:value=draft.into.clone().unwrap_or_default()
                                disabled=running
                                class="h-[30px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust placeholder:text-label-3"
                                on:input=move |event| {
                                    let folder = event_target_value(&event);
                                    state.git.clone.update(|d| {
                                        if let Some(d) = d {
                                            d.into = Some(folder.trim().to_string()).filter(|f| !f.is_empty());
                                        }
                                    });
                                }
                            />
                            <Button
                                label=t!("git.clone-choose")
                                disabled=Signal::derive(move || running)
                                on_click=Callback::new(move |_| controller::choose_clone_folder(state))
                            />
                        </div>
                    </div>
                    <p class="text-footnote text-label-4">
                        {creates.unwrap_or_else(|| t!("git.clone-hint"))}
                    </p>
                    <div class="flex items-center justify-end gap-2">
                        <Button
                            label=t!("git.cancel")
                            disabled=Signal::derive(move || running)
                            on_click=Callback::new(move |_| state.git.clone.set(None))
                        />
                        <Button
                            label=if running { t!("git.clone-running") } else { t!("git.clone-run") }
                            kind=ButtonKind::Primary
                            disabled=Signal::derive(move || !ready)
                            on_click=Callback::new(move |_| controller::clone_repository(state))
                        />
                    </div>
                </div>
            </div>
        })
    }
}
