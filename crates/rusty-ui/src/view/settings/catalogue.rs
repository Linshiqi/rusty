//! Chips and boards, and what would not load.

use leptos::prelude::*;

use rusty_i18n::t;

use crate::state::AppState;

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
        <Group title=t!("settings.catalogue.sources") footer=t!("settings.catalogue.sources-note")>
            <Row label=t!("settings.catalogue.built-in")>
                <span class="text-callout text-label-2">{t!("settings.catalogue.built-in-where")}</span>
            </Row>
            <Row label=t!("settings.catalogue.yours")>
                <code class="font-mono text-footnote text-label-2 select-text">
                    "%APPDATA%\\rusty\\boards\\*.toml"
                </code>
            </Row>
            <Row label=t!("settings.catalogue.project")>
                <code class="font-mono text-footnote text-label-2 select-text">".rusty/boards/*.toml"</code>
            </Row>
            <Row label=t!("settings.catalogue.loaded")>
                <span class="tnum text-callout text-label-2">
                    {move || {
                        t!(
                            "settings.catalogue.loaded-count",
                            chips = state.project.chips.with(Vec::len).to_string(),
                            boards = state.project.boards.with(Vec::len).to_string()
                        )
                    }}
                </span>
            </Row>
        </Group>
        {move || {
            let problems = state.project.catalog_problems.get();
            (!problems.is_empty())
                .then(|| {
                    view! {
                        <Group
                            title=t!("settings.catalogue.broken")
                            footer=t!("settings.catalogue.broken-note")
                        >
                            {problems
                                .into_iter()
                                .map(|problem| {
                                    view! {
                                        <div class="px-3.5 py-2.5">
                                            <p class="font-mono text-footnote text-amber select-text">
                                                {problem.path}
                                            </p>
                                            <p class="mt-0.5 text-footnote leading-relaxed text-label-2 select-text">
                                                {problem.detail}
                                            </p>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </Group>
                    }
                })
        }}
    }
}
