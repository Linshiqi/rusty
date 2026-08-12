//! Project-wide text search.
//!
//! A debounced box over `rusty-edit`'s walker: results grouped by file, a
//! click lands in the editor on the exact line and column, the same way
//! goto-definition lands. State lives in [`AppState`] so switching panels
//! does not throw the results away.

use leptos::{ev, prelude::*};

use rusty_edit::SearchHit;

use crate::{controller, state::AppState, view::components::Empty};

#[component]
pub fn SearchPanel() -> impl IntoView {
    let state = AppState::expect();

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title="No project open"
                    detail="Open a folder to search what is in it."
                />
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <div class="flex items-center gap-2 border-b border-line px-3 py-2">
                    <input
                        type="text"
                        placeholder="Search the project…"
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || state.search_query.get()
                        on:input=move |event: ev::Event| {
                            state.search_query.set(event_target_value(&event));
                            controller::schedule_search(state);
                        }
                        class="min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 py-1.5 font-mono text-footnote text-label placeholder:text-label-3"
                    />
                    <button
                        type="button"
                        title="Match case"
                        on:click=move |_| {
                            state.search_case.update(|case| *case = !*case);
                            controller::schedule_search(state);
                        }
                        class=move || {
                            let base = "rounded-[6px] px-2 py-1 font-mono text-footnote transition-colors";
                            if state.search_case.get() {
                                format!("{base} bg-selection text-rust")
                            } else {
                                format!("{base} text-label-3 hover:text-label")
                            }
                        }
                    >
                        "Aa"
                    </button>
                </div>

                <div class="min-h-0 flex-1 overflow-auto px-1 py-1">
                    {move || {
                        let Some(results) = state.search_results.get() else {
                            return view! {
                                <p class="px-3 py-2 text-footnote text-label-3">
                                    "Matches appear as you type."
                                </p>
                            }
                            .into_any();
                        };
                        if results.hits.is_empty() {
                            return view! {
                                <p class="px-3 py-2 text-footnote text-label-3">
                                    "Nothing matches."
                                </p>
                            }
                            .into_any();
                        }

                        let summary = if results.truncated {
                            format!(
                                "first {} matches in {} files — narrow the query for the rest",
                                results.hits.len(),
                                results.files,
                            )
                        } else {
                            format!(
                                "{} matches in {} files",
                                results.hits.len(),
                                results.files,
                            )
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
                                    view! {
                                        <div class="mb-1.5">
                                            <div class="truncate px-3 py-0.5 font-mono text-caption font-semibold text-label-2">
                                                {path}
                                            </div>
                                            {hits.into_iter().map(row).collect_view()}
                                        </div>
                                    }
                                })
                                .collect_view()}
                        }
                        .into_any()
                    }}
                </div>
            </div>
        }
        .into_any()
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
                controller::open_search_hit(state, path.clone(), hit.line, hit.col)
            }
            class="flex w-full items-baseline gap-2 rounded-[5px] px-3 py-[2px] text-left font-mono text-footnote text-label-2 hover:bg-sunken"
        >
            <span class="w-[4ch] shrink-0 text-right text-label-3">{hit.line + 1}</span>
            <span class="min-w-0 flex-1 truncate whitespace-pre">
                <span>{before}</span>
                <span class="rounded-[3px] bg-amber-fill text-label">{matched}</span>
                <span>{after}</span>
            </span>
        </button>
    }
}
