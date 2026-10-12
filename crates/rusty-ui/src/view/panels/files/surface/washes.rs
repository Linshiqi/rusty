//! What is washed under or outlined over the text: find matches, the
//! other places the name at the caret occurs, the bracket pair, and the
//! line the debugger stopped on. Rectangles placed as the text draws its
//! columns, hints included.

use super::*;

/// The line the target is stopped on.
#[component]
pub(super) fn StopLine(pane: Pane) -> impl IntoView {
    let Pane { state, zoom, .. } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let debug = state.debug.session.get()?;
                if debug.running {
                    return None;
                }
                let frame = debug.stack.get(debug.frame as usize)?;
                if frame.file.as_deref() != Some(path.as_str()) {
                    return None;
                }
                let line = frame.line?;
                let y = row_top(state, line, zoom.get());
                let height = row_height(zoom.get());
                Some(view! {
                    <div
                        class="pointer-events-none absolute left-0 w-full bg-amber-fill"
                        style=format!("top: {y}px; height: {height}px")
                    />
                })
            }
        }
    }
}

/// The bracket beside the caret and its pair, outlined.
#[component]
pub(super) fn BracketMarks(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        zoom,
        window,
        brackets,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
            let Some((open, close)) = brackets.get() else {
                return ().into_any();
            };
            let range = window.get();
            let z = zoom.get();
            state
                .editor
                .draft
                .with(|text| {
                    [open, close]
                        .into_iter()
                        .filter(|(line, _)| {
                            !state.editor.folds.with(|f| f.hides(*line))
                                && range.contains(&row_for(state, *line))
                        })
                        .filter_map(|(line, col)| {
                            let content = text.split('\n').nth(line as usize)?;
                            let placed = columns_at(state, &path, content, line, col, col + 1, z);
                            Some(wash_box(
                                "pointer-events-none absolute rounded-[2px] ring-1 ring-label-3",
                                placed,
                                z,
                            ))
                        })
                        .collect_view()
                })
                .into_any()
        }}
    }
}

/// The other places the name at the caret occurs.
#[component]
pub(super) fn OccurrenceWash(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        zoom,
        window,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
                let range = window.get();
                let z = zoom.get();
                state
                    .editor
                    .occurrences
                    .with(|found| {
                        let Some((_, ranges)) = found
                            .as_ref()
                            .filter(|(for_path, _)| for_path == &path)
                        else {
                            return ().into_any();
                        };
                        state
                            .editor
                            .draft
                            .with(|text| {
                                ranges
                                    .iter()
                                    .filter(|r| {
                                        r.start_line == r.end_line
                                            && !state.editor.folds.with(|f| f.hides(r.start_line))
                                            && range.contains(&row_for(state, r.start_line))
                                    })
                                    .filter_map(|r| {
                                        let content = text.split('\n').nth(r.start_line as usize)?;
                                        let (x, y, width) = columns_at(
                                            state,
                                            &path,
                                            content,
                                            r.start_line,
                                            r.start_col,
                                            r.end_col,
                                            z,
                                        );
                                        Some(wash_box(
                                            "pointer-events-none absolute rounded-[3px] bg-slate-fill",
                                            (x, y, width.max(2.0)),
                                            z,
                                        ))
                                    })
                                    .collect_view()
                            })
                            .into_any()
                    })
            }
        }
    }
}

/// The find matches in the window, the current one in its own colour.
#[component]
pub(super) fn FindWash(pane: Pane) -> impl IntoView {
    let Pane {
        state,
        zoom,
        window,
        ..
    } = pane;
    let path = pane.path.get_value();
    view! {
        {
            let path = path.clone();
            move || {
            if !state.find.open.get() {
                return ().into_any();
            }
            let query = state.find.query.get();
            let case = state.find.case.get();
            let chosen = state.find.index.get();
            let range = window.get();
            let z = zoom.get();
            state
                .editor
                .draft
                .with(|text| {
                    let matches = find_matches(text, &query, case);
                    let current = chosen.min(matches.len().saturating_sub(1));
                    match_lines(text, &matches)
                        .into_iter()
                        .enumerate()
                        .filter(|(_, found)| {
                            !state.editor.folds.with(|folds| folds.hides(found.line))
                                && range.contains(&row_for(state, found.line))
                        })
                        .map(|(index, found)| {
                            let (x, y, width) = columns_at(
                                state,
                                &path,
                                found.text,
                                found.line,
                                found.col,
                                found.end_col,
                                z,
                            );
                            let wash = if index == current {
                                "pointer-events-none absolute rounded-[3px] bg-amber-fill"
                            } else {
                                "pointer-events-none absolute rounded-[3px] bg-selection"
                            };
                            wash_box(wash, (x, y, width.max(2.0)), z)
                        })
                        .collect_view()
                })
                .into_any()
        }}
    }
}

/// Where columns `from..to` of `line` are drawn, the text's own placement
/// with its hints: (left, top, width) at zoom `z`.
fn columns_at(
    state: AppState,
    path: &str,
    content: &str,
    line: u32,
    from: u32,
    to: u32,
    z: f64,
) -> (f64, f64, f64) {
    let hints = hints_on(state, path, line);
    let x = char_left(content, &hints, from, z);
    let width = edge_left(content, &hints, to, z) - x;
    (x, row_top(state, line, z), width)
}

/// A box of `class` over the text, a row tall.
fn wash_box(class: &'static str, (x, y, width): (f64, f64, f64), z: f64) -> impl IntoView {
    view! {
        <div
            class=class
            style=format!(
                "left: {x}px; top: {y}px; width: {width}px; height: {h}px",
                h = row_height(z),
            )
        />
    }
}
