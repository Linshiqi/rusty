//! The workbench's own affairs: what it remembers, where it keeps it, and
//! whether there is a newer one.
//!
//! These are model types. `workbench.toml` is written and read through
//! `config.rs`'s own private structs, so a field renamed here for the
//! window's sake cannot silently drop a key from everybody's file.

use serde::{Deserialize, Serialize};

/// The assistant profile last chosen, as the window and `config` hold it.
///
/// A separate type from `rusty_ai::ProviderConfig` on purpose — the same rule
/// that keeps `catalog.rs` from serialising `model` types. That one is a
/// contract with the provider layer; this one is the window's, and coupling
/// them means a refactor on one side rewriting the other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantChoice {
    pub profile: String,
    pub kind: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
}

/// One project's open editors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectTabs {
    /// The project root as the user spelled it when it was recorded. Matching
    /// is by the filesystem's idea of the same directory, so a different
    /// spelling finds it — the trap `recent_projects` already learned.
    pub root: String,
    pub tabs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// The second editor group's strip and its file, when the editor was
    /// split. Absent from files written before there were two groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub second: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second_active: Option<String>,
}

/// What the editor draws around the code, beyond the text: the four things
/// VS Code draws there and lets people turn off. On unless turned off, as in
/// VS Code; a file written before they existed has them on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorView {
    /// Types and names rust-analyzer infers, beside the line they are about.
    pub inlay_hints: bool,
    /// The whole file, small, down the right-hand edge.
    pub minimap: bool,
    /// The lines that open the blocks the top of the view is in, kept at the
    /// top while the view scrolls through them.
    pub sticky_scroll: bool,
    /// A faint line at every indentation stop.
    pub indent_guides: bool,
}

impl Default for EditorView {
    fn default() -> Self {
        EditorView {
            inlay_hints: true,
            minimap: true,
            sticky_scroll: true,
            indent_guides: true,
        }
    }
}

/// How the window is laid out and looks: the theme, the interface's scale,
/// the three docks and where the two editor groups part. A file and not the
/// window's own storage, because a second window opens laid out as the
/// first was left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowLayout {
    /// `system`, `dark` or `light`.
    pub theme: String,
    /// The interface's scale, 1.0 as designed: Ctrl+= and Ctrl+-.
    pub zoom: f32,
    /// The editor's text, 1.0 as designed: Ctrl and the wheel over the
    /// code, VS Code's `editor.mouseWheelZoom`.
    pub editor_zoom: f32,
    /// A Markdown page's text, apart from the editor's: prose and a
    /// listing are read at different sizes.
    pub page_zoom: f32,
    /// The terminal's text.
    pub terminal_zoom: f32,
    pub left: DockLayout,
    pub bottom: DockLayout,
    pub right: DockLayout,
    /// The share of the editor area the left group takes while the editor
    /// is split, 0.5 for halves.
    pub split: f32,
    /// Where the History page's panes part.
    pub history: HistoryLayout,
}

/// The History page's four dividers, Fork's shape: the refs against the
/// log, the log against the commit opened below it, the commit's message
/// and files against its diff, and the diff's two columns. A width is in
/// logical pixels and a share is of the room the two panes beside it have;
/// the page still keeps each pane a minimum while there is room for both.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryLayout {
    /// The refs column's width.
    pub refs: f32,
    /// The log's share of the height, the commit opened below it taking
    /// the rest.
    pub log: f32,
    /// The commit's message and files' share of the width beside its diff.
    pub details: f32,
    /// The left column's share of a side-by-side diff — the Changes page's
    /// too, which is the same view.
    pub sides: f32,
}

impl HistoryLayout {
    /// Each divider by its name, as the window names them.
    pub const NAMES: [&'static str; 4] = ["refs", "log", "details", "sides"];

    /// A width a hand could not have meant, or a share outside the room,
    /// brought back to one the page can draw: NaN or an infinity is the
    /// default, a width is between 120 and 1200 pixels, a share between a
    /// twentieth and nineteen twentieths.
    pub fn sane(self) -> HistoryLayout {
        let defaults = HistoryLayout::default();
        let share = |value: f32, default: f32| {
            if value.is_finite() {
                value.clamp(0.05, 0.95)
            } else {
                default
            }
        };
        HistoryLayout {
            refs: if self.refs.is_finite() {
                self.refs.clamp(120.0, 1200.0)
            } else {
                defaults.refs
            },
            log: share(self.log, defaults.log),
            details: share(self.details, defaults.details),
            sides: share(self.sides, defaults.sides),
        }
    }

    /// One divider's value by its name: `None` for a name that is not one.
    pub fn get(&self, name: &str) -> Option<f32> {
        match name {
            "refs" => Some(self.refs),
            "log" => Some(self.log),
            "details" => Some(self.details),
            "sides" => Some(self.sides),
            _ => None,
        }
    }

    /// One divider moved, kept sane; a name that is not a divider changes
    /// nothing.
    pub fn set(&mut self, name: &str, value: f32) {
        let mut next = *self;
        match name {
            "refs" => next.refs = value,
            "log" => next.log = value,
            "details" => next.details = value,
            "sides" => next.sides = value,
            _ => return,
        }
        *self = next.sane();
    }

    /// One divider put back where it starts: a double-click on it.
    pub fn reset(&mut self, name: &str) {
        if let Some(value) = HistoryLayout::default().get(name) {
            self.set(name, value);
        }
    }
}

impl Default for HistoryLayout {
    /// Fork's proportions: a refs column a little narrower than the left
    /// dock, the log over three fifths of the height, the message and files
    /// beside their diff at a little over a third, the diff's columns
    /// halves.
    fn default() -> Self {
        HistoryLayout {
            refs: 220.0,
            log: 0.6,
            details: 0.38,
            sides: 0.5,
        }
    }
}

/// One dock: whether it is open, how wide (or for the bottom one, how tall)
/// in logical pixels, which of its panels was in front, and which were
/// taken off its strip (VS Code's *Hide 'Output'*), until they are shown.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockLayout {
    pub open: bool,
    pub size: f32,
    pub front: Option<String>,
    pub hidden: Vec<String>,
}

impl Default for WindowLayout {
    fn default() -> Self {
        WindowLayout {
            theme: "system".into(),
            zoom: 1.0,
            editor_zoom: 1.0,
            page_zoom: 1.0,
            terminal_zoom: 1.0,
            left: DockLayout {
                open: true,
                size: 260.0,
                front: None,
                hidden: Vec::new(),
            },
            bottom: DockLayout {
                open: false,
                size: 240.0,
                front: None,
                hidden: Vec::new(),
            },
            right: DockLayout {
                open: false,
                size: 420.0,
                front: None,
                hidden: Vec::new(),
            },
            split: 0.5,
            history: HistoryLayout::default(),
        }
    }
}

/// Where rusty keeps its data, for the settings screen to show.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageLocation {
    pub path: String,
    /// True when no pointer and no env override is in play.
    pub is_default: bool,
    /// True when `RUSTY_CONFIG_DIR` decided — relocating from the UI would be
    /// silently outvoted, so the UI disables it and says why.
    pub env_override: bool,
}

/// What a relocation did, so the user can verify before deleting the old copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelocateReport {
    pub from: String,
    pub to: String,
    pub copied_files: usize,
    pub adopted: bool,
}

/// How far an update's download has got.
///
/// `total` is the server's `Content-Length`, absent when it sent none — a
/// bar with no end is drawn as activity rather than as progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub received: u64,
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::HistoryLayout;

    /// A divider moved by its name stays inside the room, a double-click
    /// puts that one back and leaves the others, and a name that is no
    /// divider changes nothing.
    #[test]
    fn a_divider_moves_by_its_name_and_comes_back_on_its_own() {
        let mut layout = HistoryLayout::default();
        layout.set("refs", 400.0);
        layout.set("sides", 0.7);
        layout.set("log", 3.0);
        assert_eq!((layout.refs, layout.sides, layout.log), (400.0, 0.7, 0.95));
        layout.reset("refs");
        assert_eq!(layout.refs, HistoryLayout::default().refs);
        assert_eq!(layout.sides, 0.7, "the others stay where they were");
        let before = layout;
        layout.set("graph", 0.1);
        layout.reset("graph");
        assert_eq!(layout, before);
        for name in HistoryLayout::NAMES {
            assert!(layout.get(name).is_some(), "{name} is a divider");
        }
    }
}
