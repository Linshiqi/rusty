//! The other rusty windows open on this machine.
//!
//! New Window starts another instance of the app — another process, with a
//! project, a language server and sessions of its own — so an instance is no
//! longer the only one of itself, and one thing it does reaches the others:
//! installing an update. On Windows the installer ends every process running
//! the binary it replaces, so a restart that asked only about this window's
//! own work would close another window's unsaved edits, and cut its build or
//! its install off halfway, without a word. The restart's question counts
//! them (`update::update_closes`).
//!
//! Each instance holds an exclusive lock on a file named for its process for
//! as long as it runs. The OS lets go of a lock when its process ends,
//! however it ends — a crash included — so a file nobody holds is a window
//! that has gone, and the next count removes it. Nothing has to tidy up on
//! the way out, which is the moment least certain to happen.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// This instance, counted by the others: the lock on its file is the
/// registration, held for as long as the value lives.
pub struct Instance {
    _held: File,
    path: PathBuf,
}

impl Instance {
    /// Register this process in `dir`.
    pub fn register(dir: &Path) -> io::Result<Instance> {
        Instance::register_as(dir, &std::process::id().to_string())
    }

    /// The file is locked under a name the count never reads and only then
    /// renamed into place: an instance counting in that moment would
    /// otherwise find it unheld, take it for a window that had gone, and
    /// delete it before this one had locked it.
    fn register_as(dir: &Path, name: &str) -> io::Result<Instance> {
        fs::create_dir_all(dir)?;
        let pending = dir.join(format!("{name}.pending"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&pending)?;
        file.try_lock().map_err(io::Error::from)?;
        let path = dir.join(format!("{name}.lock"));
        fs::rename(&pending, &path)?;
        Ok(Instance { _held: file, path })
    }

    /// How many other instances are open: every file beside this one's that
    /// somebody holds. One nobody holds is removed on the way, and so is a
    /// registration a crash left half made.
    pub fn others(&self) -> usize {
        let Some(dir) = self.path.parent() else {
            return 0;
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| *path != self.path)
            .filter(|path| match path.extension().and_then(|e| e.to_str()) {
                Some("lock") => held(path),
                // Looked at only to tidy: a registration still being made
                // is a window a moment from counting, not one yet.
                Some("pending") => {
                    let _ = held(path);
                    false
                }
                _ => false,
            })
            .count()
    }
}

/// Whether somebody holds `path`. When nobody does, the file was left by an
/// instance that has gone, and it goes too.
fn held(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().write(true).open(path) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = fs::remove_file(path);
            false
        }
        Err(TryLockError::WouldBlock) => true,
        // Neither answer: counted as no window, since a restart question
        // about a window that is not there is the worse of the two.
        Err(TryLockError::Error(_)) => false,
    }
}

/// The registration the app keeps: `None` when this machine had nowhere to
/// put it, and then no other window is ever counted — a question about
/// windows that may not exist would be asked of every restart.
pub struct Registered(pub Option<Instance>);

impl Registered {
    pub fn others(&self) -> usize {
        self.0.as_ref().map_or(0, Instance::others)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two registrations in one process are two owners to the OS — a lock
    /// belongs to its handle — which is what lets one test stand in for two
    /// windows.
    #[test]
    fn each_window_counts_the_others_and_not_itself() {
        let dir = tempfile::tempdir().unwrap();
        let first = Instance::register_as(dir.path(), "first").unwrap();
        assert_eq!(first.others(), 0);

        let second = Instance::register_as(dir.path(), "second").unwrap();
        assert_eq!(first.others(), 1);
        assert_eq!(second.others(), 1);

        drop(second);
        assert_eq!(first.others(), 0, "a window that closed is not counted");
        assert!(
            !dir.path().join("second.lock").exists(),
            "and its file went with it"
        );
    }

    /// A crash leaves the file and not the lock: nothing counted, and
    /// nothing left behind — a half-made registration included.
    #[test]
    fn a_window_that_crashed_is_not_counted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("gone.lock"), "").unwrap();
        fs::write(dir.path().join("half.pending"), "").unwrap();
        let this = Instance::register_as(dir.path(), "this").unwrap();
        assert_eq!(this.others(), 0);
        assert!(!dir.path().join("gone.lock").exists());
        assert!(!dir.path().join("half.pending").exists());
    }

    /// With nowhere to register, the question is never asked.
    #[test]
    fn no_registration_counts_no_one() {
        assert_eq!(Registered(None).others(), 0);
    }
}
