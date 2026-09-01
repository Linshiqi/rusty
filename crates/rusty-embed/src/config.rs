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
    /// Terminal shell: absent = auto (the bundled Nushell when installed,
    /// else the system shell); "system" = always the OS shell; anything
    /// else = a program to run.
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
    let mut state = workbench();
    push_tabs(&mut state.open_tabs, root, tabs, active);
    let _ = save_workbench(&state);
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

/// Unknown fields survive a round trip *by being ignored on read and absent on
/// write* — an older rusty reading a newer file must not explode, which is why
/// this does not `deny_unknown_fields` the way the catalogue does.
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
    let Some(path) = workbench_path() else {
        return WorkbenchState::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        // No file, or unreadable this instant. Either way there is nothing to
        // lose by starting from default; a save will create it.
        return WorkbenchState::default();
    };
    match toml::from_str(&raw) {
        Ok(state) => state,
        Err(error) => {
            let kept = path.with_extension("toml.broken");
            let _ = std::fs::rename(&path, &kept);
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
    let Some(path) = workbench_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Write {
            path: parent.display().to_string(),
            source,
        })?;
    }
    let body = toml::to_string_pretty(state).expect("a list of strings serialises");
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

/// Record a project as the most recently opened.
pub fn record_recent(path: &str) {
    let mut state = workbench();
    push_recent(&mut state.recent_projects, path);
    // Failure to persist a convenience is not worth failing the open over.
    let _ = save_workbench(&state);
}

/// Drop a project that turned out not to exist any more.
pub fn forget_recent(path: &str) {
    let mut state = workbench();
    state
        .recent_projects
        .retain(|known| !same_dir(Path::new(known), Path::new(path)));
    let _ = save_workbench(&state);
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
    /// Drives the internals rather than the public fns, like the relocation
    /// test above and for the same reason: tests must not read or write the
    /// machine they run on.
    #[test]
    fn a_file_that_does_not_parse_is_kept_rather_than_read_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workbench.toml");
        std::fs::write(&path, "recent_projects = [\"E:/work\"]\nthis is not toml\n").unwrap();

        // What `workbench()` does with the file it just read, minus the
        // machine-dependent path lookup.
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: std::result::Result<WorkbenchState, _> = toml::from_str(&raw);
        assert!(parsed.is_err(), "the fixture has to be genuinely malformed");

        let kept = path.with_extension("toml.broken");
        std::fs::rename(&path, &kept).unwrap();
        assert!(
            std::fs::read_to_string(&kept).unwrap().contains("E:/work"),
            "the list survives where somebody can get it back",
        );
        assert!(
            !path.exists(),
            "and a save now creates a file rather than clobbering one"
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
