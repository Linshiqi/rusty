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
//! frontend with refreshes of a directory nobody is looking at. The rule is
//! the one the file tree and search already apply — dot entries through
//! [`hidden_entry`], `target/` by name, and whatever the root `.gitignore`
//! names. The last matters for the project whose `.cargo/config.toml` says
//! `target-dir = "build"`: without it, that build stormed exactly as `target/`
//! would have, while this paragraph claimed the ignore file was honoured.
//!
//! **Events are batched, not forwarded.** Saving a file in another editor is
//! frequently a write, a rename and a second write; on Windows a single save
//! can produce four notifications. A quiet window closes the batch, so the
//! frontend gets one message per human action rather than one per syscall.
//! Only *reported* events hold the window open — a build writing into
//! `target/` for a minute must not delay the one real save made as it began.
//!
//! **Content changes and structural changes are told apart.** Re-reading one
//! open file is free; walking a project is not. A modify says "this file";
//! a create, remove or rename says "the tree", and the frontend can then do
//! the cheap thing in the common case.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use ignore::gitignore::Gitignore;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::hidden::hidden_entry;
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
    let rules = Rules::load(root);
    let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let (batched_tx, batched_rx) = mpsc::channel::<FileChanges>();

    let mut watcher = notify::recommended_watcher(raw_tx)
        .map_err(|e| crate::Error::Io(format!("could not start a file watcher: {e}")))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| crate::Error::Io(format!("could not watch {}: {e}", root.display())))?;

    std::thread::spawn(move || collect(rules, raw_rx, batched_tx));

    Ok((Watch { _watcher: watcher }, batched_rx))
}

/// What the watcher does not report, and where it reads that from.
struct Rules {
    root: PathBuf,
    /// The root `.gitignore`, so build output the project keeps somewhere
    /// other than `target/` is skipped by the same rule the tree and search
    /// already apply. Read again when the file itself changes.
    gitignore: Gitignore,
}

impl Rules {
    fn load(root: &Path) -> Rules {
        // Errors are dropped on purpose: a `.gitignore` with one bad glob
        // still yields a matcher for the rest, and no `.gitignore` yields an
        // empty one. A scratch directory need not have the file; `target/`
        // is skipped by name regardless.
        let (gitignore, _) = Gitignore::new(root.join(".gitignore"));
        Rules {
            root: root.to_path_buf(),
            gitignore,
        }
    }

    /// Paths the workbench deliberately does not watch.
    fn ignored(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            // Outside the project. Some backends report the watched root's
            // parent on a rename; nothing in there is ours.
            return true;
        };
        let by_name = relative.components().any(|c| match c {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                part == "target" || hidden_entry(&part)
            }
            _ => false,
        });
        if by_name {
            return true;
        }
        // `is_dir` is not asked of the disk: that would be a stat per event
        // in exactly the storm this exists to absorb. A `build/` pattern
        // still matches everything *inside* the directory through the parent
        // walk; the one miss is the directory entry itself, which costs a
        // single tree refresh rather than thousands.
        self.gitignore
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    /// Whether this event is the ignore file itself changing — the one dot
    /// entry that is read rather than dropped, because it changes what else
    /// gets reported.
    fn touches_ignore_file(event: &notify::Event) -> bool {
        event
            .paths
            .iter()
            .any(|p| p.file_name().is_some_and(|name| name == ".gitignore"))
    }
}

/// Gather events until the tree goes quiet, then send what they amounted to.
fn collect(
    mut rules: Rules,
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
        if !absorb(&mut rules, first, &mut batch, &mut changed) {
            // Build output, a dot file: nothing anybody can see. Back to
            // sleep, rather than opening a window only more noise can fill.
            continue;
        }

        // Then drain everything that arrives inside the quiet window, and
        // extend the window for each *reported* event — a `git checkout` is
        // one action even though it takes a second of syscalls. An ignored
        // event does not extend it, or a build would hold back every save
        // made while it ran.
        let mut deadline = Instant::now() + QUIET;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match raw.recv_timeout(remaining) {
                Ok(event) => {
                    if absorb(&mut rules, event, &mut batch, &mut changed) {
                        deadline = Instant::now() + QUIET;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    // The watcher is gone. What was gathered still happened.
                    batch.changed = changed.into_iter().collect();
                    let _ = out.send(batch);
                    return;
                }
            }
        }

        batch.changed = changed.into_iter().collect();
        if out.send(batch).is_err() {
            return;
        }
    }
}

/// Fold one event into the batch. Returns whether it recorded anything — an
/// ignored path, a failed event or pure access noise does not, and must not
/// keep the quiet window open.
fn absorb(
    rules: &mut Rules,
    event: notify::Result<notify::Event>,
    batch: &mut FileChanges,
    changed: &mut BTreeSet<String>,
) -> bool {
    let Ok(event) = event else {
        return false;
    };
    if Rules::touches_ignore_file(&event) {
        *rules = Rules::load(&rules.root);
    }
    let paths: Vec<&PathBuf> = event.paths.iter().filter(|p| !rules.ignored(p)).collect();
    if paths.is_empty() {
        return false;
    }
    match event.kind {
        // A rename reports both ends, and either may be outside the project,
        // so the tree is the honest answer rather than a pair of paths.
        EventKind::Create(_) | EventKind::Remove(_) => batch.tree = true,
        EventKind::Modify(notify::event::ModifyKind::Name(_)) => batch.tree = true,
        EventKind::Modify(_) => {
            for path in paths {
                if let Some(relative) = relative(&rules.root, path) {
                    changed.insert(relative);
                }
            }
        }
        // `Any` is what several platform backends report when they cannot say
        // more. Treated as structural: claiming a file changed when the tree
        // did would leave a deleted file in the panel.
        EventKind::Any => batch.tree = true,
        EventKind::Access(_) | EventKind::Other => return false,
    }
    true
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

    /// Rules for a project that has no `.gitignore` on disk: only the names
    /// apply, which is what a scratch directory gets.
    fn bare(root: &str) -> Rules {
        Rules::load(Path::new(root))
    }

    fn event(kind: EventKind, path: &str) -> notify::Result<notify::Event> {
        Ok(notify::Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        })
    }

    fn modify(path: &str) -> notify::Result<notify::Event> {
        event(
            EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            path,
        )
    }

    fn create(path: &str) -> notify::Result<notify::Event> {
        event(EventKind::Create(notify::event::CreateKind::File), path)
    }

    #[test]
    fn build_output_is_not_watched() {
        let rules = bare("/project");
        assert!(rules.ignored(Path::new("/project/target/debug/app")));
        assert!(rules.ignored(Path::new("/project/target")));
        assert!(!rules.ignored(Path::new("/project/src/main.rs")));
    }

    /// The tree hides dot entries, so a change to one cannot be visible — and
    /// `.git` alone would fire on every command git runs.
    #[test]
    fn dot_directories_are_not_watched() {
        let rules = bare("/project");
        assert!(rules.ignored(Path::new("/project/.git/index")));
        assert!(rules.ignored(Path::new("/project/.rusty/sim.toml")));
        assert!(rules.ignored(Path::new("/project/.cargo/config.toml")));
        assert!(!rules.ignored(Path::new("/project/src/lib.rs")));
    }

    /// A backend reporting the root's parent during a rename must not produce
    /// a path that escapes the project.
    #[test]
    fn anything_outside_the_project_is_ignored() {
        let rules = bare("/project");
        assert!(rules.ignored(Path::new("/elsewhere/file.rs")));
        assert!(rules.ignored(Path::new("/")));
    }

    /// `target-dir = "build"` in `.cargo/config.toml` puts the build somewhere
    /// only the ignore file knows about. A watcher that skipped `target/` by
    /// name alone stormed on that project exactly as it would have on any
    /// other without the rule.
    #[test]
    fn what_the_gitignore_names_is_not_watched() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".gitignore"), "build/\n*.o\n").expect("write");
        let rules = Rules::load(dir.path());

        assert!(rules.ignored(&dir.path().join("build/debug/deps/x.rlib")));
        assert!(rules.ignored(&dir.path().join("src/x.o")));
        assert!(!rules.ignored(&dir.path().join("src/main.rs")));
        assert!(
            !rules.ignored(&dir.path().join("build.rs")),
            "a file, not the directory"
        );
    }

    /// The ignore file is itself a dot entry, so its change is never
    /// reported — but it changes what else is, so it is read again.
    #[test]
    fn a_changed_gitignore_is_read_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ignore_file = dir.path().join(".gitignore");
        std::fs::write(&ignore_file, "").expect("write");
        let mut rules = Rules::load(dir.path());
        let out = dir.path().join("build/x.rlib");
        assert!(!rules.ignored(&out), "nothing named yet");

        std::fs::write(&ignore_file, "build/\n").expect("write");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        let recorded = absorb(
            &mut rules,
            modify(&ignore_file.to_string_lossy()),
            &mut batch,
            &mut changed,
        );
        assert!(!recorded, "the ignore file's own change is not reported");
        assert!(rules.ignored(&out), "but the new rule applies from here on");
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
        let mut rules = bare("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        assert!(absorb(
            &mut rules,
            modify("/project/src/main.rs"),
            &mut batch,
            &mut changed,
        ));
        assert!(absorb(
            &mut rules,
            create("/project/src/new.rs"),
            &mut batch,
            &mut changed,
        ));
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
        let mut rules = bare("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        for _ in 0..4 {
            absorb(
                &mut rules,
                event(
                    EventKind::Modify(notify::event::ModifyKind::Data(
                        notify::event::DataChange::Any,
                    )),
                    "/project/src/main.rs",
                ),
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
        let mut rules = bare("/project");
        let mut batch = FileChanges::default();
        let mut changed = BTreeSet::new();
        for n in 0..1000 {
            let recorded = absorb(
                &mut rules,
                create(&format!("/project/target/debug/deps/{n}.o")),
                &mut batch,
                &mut changed,
            );
            assert!(!recorded, "an ignored event must not count as activity");
        }
        assert!(!batch.tree);
        assert!(changed.is_empty());
    }

    /// Drive `collect` the way the platform watcher does: raw events in, a
    /// batch out once the tree is quiet.
    fn collector(
        root: &Path,
    ) -> (
        mpsc::Sender<notify::Result<notify::Event>>,
        mpsc::Receiver<FileChanges>,
    ) {
        let rules = Rules::load(root);
        let (raw_tx, raw_rx) = mpsc::channel();
        let (out_tx, out_rx) = mpsc::channel();
        std::thread::spawn(move || collect(rules, raw_rx, out_tx));
        (raw_tx, out_rx)
    }

    /// Two edits inside one quiet window are one batch: this is the property
    /// the window exists for.
    #[test]
    fn edits_inside_the_quiet_window_arrive_as_one_batch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (raw, out) = collector(dir.path());
        let root = dir.path().to_path_buf();

        raw.send(modify(&root.join("src/a.rs").to_string_lossy()))
            .unwrap();
        std::thread::sleep(QUIET / 3);
        raw.send(modify(&root.join("src/b.rs").to_string_lossy()))
            .unwrap();

        let batch = out.recv_timeout(Duration::from_secs(5)).expect("a batch");
        assert_eq!(
            batch.changed,
            vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
        );
        assert!(!batch.tree);
        assert!(
            out.recv_timeout(QUIET * 2).is_err(),
            "nothing else was reported, so nothing else arrives",
        );
    }

    /// A build that starts as somebody saves writes into `target/` for a
    /// long time. Every one of those events used to extend the quiet window,
    /// so the save was reported when the build finished rather than when it
    /// happened. Ignored events must not hold the window open.
    #[test]
    fn ignored_events_do_not_hold_the_window_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (raw, out) = collector(dir.path());
        let root = dir.path().to_path_buf();

        let started = Instant::now();
        raw.send(modify(&root.join("src/main.rs").to_string_lossy()))
            .unwrap();
        // The build: an ignored event every 100 ms for 1.5 s. Under the old
        // rule the batch could not arrive before the storm ended.
        let storm = {
            let raw = raw.clone();
            let root = root.clone();
            std::thread::spawn(move || {
                for n in 0..15 {
                    let path = root.join(format!("target/debug/deps/{n}.o"));
                    if raw.send(create(&path.to_string_lossy())).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
        };

        let batch = out
            .recv_timeout(Duration::from_millis(1000))
            .expect("the save is reported while the build is still running");
        assert!(
            started.elapsed() < Duration::from_millis(1000),
            "took {:?}: the build held the window open",
            started.elapsed(),
        );
        assert_eq!(batch.changed, vec!["src/main.rs".to_string()]);
        assert!(!batch.tree, "the build itself is not reported");
        storm.join().unwrap();
    }
}
