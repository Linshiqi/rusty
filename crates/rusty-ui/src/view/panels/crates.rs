//! The workspace's direct dependencies, against crates.io.
//!
//! Current versus latest, with an Upgrade that runs `cargo add name@version`
//! through the shared session slot — the same visible, stoppable path every
//! other tool takes. An unreachable index shows up as a note on the row it
//! failed for; nothing here invents a version.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::view::icon::{Icon, IconView};
use crate::{
    controller,
    state::AppState,
    view::components::{Empty, register_toolbar},
};

#[component]
pub fn Crates() -> impl IntoView {
    let state = AppState::expect();

    let toolbar = Callback::new(move |_| {
        view! {
            <button
                type="button"
                title=t!("crates.refresh")
                on:click=move |_| controller::load_crate_report(state)
                class="grid size-8 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Refresh size=15 />
            </button>
        }
        .into_any()
    });
    register_toolbar(state, toolbar);

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.project.crate_rows.with(Option::is_none) {
            controller::load_crate_report(state);
        }
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title=t!("crates.no-project-title")
                    detail=t!("crates.no-project-detail")
                />
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <div class="flex items-center gap-2 border-b border-line px-5 py-2">
                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        {t!("crates.direct")}
                    </span>
                </div>
                <div class="min-h-0 flex-1 overflow-y-auto">
                    <div class="px-5 py-3">
                    {move || {
                        let Some(rows) = state.project.crate_rows.get() else {
                            return view! {
                                <p class="text-callout text-label-3">
                                    {t!("crates.asking")}
                                </p>
                            }
                            .into_any();
                        };
                        if rows.is_empty() {
                            return view! {
                                <p class="text-callout text-label-3">
                                    {t!("crates.none")}
                                </p>
                            }
                            .into_any();
                        }
                        let running = state.app.session_running;
                        rows.into_iter()
                            .map(|row| {
                                let behind = row
                                    .latest
                                    .as_deref()
                                    .is_some_and(|latest| latest != row.current);
                                let name = row.name.clone();
                                let target = row.latest.clone().unwrap_or_default();
                                view! {
                                    <div class="flex max-w-[80ch] items-baseline gap-3 border-b border-line py-1.5 last:border-b-0">
                                        <span class="w-[26ch] shrink-0 truncate font-mono text-footnote text-label">
                                            {row.name.clone()}
                                        </span>
                                        <span class="w-[10ch] shrink-0 font-mono text-footnote text-label-2">
                                            {row.current.clone()}
                                        </span>
                                        {match (&row.latest, &row.note) {
                                            (Some(latest), _) if behind => {
                                                view! {
                                                    <span class="w-[10ch] shrink-0 font-mono text-footnote text-amber">
                                                        {latest.clone()}
                                                    </span>
                                                    <button
                                                        type="button"
                                                        disabled=move || running.get()
                                                        on:click=move |_| {
                                                            controller::upgrade_crate(
                                                                state,
                                                                name.clone(),
                                                                target.clone(),
                                                            )
                                                        }
                                                        class="rounded-[6px] bg-rust px-2 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                                    >
                                                        {t!("crates.upgrade")}
                                                    </button>
                                                }
                                                    .into_any()
                                            }
                                            (Some(_), _) => view! {
                                                <span class="text-footnote text-patina">
                                                    {t!("misc.up-to-date")}
                                                </span>
                                            }
                                                .into_any(),
                                            (None, Some(note)) => view! {
                                                <span class="min-w-0 truncate text-footnote text-label-3">
                                                    {note.clone()}
                                                </span>
                                            }
                                                .into_any(),
                                            (None, None) => ().into_any(),
                                        }}
                                    </div>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
                    </div>
                    // The feature matrix rides below the dependency list —
                    // one page about what the workspace pulls in and why,
                    // instead of a second panel that was mostly empty space.
                    <div class="border-t border-line">
                        <super::features::Features />
                    </div>
                </div>
            </div>
        }
        .into_any()
    }
}
