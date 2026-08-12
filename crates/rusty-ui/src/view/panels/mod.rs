//! Panel registrations.
//!
//! The shell imports nothing from here but [`all`]. Order is the sidebar order,
//! and the sidebar order is the order the work happens in: understand the
//! project, then talk to the device.

mod assistant;
mod crates;
mod features;
mod files;
mod flash;
mod memory;
mod monitor;
mod overview;
mod search;
mod session;
mod simulate;
mod toolchain;
mod wizard;

use leptos::prelude::*;

use crate::view::{Panel, icon::Icon};

/// The device picker, so the dock can offer the same one the panels do.
pub use session::Devices;

/// The assistant's content, for the right-hand drawer.
pub fn assistant_view() -> leptos::prelude::AnyView {
    use leptos::prelude::IntoAny;
    assistant::Assistant().into_any()
}

pub fn all() -> Vec<Panel> {
    vec![
        Panel {
            id: "files",
            title: "Files",
            section: "Project",
            icon: Icon::Files,
            needs_project: true,
            hidden: false,
            render: || files::FilesPanel().into_any(),
        },
        Panel {
            id: "search",
            title: "Search",
            section: "Project",
            icon: Icon::Search,
            needs_project: true,
            hidden: false,
            render: || search::SearchPanel().into_any(),
        },
        Panel {
            id: "overview",
            title: "Overview",
            section: "Project",
            icon: Icon::Overview,
            needs_project: false,
            hidden: false,
            render: || overview::Overview().into_any(),
        },
        Panel {
            id: "toolchain",
            title: "Toolchain",
            section: "Project",
            icon: Icon::Toolchain,
            // Answers "is my machine set up?", which is a fair question to ask
            // before there is a project to set it up for.
            needs_project: false,
            hidden: false,
            render: || toolchain::Toolchain().into_any(),
        },
        Panel {
            id: "memory",
            title: "Memory",
            section: "Project",
            icon: Icon::Memory,
            needs_project: true,
            hidden: false,
            render: || memory::Memory().into_any(),
        },
        Panel {
            id: "crates",
            title: "Crates",
            section: "Project",
            icon: Icon::Crates,
            needs_project: true,
            hidden: false,
            render: || crates::Crates().into_any(),
        },
        Panel {
            id: "features",
            title: "Features",
            section: "Project",
            icon: Icon::Features,
            needs_project: true,
            hidden: false,
            render: || features::Features().into_any(),
        },
        Panel {
            id: "flash",
            title: "Flash",
            section: "Device",
            icon: Icon::Flash,
            needs_project: true,
            hidden: false,
            render: || flash::Flash().into_any(),
        },
        Panel {
            id: "simulate",
            title: "Simulate",
            section: "Device",
            icon: Icon::Simulate,
            needs_project: true,
            hidden: false,
            render: || simulate::Simulate().into_any(),
        },
        Panel {
            id: "monitor",
            title: "Monitor",
            section: "Device",
            icon: Icon::Monitor,
            needs_project: true,
            hidden: false,
            render: || monitor::Monitor().into_any(),
        },
        Panel {
            id: "wizard",
            title: "New project",
            section: "",
            icon: Icon::Wizard,
            needs_project: false,
            hidden: true,
            render: || wizard::Wizard().into_any(),
        },
        Panel {
            id: "assistant",
            title: "Assistant",
            section: "",
            icon: Icon::Assistant,
            // Useful with nothing open: "which ESP32 has 802.15.4?" needs no
            // project, and the chip catalogue can answer it.
            needs_project: false,
            hidden: true,
            render: || assistant::Assistant().into_any(),
        },
    ]
}
