//! Getting the binary onto the board.

use leptos::prelude::*;

use rusty_embed::FlashAction;

use crate::view::panels::session::Session;

#[component]
pub fn Flash() -> impl IntoView {
    view! {
        <Session
            // Flash *and* monitor is the inner loop — writing an image and then
            // not seeing what it printed is a step nobody wants on its own, and
            // the Monitor panel is there for attaching without rewriting.
            action=FlashAction::FlashAndMonitor
            verb="Flash and monitor"
        />
    }
}
