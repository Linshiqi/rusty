//! Chips and boards, and what would not load.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    state::AppState,
    view::components::{Dot, Tone},
};

use super::*;

#[component]
pub(super) fn CatalogueSettings() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            crate::controller::load_catalog_problems(state);
        }
    });

    view! {
        <Field
            label=t!("settings.catalogue.sources")
            help=t!("settings.catalogue.sources-help")
        >
            <dl class="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1.5 font-mono text-footnote select-text">
                <dt class="text-label-3">{t!("settings.catalogue.built-in")}</dt>
                <dd class="m-0 text-label-2">"compiled into rusty"</dd>
                <dt class="text-label-3">"yours"</dt>
                <dd class="m-0">"%APPDATA%\\rusty\\boards\\*.toml"</dd>
                <dt class="text-label-3">"project"</dt>
                <dd class="m-0">".rusty/boards/*.toml"</dd>
            </dl>
        </Field>
        {move || {
            let problems = state.project.catalog_problems.get();
            (!problems.is_empty())
                .then(|| {
                    view! {
                        <Field
                            label=t!("settings.catalogue.broken")
                            help=t!("settings.catalogue.broken-help")
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
        <Field label=t!("settings.catalogue.loaded")>
            <div class="flex items-center gap-2">
                <Dot tone=Tone::Patina />
                <span class="tnum text-callout text-label-2">
                    {move || {
                        format!(
                            "{} chips, {} boards",
                            state.project.chips.with(Vec::len),
                            state.project.boards.with(Vec::len),
                        )
                    }}
                </span>
            </div>
        </Field>
    }
}
