//! The interface's scale, on top of the display's own.
//!
//! Slint lays everything out in logical pixels and the display decides how
//! many physical pixels one is. Zooming the whole interface is telling the
//! window the display's factor times the chosen scale, so text, icons and
//! hit-testing all scale together and nothing that measures drifts.

use std::cell::Cell;

use slint::{ComponentHandle, platform::WindowEvent};

use crate::AppWindow;

thread_local! {
    /// The display's own factor, as it was before any zoom was applied.
    static DISPLAY: Cell<Option<f32>> = const { Cell::new(None) };
}

pub fn apply(window: &AppWindow, zoom: f32) {
    let window = window.window();
    let display = DISPLAY.with(|cell| {
        let known = cell.get().unwrap_or_else(|| window.scale_factor());
        cell.set(Some(known));
        known
    });
    let wanted = display * zoom;
    if (window.scale_factor() - wanted).abs() > 0.001 {
        window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: wanted,
        });
    }
}
