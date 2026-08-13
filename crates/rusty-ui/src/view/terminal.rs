//! A real terminal.
//!
//! The backend runs a shell behind a pseudo-terminal and emulates the escape
//! sequences, so what arrives here is already a screen: rows of styled runs and
//! a cursor position. This module's whole job is to paint that and to turn
//! keystrokes back into the bytes a terminal expects.
//!
//! Painting it ourselves rather than embedding xterm.js is what keeps the
//! promise of no npm in this repository — and the emulator is `vt100`, which is
//! a solved problem, not something invented here.

use leptos::{ev, html, prelude::*};

use rusty_term::{Colour, Screen, Style};

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator, copy_to_clipboard},
};

/// Cell metrics, in pixels.
///
/// Fixed rather than measured from the font: the size the shell is *told* and
/// the size it is *drawn at* have to agree exactly, or a full-screen program
/// wraps in the wrong place. Measuring is done for the width, because a
/// monospace advance is not derivable from the font size.
const LINE_HEIGHT: f64 = 17.0;
const FONT_SIZE: f64 = 12.0;

#[component]
pub fn TerminalView() -> impl IntoView {
    let state = AppState::expect();
    let host: NodeRef<html::Div> = NodeRef::new();
    let ruler: NodeRef<html::Span> = NodeRef::new();
    // Last size sent, so a resize observer firing on every layout pass does not
    // send an identical size to the shell dozens of times a second.
    let sent = RwSignal::new((0u16, 0u16));
    // One cell's advance in px, measured off the ruler — what turns a mouse
    // position into a column.
    let cell_w = RwSignal::new(0.0f64);
    // A drag selection as (anchor, head), each (row, col) in screen cells.
    // Ours, not the browser's: the grid is a pile of spans the native
    // selection walks in DOM order, which crosses rows diagonally.
    let selection = RwSignal::new(None::<((usize, usize), (usize, usize))>);
    let selecting = RwSignal::new(false);

    // Where the pointer is, in cells — from client coordinates against the
    // host's rect. offsetX cannot be used: it is relative to whichever SPAN
    // the pointer happens to be over, which put selections a prompt's width
    // away from the mouse. The 8/4 are the host's px-2 py-1.
    let cell_at = move |client_x: f64, client_y: f64| {
        let Some(element) = host.get_untracked() else {
            return (0usize, 0usize);
        };
        let rect = element.get_bounding_client_rect();
        let cell = cell_w.get_untracked().max(1.0);
        let col = (((client_x - rect.left()) - 8.0) / cell).floor().max(0.0) as usize;
        let row = (((client_y - rect.top()) - 4.0) / LINE_HEIGHT).floor().max(0.0) as usize;
        (row, col)
    };

    // The selected text, read straight off the screen the frontend already
    // holds — rows of spans, flattened and sliced by column.
    let selection_text = move || -> Option<String> {
        let ((ar, ac), (br, bc)) = selection.get_untracked()?;
        let (start, end) = if (ar, ac) <= (br, bc) {
            ((ar, ac), (br, bc))
        } else {
            ((br, bc), (ar, ac))
        };
        let screen = state.terminal.get_untracked()?;
        let mut out = Vec::new();
        for (row_index, row) in screen.rows.iter().enumerate() {
            if row_index < start.0 || row_index > end.0 {
                continue;
            }
            let text: String = row.spans.iter().map(|span| span.text.as_str()).collect();
            let chars: Vec<char> = text.chars().collect();
            let from = if row_index == start.0 { start.1.min(chars.len()) } else { 0 };
            let to = if row_index == end.0 { end.1.min(chars.len()) } else { chars.len() };
            out.push(chars[from..to].iter().collect::<String>().trim_end().to_string());
        }
        let joined = out.join("\n");
        (!joined.is_empty()).then_some(joined)
    };

    let measure = move || {
        let (Some(host), Some(ruler)) = (host.get_untracked(), ruler.get_untracked()) else {
            return;
        };
        // One hundred characters, so rounding on a fractional advance is a
        // hundredth of a column rather than a whole one.
        let cell = ruler.get_bounding_client_rect().width() / 100.0;
        if cell <= 0.0 {
            return;
        }
        cell_w.set(cell);
        let box_rect = host.get_bounding_client_rect();
        let cols = ((box_rect.width() / cell).floor() as i64).clamp(2, 500) as u16;
        let rows = ((box_rect.height() / LINE_HEIGHT).floor() as i64).clamp(1, 200) as u16;

        if sent.get_untracked() == (cols, rows) {
            return;
        }
        sent.set((cols, rows));
        if state.terminal.with_untracked(Option::is_some) {
            controller::terminal_resize(cols, rows);
        } else {
            controller::open_terminal(state, cols, rows);
        }
    };

    // Re-measure whenever the dock is dragged. A shell told it has 80 columns
    // while being drawn in 120 wraps its own prompt in the wrong place, which
    // looks like a corrupted terminal rather than a stale size.
    Effect::new(move |_| {
        let _ = state.dock_height.get();
        if let Some(element) = host.get() {
            measure();
            // Focus on open, so the shell takes keystrokes without a click
            // first. Every terminal does this and its absence reads as "the
            // terminal is broken".
            let _ = element.focus();
        }
    });

    // The window itself resizing changes the column count just as much.
    let handle = window_event_listener(ev::resize, move |_| measure());
    on_cleanup(move || handle.remove());

    // The reopen half of "changing the shell restarts it". Settings closes
    // the session and clears the screen signal; nothing else would ever call
    // open again — the size has not changed, so the measurer stays quiet —
    // and the panel sat on "Starting a shell…" for ever. Throttled so a
    // shell that fails to start paces at the throttle instead of spinning:
    // the error banner stays readable and the machine stays quiet.
    // Reopen only on a Some -> None transition — an actual close (settings
    // switch, Restart, an error). On first mount the measurer owns the open;
    // this effect also firing there started a SECOND session whose epoch
    // orphaned the first, and the two shells fought over one screen.
    Effect::new(move |prev: Option<bool>| {
        let is_none = state.terminal.with(Option::is_none);
        if is_none && prev == Some(false) {
            let (cols, rows) = sent.get_untracked();
            if cols > 0 && host.get_untracked().is_some() {
                controller::open_terminal(state, cols, rows);
            }
        }
        is_none
    });

    let on_key = move |event: ev::KeyboardEvent| {
        // Let the window's own shortcuts through. `Ctrl K` belongs to the
        // palette even while the terminal has focus; `Ctrl C` does not.
        if event.ctrl_key() && matches!(event.key().as_str(), "k" | "K" | "," | "`") {
            return;
        }
        // Ctrl+C with a selection copies it, as VSCode's terminal does; the
        // interrupt meaning returns the moment nothing is selected.
        if event.ctrl_key()
            && matches!(event.key().as_str(), "c" | "C")
            && selection.get_untracked().is_some()
        {
            event.prevent_default();
            event.stop_propagation();
            if let Some(text) = selection_text() {
                crate::view::components::copy_to_clipboard(&text);
            }
            selection.set(None);
            return;
        }
        // A key on an exited shell starts a fresh one — the final screen
        // stays readable until then, instead of vanishing mid-glance.
        if state
            .terminal
            .with_untracked(|t| t.as_ref().is_some_and(|s| s.exited.is_some()))
        {
            event.prevent_default();
            let (cols, rows) = sent.get_untracked();
            if cols > 0 {
                controller::open_terminal(state, cols, rows);
            }
            return;
        }
        if let Some(bytes) = encode(&event) {
            event.prevent_default();
            event.stop_propagation();
            controller::terminal_input(state, bytes);
        }
    };

    let menu = RwSignal::new(None::<(f64, f64)>);
    // Paste through the same normalisation the keyboard path uses — a bare \n
    // would be swallowed by shells that expect \r for Enter.
    let paste_clipboard = move || {
        use wasm_bindgen_futures::JsFuture;
        leptos::task::spawn_local(async move {
            let Some(window) = web_sys::window() else {
                return;
            };
            let promise = window.navigator().clipboard().read_text();
            if let Ok(value) = JsFuture::from(promise).await
                && let Some(text) = value.as_string()
                && !text.is_empty()
            {
                controller::terminal_input(state, text.replace('\n', "\r").into_bytes());
            }
        });
    };

    view! {
        <div
            class="relative flex min-h-0 flex-1 flex-col bg-content"
            on:contextmenu=move |event: ev::MouseEvent| {
                event.prevent_default();
                menu.set(Some((event.client_x() as f64, event.client_y() as f64)));
            }
        >
            // The ruler is a hundred characters of the same font, off screen.
            // Its width divided by a hundred is the cell advance, which is the
            // only reliable way to know how many columns fit.
            <span
                node_ref=ruler
                aria-hidden="true"
                class="pointer-events-none absolute -top-[999px] left-0 font-mono whitespace-pre"
                style=format!("font-size: {FONT_SIZE}px; line-height: {LINE_HEIGHT}px")
            >
                "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM"
            </span>

            <div
                node_ref=host
                tabindex="0"
                on:keydown=on_key
                on:wheel=move |event: ev::WheelEvent| {
                    // Three lines a notch, as every terminal does. Positive
                    // deltaY is scrolling down, which is *towards* the present.
                    let lines = if event.delta_y() > 0.0 { -3 } else { 3 };
                    event.prevent_default();
                    controller::terminal_scroll(state, lines);
                }
                on:paste=move |event: web_sys::ClipboardEvent| {
                    // Bracketed paste is deliberately not used: without knowing
                    // whether the program asked for it, wrapping the text in
                    // markers would make them appear literally in every shell
                    // that did not.
                    let Some(clipboard) = event.clipboard_data() else {
                        return;
                    };
                    if let Ok(text) = clipboard.get_data("text") {
                        event.prevent_default();
                        controller::terminal_input(state, text.replace('\n', "\r").into_bytes());
                    }
                }
                on:mousedown=move |event: ev::MouseEvent| {
                    if event.button() != 0 {
                        return;
                    }
                    // Take the drag ourselves; keep focus with the shell.
                    event.prevent_default();
                    if let Some(element) = host.get_untracked() {
                        let _ = element.focus();
                    }
                    let at = cell_at(f64::from(event.client_x()), f64::from(event.client_y()));
                    selection.set(Some((at, at)));
                    selecting.set(true);
                }
                on:mousemove=move |event: ev::MouseEvent| {
                    if !selecting.get_untracked() {
                        return;
                    }
                    let at = cell_at(f64::from(event.client_x()), f64::from(event.client_y()));
                    selection.update(|sel| {
                        if let Some((_, head)) = sel {
                            *head = at;
                        }
                    });
                }
                on:mouseup=move |_| {
                    selecting.set(false);
                    // A click is not a selection.
                    if let Some((anchor, head)) = selection.get_untracked()
                        && anchor == head
                    {
                        selection.set(None);
                    }
                }
                class="relative min-h-0 flex-1 cursor-text overflow-hidden px-2 py-1 font-mono outline-none select-none"
                style=format!("font-size: {FONT_SIZE}px; line-height: {LINE_HEIGHT}px")
            >
                // The selection wash: one rectangle per touched row, over the
                // text and under the pointer (pointer-events-none), so the
                // grid itself never re-renders while dragging.
                {move || {
                    let ((ar, ac), (br, bc)) = selection.get()?;
                    let cell = cell_w.get();
                    if cell <= 0.0 {
                        return None;
                    }
                    let cols = state
                        .terminal
                        .with(|t| t.as_ref().map(|s| s.cols as usize))
                        .unwrap_or(0);
                    let (start, end) = if (ar, ac) <= (br, bc) {
                        ((ar, ac), (br, bc))
                    } else {
                        ((br, bc), (ar, ac))
                    };
                    Some(
                        (start.0..=end.0)
                            .map(|row| {
                                let from = if row == start.0 { start.1 } else { 0 };
                                let to = if row == end.0 { end.1 } else { cols };
                                let left = 8.0 + from as f64 * cell;
                                let width = (to.saturating_sub(from)) as f64 * cell;
                                let top = 4.0 + row as f64 * LINE_HEIGHT;
                                view! {
                                    <div
                                        class="pointer-events-none absolute bg-selection"
                                        style=format!(
                                            "left: {left}px; top: {top}px; width: {width}px; height: {LINE_HEIGHT}px",
                                        )
                                    />
                                }
                            })
                            .collect_view(),
                    )
                }}
                {move || {
                    match state.terminal.get() {
                        Some(screen) => view! { <Grid screen=screen /> }.into_any(),
                        None => {
                            view! {
                                <p class="text-callout text-label-2">"Starting a shell…"</p>
                            }
                                .into_any()
                        }
                    }
                }}
            </div>

            {move || {
                let (x, y) = menu.get()?;
                let close = Callback::new(move |_| menu.set(None));
                let grabbed = selection_text();
                let has_selection = grabbed.is_some();
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label="Copy selection"
                                disabled=!has_selection
                                on_select=Callback::new(move |_| {
                                    if let Some(text) = &grabbed {
                                        copy_to_clipboard(text);
                                    }
                                    selection.set(None);
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label="Paste"
                                on_select=Callback::new(move |_| {
                                    paste_clipboard();
                                    menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label="Restart shell"
                                danger=true
                                on_select=Callback::new(move |_| {
                                    controller::close_terminal(state);
                                    menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
    }
}

#[component]
fn Grid(screen: Screen) -> impl IntoView {
    let cursor = screen.cursor;
    let exited = screen.exited;

    view! {
        <div class="relative">
            {screen
                .rows
                .into_iter()
                .map(|row| {
                    view! {
                        // Every row is rendered even when empty, and at a fixed
                        // height: the cursor is positioned by multiplying its
                        // row by this, so a collapsed blank line would put it in
                        // the wrong place for the rest of the screen.
                        <div
                            class="whitespace-pre"
                            style=format!("height: {LINE_HEIGHT}px")
                        >
                            {row
                                .spans
                                .into_iter()
                                .map(|span| {
                                    view! {
                                        <span style=inline_style(&span.style)>{span.text}</span>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
                .collect_view()}

            {cursor
                .map(|cursor| {
                    // A block, drawn over the cell rather than between them.
                    // `mix-blend-mode` keeps the character underneath readable
                    // without having to know what colour it is.
                    view! {
                        <span
                            class="pointer-events-none absolute bg-rust mix-blend-difference"
                            style=format!(
                                "left: calc({} * 1ch); top: {}px; width: 1ch; height: {LINE_HEIGHT}px",
                                cursor.col,
                                f64::from(cursor.row) * LINE_HEIGHT,
                            )
                        />
                    }
                })}
        </div>

        {exited
            .map(|code| {
                view! {
                    <p class="mt-1 text-callout text-label-3">
                        {format!("The shell exited with status {code}.")}
                    </p>
                }
            })}
    }
}

/// Inline styles, because a cell's colour is data rather than a class.
fn inline_style(style: &Style) -> String {
    let (fg, bg) = if style.inverse {
        (colour(style.bg, "var(--content)"), colour(style.fg, "var(--label)"))
    } else {
        (colour(style.fg, "var(--label)"), colour(style.bg, "transparent"))
    };

    let mut css = format!("color:{fg};background:{bg}");
    if style.bold {
        css.push_str(";font-weight:600");
    }
    if style.italic {
        css.push_str(";font-style:italic");
    }
    if style.underline {
        css.push_str(";text-decoration:underline");
    }
    css
}

/// One terminal colour as CSS.
///
/// The sixteen base colours stay as theme variables: a program asking for "red"
/// means the theme's red, and a palette baked for a dark background is
/// unreadable on a light one. Above sixteen the values are fixed by the xterm
/// specification and there is nothing to theme.
fn colour(colour: Colour, fallback: &str) -> String {
    match colour {
        Colour::Default => fallback.to_string(),
        Colour::Indexed { index } if index < 16 => format!("var(--term-{index})"),
        Colour::Indexed { index } if index < 232 => {
            // The 6×6×6 cube. Levels are not evenly spaced — the first step is
            // 0 to 95, the rest are 40 apart.
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + u16::from(v) * 40 };
            let (r, g, b) = (level(i / 36), level((i % 36) / 6), level(i % 6));
            format!("rgb({r} {g} {b})")
        }
        Colour::Indexed { index } => {
            let grey = 8 + u16::from(index - 232) * 10;
            format!("rgb({grey} {grey} {grey})")
        }
        Colour::Rgb { r, g, b } => format!("rgb({r} {g} {b})"),
    }
}

/// A keypress as the bytes a terminal expects.
///
/// `None` for keys with no terminal meaning — modifiers on their own, function
/// keys nothing here binds — so the caller can leave those to the browser
/// rather than swallowing them.
fn encode(event: &ev::KeyboardEvent) -> Option<Vec<u8>> {
    let key = event.key();

    // Control codes first: `Ctrl C` is 0x03, and it must reach the shell rather
    // than being read as the letter C.
    if event.ctrl_key() && !event.alt_key() {
        let mut chars = key.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii_alphabetic()
        {
            return Some(vec![(c.to_ascii_uppercase() as u8) & 0x1f]);
        }
        match key.as_str() {
            "[" => return Some(vec![0x1b]),
            "\\" => return Some(vec![0x1c]),
            "]" => return Some(vec![0x1d]),
            _ => {}
        }
    }

    let bytes: &[u8] = match key.as_str() {
        "Enter" => b"\r",
        // DEL, not BS. Every modern shell's line editor expects 0x7f, and
        // sending 0x08 makes backspace print `^H` instead of deleting.
        "Backspace" => b"\x7f",
        "Tab" => b"\t",
        "Escape" => b"\x1b",
        "ArrowUp" => b"\x1b[A",
        "ArrowDown" => b"\x1b[B",
        "ArrowRight" => b"\x1b[C",
        "ArrowLeft" => b"\x1b[D",
        "Home" => b"\x1b[H",
        "End" => b"\x1b[F",
        "Delete" => b"\x1b[3~",
        "PageUp" => b"\x1b[5~",
        "PageDown" => b"\x1b[6~",
        _ => {
            // Anything that is a single character is text. Longer names are
            // keys like "Shift" and "F5", which have nothing to send.
            let mut chars = key.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c.to_string().into_bytes()),
                _ => None,
            };
        }
    };
    Some(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::colour;
    use rusty_term::Colour;

    #[test]
    fn base_colours_stay_themeable() {
        assert_eq!(colour(Colour::Indexed { index: 1 }, "x"), "var(--term-1)");
        assert_eq!(colour(Colour::Indexed { index: 15 }, "x"), "var(--term-15)");
        assert_eq!(colour(Colour::Default, "var(--label)"), "var(--label)");
    }

    #[test]
    fn the_xterm_cube_is_not_evenly_spaced() {
        // 16 is the cube's black corner; 21 is pure blue at full level.
        assert_eq!(colour(Colour::Indexed { index: 16 }, "x"), "rgb(0 0 0)");
        assert_eq!(colour(Colour::Indexed { index: 21 }, "x"), "rgb(0 0 255)");
        // The first step is 95, not 51 — an even division would put it at 51
        // and every dim colour would come out wrong.
        assert_eq!(colour(Colour::Indexed { index: 17 }, "x"), "rgb(0 0 95)");
    }

    #[test]
    fn the_greyscale_ramp_runs_from_eight() {
        assert_eq!(colour(Colour::Indexed { index: 232 }, "x"), "rgb(8 8 8)");
        assert_eq!(colour(Colour::Indexed { index: 255 }, "x"), "rgb(238 238 238)");
    }
}
