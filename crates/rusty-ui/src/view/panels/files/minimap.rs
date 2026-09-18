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

use rusty_edit::{Line, Token};

use super::*;
use crate::state::AppState;

/// How wide the minimap is, in CSS pixels.
const WIDTH: f64 = 96.0;
/// How tall one of its rows is, and how wide one character.
const ROW: f64 = 2.0;
const CHAR: f64 = 1.0;

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

/// Where the minimap stands against the editor: how far its own rows are
/// scrolled, in its pixels, and where the slider marking the view is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    scroll: f64,
    slider_top: f64,
    slider_height: f64,
}

/// The minimap's scroll and the slider for an editor `view_top` pixels down a
/// text of `rows` rows `row_px` tall, seen through a view `view_height` tall,
/// on a minimap `height` tall. A file that fits is drawn from its top; one
/// that does not is scrolled in step with the editor, so the slider travels
/// the minimap's whole height as the view travels the file — VS Code's
/// arithmetic.
fn frame(rows: u32, row_px: f64, view_top: f64, view_height: f64, height: f64) -> Frame {
    let content = f64::from(rows) * ROW;
    let travel = (f64::from(rows) * row_px + 2.0 * PAD_PX - view_height).max(0.0);
    let scroll = if content <= height || travel <= 0.0 {
        0.0
    } else {
        (view_top / travel).clamp(0.0, 1.0) * (content - height)
    };
    let first = (view_top - PAD_PX).max(0.0) / row_px;
    Frame {
        scroll,
        slider_top: first * ROW - scroll,
        slider_height: (view_height / row_px * ROW).max(ROW),
    }
}

/// The editor scroll that puts minimap `y` — pixels from its top edge — in
/// the middle of the view.
fn scroll_for(y: f64, frame: Frame, row_px: f64, view_height: f64) -> f64 {
    let row = (y + frame.scroll) / ROW;
    (row * row_px + PAD_PX - view_height / 2.0).max(0.0)
}

/// A painted line's runs of ink, as (first column, columns, token):
/// whitespace is paper, and a tab reaches its stop.
fn ink(line: &Line) -> Vec<(u32, u32, Token)> {
    let tab = TAB_SIZE as u32;
    let mut runs = Vec::new();
    let mut col = 0u32;
    for span in &line.spans {
        let mut start = None;
        for ch in span.text.chars() {
            if ch.is_whitespace() {
                if let Some(from) = start.take() {
                    runs.push((from, col - from, span.token));
                }
                col = if ch == '\t' {
                    (col / tab + 1) * tab
                } else {
                    col + 1
                };
            } else {
                start.get_or_insert(col);
                col += 1;
            }
        }
        if let Some(from) = start {
            runs.push((from, col - from, span.token));
        }
    }
    runs
}

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
        let at = frame(rows, row_px, top, view, height);
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
                    for (col, len, token) in ink(line) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_edit::Span;

    fn span(text: &str, token: Token) -> Span {
        Span {
            text: text.to_string(),
            token,
        }
    }

    /// A short file is drawn from its top, and the slider is the view's rows.
    #[test]
    fn a_file_that_fits_is_drawn_from_its_top() {
        let at = frame(100, 20.0, PAD_PX + 10.0 * 20.0, 400.0, 600.0);
        assert_eq!(at.scroll, 0.0);
        assert_eq!(at.slider_top, 10.0 * ROW);
        assert_eq!(at.slider_height, 20.0 * ROW);
    }

    /// A long one scrolls with the editor: at the top of the file the
    /// minimap is at its top, at the bottom at its bottom, and the slider
    /// never leaves it.
    #[test]
    fn a_long_file_scrolls_in_step_with_the_editor() {
        let (rows, row_px, view, height) = (10_000, 20.0, 400.0, 600.0);
        let travel = f64::from(rows) * row_px + 2.0 * PAD_PX - view;
        let top = frame(rows, row_px, 0.0, view, height);
        assert_eq!(top.scroll, 0.0);
        let bottom = frame(rows, row_px, travel, view, height);
        assert_eq!(bottom.scroll, f64::from(rows) * ROW - height);
        assert!(bottom.slider_top + bottom.slider_height <= height + ROW);
        let middle = frame(rows, row_px, travel / 2.0, view, height);
        assert!(middle.slider_top > 0.0 && middle.slider_top < height);
    }

    /// A press puts the row under it in the middle of the view.
    #[test]
    fn a_press_centres_its_row() {
        let at = frame(100, 20.0, 0.0, 400.0, 600.0);
        let top = scroll_for(50.0 * ROW, at, 20.0, 400.0);
        assert_eq!(top, 50.0 * 20.0 + PAD_PX - 200.0);
        assert_eq!(scroll_for(0.0, at, 20.0, 400.0), 0.0, "never above the top");
    }

    #[test]
    fn ink_is_the_runs_between_the_spaces_and_a_tab_reaches_its_stop() {
        let line = Line {
            spans: vec![
                span("\tlet", Token::Keyword),
                span(" x = ", Token::Plain),
                span("\"a b\"", Token::Str),
            ],
        };
        assert_eq!(
            ink(&line),
            [
                (4, 3, Token::Keyword),
                (8, 1, Token::Plain),
                (10, 1, Token::Plain),
                (12, 2, Token::Str),
                (15, 2, Token::Str),
            ]
        );
    }
}
