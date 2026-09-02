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
use crate::{
    controller,
    state::AppState,
    view::{
        components::Empty,
        icon::{Icon, IconView},
    },
};

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
                        <button
                            type="button"
                            title=t!("replace.toggle")
                            on:click=move |_| {
                                state.search.replacing.update(|open| *open = !*open)
                            }
                            class=move || {
                                let base = "grid size-6 shrink-0 place-items-center rounded-[6px] transition-colors";
                                if state.search.replacing.get() {
                                    format!("{base} bg-selection text-rust")
                                } else {
                                    format!("{base} text-label-3 hover:text-label")
                                }
                            }
                        >
                            <span class=move || {
                                if state.search.replacing.get() {
                                    "grid rotate-90 transition-transform"
                                } else {
                                    "grid transition-transform"
                                }
                            }>
                                <IconView icon=Icon::Chevron size=12 />
                            </span>
                        </button>
                    </div>
                    <Show when=move || state.search.replacing.get()>
                        <div class="flex items-center gap-1.5">
                            <input
                                type="text"
                                placeholder=t!("replace.placeholder")
                                autocomplete="off"
                                spellcheck="false"
                                prop:value=move || state.search.replacement.get()
                                on:input=move |event: ev::Event| {
                                    state.search.replacement.set(event_target_value(&event));
                                }
                                class="min-w-0 flex-1 rounded-[6px] bg-sunken px-2.5 py-1.5 font-mono text-footnote text-label placeholder:text-label-3"
                            />
                            // Disabled until there is something to act on, so
                            // the button cannot be a rewrite of nothing.
                            <button
                                type="button"
                                title=t!("replace.run-hint")
                                disabled=move || {
                                    state
                                        .search
                                        .results
                                        .with(|r| {
                                            r.as_ref().is_none_or(|r| r.hits.is_empty())
                                        })
                                }
                                on:click=move |_| controller::replace_all(state)
                                class="shrink-0 rounded-[6px] bg-rust px-2.5 py-1.5 text-footnote font-medium text-canvas hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                            >
                                {t!("replace.run")}
                            </button>
                        </div>
                    </Show>
                    <GlobBox
                        placeholder=t!("search.include")
                        value=state.search.include
                    />
                    <GlobBox
                        placeholder=t!("search.exclude")
                        value=state.search.exclude
                    />
                </div>

                <ReplaceOutcomeNote />

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
                            t!(
                                "results.truncated",
                                count = results.hits.len(),
                                files = results.files,
                            )
                        } else {
                            t!(
                                "results.count",
                                count = results.hits.len(),
                                files = results.files,
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

/// What the last replace did, and what it would not touch.
///
/// Stays until dismissed rather than fading: a replace cannot be undone from
/// here, and the files it refused are the half somebody has to go and fix.
#[component]
fn ReplaceOutcomeNote() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let outcome = state.search.outcome.get()?;
        let tone = if outcome.error.is_some() || !outcome.skipped.is_empty() {
            "border-amber bg-amber-fill"
        } else {
            "border-line bg-sunken"
        };
        let summary = match &outcome.error {
            Some(error) => error.clone(),
            None if outcome.changed.is_empty() && outcome.skipped.is_empty() => {
                t!("replace.none")
            }
            None => t!(
                "replace.done",
                count = outcome.replaced,
                files = outcome.changed.len(),
            ),
        };
        let skipped = outcome.skipped.clone();
        Some(view! {
            <div class=format!("mx-2 mt-2 rounded-[8px] border px-2.5 py-2 {tone}")>
                <div class="flex items-start gap-2">
                    <p class="min-w-0 flex-1 text-footnote leading-relaxed text-label select-text">
                        {summary}
                    </p>
                    <button
                        type="button"
                        title=t!("replace.dismiss")
                        on:click=move |_| state.search.outcome.set(None)
                        class="shrink-0 rounded-[5px] px-1 text-footnote text-label-3 hover:text-label"
                    >
                        "×"
                    </button>
                </div>
                {(!skipped.is_empty())
                    .then(|| {
                        view! {
                            <p class="mt-1 text-caption text-label-2">
                                {t!("replace.skipped", count = skipped.len())}
                            </p>
                            <ul class="mt-0.5 flex flex-col gap-0.5">
                                {skipped
                                    .into_iter()
                                    .map(|skip| {
                                        // Keyed on the reason name, never on a
                                        // sentence the backend composed.
                                        let why = rusty_i18n::translate(
                                                &format!("replace.skip-{}", skip.reason),
                                            )
                                            .unwrap_or(skip.reason);
                                        view! {
                                            <li class="font-mono text-caption text-label-3 select-text">
                                                {skip.path}" — "{why}
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })}
            </div>
        })
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

    let (before, matched, after) =
        preview(&hit.text, hit.span_start as usize, hit.span_end as usize);

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

/// How much of the line before the match is worth keeping.
///
/// Enough to see what the match sits in — a `let`, a `fn`, the opening of a
/// call — and no more. Wider and a deeply indented hit is off screen again.
const LEAD: usize = 14;

/// One hit's line, cut so the match is on screen.
///
/// **The panel exists to show where a word is, and it was showing everything
/// but.** A result row rendered `text` verbatim and truncated on the right, so
/// a match at column 90 of a long line — or at the end of four levels of
/// indentation — was a row of leading spaces and unrelated code with the
/// keyword somewhere past the edge.
///
/// Two cuts, in order. The indentation goes first, because it is never the
/// answer to "where is this word". If the match is *still* too far in, the
/// prefix is elided to its last [`LEAD`] characters behind a `…`, which is
/// what VSCode does and reads as "there is more to the left" rather than as a
/// truncation.
///
/// Byte offsets in, three strings out. Offsets that do not land on character
/// boundaries — a windowed line cut mid-codepoint — give up and show the line
/// with nothing lit, which is wrong but never panics.
fn preview(text: &str, start: usize, end: usize) -> (String, String, String) {
    let end = end.min(text.len());
    if start > end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return (text.to_string(), String::new(), String::new());
    }

    // A match *inside* the indentation is a search for whitespace, and
    // trimming it would delete the thing being looked for.
    let indent = text.len() - text.trim_start().len();
    let cut = if start >= indent { indent } else { 0 };

    let mut before = &text[cut..start];
    let mut ellipsis = false;
    if before.chars().count() > LEAD {
        let keep = before
            .char_indices()
            .nth_back(LEAD - 1)
            .map_or(0, |(at, _)| at);
        before = &before[keep..];
        ellipsis = true;
    }

    let before = if ellipsis {
        format!("…{before}")
    } else {
        before.to_string()
    };
    (
        before,
        text[start..end].to_string(),
        text[end..].to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::{LEAD, preview};

    #[test]
    fn indentation_never_pushes_the_match_off_the_row() {
        let line = "                let gain = compute();";
        let at = line.find("gain").unwrap();
        let (before, matched, _) = preview(line, at, at + 4);
        assert_eq!(matched, "gain");
        assert_eq!(before, "let ", "the indentation is not the answer to where");
    }

    #[test]
    fn a_far_match_keeps_its_immediate_context_behind_an_ellipsis() {
        let line = "let result = some_module::helper(alpha, beta, gamma, needle);";
        let at = line.find("needle").unwrap();
        let (before, matched, after) = preview(line, at, at + 6);
        assert_eq!(matched, "needle");
        assert!(
            before.starts_with('…'),
            "expected an elision, got {before:?}"
        );
        assert!(
            before.chars().count() <= LEAD + 1,
            "the prefix is still too long: {before:?}"
        );
        assert!(
            before.ends_with("gamma, "),
            "the nearest context is what to keep"
        );
        assert_eq!(after, ");");
    }

    /// Searching for whitespace is the one case where the indentation *is* the
    /// match, and trimming it would delete what was asked for.
    #[test]
    fn a_match_inside_the_indentation_survives() {
        let line = "\t\tlet x = 1;";
        let (before, matched, _) = preview(line, 0, 2);
        assert_eq!(matched, "\t\t");
        assert_eq!(before, "");
    }

    /// A windowed line can be cut mid-codepoint. Wrong is acceptable here;
    /// panicking in a results list is not.
    #[test]
    fn an_offset_off_a_character_boundary_does_not_panic() {
        let line = "中文 needle";
        let (before, matched, after) = preview(line, 1, 2);
        assert_eq!(before, line);
        assert!(matched.is_empty() && after.is_empty());
    }

    #[test]
    fn a_short_line_is_left_alone() {
        let line = "let a = b;";
        let at = line.find('b').unwrap();
        let (before, matched, after) = preview(line, at, at + 1);
        assert_eq!(
            (before.as_str(), matched.as_str(), after.as_str()),
            ("let a = ", "b", ";")
        );
    }
}
