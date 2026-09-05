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
    Toolchain,
    Memory,
    Flash,
    Simulate,
    Crates,
    Settings,
    Wizard,
    Assistant,
    Play,
    Stop,
    Bug,
    Save,
    Hammer,
    Fit,
    Grid,
    Refresh,
    Chevron,
    StepOver,
    StepInto,
    StepOut,
    Pause,
    /// A branch: a trunk with a line forking off and rejoining — the
    /// repository's history.
    Branch,
    Plus,
    /// Arrows against a baseline: down into it is pull, up out of it is
    /// push, and down between brackets is fetch — brought in, not applied.
    Pull,
    Push,
    Fetch,
    /// Three lines: a diff read as one column.
    Rows,
    /// A box split down the middle: a diff read side by side.
    Columns,
}

impl Icon {
    pub fn path(self) -> &'static str {
        match self {
            // A document with a folded corner.
            Icon::Files => "M5 2.5h6l4 4V17.5H5zM11 2.5V7h4",
            // A magnifier.
            Icon::Search => "M8.7 3.2a5.5 5.5 0 1 1 0 11 5.5 5.5 0 0 1 0-11zM12.8 12.8 17 17",
            // A toolbox — the wrench read as Settings, which every other
            // app taught people a wrench means.
            Icon::Toolchain => {
                "M3.5 8.5h13v7h-13zM8 8.5V7.2a2 2 0 0 1 4 0v1.3M3.5 11.5h5M11.5 11.5h5M8.5 10.5h3v2h-3z"
            }
            // A RAM chip: body, legs, a window — three bare lines read as
            // a text list, not as silicon.
            Icon::Memory => {
                "M4.5 6.5h11v7h-11zM7 6.5V4.2M10 6.5V4.2M13 6.5V4.2M7 13.5v2.3M10 13.5v2.3M13 13.5v2.3M7.5 9h5v2h-5z"
            }
            // A bolt.
            Icon::Flash => "M11 2.5 4.5 11.5H9l-1 6 6.5-9H10z",
            // A play triangle in a rounded frame — run, without hardware.
            Icon::Simulate => "M4 4.5h12v11H4zM8.5 7.5l4 2.5-4 2.5z",
            // A gear — teeth and a bore, generated from the geometry, because
            // the earlier circle-with-rays read as a sun.
            Icon::Settings => {
                "M8.8 4.7 L9.2 2.4 L10.8 2.4 L11.2 4.7 L12.9 5.5 L14.8 4.1 L15.9 5.2 L14.5 7.1 L15.3 8.8 L17.6 9.2 L17.6 10.8 L15.3 11.2 L14.5 12.9 L15.9 14.8 L14.8 15.9 L12.9 14.5 L11.2 15.3 L10.8 17.6 L9.2 17.6 L8.8 15.3 L7.1 14.5 L5.2 15.9 L4.1 14.8 L5.5 12.9 L4.7 11.2 L2.4 10.8 L2.4 9.2 L4.7 8.8 L5.5 7.1 L4.1 5.2 L5.2 4.1 L7.1 5.5 Z M12.4 10a2.4 2.4 0 1 1 -4.8 0a2.4 2.4 0 1 1 4.8 0"
            }
            // A play triangle.
            Icon::Play => "M6 3.5 16 10 6 16.5z",
            // A stop square.
            Icon::Stop => "M5 5h10v10H5z",
            // A bug: body, head, legs.
            Icon::Bug => {
                "M10 6.5a3.5 3.5 0 0 1 3.5 3.5v3a3.5 3.5 0 1 1-7 0v-3A3.5 3.5 0 0 1 10 6.5zM8 6.8 6.2 4.6M12 6.8l1.8-2.2M6.5 10H3.4M16.6 10h-3.1M7 13.5l-2.4 1.8M13 13.5l2.4 1.8"
            }
            // A floppy disk, the save glyph two generations still read.
            Icon::Save => "M4 4h9l3 3v9H4zM7 4v4h5V4M6.8 16v-4.5h6.4V16",
            // A hammer.
            Icon::Hammer => "M4 15.8 9.6 10.2M9 5.5l2.3-2.3 5.5 5.5L14.5 11zM11.3 3.2l5.5 5.5",
            // Corners of a frame — fit to view.
            Icon::Fit => "M3 7V3h4M13 3h4v4M17 13v4h-4M7 17H3v-4",
            // A grid.
            Icon::Grid => "M3.5 3.5h13v13h-13zM3.5 7.8h13M3.5 12.2h13M7.8 3.5v13M12.2 3.5v13",
            // Circular arrows.
            Icon::Refresh => {
                "M16.5 8A6.6 6.6 0 0 0 4.4 6.1M3.5 3v3.5H7M3.5 12a6.6 6.6 0 0 0 12.1 1.9M16.5 17v-3.5H13"
            }
            // An arc hopping over a dot — step over the call on this line.
            Icon::StepOver => {
                "M4.5 8.5a5.5 5.5 0 0 1 11 0M15.5 8.5V5M15.5 8.5H12M10 14.5a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0"
            }
            // An arrow diving into the dot — step into the call.
            Icon::StepInto => {
                "M10 3v7.5M6.8 7.6 10 10.8l3.2-3.2M11.6 15a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0"
            }
            // An arrow leaving upward — step out of this frame.
            Icon::StepOut => {
                "M10 10.5V3M6.8 6.2 10 3l3.2 3.2M11.6 15a1.6 1.6 0 1 1-3.2 0a1.6 1.6 0 1 1 3.2 0"
            }
            // Two bars.
            Icon::Pause => "M7 4.5v11M13 4.5v11",
            // Two commits on a trunk and one off to the side, joined: the
            // shape every git client draws for a branch.
            Icon::Branch => {
                "M6.5 5.5a1.75 1.75 0 1 0 0 .01zM6.5 14.5a1.75 1.75 0 1 0 0 .01zM13.5 6.5a1.75 1.75 0 1 0 0 .01zM6.5 7.25v5.5M13.5 8.25c0 3-7 2.5-7 5.5"
            }
            Icon::Plus => "M10 4.5v11M4.5 10h11",
            Icon::Pull => "M10 4v9.5M6.5 10l3.5 3.5 3.5-3.5M4.5 16.5h11",
            Icon::Push => "M10 16V6.5M6.5 10 10 6.5 13.5 10M4.5 3.5h11",
            Icon::Fetch => "M10 4v9M7 10l3 3 3-3M4.5 4v12M15.5 4v12",
            Icon::Rows => "M4 6h12M4 10h12M4 14h12",
            Icon::Columns => "M4 4.5h12v11H4zM10 4.5v11",
            // A chevron pointing down; callers rotate it for the other ways.
            Icon::Chevron => "M5.5 8 10 12.5 14.5 8",
            // A parcel — a crate, literally.
            Icon::Crates => "M10 2.5 17 6.5v7l-7 4-7-4v-7zM10 2.5v8M3 6.5l7 4 7-4",
            // A plus in a rounded square.
            Icon::Wizard => "M4 4.5h12v11H4zM10 7.5v5M7.5 10h5",
            // A speech outline.
            Icon::Assistant => {
                "M3.5 5.2a1.7 1.7 0 0 1 1.7-1.7h9.6a1.7 1.7 0 0 1 1.7 1.7v6.4a1.7 1.7 0 0 1-1.7 1.7H8.2L4.6 16.7v-3.4a1.7 1.7 0 0 1-1.1-1.7z"
            }
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
    let fill = if icon.filled() {
        "currentColor"
    } else {
        "none"
    };
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
