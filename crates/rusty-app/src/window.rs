//! Window controls.
//!
//! The OS title bar is off (`decorations: false`), so the app draws its own.
//! That is one row instead of two, and it is where every desktop application
//! that cares about its chrome has ended up.
//!
//! Dragging is handled by `data-tauri-drag-region` in the frontend rather than
//! from here; only the buttons need to reach the backend.
//!
//! The `Window` argument is injected by Tauri and is the window that made the
//! call — no lookup by label, which would only be able to get it wrong.

use tauri::Window;

use crate::error::CommandError;

fn fail(e: tauri::Error) -> CommandError {
    CommandError::new(e.to_string())
}

#[tauri::command]
pub fn window_minimize(window: Window) -> Result<(), CommandError> {
    window.minimize().map_err(fail)
}

/// Toggle, returning the new state so the button can show the right glyph.
#[tauri::command]
pub fn window_toggle_maximize(window: Window) -> Result<bool, CommandError> {
    let maximized = window.is_maximized().map_err(fail)?;
    if maximized {
        window.unmaximize().map_err(fail)?;
    } else {
        window.maximize().map_err(fail)?;
    }
    Ok(!maximized)
}

/// Scale the whole interface, browser-zoom style. CSS pixels stay
/// self-consistent at every factor, so nothing that measures text — the
/// editor's hit-testing above all — drifts.
#[tauri::command]
pub fn window_set_zoom(factor: f64, webview: tauri::WebviewWindow) -> Result<(), CommandError> {
    webview.set_zoom(factor.clamp(0.7, 1.6)).map_err(fail)
}

#[tauri::command]
pub fn window_close(window: Window) -> Result<(), CommandError> {
    // `close()` rather than `destroy()`, so a future CloseRequested handler can
    // intervene — an unsaved wizard, or a flash in progress.
    window.close().map_err(fail)
}
