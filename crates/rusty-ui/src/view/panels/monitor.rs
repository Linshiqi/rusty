//! Watching a board that is already running.

use leptos::prelude::*;

use rusty_embed::FlashAction;

use crate::view::panels::session::Session;

#[component]
pub fn Monitor() -> impl IntoView {
    view! {
        <Session
            // Attach without rewriting flash. The case this exists for is a
            // board that is already misbehaving — reflashing it first destroys
            // the state you were trying to look at.
            action=FlashAction::Monitor
            verb="Attach"
        />
    }
}
