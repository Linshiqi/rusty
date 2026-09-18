//! The mouse in the text: where a press, a drag, and a double or triple
//! click put the selection.
//!
//! The browser does all of this for a textarea — by the textarea's layout,
//! which knows nothing of the inlay hints the echo draws inside a line
//! (`hints.rs`): a press after a hint landed as many characters along as
//! the hint is wide. So a press is placed here, from the point and the one
//! measure every overlay uses, and the textarea is only told the selection
//! that came of it — `preventDefault` on the press, then the focus and the
//! selection by hand, then the drag followed from the window as the browser
//! follows one, scrolling the view while the pointer is outside it.
//!
//! Double-click takes the word and not the space after it, VS Code's rule
//! rather than Chromium's on Windows; triple-click takes the line and its
//! break; a drag after either grows by words or by lines. A right-click
//! outside the selection moves the caret to it first, so the menu's
//! commands act where the pointer is.

use leptos::{ev, html, prelude::*};
use web_sys::HtmlTextAreaElement;

use super::*;
use crate::{controller, state::AppState};

/// How much a press takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unit {
    Caret,
    Word,
    Line,
}

/// A drag under way: the span the press took, how it grows, where the
/// pointer last was, and the timer scrolling the view while it is outside.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    anchor: (u32, u32),
    unit: Unit,
    last: (f64, f64),
    scrolling: Option<IntervalHandle>,
}

/// The span of what a double-click at `col` of `line` takes, in scalar
/// columns: a run of word characters, a run of spaces, or one character of
/// anything else. Past the end of the line, what the line ends with.
pub(super) fn word_span(line: &str, col: u32) -> (u32, u32) {
    #[derive(PartialEq)]
    enum Kind {
        Word,
        Space,
        Other,
    }
    let kind = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Kind::Word
        } else if c == ' ' || c == '\t' {
            Kind::Space
        } else {
            Kind::Other
        }
    };
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let at = (col as usize).min(chars.len() - 1);
    let here = kind(chars[at]);
    if here == Kind::Other {
        return (at as u32, at as u32 + 1);
    }
    let mut start = at;
    while start > 0 && kind(chars[start - 1]) == here {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && kind(chars[end]) == here {
        end += 1;
    }
    (start as u32, end as u32)
}

/// The name at `col` of `line`, in scalar columns: what Ctrl over it would
/// go to the definition of, and so what is underlined while it is held.
/// Nothing between names.
pub(super) fn name_span(line: &str, col: u32) -> Option<(u32, u32)> {
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = line.chars().collect();
    let at = col as usize;
    if !chars.get(at).copied().is_some_and(is_name) {
        return None;
    }
    let mut start = at;
    while start > 0 && is_name(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at + 1;
    while end < chars.len() && is_name(chars[end]) {
        end += 1;
    }
    Some((start as u32, end as u32))
}

/// Where a selection runs when it grows from what the press took to what
/// the pointer is on now: from whichever starts first to whichever ends
/// last — and backward, its moving end at the start, when the pointer is
/// before the press.
pub(super) fn grown(anchor: (u32, u32), at: (u32, u32)) -> (u32, u32, bool) {
    if at.0 < anchor.0 {
        (at.0, anchor.1, true)
    } else {
        (anchor.0, at.1.max(anchor.1), false)
    }
}

/// A point in client pixels, from the text column's corner.
///
/// From the column, not the textarea: the textarea is shifted while an
/// input method composes (`surface.rs`), and a point read off it would be
/// off by the shift.
pub(super) fn point_in_column(area: &HtmlTextAreaElement, client: (f64, f64)) -> (f64, f64) {
    match area.parent_element() {
        Some(column) => {
            let rect = column.get_bounding_client_rect();
            (client.0 - rect.left(), client.1 - rect.top())
        }
        None => client,
    }
}

/// What a press at a point takes, in the textarea's units.
fn span_at(
    state: AppState,
    area: &HtmlTextAreaElement,
    client: (f64, f64),
    unit: Unit,
) -> (u32, u32) {
    let (x, y) = point_in_column(area, client);
    let (row, col) = caret_at_point(state, x, y);
    let text = screen(state);
    let line = text.split('\n').nth(row as usize).unwrap_or_default();
    let (from, to) = match unit {
        Unit::Caret => (col, col),
        Unit::Word => word_span(line, col),
        Unit::Line => (0, line.chars().count() as u32),
    };
    let start = utf16_offset_of(&text, row, from);
    let end = if unit == Unit::Line && (row as usize) < text.matches('\n').count() {
        utf16_offset_of(&text, row + 1, 0)
    } else {
        utf16_offset_of(&text, row, to)
    };
    (start, end)
}

fn select(area: &HtmlTextAreaElement, (start, end, backward): (u32, u32, bool)) {
    let direction = if backward { "backward" } else { "forward" };
    let _ = area.set_selection_range_with_direction(start, end, direction);
}

/// The textarea's selection, in its units: (start, end, backward).
fn selection_units(area: &HtmlTextAreaElement) -> (u32, u32, bool) {
    let start = area.selection_start().ok().flatten().unwrap_or(0);
    let end = area.selection_end().ok().flatten().unwrap_or(start);
    let backward = area.selection_direction().ok().flatten().as_deref() == Some("backward");
    (start, end, backward)
}

fn take_focus(area: &HtmlTextAreaElement) {
    let options = web_sys::FocusOptions::new();
    options.set_prevent_scroll(true);
    let _ = area.focus_with_options(&options);
}

/// A press in the text, placed here. True when it was taken.
pub(super) fn press(
    state: AppState,
    area: &HtmlTextAreaElement,
    drag: StoredValue<Option<Drag>>,
    event: &ev::MouseEvent,
) -> bool {
    let client = (f64::from(event.client_x()), f64::from(event.client_y()));
    match event.button() {
        0 => {}
        // The menu acts on the selection, so a right-click inside it leaves
        // it be; outside it, the caret goes to the pointer first.
        2 => {
            let (at, _) = span_at(state, area, client, Unit::Caret);
            let (start, end, _) = selection_units(area);
            if at < start || at > end {
                event.prevent_default();
                controller::catch_up(state);
                take_focus(area);
                select(area, (at, at, false));
            }
            return false;
        }
        _ => return false,
    }
    event.prevent_default();
    // The view this press lands in is written first if an edit in the other
    // view of its file left it behind — before a selection is set in it.
    controller::catch_up(state);
    take_focus(area);
    let unit = match event.detail() {
        ..=1 => Unit::Caret,
        2 => Unit::Word,
        _ => Unit::Line,
    };
    let at = span_at(state, area, client, unit);
    let anchor = if event.shift_key() && unit == Unit::Caret {
        let (start, end, backward) = selection_units(area);
        let fixed = if backward { end } else { start };
        (fixed, fixed)
    } else {
        at
    };
    select(area, grown(anchor, at));
    end_drag(drag);
    drag.set_value(Some(Drag {
        anchor,
        unit,
        last: client,
        scrolling: None,
    }));
    true
}

/// Stop following a drag: the timer goes with it.
fn end_drag(drag: StoredValue<Option<Drag>>) {
    if let Some(Some(Drag {
        scrolling: Some(timer),
        ..
    })) = drag.try_get_value()
    {
        timer.clear();
    }
    drag.try_set_value(None);
}

/// Follow the drags that start in this view, from the window: a drag goes
/// on outside the text, and scrolls the view while it is outside it. The
/// listeners go with the view.
pub(super) fn follow_drags(
    state: AppState,
    area: NodeRef<html::Textarea>,
    scroller: NodeRef<html::Div>,
    drag: StoredValue<Option<Drag>>,
) {
    // Where the drag's moving end is now: select from the press to it.
    let extend = move |client: (f64, f64)| {
        let (Some(Some(current)), Some(Some(element))) =
            (drag.try_get_value(), area.try_get_untracked())
        else {
            return;
        };
        let at = span_at(state, &element, client, current.unit);
        select(&element, grown(current.anchor, at));
    };
    // How far outside the view the pointer is, per axis; nothing inside it.
    let outside = move |client: (f64, f64)| -> (f64, f64) {
        let Some(Some(element)) = scroller.try_get_untracked() else {
            return (0.0, 0.0);
        };
        let rect = element.get_bounding_client_rect();
        let past = |at: f64, low: f64, high: f64| {
            if at < low {
                at - low
            } else if at > high {
                at - high
            } else {
                0.0
            }
        };
        (
            past(client.0, rect.left(), rect.right()),
            past(client.1, rect.top(), rect.bottom()),
        )
    };
    // One step of scrolling towards the pointer, faster the further out it
    // is, and the selection taken along to where that leaves it.
    let step = move || {
        let Some(Some(current)) = drag.try_get_value() else {
            return;
        };
        let (dx, dy) = outside(current.last);
        let Some(Some(element)) = scroller.try_get_untracked() else {
            return;
        };
        let speed = |over: f64| (over.abs() / 3.0).clamp(2.0, 60.0).copysign(over);
        if dy != 0.0 {
            element.set_scroll_top(element.scroll_top() + speed(dy) as i32);
        }
        if dx != 0.0 {
            element.set_scroll_left(element.scroll_left() + speed(dx) as i32);
        }
        extend(current.last);
    };
    let moving = window_event_listener(ev::mousemove, move |event| {
        let Some(Some(mut current)) = drag.try_get_value() else {
            return;
        };
        // The button went up somewhere the window never heard of it.
        if event.buttons() & 1 == 0 {
            end_drag(drag);
            return;
        }
        current.last = (f64::from(event.client_x()), f64::from(event.client_y()));
        let out = outside(current.last) != (0.0, 0.0);
        match (out, current.scrolling) {
            (true, None) => {
                current.scrolling =
                    set_interval_with_handle(step, std::time::Duration::from_millis(16)).ok();
            }
            (false, Some(timer)) => {
                timer.clear();
                current.scrolling = None;
            }
            _ => {}
        }
        drag.set_value(Some(current));
        extend(current.last);
    });
    let released = window_event_listener(ev::mouseup, move |_| end_drag(drag));
    let listeners = StoredValue::new_local(Some((moving, released)));
    on_cleanup(move || {
        end_drag(drag);
        if let Some(Some((moving, released))) = listeners.try_update_value(Option::take) {
            moving.remove();
            released.remove();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word under a double-click, the run of spaces, or the one
    /// character of punctuation — never the word and the space after it.
    #[test]
    fn a_double_click_takes_the_run_it_is_on() {
        let line = "let total = both(中文);";
        assert_eq!(word_span(line, 5), (4, 9), "total");
        assert_eq!(word_span(line, 9), (9, 10), "the space");
        assert_eq!(word_span(line, 10), (10, 11), "=");
        assert_eq!(word_span(line, 17), (17, 19), "中文 is one word");
        assert_eq!(word_span(line, 99), (20, 21), "past the end: the last");
        assert_eq!(word_span("", 0), (0, 0));
        assert_eq!(word_span("a    b", 2), (1, 5));
    }

    #[test]
    fn a_ctrl_hover_underlines_the_name_and_nothing_between_names() {
        let line = "let p = path.clone();";
        assert_eq!(name_span(line, 9), Some((8, 12)), "path");
        assert_eq!(name_span(line, 14), Some((13, 18)), "clone");
        assert_eq!(name_span(line, 12), None, "the dot");
        assert_eq!(name_span(line, 99), None, "past the end");
        assert_eq!(name_span("中文_x", 1), Some((0, 4)));
    }

    /// A drag grows from what the press took: forward past it, backward
    /// before it, and never smaller than it — a word double-clicked stays
    /// selected while the drag is inside it.
    #[test]
    fn a_selection_grows_from_the_press_both_ways() {
        assert_eq!(grown((5, 5), (9, 9)), (5, 9, false));
        assert_eq!(grown((5, 5), (2, 2)), (2, 5, true));
        assert_eq!(grown((4, 9), (6, 7)), (4, 9, false));
        assert_eq!(grown((4, 9), (12, 15)), (4, 15, false));
        assert_eq!(grown((4, 9), (0, 3)), (0, 9, true));
    }
}
