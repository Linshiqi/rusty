//! Noticing that somebody else changed the project.
//!
//! A workbench is never the only thing writing to a checkout: `git checkout`
//! moves half the tree, `cargo add` rewrites a manifest, another editor saves,
//! a build script regenerates a file. Until this existed, the window kept
//! showing whatever it had read when the project was opened, and the only way
//! back was a refresh button somebody had to know about.
//!
//! Three decisions carry the whole module.
//!
//! **`target/` is not watched.** One `cargo build` writes tens of thousands of
//! files, and a watcher that reported them would spend the build storming the
//! frontend with refreshes of a directory nobody is looking at. The same
//! walker rule the file tree already uses applies here — `.git`, dot
//! directories, and anything `.gitignore` names.
//!
//! **Events are batched, not forwarded.** Saving a file in another editor is
//! frequently a write, a rename and a second write; on Windows a single save
//! can produce four notifications. A quiet window closes the batch, so the
//! frontend gets one message per human action rather than one per syscall.
//!
//! **Content changes and structural changes are told apart.** Re-reading one
//! open file is free; walking a project is not. A modify says "this file";
//! a create, remove or rename says "the tree", and the frontend can then do
//! the cheap thing in the common case.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::model::FileChanges;

/// How long the tree must be quiet before a batch is delivered.
///
/// Long enough to swallow a multi-step save, short enough that a change made
/// in another window appears in this one while the user is still looking at
/// the same thing.
const QUIET: Duration = Duration::from_millis(250);

/// A running watcher. Dropping it stops the thread and releases the OS handles.
pub struct Watch {
    _watcher: RecommendedWatcher,
}

/// Watch `root`, and send a batch every time it settles.
///
/// The receiver ends when the [`Watch`] is dropped. Errors from the platform
/// watcher end the stream rather than being reported per-event: a watcher that
/// has failed reports nothing, and a frontend told about each individual
/// failure would show a banner per file.
pub fn watch(root: &Path) -> crate::Result<(Watch, mpsc::Receiver<FileChanges>)> {
    let root = root.to_path_buf();
    let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let (batched_tx, batched_rx) = mpsc::channel::<FileChanges>();

    let mut watcher = notify::recommended_watcher(raw_tx)
        .map_err(|e| crate::Error::Io(format!("could not start a file watcher: {e}")))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| crate::Error::Io(format!("could not watch {}: {e}", root.display())))?;

    std::thread::spawn(move || collect(&root, raw_rx, batched_tx));

    Ok((Watch { _watcher: watcher }, batched_rx))
}

/// Gather events until the tree goes quiet, then send what they amounted to.
fn collect(
    root: &Path,
    raw: mpsc::Receiver<notify::Result<notify::Event>>,
    out: mpsc::Sender<FileChanges>,
) {
    loop {
        // Block until something happens at all — no polling while a project
        // sits untouched.
        let Ok(first) = raw.recv() else {
            return;
        };
        let mut batch = FileChanges::default();
        let mut changed: BTreeSet<String> = BTreeSet::new();
        absorb(root, first, &mut batch, &mut changed);

        // Then drain everything that arrives inside the quiet window, and
        // extend the window each time — a `git checkout` is one action even
        // though it takes a second of syscalls.
        while let Ok(event) = raw.recv_timeout(QUIET) {
            absorb(root, event, &mut batch, &mut changed);
        }

        if changed.is_empty() && !batch.tree {
            continue;
        }
        batch.changed = changed.into_iter().collect();
        if out.send(batch).is_err() {
            return;
        }
    }
}

fn absorb(
    root: &Path,
    event: notify::Result<notify::Event>,
    batch: &mut FileChanges,
    changed: &mut BTreeSet<String>,
) {
    let Ok(event) = event else {
        return;
    };
    let paths: Vec<&PathBuf> = event.paths.iter().filter(|p| !ignored(root, p)).collect();
    if paths.is_empty() {
        return;
    }
    match event.kind {
        // A rename reports both ends, and either may be outside the project,
        // so the tree is the honest answer rather than a pair of paths.
        EventKind::Create(_) | EventKind::Remove(_) => batch.tree = true,
        EventKind::Modify(notify::event::ModifyKind::Name(_)) => batch.tree = true,
        EventKind::Modify(_) => {
            for path in paths {
                if let Some(relative) = relative(root, path) {
                    changed.insert(relative);
                }
            }
        }
        // `Any` is what several platform backends report when they cannot say
        // more. Treated as structural: claiming a file changed when the tree
        // did would leave a deleted file in the panel.
        EventKind::Any => batch.tree = true,
        EventKind::Access(_) | EventKind::Other => {}
    }
}

/// Paths the workbench deliberately does not watch.
///
/// `target/` is the one that matters — one build writes tens of thousands of
/// files there. The rest is the same rule the file tree draws: dot entries are
/// not shown, so a change to one cannot be visible.
fn ignored(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        // Outside the project. Some backends report the watched root's parent
        // on a rename; nothing in there is ours.
        return true;
    };
    relative.components().any(|c| match c {
        Component::Normal(part) => {
            let part = part.to_string_lossy();
            part == "target" || part.starts_with('.')
        }
        _ => false,
    })
}

/// Project-relative with `/` separators — the identity the frontend uses.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let text = relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_is_not_watched() {
        let root = Path::new("/project");
        assert!(ignored(root, Path::new("/project/target/debug/app")));
        assert!(ignored(root, Path::new("/project/target")));
        assert!(!ignored(root, Path::new("/project/src/main.rs")));
    }

    /// The tree hides dot entries, so a change to one cannot be visible — and
    /// `.git` alone would fire on every command git runs.
    #[test]
    fn dot_directories_are_not_watched() {
        let root = Path::new("/project");
        assert!(ignored(root, Path::new("/project/.git/index")));
        assert!(ignored(root, Path::new("/project/.rusty/sim.toml")));
        assert!(!ignored(root, Path::new("/project/src/lib.rs")));
    }

    /// A backend reporting the root's parent during a rename must not produce
    /// a path that escapes the project.
    #[test]
    fn anything_outside_the_project_is_ignored() {
        let root = Path::new("/project");
        assert!(ignored(root, Path::new("/elsewhere/file.rs")));
        assert!(ignored(root, Path::new("/")));
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("/project");
        assert_eq!(
            relative(root, Path::new("/project/src/main.rs")).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(relative(root, Path::new("/project")), None);
    }

    /// A create and a modify in one batch must not lose the create: the panel
    /// needs to know the tree moved, and re-reading one file would not show a
    /// new one.
    #[test]
    fn a_structural_change_survives_a_batch_of_edits() {
        let root = Path::new("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        absorb(
            root,
            Ok(notify::Event {
                kind: EventKind::Modify(notify::event::ModifyKind::Data(
                    notify::event::DataChange::Content,
                )),
                paths: vec![PathBuf::from("/project/src/main.rs")],
                attrs: Default::default(),
            }),
            &mut batch,
            &mut changed,
        );
        absorb(
            root,
            Ok(notify::Event {
                kind: EventKind::Create(notify::event::CreateKind::File),
                paths: vec![PathBuf::from("/project/src/new.rs")],
                attrs: Default::default(),
            }),
            &mut batch,
            &mut changed,
        );
        assert!(batch.tree);
        assert_eq!(
            changed.into_iter().collect::<Vec<_>>(),
            vec!["src/main.rs".to_string()]
        );
    }

    /// The same file saved four times — which is what one Ctrl+S in another
    /// editor looks like on Windows — is one entry, not four.
    #[test]
    fn one_save_is_one_entry_however_many_events_it_took() {
        let root = Path::new("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        for _ in 0..4 {
            absorb(
                root,
                Ok(notify::Event {
                    kind: EventKind::Modify(notify::event::ModifyKind::Data(
                        notify::event::DataChange::Any,
                    )),
                    paths: vec![PathBuf::from("/project/src/main.rs")],
                    attrs: Default::default(),
                }),
                &mut batch,
                &mut changed,
            );
        }
        assert_eq!(changed.len(), 1);
        assert!(!batch.tree, "a save is not a structural change");
    }

    /// A build writing into `target/` must produce nothing at all — this is
    /// the difference between a watcher and a refresh storm.
    #[test]
    fn a_build_produces_no_batch() {
        let root = Path::new("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        for n in 0..1000 {
            absorb(
                root,
                Ok(notify::Event {
                    kind: EventKind::Create(notify::event::CreateKind::File),
                    paths: vec![PathBuf::from(format!("/project/target/debug/deps/{n}.o"))],
                    attrs: Default::default(),
                }),
                &mut batch,
                &mut changed,
            );
        }
        assert!(!batch.tree);
        assert!(changed.is_empty());
    }
}
