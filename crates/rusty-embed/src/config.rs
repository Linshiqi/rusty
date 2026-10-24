//! Where rusty's data lives, and moving it.
//!
//! The location is configurable, which poses the classic bootstrap question:
//! where does the *pointer to the location* live? In a fixed anchor directory
//! that never moves — `%APPDATA%\rusty` — holding at most one small file,
//! `location.toml`, naming the real data directory. No pointer means the data
//! is in the anchor itself, which is where everyone starts.
//!
//! Pointing the data directory at a synced folder is the whole cloud-sync
//! story: the pointer stays machine-local — each machine names its own path to
//! the shared folder — and secrets never enter the data directory at all, so
//! syncing it never syncs a key.
//!
//! Resolution order: `RUSTY_CONFIG_DIR` (tests, portable installs, CI) beats
//! the pointer beats the anchor.
//!
//! **Where the anchor is, per platform.** `%APPDATA%\rusty` on Windows;
//! `$XDG_CONFIG_HOME/rusty` or `~/.config/rusty` on Linux. On macOS the
//! platform's own place is `~/Library/Application Support/rusty`, and rusty
//! kept its data in `~/.config/rusty` for every release before this rule
//! existed — so the anchor there is **the existing `~/.config/rusty` (or
//! `$XDG_CONFIG_HOME/rusty`) if either exists, else the Library directory**.
//! Nothing is moved silently: a user who already has boards and a
//! `workbench.toml` keeps them exactly where they are, and only a machine
//! with no rusty data at all starts in the Library. Relocation is still the
//! one way data moves, by the user's hand ([`relocate`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    model::{
        AssistantChoice, EditorView, ProjectTabs, RelocateReport, StorageLocation, WindowLayout,
    },
};

/// The fixed anchor. Everything else is reachable from here.
fn anchor_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("rusty"))
    }
    #[cfg(target_os = "macos")]
    {
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let home = PathBuf::from(std::env::var_os("HOME")?);
        Some(macos_anchor(xdg.as_deref(), &home, &|dir| dir.is_dir()))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(base).join("rusty"));
        }
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".config").join("rusty"))
    }
}

/// The macOS anchor, by the rule in the module header: a directory rusty
/// already keeps data in wins — `$XDG_CONFIG_HOME/rusty` when the variable
/// is set, then `~/.config/rusty` — and a machine with neither starts in
/// `~/Library/Application Support/rusty`. `exists` is handed in so the rule
/// is tested against a filesystem the test describes, not the one it runs on.
#[cfg(any(target_os = "macos", test))]
fn macos_anchor(xdg: Option<&Path>, home: &Path, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let legacy = xdg
        .map(|base| base.join("rusty"))
        .into_iter()
        .chain(std::iter::once(home.join(".config").join("rusty")));
    for dir in legacy {
        if exists(&dir) {
            return dir;
        }
    }
    home.join("Library")
        .join("Application Support")
        .join("rusty")
}

/// Where the data actually is.
pub fn data_dir() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("RUSTY_CONFIG_DIR") {
        return Some(PathBuf::from(explicit));
    }
    let anchor = anchor_dir()?;
    Some(resolve_pointer(&anchor).unwrap_or(anchor))
}

pub fn location() -> Option<StorageLocation> {
    let env_override = std::env::var_os("RUSTY_CONFIG_DIR").is_some();
    let data = data_dir()?;
    let is_default = !env_override && anchor_dir().is_some_and(|anchor| same_dir(&anchor, &data));
    Some(StorageLocation {
        path: data.display().to_string(),
        is_default,
        env_override,
    })
}

/// Move the data directory, copying what exists today.
///
/// Copy, not move: on any failure the old location is still whole, and the
/// switch is the last step. The old files stay behind on purpose — disk is
/// cheap and "the migration ate my board definitions" is not — and the report
/// says so, so the user can delete them once satisfied.
pub fn relocate(new_dir: &Path, take_existing: bool) -> Result<RelocateReport> {
    let anchor = anchor_dir().ok_or_else(|| Error::Config {
        detail: "no home directory to anchor the configuration in".into(),
    })?;
    let current = data_dir().unwrap_or_else(|| anchor.clone());

    if same_dir(&current, new_dir) {
        return Err(Error::Config {
            detail: format!("{} is already the data directory", new_dir.display()),
        });
    }

    let target_has_data =
        new_dir.join("workbench.toml").exists() || new_dir.join("boards").is_dir();

    let mut copied = 0usize;
    if take_existing {
        // Adopt what is there; copy nothing. The old directory keeps its
        // files, so switching back later loses nothing either.
        if !target_has_data {
            return Err(Error::Config {
                detail: format!(
                    "{} has no rusty data to adopt — choose \"copy current data\" instead",
                    new_dir.display(),
                ),
            });
        }
    } else {
        if target_has_data {
            return Err(Error::Config {
                detail: format!(
                    "{} already contains rusty data. Adopt it explicitly, or pick an \
                     empty folder — merging two data directories by accident is how \
                     board definitions vanish.",
                    new_dir.display(),
                ),
            });
        }
        std::fs::create_dir_all(new_dir).map_err(Error::writing(new_dir))?;
        copied = copy_tree(&current, new_dir, &anchor)?;
    }

    write_pointer(&anchor, new_dir)?;
    Ok(RelocateReport {
        from: current.display().to_string(),
        to: new_dir.display().to_string(),
        copied_files: copied,
        adopted: take_existing,
    })
}

// ─── workbench state ─────────────────────────────────────────────────────────

/// What the workbench remembers between runs. TOML in the data directory, so
/// it moves when the directory moves and a user can read what rusty knows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkbenchState {
    /// Newest first. The first entry is what launch reopens.
    #[serde(default)]
    pub recent_projects: Vec<String>,
    /// Network proxy: absent = detect (env, then the OS setting); "none" =
    /// force direct; anything else = an explicit proxy URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Keyboard shortcut overrides: action id → chord ("Ctrl+K"). Only the
    /// changed ones live here; defaults stay in code, so a new default in a
    /// newer rusty reaches everyone who has not overridden that action.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub keybinds: std::collections::BTreeMap<String, String>,
    /// Modal editing in the editor.
    ///
    /// A file by this project's own rule: a second window reads the same
    /// file, and someone who edits in Vim keys wants both windows in the same
    /// mode. Landing in the wrong one is not a shrug — the next twenty
    /// keystrokes do something else entirely.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vim: bool,
    /// Write the file a beat after typing stops, instead of waiting for
    /// Ctrl+S — VS Code's `files.autoSave: afterDelay`.
    ///
    /// A file for `vim`'s reason: a second window that did not auto-save
    /// would be a window whose work is lost on the assumption that it was.
    /// Off by default, because writing somebody's file without being asked
    /// is not a default.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_save: bool,
    /// Sweep stale build artifacts after every successful cargo command run
    /// from the dock — the opt-in that keeps a build directory from growing
    /// back. Off by default: deleting anything without being asked is not a
    /// default, even when the deletion is safe.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disk_auto_sweep: bool,
    /// Display language, as a BCP-47 tag. `None` means follow the system,
    /// which is what a first run gets.
    ///
    /// A file for the same reason `vim` is: a second window opening in a
    /// different language is not a shrug. And the choice goes with the data
    /// directory when it is relocated.
    #[serde(default)]
    pub locale: Option<String>,
    /// Terminal shell: absent = auto (rusty's own built-in shell); "system" =
    /// always the OS shell; anything else = a program to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_shell: Option<String>,
    /// A rust-analyzer to use instead of the one rusty would find.
    ///
    /// For a copy rusty's own ladder would not reach — the one an editor
    /// bundles, a build somebody is trying out. It was added as a way round
    /// the `--lockfile-path` failure and measured afterwards as not being
    /// one: every rust-analyzer since 1.83 sends that flag, and the fix is a
    /// newer Xtensa toolchain (`model::cargo_loses_dependencies`). What is
    /// left is an ordinary setting, and its footer says only that.
    ///
    /// A file rather than an environment variable because the app is
    /// launched from a Start menu, not a shell, and a second window has to
    /// agree with the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_analyzer: Option<String>,
    /// The assistant profile last chosen — never the key, which lives in the
    /// OS credential store and never enters the window at all.
    ///
    /// A file because a second window must agree with the first and the
    /// assistant reads it at the moment of a request: by this project's own
    /// rule, anything the libraries, the CLI or another window could care
    /// about is a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantChoice>,
    /// Editor tabs per project, newest project first — what reopens when a
    /// project is opened again.
    ///
    /// Here rather than in the project's `.rusty/` because which files *you*
    /// have open is not your team's business and has no place in a diff.
    /// Capped, for the reason `recent_projects` is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_tabs: Vec<ProjectTabs>,
    /// A version the user asked not to be prompted about again. The
    /// automatic check at launch stays quiet about it; the next release, or
    /// a check asked for by hand, is offered as usual.
    ///
    /// A file because a second window must not re-ask what the first was
    /// told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_update: Option<String>,
    /// What the editor draws around the code — inlay hints, the minimap,
    /// sticky scroll, indent guides. A file for `vim`'s reason: a second
    /// window drawing the editor differently from the first is not a shrug.
    #[serde(default)]
    pub editor: EditorView,
    /// The window's layout and appearance (`rusty-desk`).
    #[serde(default)]
    pub layout: WindowLayout,
}

/// How many projects keep their tab strip. Beyond this the oldest is dropped:
/// a list that grows forever is a file that eventually nobody can read, and
/// the tabs of a project untouched for fifty projects are not missed.
const TABS_KEPT: usize = 40;

/// Remember a project's open editors, replacing whatever that project had.
pub fn record_tabs(tabs: ProjectTabs) {
    let _ = update(|state| push_tabs(&mut state.open_tabs, tabs));
}

/// The list discipline, separate from storage so it is testable — the same
/// shape `push_recent` has, and for the same reason.
fn push_tabs(list: &mut Vec<ProjectTabs>, tabs: ProjectTabs) {
    list.retain(|known| !same_dir(Path::new(&known.root), Path::new(&tabs.root)));
    list.insert(0, tabs);
    list.truncate(TABS_KEPT);
}

/// What a project had open last time, if anything.
///
/// Matched by [`same_dir`], so `E:\x` finds what `E:/x` saved: keyed on the
/// path as typed, a different spelling of the same directory would silently
/// have no tabs.
pub fn tabs_for(root: &str) -> Option<ProjectTabs> {
    workbench()
        .open_tabs
        .into_iter()
        .find(|known| same_dir(Path::new(&known.root), Path::new(root)))
}

fn workbench_path() -> Option<PathBuf> {
    Some(data_dir()?.join("workbench.toml"))
}

/// The file's own records, kept apart from the model types by the rule that
/// keeps `catalog.rs` apart from `model`: `workbench.toml` is a contract with
/// every installed copy of rusty, and `AssistantChoice` and `ProjectTabs` are
/// contracts with this build's window. When the two were one struct, a
/// field renamed for the window's sake would have silently dropped that key
/// from everybody's file — read as absent, written back without it.
///
/// Unknown fields survive a round trip *by being ignored on read and absent on
/// write* — an older rusty reading a newer file must not explode, which is why
/// this does not `deny_unknown_fields` the way the catalogue does.
mod file {
    use serde::{Deserialize, Serialize};

    use crate::model::{
        AssistantChoice, DockLayout, EditorView, GitLayout, HistoryLayout, PagePlace, ProjectTabs,
        TabPlace, WindowLayout,
    };

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub(super) struct Workbench {
        #[serde(default)]
        pub recent_projects: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub proxy: Option<String>,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        pub keybinds: std::collections::BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub vim: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub auto_save: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub disk_auto_sweep: bool,
        #[serde(default)]
        pub locale: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub terminal_shell: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub rust_analyzer: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub assistant: Option<Assistant>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub open_tabs: Vec<Tabs>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub skipped_update: Option<String>,
        /// `[editor]`, written only when something in it is off.
        #[serde(default, skip_serializing_if = "Editor::is_default")]
        pub editor: Editor,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub layout: Option<Layout>,
    }

    /// `[layout]`: written whole once the window has been laid out, and
    /// read key by key, so a file from before a key existed still reads.
    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Layout {
        #[serde(default = "system")]
        pub theme: String,
        #[serde(default = "one")]
        pub zoom: f32,
        #[serde(default = "one")]
        pub editor_zoom: f32,
        #[serde(default = "one")]
        pub page_zoom: f32,
        #[serde(default = "one")]
        pub terminal_zoom: f32,
        #[serde(default)]
        pub left: Option<Dock>,
        #[serde(default)]
        pub bottom: Option<Dock>,
        #[serde(default)]
        pub right: Option<Dock>,
        #[serde(default = "half")]
        pub split: f32,
        /// `[layout.history]`, the History page's dividers.
        #[serde(default)]
        pub history: Option<History>,
        /// `[layout.git]`, how the Git panel draws its lists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub git: Option<GitPanel>,
    }

    #[derive(Debug, Default, Serialize, Deserialize)]
    pub(super) struct GitPanel {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        pub tree: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub folded: Vec<String>,
    }

    /// The History page's dividers, each key read on its own so a file
    /// that names one keeps the others' defaults.
    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct History {
        #[serde(default = "History::refs")]
        pub refs: f32,
        #[serde(default = "History::log")]
        pub log: f32,
        #[serde(default = "History::details")]
        pub details: f32,
        #[serde(default = "History::sides")]
        pub sides: f32,
    }

    impl History {
        fn refs() -> f32 {
            HistoryLayout::default().refs
        }
        fn log() -> f32 {
            HistoryLayout::default().log
        }
        fn details() -> f32 {
            HistoryLayout::default().details
        }
        fn sides() -> f32 {
            HistoryLayout::default().sides
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Dock {
        pub open: bool,
        pub size: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub front: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub hidden: Vec<String>,
    }

    fn system() -> String {
        "system".into()
    }

    fn one() -> f32 {
        1.0
    }

    fn half() -> f32 {
        0.5
    }

    impl From<&Layout> for WindowLayout {
        fn from(file: &Layout) -> Self {
            let defaults = WindowLayout::default();
            let dock = |dock: &Option<Dock>, default: DockLayout| {
                dock.as_ref().map_or(default, |d| DockLayout {
                    open: d.open,
                    size: d.size,
                    front: d.front.clone(),
                    hidden: d.hidden.clone(),
                })
            };
            WindowLayout {
                theme: file.theme.clone(),
                zoom: file.zoom,
                editor_zoom: file.editor_zoom,
                page_zoom: file.page_zoom,
                terminal_zoom: file.terminal_zoom,
                left: dock(&file.left, defaults.left),
                bottom: dock(&file.bottom, defaults.bottom),
                right: dock(&file.right, defaults.right),
                // A share, whatever a hand put in the file: the window
                // still keeps each group a minimum width.
                split: if file.split.is_finite() {
                    file.split.clamp(0.05, 0.95)
                } else {
                    defaults.split
                },
                history: file.history.as_ref().map_or(defaults.history, |h| {
                    HistoryLayout {
                        refs: h.refs,
                        log: h.log,
                        details: h.details,
                        sides: h.sides,
                    }
                    .sane()
                }),
                git: file
                    .git
                    .as_ref()
                    .map_or_else(GitLayout::default, |g| GitLayout {
                        tree: g.tree,
                        folded: g.folded.clone(),
                    }),
            }
        }
    }

    impl From<&WindowLayout> for Layout {
        fn from(layout: &WindowLayout) -> Self {
            let dock = |d: &DockLayout| {
                Some(Dock {
                    open: d.open,
                    size: d.size,
                    front: d.front.clone(),
                    hidden: d.hidden.clone(),
                })
            };
            Layout {
                theme: layout.theme.clone(),
                zoom: layout.zoom,
                editor_zoom: layout.editor_zoom,
                page_zoom: layout.page_zoom,
                terminal_zoom: layout.terminal_zoom,
                left: dock(&layout.left),
                bottom: dock(&layout.bottom),
                right: dock(&layout.right),
                split: layout.split,
                history: Some(History {
                    refs: layout.history.refs,
                    log: layout.history.log,
                    details: layout.history.details,
                    sides: layout.history.sides,
                }),
                git: (layout.git != GitLayout::default()).then(|| GitPanel {
                    tree: layout.git.tree,
                    folded: layout.git.folded.clone(),
                }),
            }
        }
    }

    /// The editor's switches as the file spells them: each key present only
    /// when it is off, since on is what an absent key means.
    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Editor {
        #[serde(default = "on", skip_serializing_if = "is_on")]
        pub inlay_hints: bool,
        #[serde(default = "on", skip_serializing_if = "is_on")]
        pub minimap: bool,
        #[serde(default = "on", skip_serializing_if = "is_on")]
        pub sticky_scroll: bool,
        #[serde(default = "on", skip_serializing_if = "is_on")]
        pub indent_guides: bool,
    }

    fn on() -> bool {
        true
    }

    fn is_on(value: &bool) -> bool {
        *value
    }

    impl Default for Editor {
        fn default() -> Self {
            EditorView::default().into()
        }
    }

    impl Editor {
        fn is_default(&self) -> bool {
            EditorView::from(self) == EditorView::default()
        }
    }

    impl From<EditorView> for Editor {
        fn from(view: EditorView) -> Self {
            Editor {
                inlay_hints: view.inlay_hints,
                minimap: view.minimap,
                sticky_scroll: view.sticky_scroll,
                indent_guides: view.indent_guides,
            }
        }
    }

    impl From<&Editor> for EditorView {
        fn from(file: &Editor) -> Self {
            EditorView {
                inlay_hints: file.inlay_hints,
                minimap: file.minimap,
                sticky_scroll: file.sticky_scroll,
                indent_guides: file.indent_guides,
            }
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Assistant {
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

    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct Tabs {
        pub root: String,
        pub tabs: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub active: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub second: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub second_active: Option<String>,
        /// `[open_tabs.places."src/main.rs"]`: where each file was left.
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        pub places: std::collections::BTreeMap<String, Place>,
    }

    /// One file's place, flat as a hand would write it.
    #[derive(Debug, Default, Serialize, Deserialize)]
    pub(super) struct Place {
        #[serde(default)]
        pub top: u32,
        #[serde(default)]
        pub line: u32,
        #[serde(default)]
        pub column: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub block: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub within: Option<f32>,
    }

    impl From<&Place> for TabPlace {
        fn from(p: &Place) -> Self {
            TabPlace {
                top: p.top,
                line: p.line,
                column: p.column,
                page: p.block.map(|block| PagePlace {
                    block,
                    within: p
                        .within
                        .filter(|w| w.is_finite())
                        .unwrap_or(0.0)
                        .clamp(0.0, 1.0),
                }),
            }
        }
    }

    impl From<&TabPlace> for Place {
        fn from(p: &TabPlace) -> Self {
            Place {
                top: p.top,
                line: p.line,
                column: p.column,
                block: p.page.map(|page| page.block),
                within: p.page.map(|page| page.within),
            }
        }
    }

    impl From<Workbench> for super::WorkbenchState {
        fn from(file: Workbench) -> Self {
            Self {
                recent_projects: file.recent_projects,
                proxy: file.proxy,
                keybinds: file.keybinds,
                vim: file.vim,
                auto_save: file.auto_save,
                disk_auto_sweep: file.disk_auto_sweep,
                locale: file.locale,
                terminal_shell: file.terminal_shell,
                rust_analyzer: file.rust_analyzer,
                assistant: file.assistant.map(|a| AssistantChoice {
                    profile: a.profile,
                    kind: a.kind,
                    base_url: a.base_url,
                    model: a.model,
                    max_tokens: a.max_tokens,
                    temperature: a.temperature,
                    supports_tools: a.supports_tools,
                }),
                open_tabs: file
                    .open_tabs
                    .into_iter()
                    .map(|t| ProjectTabs {
                        root: t.root,
                        tabs: t.tabs,
                        active: t.active,
                        second: t.second,
                        second_active: t.second_active,
                        places: t
                            .places
                            .iter()
                            .map(|(path, place)| (path.clone(), TabPlace::from(place)))
                            .collect(),
                    })
                    .collect(),
                skipped_update: file.skipped_update,
                editor: EditorView::from(&file.editor),
                layout: file
                    .layout
                    .as_ref()
                    .map(WindowLayout::from)
                    .unwrap_or_default(),
            }
        }
    }

    impl From<&super::WorkbenchState> for Workbench {
        fn from(state: &super::WorkbenchState) -> Self {
            Self {
                recent_projects: state.recent_projects.clone(),
                proxy: state.proxy.clone(),
                keybinds: state.keybinds.clone(),
                vim: state.vim,
                auto_save: state.auto_save,
                disk_auto_sweep: state.disk_auto_sweep,
                locale: state.locale.clone(),
                terminal_shell: state.terminal_shell.clone(),
                rust_analyzer: state.rust_analyzer.clone(),
                assistant: state.assistant.as_ref().map(|a| Assistant {
                    profile: a.profile.clone(),
                    kind: a.kind.clone(),
                    base_url: a.base_url.clone(),
                    model: a.model.clone(),
                    max_tokens: a.max_tokens,
                    temperature: a.temperature,
                    supports_tools: a.supports_tools,
                }),
                open_tabs: state
                    .open_tabs
                    .iter()
                    .map(|t| Tabs {
                        root: t.root.clone(),
                        tabs: t.tabs.clone(),
                        active: t.active.clone(),
                        second: t.second.clone(),
                        second_active: t.second_active.clone(),
                        places: t
                            .places
                            .iter()
                            .map(|(path, place)| (path.clone(), Place::from(place)))
                            .collect(),
                    })
                    .collect(),
                skipped_update: state.skipped_update.clone(),
                editor: state.editor.into(),
                layout: (state.layout != WindowLayout::default())
                    .then(|| Layout::from(&state.layout)),
            }
        }
    }
}

/// One writer at a time within this process. Every writer is a
/// read-modify-write of the whole file, and two of them interleaving —
/// the tab strip is recorded on every tab switch, the recents on every open
/// — lost whichever wrote first. Held by [`update`], across the read and the
/// write both.
static WRITERS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the workbench remembers, or a fresh state.
///
/// **"Not there yet" and "there and unreadable" are different, and conflating
/// them destroys data.** Every writer here is a read-modify-write; a file that
/// failed to parse but read back as `default()` is one the very next save
/// overwrites with nothing, silently, taking every recent project with it.
/// That is how a whole list can vanish between two launches with nothing in
/// the logs.
///
/// So a file that exists and does not parse is moved aside rather than read as
/// empty. Nothing is lost — it is still there under `.broken` — the next save
/// cannot clobber it, and the workbench starts clean instead of refusing to
/// work until somebody edits TOML by hand.
pub fn workbench() -> WorkbenchState {
    workbench_path()
        .map(|path| workbench_at(&path))
        .unwrap_or_default()
}

/// [`workbench`] against a named file — the whole of the logic, so a test can
/// run it against a directory of its own rather than the machine's.
fn workbench_at(path: &Path) -> WorkbenchState {
    let Ok(raw) = std::fs::read_to_string(path) else {
        // No file, or unreadable this instant. Either way there is nothing to
        // lose by starting from default; a save will create it.
        return WorkbenchState::default();
    };
    match toml::from_str::<file::Workbench>(&raw) {
        Ok(state) => state.into(),
        Err(error) => {
            let kept = path.with_extension("toml.broken");
            let _ = std::fs::rename(path, &kept);
            eprintln!(
                "rusty: {} did not parse ({error}); kept it as {} and started fresh",
                path.display(),
                kept.display(),
            );
            WorkbenchState::default()
        }
    }
}

fn save_workbench_at(path: &Path, state: &WorkbenchState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(Error::writing(parent))?;
    }
    let body =
        toml::to_string_pretty(&file::Workbench::from(state)).expect("plain fields serialise");
    write_atomically(path, &body)
}

/// Write a file through a temporary of this process's own and a rename, so
/// a reader never meets half of it. The temporary's name carries the process
/// id: a fixed `workbench.toml.tmp` shared by two windows saving at once was
/// written by both, renamed by one, and the file that landed was neither's.
pub(crate) fn write_atomically(path: &Path, text: &str) -> Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(Error::writing(&temp))?;
    std::fs::rename(&temp, path).map_err(Error::writing(path))
}

/// Read, change, write — as one step, under the writers' lock. The one way
/// to change the file from this crate, so no two writers can interleave.
pub fn update(change: impl FnOnce(&mut WorkbenchState)) -> Result<()> {
    match workbench_path() {
        Some(path) => update_at(&path, change),
        None => Ok(()),
    }
}

/// [`update`] against a named file, so a test can hold the file's lock from
/// elsewhere and watch a writer wait for it.
fn update_at(path: &Path, change: impl FnOnce(&mut WorkbenchState)) -> Result<()> {
    let _held = WRITERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _across = hold_across_processes(path);
    let mut state = workbench_at(path);
    change(&mut state);
    save_workbench_at(path, &state)
}

/// An exclusive lock on `workbench.lock` beside the file, for as long as the
/// handle lives: what keeps a writer in *another process* out of the
/// read-modify-write. Every window New Window opens is another rusty process,
/// sharing this file with the first and invisible to [`WRITERS`] — one
/// window's tab switch could read the file, the other's keybinding change
/// land, and the tab strip written back over it. The lock file is never
/// written, so a data directory in a synced folder sees it once.
///
/// Best effort: a directory that cannot hold the file leaves the writers'
/// lock doing what it always did.
fn hold_across_processes(path: &Path) -> Option<std::fs::File> {
    let lock = path.with_file_name("workbench.lock");
    if let Some(parent) = lock.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock)
        .ok()?;
    file.lock().ok()?;
    Some(file)
}

/// Record a project as the most recently opened.
pub fn record_recent(path: &str) {
    // Failure to persist a convenience is not worth failing the open over.
    let _ = update(|state| push_recent(&mut state.recent_projects, path));
}

/// The list discipline, separate from storage so it is testable: newest
/// first, no duplicates, capped. A recents list that grows forever is a
/// history, and a history is a different feature.
fn push_recent(list: &mut Vec<String>, path: &str) {
    list.retain(|known| !same_dir(Path::new(known), Path::new(path)));
    list.insert(0, path.to_string());
    list.truncate(8);
}

// ─── the pointer file ────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct Pointer {
    data_dir: String,
}

fn pointer_path(anchor: &Path) -> PathBuf {
    anchor.join("location.toml")
}

/// A missing or unreadable pointer degrades to "data in the anchor" — the
/// state every install starts in — rather than to an error nothing can act on.
fn resolve_pointer(anchor: &Path) -> Option<PathBuf> {
    let raw = std::fs::read_to_string(pointer_path(anchor)).ok()?;
    let pointer: Pointer = toml::from_str(&raw).ok()?;
    Some(PathBuf::from(pointer.data_dir))
}

/// Written atomically: a pointer half-written at the moment of a crash would
/// silently strand the data directory.
fn write_pointer(anchor: &Path, data: &Path) -> Result<()> {
    std::fs::create_dir_all(anchor).map_err(Error::writing(anchor))?;
    let body = toml::to_string_pretty(&Pointer {
        data_dir: data.display().to_string(),
    })
    .expect("two fields cannot fail to serialise");

    write_atomically(&pointer_path(anchor), &body)
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Copy the data tree, skipping the pointer itself — it is the one file that
/// must never travel, being the map rather than the territory.
fn copy_tree(from: &Path, to: &Path, anchor: &Path) -> Result<usize> {
    let mut copied = 0;
    if !from.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(from).map_err(Error::reading(from))?;
    for entry in entries.flatten() {
        let source_path = entry.path();
        let name = entry.file_name();
        if same_dir(from, anchor) && name == "location.toml" {
            continue;
        }
        let target_path = to.join(&name);
        if source_path.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(Error::writing(&target_path))?;
            copied += copy_tree(&source_path, &target_path, anchor)?;
        } else {
            std::fs::copy(&source_path, &target_path).map_err(Error::writing(&target_path))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Path equality as the filesystem sees it, not as the string does. The
/// recents, the tab strips and the window that has a project open all
/// answer "is this that directory" by it.
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        // One of them may not exist yet; fall back to a spelling-insensitive
        // comparison so `E:\x` and `e:/x` still match.
        _ => {
            let fold = |p: &Path| {
                p.to_string_lossy()
                    .replace('\\', "/")
                    .trim_end_matches('/')
                    .to_lowercase()
            };
            fold(a) == fold(b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this is written against: a workbench.toml that does not
    /// parse used to read back as an empty state, and the next save — which
    /// every writer here does as read-modify-write — wrote that emptiness
    /// over it. One transient bad file, and every recent project is gone with
    /// nothing said.
    ///
    /// Runs the real loader against a directory of its own — tests must not
    /// read or write the machine they run on — and it *is* the real loader:
    /// an earlier version of this test re-did the rename by hand and would
    /// have stayed green with the production move deleted.
    #[test]
    fn a_file_that_does_not_parse_is_kept_rather_than_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(&path, "recent_projects = [\"E:/work\"]\nthis is not toml\n").unwrap();

        let state = workbench_at(&path);
        assert!(
            state.recent_projects.is_empty(),
            "a fresh start, not a guess"
        );

        let kept = path.with_extension("toml.broken");
        assert!(
            std::fs::read_to_string(&kept).unwrap().contains("E:/work"),
            "the list survives where somebody can get it back",
        );
        assert!(
            !path.exists(),
            "and a save now creates a file rather than clobbering one"
        );
        save_workbench_at(&path, &state).unwrap();
        assert!(path.exists(), "the next save creates rather than clobbers");
    }

    /// Another rusty window is another process, out of reach of the
    /// writers' mutex: the file's own lock is what keeps its
    /// read-modify-write from landing inside this one's. Held here through a
    /// handle of its own, which the OS counts as a second owner, as it would
    /// the other window.
    #[test]
    fn a_writer_waits_while_another_window_holds_the_file() {
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        let other = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.path().join("workbench.lock"))
            .unwrap();
        other.lock().unwrap();
        let holder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(other);
        });

        let started = Instant::now();
        update_at(&path, |state| state.vim = true).unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(250),
            "the write went ahead while the other window held the file"
        );
        holder.join().unwrap();
        assert!(workbench_at(&path).vim, "and it landed once it could");
    }

    /// The file records and the model types are different structs now, and a
    /// round trip through the file must lose nothing. Every field is set to
    /// something that is not its default, per the fixture rule: a field the
    /// writer forgot and the reader defaulted would otherwise agree perfectly.
    #[test]
    fn every_field_survives_the_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        let state = WorkbenchState {
            recent_projects: vec!["E:/work/blinky".into()],
            proxy: Some("http://proxy:3128".into()),
            keybinds: [("palette".to_string(), "Ctrl+P".to_string())]
                .into_iter()
                .collect(),
            vim: true,
            auto_save: true,
            disk_auto_sweep: true,
            locale: Some("zh-CN".into()),
            terminal_shell: Some("system".into()),
            rust_analyzer: Some("C:/ra/rust-analyzer.exe".into()),
            assistant: Some(AssistantChoice {
                profile: "work".into(),
                kind: "anthropic".into(),
                base_url: "https://api.example".into(),
                model: "claude".into(),
                max_tokens: Some(2048),
                temperature: Some(0.3),
                supports_tools: Some(false),
            }),
            open_tabs: vec![ProjectTabs {
                root: "E:/work/blinky".into(),
                tabs: vec!["src/main.rs".into()],
                active: Some("src/main.rs".into()),
                // Off their defaults, or the round trip proves nothing about
                // the fields it leaves alone.
                second: vec!["src/lib.rs".into()],
                second_active: Some("src/lib.rs".into()),
                // A code file's place and a page's, every field off zero.
                places: [
                    (
                        "src/main.rs".to_string(),
                        crate::model::TabPlace {
                            top: 40,
                            line: 52,
                            column: 7,
                            page: None,
                        },
                    ),
                    (
                        "README.md".to_string(),
                        crate::model::TabPlace {
                            top: 3,
                            line: 9,
                            column: 1,
                            page: Some(crate::model::PagePlace {
                                block: 17,
                                within: 0.25,
                            }),
                        },
                    ),
                ]
                .into(),
            }],
            skipped_update: Some("0.9.9".into()),
            // Every switch off its default, which is on.
            editor: EditorView {
                inlay_hints: false,
                minimap: false,
                sticky_scroll: false,
                indent_guides: false,
            },
            layout: crate::model::WindowLayout {
                theme: "light".into(),
                zoom: 1.25,
                editor_zoom: 1.3,
                page_zoom: 0.8,
                terminal_zoom: 1.5,
                left: crate::model::DockLayout {
                    open: false,
                    size: 300.0,
                    front: Some("git".into()),
                    hidden: vec!["search".into()],
                },
                bottom: crate::model::DockLayout {
                    open: true,
                    size: 180.0,
                    front: Some("terminal".into()),
                    hidden: vec!["output".into(), "problems".into()],
                },
                right: crate::model::DockLayout {
                    open: true,
                    size: 500.0,
                    front: Some("board".into()),
                    hidden: vec!["assistant".into()],
                },
                split: 0.3,
                // Every divider off its default.
                history: crate::model::HistoryLayout {
                    refs: 300.0,
                    log: 0.45,
                    details: 0.25,
                    sides: 0.65,
                },
                git: crate::model::GitLayout {
                    tree: true,
                    folded: vec!["staged".into(), "stashes".into()],
                },
            },
        };
        save_workbench_at(&path, &state).unwrap();
        let back = workbench_at(&path);

        assert_eq!(back.recent_projects, state.recent_projects);
        assert_eq!(back.proxy, state.proxy);
        assert_eq!(back.keybinds, state.keybinds);
        assert_eq!(back.vim, state.vim);
        assert_eq!(back.auto_save, state.auto_save);
        assert_eq!(back.disk_auto_sweep, state.disk_auto_sweep);
        assert_eq!(back.locale, state.locale);
        assert_eq!(back.terminal_shell, state.terminal_shell);
        assert_eq!(back.rust_analyzer, state.rust_analyzer);
        assert_eq!(back.assistant, state.assistant);
        assert_eq!(back.open_tabs, state.open_tabs);
        assert_eq!(back.skipped_update, state.skipped_update);
        assert_eq!(back.editor, state.editor);
        assert_eq!(back.layout, state.layout);
        assert!(
            !path
                .with_extension(format!("toml.{}.tmp", std::process::id()))
                .exists(),
            "the temporary is renamed away"
        );
    }

    /// The editor's switches are on unless a file says otherwise, and a file
    /// that leaves them all on does not mention them.
    #[test]
    fn the_editor_switches_are_on_until_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(&path, "recent_projects = []\n").unwrap();
        assert_eq!(workbench_at(&path).editor, EditorView::default());

        std::fs::write(&path, "[editor]\nminimap = false\n").unwrap();
        let read = workbench_at(&path);
        assert!(!read.editor.minimap);
        assert!(read.editor.inlay_hints && read.editor.sticky_scroll && read.editor.indent_guides);

        save_workbench_at(&path, &WorkbenchState::default()).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(!written.contains("[editor]"), "{written}");
    }

    /// A layout written before the text had sizes of its own, or the docks
    /// a hidden panel, reads with every text at its own size and every
    /// panel on its strip — not at a size of zero, which draws nothing.
    #[test]
    fn a_layout_from_before_text_zoom_reads_at_its_own_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(
            &path,
            "[layout]\ntheme = \"dark\"\nzoom = 1.1\n\n[layout.bottom]\nopen = true\nsize = 200.0\n",
        )
        .unwrap();
        let layout = workbench_at(&path).layout;
        assert_eq!(layout.zoom, 1.1);
        assert_eq!(
            (layout.editor_zoom, layout.page_zoom, layout.terminal_zoom),
            (1.0, 1.0, 1.0)
        );
        assert!(layout.bottom.open && layout.bottom.hidden.is_empty());
    }

    /// A layout written before the editor groups' divider was kept reads as
    /// halves, and a share no window could draw is brought back to one.
    #[test]
    fn the_split_reads_as_halves_until_moved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(&path, "[layout]\ntheme = \"dark\"\nzoom = 1.0\n").unwrap();
        assert_eq!(workbench_at(&path).layout.split, 0.5);

        std::fs::write(&path, "[layout]\nsplit = 7.0\n").unwrap();
        assert_eq!(workbench_at(&path).layout.split, 0.95);
    }

    /// The History page's dividers read as Fork's proportions in a file
    /// from before they were kept, a file that names one keeps the others'
    /// defaults, and a value no page could draw is brought back to one.
    #[test]
    fn the_history_dividers_read_as_their_defaults_until_moved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(&path, "[layout]\ntheme = \"dark\"\n").unwrap();
        assert_eq!(
            workbench_at(&path).layout.history,
            crate::model::HistoryLayout::default()
        );

        std::fs::write(&path, "[layout]\n\n[layout.history]\nlog = 0.3\n").unwrap();
        let history = workbench_at(&path).layout.history;
        assert_eq!(history.log, 0.3);
        assert_eq!(
            (history.refs, history.details, history.sides),
            (220.0, 0.38, 0.5)
        );

        std::fs::write(
            &path,
            "[layout]\n\n[layout.history]\nrefs = 9000.0\nlog = -2.0\ndetails = nan\nsides = 1.0\n",
        )
        .unwrap();
        let history = workbench_at(&path).layout.history;
        assert_eq!(history.refs, 1200.0, "a width a page can hold");
        assert_eq!(history.log, 0.05, "a share inside the room");
        assert_eq!(history.details, 0.38, "not a number is the default");
        assert_eq!(history.sides, 0.95);
    }

    /// The two properties the tab strips need: a different spelling of the
    /// same directory finds its tabs, and the list cannot grow without bound.
    #[test]
    fn tabs_are_kept_per_directory_and_the_list_is_capped() {
        let strip = |root: &str, file: &str| ProjectTabs {
            root: root.into(),
            tabs: vec![file.into()],
            active: None,
            second: Vec::new(),
            second_active: None,
            places: Default::default(),
        };
        let mut list = Vec::new();
        push_tabs(&mut list, strip("E:/work/blinky", "src/main.rs"));

        // The trap of keying on the path as typed: opening the same project
        // by another spelling would silently have no tabs. `recent_projects`
        // learned this already; this shares the fix.
        push_tabs(&mut list, strip("E:\\work\\blinky", "src/lib.rs"));
        assert_eq!(list.len(), 1, "a different spelling is the same project");
        assert_eq!(
            list[0].tabs,
            vec!["src/lib.rs".to_string()],
            "and it replaces"
        );

        for n in 0..TABS_KEPT + 5 {
            push_tabs(&mut list, strip(&format!("E:/p{n}"), "a.rs"));
        }
        assert_eq!(
            list.len(),
            TABS_KEPT,
            "one key per project ever opened is what this replaced",
        );
        assert_eq!(
            list[0].root,
            format!("E:/p{}", TABS_KEPT + 4),
            "newest first"
        );
    }

    #[test]
    fn relocation_copies_and_switches_without_touching_the_original() {
        let scratch = tempfile::tempdir().unwrap();
        let anchor = scratch.path().join("anchor");
        std::fs::create_dir_all(anchor.join("boards")).unwrap();
        std::fs::write(anchor.join("boards/mine.toml"), "[[board]]").unwrap();
        std::fs::write(anchor.join("workbench.toml"), "recent = []").unwrap();

        let new_home = scratch.path().join("synced/rusty");
        std::fs::create_dir_all(&new_home).unwrap();

        // Drive the internals directly: the public fns read the real APPDATA,
        // and tests must not depend on — or write to — the machine they run on.
        let copied = copy_tree(&anchor, &new_home, &anchor).unwrap();
        write_pointer(&anchor, &new_home).unwrap();

        assert_eq!(copied, 2);
        assert!(new_home.join("boards/mine.toml").exists());
        assert!(anchor.join("boards/mine.toml").exists(), "copy, not move");
        assert_eq!(
            resolve_pointer(&anchor).unwrap(),
            new_home,
            "the pointer names the new home",
        );
        assert!(
            !new_home.join("location.toml").exists(),
            "the pointer must not travel into the data it points at",
        );
    }

    #[test]
    fn recents_deduplicate_across_spellings_and_stay_capped() {
        let mut list = vec!["E:/x".to_string(), "E:/y".to_string()];
        push_recent(&mut list, "e:\\x");
        assert_eq!(list, ["e:\\x", "E:/y"], "one project, one entry");

        let mut list: Vec<String> = (0..8).map(|n| format!("E:/p{n}")).collect();
        push_recent(&mut list, "E:/new");
        assert_eq!(list.len(), 8);
        assert_eq!(list[0], "E:/new");
        assert!(!list.iter().any(|p| p == "E:/p7"), "the oldest falls off");
    }

    /// The macOS rule, against a filesystem the test states. The one thing
    /// it must never do is move a user's data by choosing a new directory
    /// over one that holds boards: an existing `~/.config/rusty` wins, and
    /// the Library is for a machine with nothing.
    #[test]
    fn a_mac_keeps_its_old_data_where_it_is_and_a_fresh_one_uses_the_library() {
        let home = Path::new("/Users/me");
        let library = PathBuf::from("/Users/me/Library/Application Support/rusty");
        let dot_config = PathBuf::from("/Users/me/.config/rusty");
        let xdg = Path::new("/Users/me/xdg");

        let nothing = |_: &Path| false;
        assert_eq!(macos_anchor(None, home, &nothing), library);
        assert_eq!(
            macos_anchor(Some(xdg), home, &nothing),
            library,
            "XDG_CONFIG_HOME set but holding no rusty data does not keep a Mac off the \
             platform's own directory",
        );

        let has_dot_config = |dir: &Path| dir == dot_config;
        assert_eq!(macos_anchor(None, home, &has_dot_config), dot_config);
        assert_eq!(
            macos_anchor(Some(xdg), home, &has_dot_config),
            dot_config,
            "the existing data is found past an XDG directory that has none",
        );

        let has_xdg = |dir: &Path| dir == xdg.join("rusty");
        assert_eq!(
            macos_anchor(Some(xdg), home, &has_xdg),
            xdg.join("rusty"),
            "a user who pointed XDG_CONFIG_HOME at their data keeps it",
        );
        let has_both = |dir: &Path| dir == xdg.join("rusty") || dir == dot_config;
        assert_eq!(
            macos_anchor(Some(xdg), home, &has_both),
            xdg.join("rusty"),
            "and XDG wins over ~/.config, as it did before this rule existed",
        );
        assert_eq!(
            macos_anchor(None, home, &|dir: &Path| dir == library),
            library,
            "the Library existing changes nothing: it is the default already",
        );
    }

    #[test]
    fn a_garbage_pointer_degrades_to_the_anchor() {
        let scratch = tempfile::tempdir().unwrap();
        let anchor = scratch.path().to_path_buf();
        std::fs::write(pointer_path(&anchor), "not toml at all [[[").unwrap();
        assert_eq!(resolve_pointer(&anchor), None);
    }

    #[test]
    fn spelling_differences_are_the_same_directory() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("Data");
        std::fs::create_dir_all(&path).unwrap();
        let lower = PathBuf::from(path.display().to_string().to_lowercase().replace('\\', "/"));
        assert!(same_dir(&path, &lower));
    }
}
