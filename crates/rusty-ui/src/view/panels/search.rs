//! Project-wide text search, laid out the way every editor since VSCode has
//! taught: query with case/word toggles, include/exclude globs, results
//! grouped per file with counts, collapsible.
//!
//! State lives in [`AppState`] so switching panels does not throw the query
//! or results away.

use leptos::{ev, prelude::*};

use rusty_edit::SearchHit;

use rusty_i18n::t;

use super::files::Editor;
use crate::{controller, state::AppState, view::components::Empty};

#[component]
pub fn SearchPanel() -> impl IntoView {
    let state = AppState::expect();
    // Paths whose match lists are folded away. Local: a fold is a viewing
    // gesture, not project state.
    let collapsed = RwSignal::new(Vec::<String>::new());

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title=t!("search.no-project-title")
                    detail=t!("search.no-project-detail")
                />
            }
            .into_any();
        }

        // VSCode's split: the search lives in a sidebar, the editor keeps the
        // rest of the window. A hit opens on the right without leaving here.
        view! {
            <div class="flex min-h-0 flex-1">
                <div class="flex w-[300px] flex-none flex-col border-r border-line bg-sidebar">
                <div class="flex flex-col gap-1.5 border-b border-line px-3 py-2">
                    <div class="flex items-center gap-1.5">
                        <input
                            type="text"
                            placeholder=t!("search.placeholder")
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || state.search.query.get()
                            on:input=move |event: ev::Event| {
                                state.search.query.set(event_target_value(&event));
                                controller::schedule_search(state);
                            }
                            class="min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 py-1.5 font-mono text-footnote text-label placeholder:text-label-3"
                        />
                        <Toggle
                            label="Aa"
                            help=t!("search.match-case")
                            on=state.search.case
                        />
                        <Toggle
                            label="ab"
                            help=t!("search.whole-word")
                            on=state.search.word
                        />
                        <Toggle
                            label=".*"
                            help=t!("search.regex")
                            on=state.search.regex
                        />
                    </div>
                    <GlobBox
                        placeholder=t!("search.include")
                        value=state.search.include
                    />
                    <GlobBox
                        placeholder=t!("search.exclude")
                        value=state.search.exclude
                    />
                </div>

                <div class="min-h-0 flex-1 overflow-auto px-1 py-1">
                    {move || {
                        let Some(results) = state.search.results.get() else {
                            return view! {
                                <p class="px-3 py-2 text-footnote text-label-3">
                                    {t!("search.as-you-type")}
                                </p>
                            }
                            .into_any();
                        };
                        if let Some(error) = results.error {
                            return view! {
                                <p class="px-3 py-2 text-footnote text-amber select-text">
                                    {error}
                                </p>
                            }
                            .into_any();
                        }
                        if results.hits.is_empty() {
                            return view! {
                                <p class="px-3 py-2 text-footnote text-label-3">
                                    {t!("search.nothing")}
                                </p>
                            }
                            .into_any();
                        }

                        let summary = if results.truncated {
                            format!(
                                "first {} results in {} files — narrow the query for the rest",
                                results.hits.len(),
                                results.files,
                            )
                        } else {
                            format!("{} results in {} files", results.hits.len(), results.files)
                        };

                        // Walk order is the tree's; group consecutive runs of
                        // the same file rather than sorting anything.
                        let mut groups: Vec<(String, Vec<SearchHit>)> = Vec::new();
                        for hit in results.hits {
                            match groups.last_mut() {
                                Some((path, hits)) if *path == hit.path => hits.push(hit),
                                _ => groups.push((hit.path.clone(), vec![hit])),
                            }
                        }

                        view! {
                            <p class="px-3 pt-1 pb-2 text-caption text-label-3">{summary}</p>
                            {groups
                                .into_iter()
                                .map(|(path, hits)| {
                                    view! { <FileGroup path=path hits=hits collapsed=collapsed /> }
                                })
                                .collect_view()}
                        }
                        .into_any()
                    }}
                </div>
                </div>

                <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                    <Editor />
                </div>
            </div>
        }
        .into_any()
    }
}

/// A small on/off square, `Aa` style.
#[component]
fn Toggle(label: &'static str, help: String, on: RwSignal<bool>) -> impl IntoView {
    let state = AppState::expect();
    view! {
        <button
            type="button"
            title=help
            on:click=move |_| {
                on.update(|value| *value = !*value);
                controller::schedule_search(state);
            }
            class=move || {
                let base = "rounded-[6px] px-2 py-1 font-mono text-footnote transition-colors";
                if on.get() {
                    format!("{base} bg-selection text-rust")
                } else {
                    format!("{base} text-label-3 hover:text-label")
                }
            }
        >
            {label}
        </button>
    }
}

/// An include/exclude pattern box.
#[component]
fn GlobBox(placeholder: String, value: RwSignal<String>) -> impl IntoView {
    let state = AppState::expect();
    view! {
        <input
            type="text"
            placeholder=placeholder
            autocomplete="off"
            spellcheck="false"
            prop:value=move || value.get()
            on:input=move |event: ev::Event| {
                value.set(event_target_value(&event));
                controller::schedule_search(state);
            }
            class="min-w-0 rounded-[6px] bg-sunken px-2.5 py-1 font-mono text-caption text-label placeholder:text-label-4"
        />
    }
}

/// One file's matches: a header row with the count, folding its hits.
#[component]
fn FileGroup(
    path: String,
    hits: Vec<SearchHit>,
    collapsed: RwSignal<Vec<String>>,
) -> impl IntoView {
    let (name, dir) = match path.rsplit_once('/') {
        Some((dir, name)) => (name.to_string(), dir.to_string()),
        None => (path.clone(), String::new()),
    };
    let count = hits.len();
    let folded = {
        let path = path.clone();
        Signal::derive(move || collapsed.with(|c| c.iter().any(|p| p == &path)))
    };
    let toggle = {
        let path = path.clone();
        move |_| {
            collapsed.update(|c| match c.iter().position(|p| p == &path) {
                Some(at) => {
                    c.remove(at);
                }
                None => c.push(path.clone()),
            })
        }
    };

    view! {
        <div class="mb-0.5">
            <button
                type="button"
                on:click=toggle
                class="flex w-full items-center gap-1.5 rounded-[5px] px-2 py-[3px] text-left hover:bg-sunken"
            >
                <span class="w-3 shrink-0 text-center text-footnote text-label-3">
                    {move || if folded.get() { "▸" } else { "▾" }}
                </span>
                <span class="shrink-0 font-mono text-footnote font-semibold text-label">
                    {name}
                </span>
                {(!dir.is_empty())
                    .then(|| {
                        view! {
                            <span class="min-w-0 truncate font-mono text-caption text-label-4">
                                {dir}
                            </span>
                        }
                    })}
                <span class="flex-1" />
                <span class="shrink-0 rounded-full bg-sunken px-1.5 text-caption text-label-3">
                    {count}
                </span>
            </button>
            <Show when=move || !folded.get()>
                {hits.iter().cloned().map(row).collect_view()}
            </Show>
        </div>
    }
}

/// One hit: line number, then the line with the match lit.
fn row(hit: SearchHit) -> impl IntoView {
    let state = AppState::expect();

    let start = hit.span_start as usize;
    let end = (hit.span_end as usize).min(hit.text.len());
    let (before, matched, after) =
        if start <= end && hit.text.is_char_boundary(start) && hit.text.is_char_boundary(end) {
            (
                hit.text[..start].to_string(),
                hit.text[start..end].to_string(),
                hit.text[end..].to_string(),
            )
        } else {
            (hit.text.clone(), String::new(), String::new())
        };

    let path = hit.path.clone();
    view! {
        <button
            type="button"
            on:click=move |_| {
                controller::open_at(state, path.clone(), hit.line, hit.col)
            }
            class="flex w-full items-baseline gap-2 rounded-[5px] py-[2px] pr-2 pl-6 text-left font-mono text-footnote text-label-2 hover:bg-sunken"
        >
            <span class="w-[4ch] shrink-0 text-right text-label-4">{hit.line + 1}</span>
            <span class="min-w-0 flex-1 truncate whitespace-pre">
                <span>{before}</span>
                <span class="rounded-[3px] bg-amber-fill text-label">{matched}</span>
                <span>{after}</span>
            </span>
        </button>
    }
}
