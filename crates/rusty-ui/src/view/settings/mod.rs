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

    fn label(self) -> &'static str {
        match self {
            Category::Appearance => "Appearance",
            Category::Editor => "Editor",
            Category::Keyboard => "Keyboard",
            Category::Terminal => "Terminal",
            Category::Language => "Language",
            Category::Assistant => "Assistant",
            Category::Catalogue => "Catalogue",
            Category::Storage => "Storage",
            Category::Network => "Network",
            Category::Updates => "Updates",
        }
    }

    /// One line under the title in the list, so a category can be chosen
    /// without opening it first.
    fn summary(self) -> &'static str {
        match self {
            Category::Appearance => "Theme",
            Category::Editor => "Modal editing, text size",
            Category::Keyboard => "Shortcuts",
            Category::Terminal => "Which shell runs",
            Category::Language => "Interface language",
            Category::Assistant => "Model and credentials",
            Category::Catalogue => "Chips and boards",
            Category::Storage => "Where rusty keeps its data",
            Category::Network => "How downloads reach the internet",
            Category::Updates => "Version and what is published",
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
