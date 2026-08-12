//! Panel registrations.
//!
//! The shell imports nothing from here but [`all`]. Order is the sidebar order,
//! and the sidebar order is the order the work happens in: understand the
//! project, then talk to the device.

mod assistant;
mod features;
mod flash;
mod memory;
mod monitor;
mod overview;
mod session;
mod toolchain;
mod wizard;

use leptos::prelude::*;

use crate::view::{Panel, icon::Icon};

/// The device picker, so the dock can offer the same one the panels do.
pub use session::Devices;

pub fn all() -> Vec<Panel> {
    vec![
        Panel {
            id: "overview",
            title: "Overview",
            section: "Project",
            icon: Icon::Overview,
            needs_project: false,
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
            render: || toolchain::Toolchain().into_any(),
        },
        Panel {
            id: "memory",
            title: "Memory",
            section: "Project",
            icon: Icon::Memory,
            needs_project: true,
            render: || memory::Memory().into_any(),
        },
        Panel {
            id: "features",
            title: "Features",
            section: "Project",
            icon: Icon::Features,
            needs_project: true,
            render: || features::Features().into_any(),
        },
        Panel {
            id: "flash",
            title: "Flash",
            section: "Device",
            icon: Icon::Flash,
            needs_project: true,
            render: || flash::Flash().into_any(),
        },
        Panel {
            id: "monitor",
            title: "Monitor",
            section: "Device",
            icon: Icon::Monitor,
            needs_project: true,
            render: || monitor::Monitor().into_any(),
        },
        Panel {
            id: "wizard",
            title: "New project",
            section: "",
            icon: Icon::Wizard,
            needs_project: false,
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
            render: || assistant::Assistant().into_any(),
        },
    ]
}
