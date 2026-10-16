//! A Rust file no `mod` declares, drawn dimmed: its name in the tree and on
//! its tab, and its text in every view of it.
//!
//! Such a file is in no crate, so rust-analyzer parses it and answers
//! nothing in it — no completion, no hover, no jump — and says why only as
//! `unlinked-file` on a file somebody has opened. The tree has to say so
//! before anybody opens it, so the window reads the `mod` lines itself
//! (`rusty_edit::modules`), off this thread, each time the project's tree is
//! read and after every `.rs` file is saved or changes on disk, since a
//! `mod` line added or taken away changes the answer.
//!
//! Two sources for one fact, and one of them wins
//! (`rusty_edit::modules::dimmed`): the scan wherever it has an opinion,
//! the server's `unlinked-file` only where it has none. The grey is the
//! whole message; the reason is one sentence on the pointer.

use std::{collections::BTreeSet, path::PathBuf, rc::Rc, time::Duration};

use rusty_edit::modules::Reading;

use super::App;
use crate::work;

/// rust-analyzer's code for a file in no crate.
const UNLINKED_FILE: &str = "unlinked-file";

#[derive(Default)]
pub(super) struct Unlinked {
    /// What the last scan of the open project said; `None` before one has
    /// answered, and for a project that is not Cargo's.
    reading: Option<Reading>,
    /// The files rust-analyzer last called `unlinked-file`.
    server: BTreeSet<String>,
    /// The files drawn dimmed now.
    dimmed: BTreeSet<String>,
    /// One scan at a time, and at most one more asked for after it: a burst
    /// of saves is two scans, not ten.
    running: bool,
    again: bool,
    /// Which project the scans are for: one answering for a project since
    /// replaced is dropped.
    generation: u64,
    /// How many scans have answered, and how long the last one took.
    scans: u32,
    took: Duration,
}

impl Unlinked {
    /// For the next project: what this one's scan and server said goes,
    /// and a scan of it still running is dropped when it answers.
    pub(super) fn next(&self) -> Unlinked {
        Unlinked {
            generation: self.generation + 1,
            ..Unlinked::default()
        }
    }

    /// Whether `path` is drawn dimmed.
    pub(super) fn is_dimmed(&self, path: &str) -> bool {
        self.dimmed.contains(path)
    }
}

impl App {
    /// The open project read for files no `mod` declares, off this thread.
    /// A project with no `Cargo.toml` at its root is not Cargo's — the file
    /// at the root decides the build system — and is not read: there it is
    /// the server's word alone, and on such a project it says nothing.
    pub(super) fn scan_unlinked(self: &Rc<Self>) {
        let Some(root) = self.root() else { return };
        let generation = {
            let mut state = self.state.borrow_mut();
            let scan = &mut state.unlinked;
            if scan.running {
                scan.again = true;
                return;
            }
            scan.running = true;
            scan.generation
        };
        work::spawn(
            move || {
                let began = std::time::Instant::now();
                let reading = root
                    .join("Cargo.toml")
                    .is_file()
                    .then(|| rusty_edit::modules::read(&root));
                (root, reading, began.elapsed())
            },
            move |app, (root, reading, took)| app.unlinked_read(generation, root, reading, took),
        );
    }

    fn unlinked_read(
        self: &Rc<Self>,
        generation: u64,
        root: PathBuf,
        reading: Option<Reading>,
        took: Duration,
    ) {
        let again = {
            let mut state = self.state.borrow_mut();
            // Another project's: its scan, if any, is its own.
            if state.unlinked.generation != generation {
                return;
            }
            let current = state.project.as_ref().is_some_and(|p| p.root == root);
            let scan = &mut state.unlinked;
            scan.running = false;
            if current {
                scan.reading = reading;
                scan.scans += 1;
                scan.took = took;
            }
            std::mem::take(&mut scan.again)
        };
        self.sync_unlinked();
        if again {
            self.scan_unlinked();
        }
    }

    /// What a server said about `path` changed: whether it now calls the
    /// file `unlinked-file`, which matters only where the scan is silent.
    pub(super) fn unlinked_heard(self: &Rc<Self>, path: &str) {
        let says = self.problems.borrow().get(path).is_some_and(|items| {
            items
                .iter()
                .any(|item| item.code.as_deref() == Some(UNLINKED_FILE))
        });
        let changed = {
            let mut state = self.state.borrow_mut();
            let server = &mut state.unlinked.server;
            if says {
                server.insert(path.to_string())
            } else {
                server.remove(path)
            }
        };
        if changed {
            self.sync_unlinked();
        }
    }

    /// The servers went: nothing they said still counts.
    pub(super) fn unlinked_unheard(self: &Rc<Self>) {
        let had = !std::mem::take(&mut self.state.borrow_mut().unlinked.server).is_empty();
        if had {
            self.sync_unlinked();
        }
    }

    /// The files to dim worked out again, and drawn where they changed: the
    /// tree's rows, the tabs, and every view showing one.
    fn sync_unlinked(self: &Rc<Self>) {
        let dimmed = {
            let mut state = self.state.borrow_mut();
            let scan = &mut state.unlinked;
            let dimmed = rusty_edit::modules::dimmed(
                scan.reading.as_ref(),
                scan.server.iter().map(String::as_str),
            );
            if dimmed == scan.dimmed {
                return;
            }
            scan.dimmed = dimmed.clone();
            dimmed
        };
        self.editor.set_dimmed(dimmed);
        self.sync_tree();
        self.sync_tabs();
    }

    /// For the probe: the files dimmed, the files rust-analyzer calls
    /// `unlinked-file`, how many scans have answered and what the last one
    /// cost.
    pub fn unlinked_now(&self) -> (Vec<String>, Vec<String>, u32, Duration) {
        let state = self.state.borrow();
        let scan = &state.unlinked;
        (
            scan.dimmed.iter().cloned().collect(),
            scan.server.iter().cloned().collect(),
            scan.scans,
            scan.took,
        )
    }
}

/// Whether a batch of paths written or changed holds a Rust file, whose
/// `mod` lines may have changed what is declared.
pub(super) fn touches_rust(paths: &[String]) -> bool {
    paths.iter().any(|path| path.ends_with(".rs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_rust_file_is_a_reason_to_read_again() {
        assert!(touches_rust(&["Cargo.toml".into(), "src/main.rs".into()]));
        assert!(!touches_rust(&[
            "README.md".into(),
            "src/main.rs.bak".into()
        ]));
    }
}
