//! One reader loop for every session that streams lines into the window.
//!
//! Seven copies of the same six lines lived in `flash.rs` and `simulate.rs`,
//! and every one of them carried the same wrong comment: that a failed `send`
//! meant the user had left the panel. It does not. Tauri's `Channel::send`
//! fails only when the WebView itself is gone — a JavaScript side that dropped
//! its handler tells the Rust side nothing — so the loop ends when the
//! *session* ends, and the session ends because its slot in `AppState` was
//! replaced or stopped. That is the contract to lean on, and it is why every
//! long-lived command owns a slot.
//!
//! Each line is one `send`, and with Tauri's implementation each `send` of a
//! small JSON payload is one `webview.eval`. Gathering lines here into frames
//! and still sending them one by one would change nothing that reaches the
//! window; only a `Vec<LogLine>` on the wire does, and that needs the
//! frontend's decoders to change with it. When they do, this is the one
//! function that changes.

use rusty_embed::LogLine;
use tauri::ipc::Channel;

/// Pump lines to the window until the source runs dry or the window is gone.
pub fn forward(mut next: impl FnMut() -> Option<LogLine>, sink: &Channel<LogLine>) {
    while let Some(line) = next() {
        if sink.send(line).is_err() {
            // The WebView is gone — not "the user closed the panel". There is
            // nothing left to draw into; whoever owns the session's slot ends
            // the session.
            break;
        }
    }
}
