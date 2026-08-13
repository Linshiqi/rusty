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
        let (from_pointer, from_size) = state.drag_from.get_untracked();
        let (min, max) = divider.bounds();
        // Delta from where the grab started, not absolute window geometry.
        // "Height is the distance to the bottom of the window" ignored the
        // status bar and the dock's own tab strip — 59 pixels between them —
        // so the divider ran that far from the pointer for the whole drag.
        match divider {
            Divider::Sidebar => {
                let width = (from_size + (event.client_x() as f64 - from_pointer))
                    .clamp(min, max);
                state.sidebar_width.set(width);
            }
            Divider::Tree => {
                let width = (from_size + (event.client_x() as f64 - from_pointer))
                    .clamp(min, max);
                state.tree_width.set(width);
            }
            // Anchored to the bottom, so dragging up grows it.
            Divider::Dock => {
                let height = (from_size + (from_pointer - event.client_y() as f64))
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
                Divider::Tree => state.tree_width.get_untracked(),
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
    let vertical = matches!(divider, Divider::Sidebar | Divider::Tree);

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
                let (pointer, size) = match divider {
                    Divider::Sidebar => {
                        (event.client_x() as f64, state.sidebar_width.get_untracked())
                    }
                    Divider::Tree => {
                        (event.client_x() as f64, state.tree_width.get_untracked())
                    }
                    Divider::Dock => {
                        (event.client_y() as f64, state.dock_height.get_untracked())
                    }
                };
                state.drag_from.set((pointer, size));
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
