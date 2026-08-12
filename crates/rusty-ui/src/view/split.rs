//! Draggable dividers.
//!
//! Two window-level listeners rather than a pair per handle: which divider is
//! moving lives in state, so adding a third split later costs a handle and
//! nothing else.
//!
//! The grab area is wider than the line it sits on. A one-pixel target is one
//! nobody hits — macOS does the same, which is why its dividers feel easy
//! despite being hairlines.

use leptos::{ev, prelude::*};

use crate::state::{AppState, Divider, remember_size};

/// Install the drag listeners. Called once, from the shell.
pub fn install(state: AppState) {
    let move_handle = window_event_listener(ev::mousemove, move |event| {
        let Some(divider) = state.dragging.get_untracked() else {
            return;
        };
        let (min, max) = divider.bounds();
        match divider {
            // The sidebar starts at the window's left edge, so the pointer's x
            // is the width.
            Divider::Sidebar => {
                let width = (event.client_x() as f64).clamp(min, max);
                state.sidebar_width.set(width);
            }
            // The dock is anchored to the bottom, so its height is the distance
            // from the pointer to the bottom of the window.
            Divider::Dock => {
                let height = web_sys::window()
                    .and_then(|w| w.inner_height().ok())
                    .and_then(|h| h.as_f64())
                    .map(|window_height| window_height - event.client_y() as f64)
                    .unwrap_or(min)
                    .clamp(min, max);
                state.dock_height.set(height);
            }
        }
    });

    let up_handle = window_event_listener(ev::mouseup, move |_| {
        // Written on release rather than on every move: dragging fires dozens
        // of events a second, and localStorage is synchronous.
        if let Some(divider) = state.dragging.get_untracked() {
            let value = match divider {
                Divider::Sidebar => state.sidebar_width.get_untracked(),
                Divider::Dock => state.dock_height.get_untracked(),
            };
            remember_size(divider, value);
            state.dragging.set(None);
        }
    });

    // Leaked deliberately: these live as long as the window does, and dropping
    // the handles would silently detach them.
    std::mem::forget(move_handle);
    std::mem::forget(up_handle);
}

/// The grab strip between two regions.
#[component]
pub fn Handle(divider: Divider) -> impl IntoView {
    let state = AppState::expect();
    let vertical = divider == Divider::Sidebar;

    let geometry = if vertical {
        "w-px cursor-col-resize before:-left-[3px] before:top-0 before:h-full before:w-[7px]"
    } else {
        "h-px cursor-row-resize before:-top-[3px] before:left-0 before:w-full before:h-[7px]"
    };

    view! {
        <div
            role="separator"
            aria-orientation=if vertical { "vertical" } else { "horizontal" }
            on:mousedown=move |event| {
                // Without this the drag selects text across the whole window,
                // which looks like the app has broken.
                event.prevent_default();
                state.dragging.set(Some(divider));
            }
            class=move || {
                let active = state.dragging.get() == Some(divider);
                format!(
                    "relative z-10 flex-none bg-line transition-colors before:absolute \
                     before:content-[''] hover:bg-rust {geometry} {}",
                    if active { "bg-rust" } else { "" },
                )
            }
        />
    }
}
