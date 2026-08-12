//! Icons.
//!
//! Hand-drawn rather than pulled from a set: there are eight of them, an icon
//! crate would be a dependency for 400 bytes of path data, and SF Symbols
//! cannot be redistributed anyway.
//!
//! All drawn on a 20-unit grid with a 1.5 stroke so they sit consistently at
//! the 17px the sidebar renders them at.

use leptos::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Files,
    Search,
    Overview,
    Toolchain,
    Memory,
    Flash,
    Simulate,
    Monitor,
    Features,
    Crates,
    Settings,
    Wizard,
    Assistant,
}

impl Icon {
    pub fn path(self) -> &'static str {
        match self {
            // A document with a folded corner.
            Icon::Files => "M5 2.5h6l4 4V17.5H5zM11 2.5V7h4",
            // A magnifier.
            Icon::Search => "M8.7 3.2a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11zM12.8 12.8 17 17",
            // Four panes — a dashboard.
            Icon::Overview => "M3 3.5h5.5v5.5H3zM11.5 3.5H17v5.5h-5.5zM3 11.5h5.5V17H3zM11.5 11.5H17V17h-5.5z",
            // A wrench.
            Icon::Toolchain => "M13.2 3.4a3.9 3.9 0 0 0-4.6 5.1L3.4 13.7a1.6 1.6 0 0 0 2.3 2.3l5.2-5.2a3.9 3.9 0 0 0 5.1-4.6l-2.4 2.4-2-2z",
            // Stacked bars — regions of a memory map.
            Icon::Memory => "M3 5h14M3 10h9M3 15h12",
            // A bolt.
            Icon::Flash => "M11 2.5 4.5 11.5H9l-1 6 6.5-9H10z",
            // A play triangle in a rounded frame — run, without hardware.
            Icon::Simulate => "M4 4.5h12v11H4zM8.5 7.5l4 2.5-4 2.5z",
            // A terminal.
            Icon::Monitor => "M2.5 4.5h15v11h-15zM5.5 8.5l2.5 2-2.5 2M10.5 12.5h4",
            // Sliders.
            Icon::Features => "M3 6h9M15 6h2M3 14h3M9 14h8M12.5 4v4M6.5 12v4",
            // A gear.
            Icon::Settings => "M10 6.5a3.5 3.5 0 1 1 0 7 3.5 3.5 0 0 1 0-7zM10 2v2.4M10 15.6V18M18 10h-2.4M4.4 10H2M15.7 4.3l-1.7 1.7M6 14l-1.7 1.7M15.7 15.7L14 14M6 6L4.3 4.3",
            // A parcel — a crate, literally.
            Icon::Crates => "M10 2.5 17 6.5v7l-7 4-7-4v-7zM10 2.5v8M3 6.5l7 4 7-4",
            // A plus in a rounded square.
            Icon::Wizard => "M4 4.5h12v11H4zM10 7.5v5M7.5 10h5",
            // A speech outline.
            Icon::Assistant => "M3.5 5.2a1.7 1.7 0 0 1 1.7-1.7h9.6a1.7 1.7 0 0 1 1.7 1.7v6.4a1.7 1.7 0 0 1-1.7 1.7H8.2L4.6 16.7v-3.4a1.7 1.7 0 0 1-1.1-1.7z",
        }
    }

    /// Filled icons read heavier at small sizes; only the ones drawn as solid
    /// shapes use it.
    fn filled(self) -> bool {
        matches!(self, Icon::Flash)
    }
}

#[component]
pub fn IconView(icon: Icon, #[prop(default = 17)] size: u32) -> impl IntoView {
    let fill = if icon.filled() { "currentColor" } else { "none" };
    view! {
        <svg
            width=size
            height=size
            viewBox="0 0 20 20"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
            class="shrink-0"
        >
            <path d=icon.path() fill=fill />
        </svg>
    }
}

/// The brandmark: a gear, because Rust's own logo is one.
#[component]
pub fn Brandmark(#[prop(default = 16)] size: u32) -> impl IntoView {
    view! {
        <svg width=size height=size viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <circle cx="8" cy="8" r="6.2" stroke="currentColor" stroke-width="1.5" />
            <circle cx="8" cy="8" r="2.1" fill="currentColor" />
            <path
                d="M8 1.8v-1M8 15.2v1M14.2 8h1M0.8 8h1M12.4 3.6l.7-.7M2.9 13.1l.7-.7M12.4 12.4l.7.7M2.9 2.9l.7.7"
                stroke="currentColor"
                stroke-width="1.3"
                stroke-linecap="round"
            />
        </svg>
    }
}
