//! Panel registrations.
//!
//! The shell imports nothing from here but [`all`]. Order is the sidebar order,
//! and the sidebar order is the order the work happens in: understand the
//! project, then talk to the device.

mod assistant;
mod crates;
mod features;
mod files;
mod memory;
mod search;
pub(crate) mod session;
mod simulate;
mod toolchain;
mod wizard;

use leptos::prelude::*;

use rusty_i18n::t;

use crate::view::{Panel, icon::Icon};

/// The assistant's content, for the right-hand drawer.
pub fn assistant_view() -> leptos::prelude::AnyView {
    use leptos::prelude::IntoAny;
    assistant::Assistant().into_any()
}

pub fn all() -> Vec<Panel> {
    vec![
        Panel {
            id: "files",
            title: t!("panel.files"),
            section: "Project",
            icon: Icon::Files,
            needs_project: true,
            hidden: false,
            render: || files::FilesPanel().into_any(),
        },
        Panel {
            id: "search",
            title: t!("panel.search"),
            section: "Project",
            icon: Icon::Search,
            needs_project: true,
            hidden: false,
            render: || search::SearchPanel().into_any(),
        },
        Panel {
            id: "toolchain",
            title: t!("panel.toolchain"),
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
            title: t!("panel.memory"),
            section: "Project",
            icon: Icon::Memory,
            needs_project: true,
            hidden: false,
            render: || memory::Memory().into_any(),
        },
        Panel {
            id: "crates",
            title: t!("panel.crates"),
            section: "Project",
            icon: Icon::Crates,
            needs_project: true,
            hidden: false,
            render: || crates::Crates().into_any(),
        },
        Panel {
            id: "simulate",
            title: t!("panel.simulate"),
            section: "Device",
            icon: Icon::Simulate,
            needs_project: true,
            hidden: false,
            render: || simulate::Simulate().into_any(),
        },
        Panel {
            id: "wizard",
            title: t!("panel.wizard"),
            section: "",
            icon: Icon::Wizard,
            needs_project: false,
            hidden: true,
            render: || wizard::Wizard().into_any(),
        },
        Panel {
            id: "assistant",
            title: t!("panel.assistant"),
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

/// The files workspace on its own — what a detached editor window renders.
pub fn files_view() -> leptos::prelude::AnyView {
    use leptos::prelude::IntoAny;
    files::FilesPanel().into_any()
}
