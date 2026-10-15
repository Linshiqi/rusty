//! Which window a surface draws into: the main window's code view, or a
//! window of its own that a tab was moved into (`app/torn.rs`).
//!
//! Slint gives every window its own instance of each global, so the main
//! window's `Code` and a torn window's `Code` are two sets of the same
//! properties and callbacks, of one Rust type. Everything a surface tells
//! its view is therefore the same call whichever window it was made for;
//! this is the one place that knows there are two kinds of window.

use slint::ComponentHandle;

use crate::{AppWindow, TornWindow};

/// The window a surface draws into, held weakly as every view holds it.
#[derive(Clone)]
pub enum Host {
    Main(slint::Weak<AppWindow>),
    Torn(slint::Weak<TornWindow>),
}

/// The window, while it exists.
pub enum Strong {
    Main(AppWindow),
    Torn(TornWindow),
}

impl Host {
    pub fn upgrade(&self) -> Option<Strong> {
        match self {
            Host::Main(window) => window.upgrade().map(Strong::Main),
            Host::Torn(window) => window.upgrade().map(Strong::Torn),
        }
    }

    pub fn is_main(&self) -> bool {
        matches!(self, Host::Main(_))
    }
}

impl Strong {
    /// The window's own instance of a global both kinds of window carry.
    pub fn global<'a, G>(&'a self) -> G
    where
        G: slint::Global<'a, AppWindow> + slint::Global<'a, TornWindow>,
    {
        match self {
            Strong::Main(window) => window.global::<G>(),
            Strong::Torn(window) => window.global::<G>(),
        }
    }

    pub fn window(&self) -> &slint::Window {
        match self {
            Strong::Main(window) => window.window(),
            Strong::Torn(window) => window.window(),
        }
    }

    pub fn invoke_focus_code(&self) {
        match self {
            Strong::Main(window) => window.invoke_focus_code(),
            Strong::Torn(window) => window.invoke_focus_code(),
        }
    }

    pub fn invoke_focus_find(&self, replace: bool) {
        match self {
            Strong::Main(window) => window.invoke_focus_find(replace),
            Strong::Torn(window) => window.invoke_focus_find(replace),
        }
    }

    /// How wide `text` is in the editor's font, measured by the window's own
    /// hidden probe.
    pub fn probe_width(&self, text: &str) -> f32 {
        match self {
            Strong::Main(window) => {
                window.set_probe_text(text.into());
                window.get_probe_width()
            }
            Strong::Torn(window) => {
                window.set_probe_text(text.into());
                window.get_probe_width()
            }
        }
    }
}
