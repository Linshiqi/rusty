//! Find and replace within the open document.

use leptos::{ev, html, prelude::*};

use super::*;
use crate::{controller, state::AppState};

/// Every occurrence of `query` in `text`, as byte ranges.
///
/// ASCII-case-folded like project search's literal mode, capped so a
/// one-letter query in a big file cannot melt the renderer.
pub(super) fn find_matches(text: &str, query: &str, case: bool) -> Vec<(usize, usize)> {
    const CAP: usize = 2000;
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let hay = text.as_bytes();
    let needle = query.as_bytes();
    let mut at = 0;
    while at + needle.len() <= hay.len() && out.len() < CAP {
        let here = &hay[at..at + needle.len()];
        let matched = if case {
            here == needle
        } else {
            here.eq_ignore_ascii_case(needle)
        };
        if matched && text.is_char_boundary(at) {
            out.push((at, at + needle.len()));
            at += needle.len().max(1);
        } else {
            at += 1;
        }
    }
    out
}

/// (line, scalar column) of a byte offset.
pub(super) fn line_col_of_byte(text: &str, at: usize) -> (u32, u32) {
    let before = &text[..at.min(text.len())];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map(|found| found + 1).unwrap_or(0);
    (line, before[start..].chars().count() as u32)
}

/// Step the current find match by `direction`, wrapping, and show it.
pub(super) fn find_jump(state: AppState, scroller: NodeRef<html::Div>, direction: i32) {
    let text = state.editor.draft.get_untracked();
    let query = state.find.query.get_untracked();
    let matches = find_matches(&text, &query, state.find.case.get_untracked());
    if matches.is_empty() {
        return;
    }
    let current = state.find.index.get_untracked().min(matches.len() - 1);
    let next = if direction >= 0 {
        (current + 1) % matches.len()
    } else {
        (current + matches.len() - 1) % matches.len()
    };
    state.find.index.set(next);
    let (line, _) = line_col_of_byte(&text, matches[next].0);
    if let Some(outer) = scroller.get_untracked() {
        let lh = row_height(state.editor.zoom.get_untracked());
        // Find searches the whole document, folds and all, so the match's
        // line has to be turned into the row it is drawn on — otherwise
        // pressing Enter past a collapsed function scrolls to empty space.
        let y = 8.0 + f64::from(row_for(state, line)) * lh;
        let top = f64::from(outer.scroll_top());
        let height = f64::from(outer.client_height());
        if y < top + lh || y + lh * 2.0 > top + height {
            outer.set_scroll_top((y - height / 3.0).max(0.0) as i32);
        }
    }
}

/// Replace the current match, or every match, through the undo pipeline.
fn find_replace(state: AppState, area: NodeRef<html::Textarea>, all: bool) {
    let text = state.editor.draft.get_untracked();
    let query = state.find.query.get_untracked();
    let matches = find_matches(&text, &query, state.find.case.get_untracked());
    if matches.is_empty() {
        return;
    }
    let replacement = state.find.replace.get_untracked();

    record_edit(state);
    let mut new = text.clone();
    if all {
        for (from, to) in matches.iter().rev() {
            new.replace_range(*from..*to, &replacement);
        }
    } else {
        let current = state.find.index.get_untracked().min(matches.len() - 1);
        let (from, to) = matches[current];
        new.replace_range(from..to, &replacement);
    }

    echo_edit(state, &new);
    if let Some(element) = area.get_untracked() {
        set_buffer(state, &element, &new);
    } else {
        state.editor.draft.set(new.clone());
    }
    controller::schedule_pulse(state);
}

/// The floating find/replace bar, top right of the editor.
#[component]
pub(super) fn FindBar(
    area: NodeRef<html::Textarea>,
    scroller: NodeRef<html::Div>,
) -> impl IntoView {
    let state = AppState::expect();
    let input: NodeRef<html::Input> = NodeRef::new();

    // Opening focuses the query box with its text selected, ready to retype.
    Effect::new(move |_| {
        if state.find.open.get()
            && let Some(element) = input.get_untracked()
        {
            let _ = element.focus();
            element.select();
        }
    });

    let counter = Signal::derive(move || {
        let text = state.editor.draft.get();
        let query = state.find.query.get();
        let matches = find_matches(&text, &query, state.find.case.get());
        if query.is_empty() {
            String::new()
        } else if matches.is_empty() {
            "no results".to_string()
        } else {
            let current = state.find.index.get().min(matches.len() - 1);
            format!("{}/{}", current + 1, matches.len())
        }
    });

    let small = "grid size-6 place-items-center rounded-[5px] text-footnote text-label-3 \
                 hover:bg-sunken hover:text-label";

    view! {
        <Show when=move || state.find.open.get()>
            <div class="absolute top-2 right-6 z-30 flex flex-col gap-1 rounded-[8px] bg-raised p-1.5 shadow-2xl ring-1 ring-line-strong">
                <div class="flex items-center gap-1">
                    <input
                        node_ref=input
                        type="text"
                        placeholder="Find"
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || state.find.query.get()
                        on:input=move |event: ev::Event| {
                            state.find.query.set(event_target_value(&event));
                            state.find.index.set(0);
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            match event.key().as_str() {
                                "Enter" => {
                                    event.prevent_default();
                                    find_jump(
                                        state,
                                        scroller,
                                        if event.shift_key() { -1 } else { 1 },
                                    );
                                }
                                "Escape" => {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    state.find.open.set(false);
                                    state.find.replace_open.set(false);
                                    if let Some(element) = area.get_untracked() {
                                        let _ = element.focus();
                                    }
                                }
                                _ => {}
                            }
                        }
                        class="w-[200px] rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote text-label placeholder:text-label-3"
                    />
                    <button
                        type="button"
                        title="Match case"
                        on:click=move |_| {
                            state.find.case.update(|case| *case = !*case);
                            state.find.index.set(0);
                        }
                        class=move || {
                            let base = "rounded-[5px] px-1.5 py-0.5 font-mono text-footnote";
                            if state.find.case.get() {
                                format!("{base} bg-selection text-rust")
                            } else {
                                format!("{base} text-label-3 hover:text-label")
                            }
                        }
                    >
                        "Aa"
                    </button>
                    <span class="min-w-[6ch] px-1 text-center font-mono text-caption text-label-3">
                        {counter}
                    </span>
                    <button
                        type="button"
                        title="Previous (Shift+Enter)"
                        on:click=move |_| find_jump(state, scroller, -1)
                        class=small
                    >
                        "↑"
                    </button>
                    <button
                        type="button"
                        title="Next (Enter)"
                        on:click=move |_| find_jump(state, scroller, 1)
                        class=small
                    >
                        "↓"
                    </button>
                    <button
                        type="button"
                        title="Replace…"
                        on:click=move |_| {
                            state.find.replace_open.update(|open| *open = !*open)
                        }
                        class=small
                    >
                        "⇄"
                    </button>
                    <button
                        type="button"
                        title="Close (Esc)"
                        on:click=move |_| {
                            state.find.open.set(false);
                            state.find.replace_open.set(false);
                        }
                        class=small
                    >
                        "×"
                    </button>
                </div>
                <Show when=move || state.find.replace_open.get()>
                    <div class="flex items-center gap-1">
                        <input
                            type="text"
                            placeholder="Replace with"
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || state.find.replace.get()
                            on:input=move |event: ev::Event| {
                                state.find.replace.set(event_target_value(&event))
                            }
                            class="w-[200px] rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote text-label placeholder:text-label-3"
                        />
                        <button
                            type="button"
                            on:click=move |_| find_replace(state, area, false)
                            class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            "Replace"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| find_replace(state, area, true)
                            class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            "All"
                        </button>
                    </div>
                </Show>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    #[test]
    fn matches_fold_ascii_case_and_respect_the_toggle() {
        let text = "Gain gain GAIN";
        assert_eq!(
            find_matches(text, "gain", false),
            vec![(0, 4), (5, 9), (10, 14)],
        );
        assert_eq!(find_matches(text, "gain", true), vec![(5, 9)]);
        assert!(find_matches(text, "", false).is_empty());
    }

    #[test]
    fn byte_offsets_convert_to_scalar_columns_past_cjk() {
        let text = "// 中文
let gain = 1;";
        let matches = find_matches(text, "gain", false);
        assert_eq!(matches.len(), 1);
        let (line, col) = line_col_of_byte(text, matches[0].0);
        assert_eq!((line, col), (1, 4));
    }

    #[test]
    fn overlapping_repeats_advance_past_each_match() {
        assert_eq!(find_matches("aaaa", "aa", false), vec![(0, 2), (2, 4)]);
    }
}
