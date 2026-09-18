//! Sticky scroll: the lines that open the blocks the top of the view is
//! inside, kept at the top of the editor while it scrolls through them — VS
//! Code's, from the same regions the fold chevrons use.
//!
//! An overlay pinned to the scroller's top edge, never rows in the text: the
//! textarea and the echo must stay glyph for glyph, and a row that exists in
//! one and not the other is a caret that drifts.

use leptos::{html, prelude::*};

use rusty_edit::Region;

use super::*;
use crate::state::AppState;

/// At most this many lines stick, the outermost first — VS Code's default.
const MOST: usize = 5;

/// The headers of the regions `line` is inside, outermost first. A header is
/// not inside its own region: on screen it needs no copy of itself.
fn headers_over(regions: &[Region], line: u32) -> Vec<u32> {
    let mut found: Vec<u32> = regions
        .iter()
        .filter(|region| region.header < line && line <= region.last)
        .map(|region| region.header)
        .collect();
    found.sort_unstable();
    found.truncate(MOST);
    found
}

/// The lines that stick when the view's rows begin with `line_at(0)`. They
/// cover the rows they take, so what they are about is the line just under
/// them, and more stick while that line is inside more blocks. Only ever
/// more: at a block's closing brace, one line fewer would uncover a line of
/// that block and one more would cover the brace, and a count that took
/// turns between the two would flicker with every row scrolled.
fn stuck(regions: &[Region], line_at: impl Fn(u32) -> u32) -> Vec<u32> {
    let mut found = headers_over(regions, line_at(0));
    loop {
        let next = headers_over(regions, line_at(found.len() as u32));
        if next.len() <= found.len() {
            return found;
        }
        found = next;
    }
}

/// The overlay, drawn when sticky scroll is on and there is something to
/// stick. `gutter` is the margin's own style, so the numbers stand where the
/// margin's numbers stand.
#[allow(clippy::too_many_arguments)]
pub(super) fn sticky_lines(
    state: AppState,
    path: String,
    scroller: NodeRef<html::Div>,
    view_top: RwSignal<f64>,
    view_left: RwSignal<f64>,
    regions: Memo<Vec<Region>>,
    metrics: Signal<String>,
    gutter: Signal<String>,
) -> impl IntoView {
    let zoom = state.editor.zoom;
    let lines = Memo::new(move |_| {
        if !state.editor.view.with(|view| view.sticky_scroll) {
            return Vec::new();
        }
        let height = row_height(zoom.get());
        let first = ((view_top.get() - PAD_PX).max(0.0) / height).floor() as u32;
        state.editor.folds.with(|folds| {
            regions.with(|regions| stuck(regions, |row| folds.doc_of_view(first + row)))
        })
    });
    move || {
        let found = lines.get();
        if found.is_empty() {
            return ().into_any();
        }
        let z = zoom.get_untracked();
        let icon_px = (row_height(z) * 0.68).round().max(7.0) as u32;
        let folds_column = regions.with_untracked(|regions| !regions.is_empty());
        let semantic = state.editor.semantic.with_untracked(|semantic| {
            semantic
                .as_ref()
                .filter(|(for_path, _)| for_path == &path)
                .map(|(_, spans)| spans.clone())
                .unwrap_or_default()
        });
        let rows = found
            .into_iter()
            .enumerate()
            .filter_map(|(at, line)| {
                let painted = state
                    .editor
                    .highlighted
                    .with_untracked(|lines| lines.get(line as usize).cloned())?;
                let painted = overlay_semantic(painted, line, semantic_on(&semantic, line));
                // To the line, with the lines still stuck above it standing
                // over the rows before it rather than over the line itself.
                let jump = move |_| {
                    if let Some(element) = scroller.get_untracked() {
                        let row = row_for(state, line).saturating_sub(at as u32);
                        let top = f64::from(row) * row_height(zoom.get_untracked());
                        element.set_scroll_top(top as i32);
                    }
                };
                let slot = format!("width: {icon_px}px");
                Some(view! {
                    <div
                        class="flex w-max min-w-full cursor-pointer hover:bg-sunken"
                        on:mousedown=|event: leptos::ev::MouseEvent| event.prevent_default()
                        on:click=jump
                    >
                        <div class="flex-none pr-2 pl-3 text-right text-label-4 select-none" style=gutter>
                            <div class="flex items-center justify-end gap-1.5">
                                <span class="text-transparent">"●"</span>
                                <span>{(line + 1).to_string()}</span>
                                {folds_column.then(|| view! { <span class="shrink-0" style=slot.clone() /> })}
                            </div>
                        </div>
                        <div class="pr-4 pl-2 whitespace-pre">{decorate(painted, line, &[])}</div>
                    </div>
                })
            })
            .collect_view();
        view! {
            // No height of its own, so nothing under it moves; sticky to the
            // scroller's top and left edges, and scrolled sideways with the
            // text inside it.
            <div class="pointer-events-none sticky top-0 left-0 z-20 h-0">
                <div
                    class="pointer-events-auto absolute inset-x-0 top-0 overflow-hidden border-b border-line bg-content shadow-sm"
                    style=move || metrics.get()
                >
                    <div style=move || format!("transform: translateX(-{}px)", view_left.get())>
                        {rows}
                    </div>
                </div>
            </div>
        }
        .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regions() -> Vec<Region> {
        vec![
            Region {
                header: 0,
                last: 10,
            },
            Region { header: 2, last: 5 },
            Region { header: 3, last: 4 },
            Region { header: 7, last: 9 },
        ]
    }

    #[test]
    fn the_blocks_a_line_is_in_stick_outermost_first() {
        assert_eq!(headers_over(&regions(), 4), [0, 2, 3]);
        assert_eq!(headers_over(&regions(), 8), [0, 7]);
        assert_eq!(headers_over(&regions(), 11), Vec::<u32>::new());
    }

    /// A line that opens a block is on screen already: it does not stick
    /// over itself.
    #[test]
    fn a_header_does_not_stick_over_itself() {
        assert_eq!(headers_over(&regions(), 3), [0, 2]);
        assert_eq!(headers_over(&regions(), 0), Vec::<u32>::new());
    }

    /// A view starting at line 7 has line 0 stuck over its first row, which
    /// leaves line 8 as the first one showing — inside line 7's block, so
    /// line 7 sticks too.
    #[test]
    fn what_sticks_is_about_the_line_under_it() {
        assert_eq!(stuck(&regions(), |row| 1 + row), [0]);
        assert_eq!(stuck(&regions(), |row| 3 + row), [0, 2]);
        assert_eq!(stuck(&regions(), |row| 7 + row), [0, 7]);
    }

    /// At an inner block's closing brace the answer does not take turns: with
    /// the view at line 4, lines 0, 2 and 3 are over the rows of lines 4 to
    /// 6, and line 7 — outside the inner blocks — would have only line 0.
    #[test]
    fn a_closing_brace_does_not_make_the_lines_flicker() {
        assert_eq!(stuck(&regions(), |row| 4 + row), [0, 2, 3]);
        assert_eq!(stuck(&regions(), |row| 5 + row), [0, 2]);
    }

    #[test]
    fn no_more_than_five_lines_stick() {
        let deep: Vec<Region> = (0..8)
            .map(|n| Region {
                header: n,
                last: 20,
            })
            .collect();
        assert_eq!(headers_over(&deep, 12), [0, 1, 2, 3, 4]);
    }
}
