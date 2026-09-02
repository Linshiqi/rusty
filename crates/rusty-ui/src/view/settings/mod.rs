//! Settings.
//!
//! An overlay over the whole window rather than a sidebar entry. Settings are
//! not part of the work loop — giving them a slot next to Flash and Monitor
//! would say they are visited as often, which they are not. Apple keeps
//! preferences out of primary navigation for the same reason.
//!
//! One category at a time, chosen from a list on the left. Stacking every
//! section in one scroll makes the reader do the filtering, and the result is
//! that nobody reads any of it.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Appearance,
    Keyboard,
    Terminal,
    Language,
    Assistant,
    Editor,
    Catalogue,
    Storage,
    Network,
    Updates,
}

impl Category {
    const ALL: [Category; 10] = [
        Category::Appearance,
        Category::Editor,
        Category::Keyboard,
        Category::Terminal,
        Category::Language,
        Category::Assistant,
        Category::Catalogue,
        Category::Storage,
        Category::Network,
        Category::Updates,
    ];

    fn label(self) -> String {
        match self {
            Category::Appearance => t!("settings.category.appearance"),
            Category::Editor => t!("settings.category.editor"),
            Category::Keyboard => t!("settings.category.keyboard"),
            Category::Terminal => t!("settings.category.terminal"),
            Category::Language => t!("settings.category.language"),
            Category::Assistant => t!("settings.category.assistant"),
            Category::Catalogue => t!("settings.category.catalogue"),
            Category::Storage => t!("settings.category.storage"),
            Category::Network => t!("settings.category.network"),
            Category::Updates => t!("settings.category.updates"),
        }
    }

    /// One line under the title in the list, so a category can be chosen
    /// without opening it first.
    fn summary(self) -> String {
        match self {
            Category::Appearance => t!("settings.summary.appearance"),
            Category::Editor => t!("settings.summary.editor"),
            Category::Keyboard => t!("settings.summary.keyboard"),
            Category::Terminal => t!("settings.summary.terminal"),
            Category::Language => t!("settings.summary.language"),
            Category::Assistant => t!("settings.summary.assistant"),
            Category::Catalogue => t!("settings.summary.catalogue"),
            Category::Storage => t!("settings.summary.storage"),
            Category::Network => t!("settings.summary.network"),
            Category::Updates => t!("settings.summary.updates"),
        }
    }
}

mod appearance;
mod assistant;
mod catalogue;
mod editor;
mod keyboard;
mod language;
mod network;
mod shell;
mod storage;
mod terminal;
mod update;

use rusty_i18n::t;

pub use shell::Settings;
// `Field` and `TextRow` are the two rows every category is built from.
use shell::*;

// Flat within the overlay: `Category::Assistant` and `assistant::Assistant`
// are the same idea named twice, and the match in `shell` reads better
// naming the component than the module path to it.
use appearance::*;
use assistant::*;
use catalogue::*;
use editor::*;
use keyboard::*;
use language::*;
use network::*;
use storage::*;
use terminal::*;
use update::*;
