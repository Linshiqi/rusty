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

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    model::{AssistantChoice, ProjectTabs, RelocateReport, StorageLocation},
};

/// The fixed anchor. Everything else is reachable from here.
fn anchor_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var("APPDATA")
            .ok()
            .map(|base| PathBuf::from(base).join("rusty"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(base) = std::env::var("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(base).join("rusty"));
        }
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".config").join("rusty"))
    }
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
        std::fs::create_dir_all(new_dir).map_err(|source| Error::Write {
            path: new_dir.display().to_string(),
            source,
        })?;
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

/// How much disk the data directory is using.
///
/// Its own call rather than a field on [`location`]: this walks the tree, and
/// `location` is asked for on paths that only want the path. Worth showing at
/// all because the number is what decides whether to move it — QEMU and the
/// two esp-gdb builds are most of it, and none of that is obvious from a
/// directory nobody opens.
pub fn footprint() -> u64 {
    fn walk(dir: &Path) -> u64 {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| match entry.file_type() {
                Ok(kind) if kind.is_dir() => walk(&entry.path()),
                _ => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            })
            .sum()
    }
    data_dir().map(|dir| walk(&dir)).unwrap_or(0)
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
    /// A file rather than the WebView's storage by this project's own rule: a
    /// second window boots the same frontend, and someone who edits in Vim
    /// keys wants both windows in the same mode. Landing in the wrong one is
    /// not a shrug — the next twenty keystrokes do something else entirely.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vim: bool,
    /// Display language, as a BCP-47 tag. `None` means follow the system,
    /// which is what a first run gets.
    ///
    /// Here rather than in the WebView's storage for the same reason `vim`
    /// is: a second window opening in a different language is not a shrug,
    /// and the WebView's storage is not carried when the data directory is
    /// relocated.
    #[serde(default)]
    pub locale: Option<String>,
    /// Terminal shell: absent = auto (rusty's own built-in shell); "system" =
    /// always the OS shell; anything else = a program to run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_shell: Option<String>,
    /// The assistant profile last chosen — never the key, which lives in the
    /// OS credential store and never enters the window at all.
    ///
    /// A file rather than the WebView's storage because a second window boots
    /// the same frontend and the backend reads it at the moment of a request:
    /// by this project's own rule, anything the backend or another window
    /// could care about is a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantChoice>,
    /// Editor tabs per project, newest project first — what reopens when a
    /// project is opened again.
    ///
    /// Here rather than in the project's `.rusty/` because which files *you*
    /// have open is not your team's business and has no place in a diff, and
    /// here rather than in the WebView because it is the one piece of this
    /// that is genuinely missed when it vanishes. Capped, for the reason
    /// `recent_projects` is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open_tabs: Vec<ProjectTabs>,
}

/// How many projects keep their tab strip. Beyond this the oldest is dropped:
/// a list that grows forever is a file that eventually nobody can read, and
/// the tabs of a project untouched for fifty projects are not missed.
const TABS_KEPT: usize = 40;

/// Remember a project's open editors, replacing whatever that project had.
pub fn record_tabs(root: &str, tabs: Vec<String>, active: Option<String>) {
    let _ = update(|state| push_tabs(&mut state.open_tabs, root, tabs, active));
}

/// The list discipline, separate from storage so it is testable — the same
/// shape `push_recent` has, and for the same reason.
fn push_tabs(list: &mut Vec<ProjectTabs>, root: &str, tabs: Vec<String>, active: Option<String>) {
    list.retain(|known| !same_dir(Path::new(&known.root), Path::new(root)));
    list.insert(
        0,
        ProjectTabs {
            root: root.to_string(),
            tabs,
            active,
        },
    );
    list.truncate(TABS_KEPT);
}

/// What a project had open last time, if anything.
///
/// Matched by [`same_dir`], so `E:\x` finds what `E:/x` saved — the trap the
/// WebView copy fell into, where a different spelling of the same directory
/// silently had no tabs.
pub fn tabs_for(root: &str) -> Option<ProjectTabs> {
    workbench()
        .open_tabs
        .into_iter()
        .find(|known| same_dir(Path::new(&known.root), Path::new(root)))
}

fn workbench_path() -> Option<PathBuf> {
    Some(data_dir()?.join("workbench.toml"))
}

/// The file's own records, kept apart from the wire types by the rule that
/// keeps `catalog.rs` apart from `model`: `workbench.toml` is a contract with
/// every installed copy of rusty, and `AssistantChoice` and `ProjectTabs` are
/// contracts with this build's frontend. When the two were one struct, a
/// field renamed for the frontend's sake would have silently dropped that key
/// from everybody's file — read as absent, written back without it.
///
/// Unknown fields survive a round trip *by being ignored on read and absent on
/// write* — an older rusty reading a newer file must not explode, which is why
/// this does not `deny_unknown_fields` the way the catalogue does.
mod file {
    use serde::{Deserialize, Serialize};

    use crate::model::{AssistantChoice, ProjectTabs};

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
        #[serde(default)]
        pub locale: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub terminal_shell: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub assistant: Option<Assistant>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub open_tabs: Vec<Tabs>,
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
    }

    impl From<Workbench> for super::WorkbenchState {
        fn from(file: Workbench) -> Self {
            Self {
                recent_projects: file.recent_projects,
                proxy: file.proxy,
                keybinds: file.keybinds,
                vim: file.vim,
                locale: file.locale,
                terminal_shell: file.terminal_shell,
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
                    })
                    .collect(),
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
                locale: state.locale.clone(),
                terminal_shell: state.terminal_shell.clone(),
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
                    })
                    .collect(),
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

pub fn save_workbench(state: &WorkbenchState) -> Result<()> {
    match workbench_path() {
        Some(path) => save_workbench_at(&path, state),
        None => Ok(()),
    }
}

fn save_workbench_at(path: &Path, state: &WorkbenchState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let body =
        toml::to_string_pretty(&file::Workbench::from(state)).expect("plain fields serialise");
    // A temporary name of this process's own, then a rename: a fixed
    // `workbench.toml.tmp` shared by two windows saving at once was written
    // by both, renamed by one, and the file that landed was neither's.
    let temp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, body).map_err(|source| Error::Write {
        path: temp.display().to_string(),
        source,
    })?;
    std::fs::rename(&temp, path).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}

/// Read, change, write — as one step, under the writers' lock. The one way
/// to change the file from this crate, so no two writers can interleave.
pub fn update(change: impl FnOnce(&mut WorkbenchState)) -> Result<()> {
    let _held = WRITERS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut state = workbench();
    change(&mut state);
    save_workbench(&state)
}

/// Record a project as the most recently opened.
pub fn record_recent(path: &str) {
    // Failure to persist a convenience is not worth failing the open over.
    let _ = update(|state| push_recent(&mut state.recent_projects, path));
}

/// Drop a project that turned out not to exist any more.
pub fn forget_recent(path: &str) {
    let _ = update(|state| {
        state
            .recent_projects
            .retain(|known| !same_dir(Path::new(known), Path::new(path)));
    });
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

/// Written atomically: temp file, then rename. A pointer half-written at the
/// moment of a crash would silently strand the data directory.
fn write_pointer(anchor: &Path, data: &Path) -> Result<()> {
    std::fs::create_dir_all(anchor).map_err(|source| Error::Write {
        path: anchor.display().to_string(),
        source,
    })?;
    let body = toml::to_string_pretty(&Pointer {
        data_dir: data.display().to_string(),
    })
    .expect("two fields cannot fail to serialise");

    let path = pointer_path(anchor);
    let temp = path.with_extension("toml.tmp");
    std::fs::write(&temp, body).map_err(|source| Error::Write {
        path: temp.display().to_string(),
        source,
    })?;
    std::fs::rename(&temp, &path).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Copy the data tree, skipping the pointer itself — it is the one file that
/// must never travel, being the map rather than the territory.
fn copy_tree(from: &Path, to: &Path, anchor: &Path) -> Result<usize> {
    let mut copied = 0;
    if !from.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(from).map_err(|source| Error::Read {
        path: from.display().to_string(),
        source,
    })?;
    for entry in entries.flatten() {
        let source_path = entry.path();
        let name = entry.file_name();
        if same_dir(from, anchor) && name == "location.toml" {
            continue;
        }
        let target_path = to.join(&name);
        if source_path.is_dir() {
            std::fs::create_dir_all(&target_path).map_err(|source| Error::Write {
                path: target_path.display().to_string(),
                source,
            })?;
            copied += copy_tree(&source_path, &target_path, anchor)?;
        } else {
            std::fs::copy(&source_path, &target_path).map_err(|source| Error::Write {
                path: target_path.display().to_string(),
                source,
            })?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// Path equality as the filesystem sees it, not as the string does.
fn same_dir(a: &Path, b: &Path) -> bool {
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

    /// The two properties the WebView's storage did not have: a different
    /// spelling of the same directory finds its tabs, and the list cannot
    /// grow without bound.
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

    /// The file records and the wire types are different structs now, and a
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
            locale: Some("zh-CN".into()),
            terminal_shell: Some("system".into()),
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
            }],
        };
        save_workbench_at(&path, &state).unwrap();
        let back = workbench_at(&path);

        assert_eq!(back.recent_projects, state.recent_projects);
        assert_eq!(back.proxy, state.proxy);
        assert_eq!(back.keybinds, state.keybinds);
        assert_eq!(back.vim, state.vim);
        assert_eq!(back.locale, state.locale);
        assert_eq!(back.terminal_shell, state.terminal_shell);
        assert_eq!(back.assistant, state.assistant);
        assert_eq!(back.open_tabs, state.open_tabs);
        assert!(
            !path
                .with_extension(format!("toml.{}.tmp", std::process::id()))
                .exists(),
            "the temporary is renamed away"
        );
    }

    #[test]
    fn tabs_are_kept_per_directory_and_the_list_is_capped() {
        let mut list = Vec::new();
        push_tabs(
            &mut list,
            "E:/work/blinky",
            vec!["src/main.rs".into()],
            None,
        );

        // The trap the WebView copy fell into: it keyed on the path as typed,
        // so opening the same project by another spelling silently had no
        // tabs. `recent_projects` learned this already; this shares the fix.
        push_tabs(
            &mut list,
            "E:\\work\\blinky",
            vec!["src/lib.rs".into()],
            None,
        );
        assert_eq!(list.len(), 1, "a different spelling is the same project");
        assert_eq!(
            list[0].tabs,
            vec!["src/lib.rs".to_string()],
            "and it replaces"
        );

        for n in 0..TABS_KEPT + 5 {
            push_tabs(&mut list, &format!("E:/p{n}"), vec!["a.rs".into()], None);
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
