//! Appearance.
//!
//! Follows the system by default, which is what a desktop application should
//! do. An explicit choice is remembered in `localStorage` and written to
//! `<html data-theme>`, where the stylesheet's overrides pick it up.
//!
//! No colour lives here — only which set of it applies. The palette is entirely
//! in `style/input.css`, so a theme change is a CSS variable swap rather than a
//! re-render.

use rusty_i18n::t;

const STORAGE_KEY: &str = "rusty.theme";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    pub fn label(self) -> String {
        match self {
            Theme::System => t!("settings.appearance.theme-system"),
            Theme::Light => t!("settings.appearance.theme-light"),
            Theme::Dark => t!("settings.appearance.theme-dark"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            "system" => Some(Theme::System),
            _ => None,
        }
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn document_element() -> Option<web_sys::Element> {
    web_sys::window()?.document()?.document_element()
}

/// Read the stored preference. `System` when nothing was ever chosen.
pub fn stored() -> Theme {
    storage()
        .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
        .and_then(|value| Theme::parse(&value))
        .unwrap_or(Theme::System)
}

/// Apply a theme and remember it.
pub fn set(theme: Theme) {
    if let Some(root) = document_element() {
        match theme {
            // Removing the attribute hands control back to the media query,
            // which then keeps tracking the system if the user changes it.
            Theme::System => {
                let _ = root.remove_attribute("data-theme");
            }
            other => {
                let _ = root.set_attribute("data-theme", other.as_str());
            }
        }
    }
    if let Some(storage) = storage() {
        let _ = storage.set_item(STORAGE_KEY, theme.as_str());
    }
}

/// Apply whatever was stored, at startup.
///
/// Called before the first render so the window never flashes the wrong theme.
pub fn init() {
    set(stored());
}
