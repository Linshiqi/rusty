//! The device workspace: what is plugged in, and getting firmware onto it.
//!
//! One page for both verbs. Flash-and-monitor is the inner loop; attach-only
//! is its sibling for a board already running — the Monitor panel that used
//! to hold it was this page minus one toggle, twice the navigation.

use leptos::prelude::*;

use crate::view::panels::session::Session;

#[component]
pub fn Flash() -> impl IntoView {
    view! { <Session /> }
}
