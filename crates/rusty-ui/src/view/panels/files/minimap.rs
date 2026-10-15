//! The minimap: the whole file, small, down the right-hand edge — VS Code's,
//! drawn as blocks of each run's colour on a canvas, with the rows the view
//! shows marked by a slider. A press on it brings that part of the file into
//! the middle of the view, and dragging keeps doing so.
//!
//! A canvas, not markup: a row of it is two pixels, and a file of any length
//! would be thousands of elements. It draws the rows its height holds and no
//! more, so a keystroke in a long file costs a few hundred rectangles.

use leptos::{ev, html, prelude::*};
use wasm_bindgen::JsCast;

use rusty_edit::Token;
use rusty_edit::minimap::{CHAR, ROW, WIDTH, frame, ink, scroll_for};

use super::*;
use crate::state::AppState;

/// Every token, for the colours the canvas reads off the stylesheet.
const TOKENS: [Token; 11] = [
    Token::Plain,
    Token::Keyword,
    Token::Str,
    Token::Number,
    Token::Comment,
    Token::Type,
    Token::Function,
    Token::Macro,
    Token::Punctuation,
    Token::Variable,
    Token::Namespace,
];

/// The minimap beside the editor's scroller, when it is on.
#[component]
pub(super) fn Minimap(
    scroller: NodeRef<html::Div>,
    view_top: RwSignal<f64>,
    view_height: RwSignal<f64>,
    rows_total: Memo<u32>,
) -> impl IntoView {
    let state = AppState::expect();
    let canvas = NodeRef::<html::Canvas>::new();
    let probes = NodeRef::<html::Div>::new();
    let zoom = state.editor.zoom;
    let hovered = RwSignal::new(false);
    let dragging = StoredValue::new(false);

    // Where the minimap stands now, for a press to be read against.
    let current = move || {
        let height = canvas
            .get_untracked()
            .map_or(0.0, |canvas| f64::from(canvas.client_height()));
        frame(
            rows_total.get_untracked(),
            row_height(zoom.get_untracked()),
            PAD_PX,
            view_top.get_untracked(),
            view_height.get_untracked(),
            height,
        )
    };
    let scroll_to = move |client_y: i32| {
        let (Some(canvas), Some(element)) = (canvas.get_untracked(), scroller.get_untracked())
        else {
            return;
        };
        let y = f64::from(client_y) - canvas.get_bounding_client_rect().top();
        let top = scroll_for(
            y,
            current(),
            row_height(zoom.get_untracked()),
            PAD_PX,
            view_height.get_untracked(),
        );
        element.set_scroll_top(top as i32);
    };

    // Dragging carries on outside the minimap, as a scrollbar's does; the
    // listeners go with the view.
    let moving = window_event_listener(ev::mousemove, move |event| {
        if dragging.get_value() {
            scroll_to(event.client_y());
        }
    });
    let released = window_event_listener(ev::mouseup, move |_| dragging.set_value(false));
    let listeners = StoredValue::new_local(Some((moving, released)));
    on_cleanup(move || {
        if let Some(Some((moving, released))) = listeners.try_update_value(Option::take) {
            moving.remove();
            released.remove();
        }
    });

    // Drawn again whenever what it shows moves: the text, the folds, the
    // scroll, the size, the zoom — and the slider's shade on hover.
    Effect::new(move |_| {
        let Some(canvas) = canvas.get() else {
            return;
        };
        let rows = rows_total.get();
        let row_px = row_height(zoom.get());
        let (top, view) = (view_top.get(), view_height.get());
        let lit = hovered.get();
        let width = f64::from(canvas.client_width());
        let height = f64::from(canvas.client_height());
        let ratio = web_sys::window().map_or(1.0, |w| w.device_pixel_ratio());
        canvas.set_width((width * ratio).round() as u32);
        canvas.set_height((height * ratio).round() as u32);
        let Some(context) = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|context| context.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
        else {
            return;
        };
        let _ = context.set_transform(ratio, 0.0, 0.0, ratio, 0.0, 0.0);
        context.clear_rect(0.0, 0.0, width, height);

        let colours = colours(probes.get_untracked());
        let at = frame(rows, row_px, PAD_PX, top, view, height);
        let first = (at.scroll / ROW).floor() as u32;
        let last = ((at.scroll + height) / ROW).ceil() as u32;
        context.set_global_alpha(0.65);
        state.editor.folds.with(|folds| {
            state.editor.highlighted.with(|lines| {
                for row in first..last.min(rows) {
                    let Some(line) = lines.get(folds.doc_of_view(row) as usize) else {
                        continue;
                    };
                    let y = f64::from(row) * ROW - at.scroll;
                    for (col, len, token) in ink(line, TAB_SIZE as u32) {
                        let x = f64::from(col) * CHAR;
                        if x >= width {
                            break;
                        }
                        let colour = colours
                            .iter()
                            .find(|(t, _)| *t == token)
                            .map_or("gray", |(_, c)| c.as_str());
                        context.set_fill_style_str(colour);
                        context.fill_rect(x, y, f64::from(len) * CHAR, ROW - 0.5);
                    }
                }
            })
        });
        // The slider: the view's rows, shaded, darker under the pointer.
        let plain = colours
            .iter()
            .find(|(t, _)| *t == Token::Plain)
            .map_or("gray", |(_, c)| c.as_str());
        context.set_global_alpha(if lit { 0.16 } else { 0.08 });
        context.set_fill_style_str(plain);
        context.fill_rect(0.0, at.slider_top, width, at.slider_height);
        context.set_global_alpha(1.0);
    });

    let probe_spans = TOKENS
        .iter()
        .map(|token| view! { <span class=class_of(*token) data-token=format!("{token:?}") /> })
        .collect_view();
    view! {
        <div
            class="relative flex-none border-l border-line"
            style=format!("width: {WIDTH}px")
            on:mouseenter=move |_| hovered.set(true)
            on:mouseleave=move |_| hovered.set(false)
        >
            <canvas
                node_ref=canvas
                class="absolute inset-0 h-full w-full cursor-default"
                // The editor keeps the keyboard: this is a way of scrolling,
                // not a place to type.
                on:mousedown=move |event: ev::MouseEvent| {
                    event.prevent_default();
                    dragging.set_value(true);
                    scroll_to(event.client_y());
                }
            />
            <div node_ref=probes class="hidden">{probe_spans}</div>
        </div>
    }
}

/// Each token's colour as the stylesheet has it now, read off hidden spans
/// wearing the same classes as the text — so the theme is the theme.
fn colours(probes: Option<web_sys::HtmlDivElement>) -> Vec<(Token, String)> {
    let (Some(probes), Some(window)) = (probes, web_sys::window()) else {
        return Vec::new();
    };
    let mut next = probes.first_element_child();
    TOKENS
        .iter()
        .filter_map(|token| {
            let probe = next.take()?;
            next = probe.next_element_sibling();
            let colour = window
                .get_computed_style(&probe)
                .ok()
                .flatten()?
                .get_property_value("color")
                .ok()?;
            Some((*token, colour))
        })
        .collect()
}
