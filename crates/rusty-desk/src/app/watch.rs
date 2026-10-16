//! Following the disk. The window is never the only thing writing to a
//! checkout — a `git checkout`, `cargo add`, another editor's save — and
//! `rusty_edit::watch` says so in batches: a file's contents, or the tree.
//!
//! What a batch does: the tree is read again when something appeared or
//! went; an open file nobody has edited takes the disk's text; an edited one
//! keeps its draft, which is somebody's work; the language servers are told
//! what moved, because they asked the client to watch rather than holding
//! handles on the project's directories (`rusty_lsp::watched`); and the Git
//! panel reads its status again.

use std::{collections::BTreeSet, path::Path, rc::Rc};

use rusty_edit::{Entry, FileChanges};

use super::App;
use crate::work;

/// The watcher running for the open project, and which it is: replacing it
/// drops it, which ends its thread.
#[derive(Default)]
pub(super) struct Watching {
    watch: Option<rusty_edit::Watch>,
    generation: u64,
}

impl App {
    /// Watch the open project, replacing any watcher there was. A watcher
    /// that cannot start costs nothing but the following: the window works
    /// without one, and a banner about an inotify budget helps nobody.
    pub(super) fn watch_project(self: &Rc<Self>) {
        let generation = {
            let mut state = self.state.borrow_mut();
            state.watching.watch = None;
            state.watching.generation += 1;
            state.watching.generation
        };
        let Some(root) = self.root() else { return };
        work::spawn(
            move || {
                let (watch, batches) = rusty_edit::watch(&root).ok()?;
                std::thread::spawn(move || {
                    let mut known = relevant_files(&root);
                    while let Ok(batch) = batches.recv() {
                        let events = if batch.tree {
                            let after = relevant_files(&root);
                            let events =
                                rusty_lsp::watched::file_events(&known, &after, &batch.changed);
                            known = after;
                            events
                        } else {
                            rusty_lsp::watched::file_events(&known, &known, &batch.changed)
                        };
                        work::deliver((batch, events), move |app, (batch, events)| {
                            app.disk_moved(generation, batch, events)
                        });
                    }
                });
                Some(watch)
            },
            move |app, watch| {
                let mut state = app.state.borrow_mut();
                if state.watching.generation == generation {
                    state.watching.watch = watch;
                }
            },
        );
    }

    fn disk_moved(
        self: &Rc<Self>,
        generation: u64,
        batch: FileChanges,
        events: Vec<(String, rusty_lsp::watched::FileChange)>,
    ) {
        if self.state.borrow().watching.generation != generation {
            return;
        }
        if !events.is_empty() {
            self.servers.watched(events);
        }
        // A manifest that changed may have moved a workspace's root.
        if batch
            .changed
            .iter()
            .any(|path| path.ends_with("Cargo.toml"))
        {
            self.forget_cargo_bases();
        }
        if batch.tree {
            self.reread_project();
        } else if super::unlinked::touches_rust(&batch.changed) {
            // A `mod` line may have come or gone; the tree's read does the
            // same when it is read again.
            self.scan_unlinked();
        }
        self.follow_changed(batch.changed);
        if self.state.borrow().git.loaded {
            self.git_status();
        }
    }

    /// Open files the disk moved under, nobody having edited them, take
    /// what the disk has now.
    pub(super) fn follow_changed(self: &Rc<Self>, changed: Vec<String>) {
        self.pictures_changed(&changed);
        // A failed test's mark in a file nobody has open is about the text
        // the disk no longer holds; an open one's go with its edits.
        self.editor.tests_disk_changed(&changed);
        let Some(root) = self.root() else { return };
        let open: Vec<(String, String)> = changed
            .into_iter()
            .filter(|path| self.editor.holds(path) && !self.editor.is_dirty(path))
            .filter_map(|path| self.editor.draft_of(&path).map(|(text, ..)| (path, text)))
            .collect();
        if open.is_empty() {
            return;
        }
        work::spawn(
            move || {
                open.into_iter()
                    .filter_map(|(path, text)| {
                        match rusty_edit::text_file::read_text(&root, &path) {
                            Ok(rusty_edit::text_file::TextFile::Text { text: now, .. })
                                if now != text =>
                            {
                                Some((path, now))
                            }
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            },
            |app, moved| {
                for (path, text) in moved {
                    // Typed into while the disk was read: the draft wins.
                    if !app.editor.is_dirty(&path) {
                        app.editor.reload(&path, text);
                    }
                }
            },
        );
    }
}

/// Every file under the project a language server reads, as the tree lists
/// it — the same walk and rules, so what the server is told exists and what
/// the Files panel shows cannot disagree.
fn relevant_files(root: &Path) -> BTreeSet<String> {
    fn gather(entries: &[Entry], out: &mut BTreeSet<String>) {
        for entry in entries {
            if entry.is_dir {
                gather(&entry.children, out);
            } else if rusty_lsp::watched::relevant(&entry.path) {
                out.insert(entry.path.clone());
            }
        }
    }
    let mut out = BTreeSet::new();
    if let Ok(tree) = rusty_edit::read_tree(root) {
        gather(&tree, &mut out);
    }
    out
}
