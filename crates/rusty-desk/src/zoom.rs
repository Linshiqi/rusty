//! The interface's scale, on top of the display's own; and the scale of the
//! text that is read rather than the chrome around it.
//!
//! Slint lays everything out in logical pixels and the display decides how
//! many physical pixels one is. Zooming the whole interface is telling the
//! window the display's factor times the chosen scale, so text, icons and
//! hit-testing all scale together and nothing that measures drifts.
//!
//! The editor's text, a Markdown page's and the terminal's each have a
//! scale of their own besides, which Ctrl and the wheel move over that
//! content — VS Code's split: Ctrl+= is the window, the wheel is the text.

use std::cell::Cell;

use slint::{ComponentHandle, platform::WindowEvent};

use crate::AppWindow;

thread_local! {
    /// The factor this window last asked for, and the zoom it asked it at.
    static ASKED: Cell<Option<(f32, f32)>> = const { Cell::new(None) };
}

/// The display's own factor, from the window's and what was asked of it:
/// while the window still has the factor last asked for, that over its
/// zoom; once it has another, the display gave it — being made on it, or
/// moved to another.
pub fn display_factor(current: f32, asked: Option<(f32, f32)>) -> f32 {
    match asked {
        Some((factor, zoom)) if (current - factor).abs() < 0.001 => factor / zoom,
        _ => current,
    }
}

/// The window drawn at `zoom` times the display's own factor; called again
/// whenever the window takes a new size, which is how the display's factor
/// arrives.
///
/// Nothing before the window is on a display: until it is made — a moment
/// into the event loop, after `show` — its factor is 1.0 whatever the
/// display. Learned then and kept, it put a 125% display back to 100%,
/// measured as a window of 1600×1025 pixels at a scale of 1, laid out at
/// 1280×820 and drawn into that corner. And a new factor is a new logical
/// size for the same pixels, so the layout is told it: a factor alone
/// leaves the layout at the size it had.
pub fn apply(window: &AppWindow, zoom: f32) {
    let window = window.window();
    let size = window.size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    let current = window.scale_factor();
    let wanted = display_factor(current, ASKED.with(Cell::get)) * zoom;
    if (current - wanted).abs() > 0.001 {
        ASKED.with(|cell| cell.set(Some((wanted, zoom))));
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: wanted,
        });
        window.dispatch_event(WindowEvent::Resized {
            size: size.to_logical(wanted),
        });
    }
}

/// How far one notch of a wheel travels, as Slint reports it: a line is
/// sixty logical pixels.
const NOTCH: f32 = 60.0;

/// A text's scale moved by `steps` tenths of its own size — VS Code's
/// editor zoom — kept between half and three times. Counted in whole
/// tenths, so steps taken and taken back land exactly where they began.
pub fn stepped(scale: f32, steps: i32) -> f32 {
    let tenths = (scale * 10.0).round() as i32 + steps;
    tenths.clamp(5, 30) as f32 / 10.0
}

/// The steps a turn of the wheel makes, up being larger: one a notch, and
/// a touchpad's small deltas summed in `carried` until they make one, so a
/// slow two-finger slide does not leap a step per event.
pub fn wheel_steps(carried: &mut f32, delta: f32) -> i32 {
    *carried += delta;
    let steps = (*carried / NOTCH).trunc();
    *carried -= steps * NOTCH;
    steps as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The display's factor is what the window has until rusty asks for
    /// one, is worked back from the zoom while the window keeps what was
    /// asked, and is the window's again once the display changes it — the
    /// window made on a 125% display after rusty had asked of the 1.0 it
    /// had before, or moved to a 150% one.
    #[test]
    fn the_display_factor_is_what_rusty_did_not_ask_for() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(display_factor(1.25, None), 1.25));
        assert!(near(display_factor(1.375, Some((1.375, 1.1))), 1.25));
        assert!(near(display_factor(1.25, Some((1.1, 1.1))), 1.25));
        assert!(near(display_factor(1.5, Some((1.375, 1.1))), 1.5));
    }

    /// A notch is a step, steps undone land where they began, and the
    /// scale stops at its ends rather than drawing text of no size.
    #[test]
    fn a_notch_is_a_tenth_and_the_ends_hold() {
        assert_eq!(stepped(1.0, 1), 1.1);
        assert_eq!(stepped(stepped(1.0, 7), -7), 1.0);
        assert_eq!(stepped(1.0, -20), 0.5);
        assert_eq!(stepped(2.9, 5), 3.0);
        let mut carried = 0.0;
        assert_eq!(wheel_steps(&mut carried, 60.0), 1);
        assert_eq!(wheel_steps(&mut carried, -120.0), -2);
        // A touchpad's slide: nothing until a notch's worth has gone by.
        assert_eq!(wheel_steps(&mut carried, 25.0), 0);
        assert_eq!(wheel_steps(&mut carried, 25.0), 0);
        assert_eq!(wheel_steps(&mut carried, 25.0), 1);
    }
}
