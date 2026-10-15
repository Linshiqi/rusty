//! Sticky scroll: the lines that open the blocks the top of the view is
//! inside, kept at the top of the editor while it scrolls through them — VS
//! Code's, from the same regions the fold chevrons use.
//!
//! An overlay pinned to the scroller's top edge, never rows in the text: the
//! textarea and the echo must stay glyph for glyph, and a row that exists in
//! one and not the other is a caret that drifts.

use leptos::{html, prelude::*};

use rusty_edit::{Region, sticky::stuck};

use super::*;
use crate::state::AppState;

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
                        <div class="pr-4 pl-2 whitespace-pre">{decorate(painted, line, &[], &[], None)}</div>
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
