//! Drafts that outlive the window — VS Code's hot exit.
//!
//! A file with changes not saved is backed up a moment after typing stops
//! and when the window closes, under the data directory's `backups/`, one
//! file per draft. Closing the window asks nothing: the drafts are kept, and
//! opening the project again puts them back on screen as they were, unsaved
//! and marked so. A save, or a draft closed without saving, removes its
//! backup. A crash costs at most the last moment's typing.
//!
//! One file per draft: a header of four lines — the project, the file, its
//! charset and its line endings, as `rusty_edit::text_file` names them —
//! and then the text exactly. Plain, so a person can find their work in it
//! whatever happens to rusty.
//!
//! **Every write and removal goes through one thread, in the order asked**
//! ([`Keeper`]). A backup written on a worker of its own could land after
//! the save that made it stale had removed it, and the next launch would
//! put back a draft somebody had already saved — older than what is on disk.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use rusty_edit::text_file::{Charset, LineEnding};

const HEADER: &str = "rusty backup 1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backup {
    pub path: String,
    pub text: String,
    pub charset: Charset,
    pub ending: LineEnding,
}

/// Where backups are kept: the data directory's `backups/`.
pub fn directory() -> Option<PathBuf> {
    Some(rusty_embed::config::data_dir()?.join("backups"))
}

enum Job {
    Write(PathBuf, Backup),
    Remove(PathBuf, String),
    Then(Box<dyn FnOnce(&Path) + Send>),
}

/// The one thread backups are written and removed on.
pub struct Keeper {
    jobs: mpsc::Sender<Job>,
}

impl Keeper {
    /// Keep backups in `dir`; a write that fails is told to `failed`, on
    /// the keeper's thread. With no directory nothing is kept.
    pub fn start(dir: Option<PathBuf>, failed: impl Fn(String) + Send + 'static) -> Keeper {
        let (jobs, queue) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("backups".into())
            .spawn(move || {
                for job in queue {
                    let Some(dir) = dir.as_deref() else {
                        if let Job::Then(then) = job {
                            then(Path::new(""));
                        }
                        continue;
                    };
                    match job {
                        Job::Write(root, backup) => {
                            if let Err(error) = write_in(dir, &root, &backup) {
                                failed(format!("{}: {error}", dir.display()));
                            }
                        }
                        Job::Remove(root, path) => {
                            let _ = std::fs::remove_file(dir.join(name(&root, &path)));
                        }
                        Job::Then(then) => then(dir),
                    }
                }
            })
            .expect("the backup thread starts");
        Keeper { jobs }
    }

    /// Keep `backup` as the draft of its file in the project at `root`.
    pub fn write(&self, root: &Path, backup: Backup) {
        let _ = self.jobs.send(Job::Write(root.to_path_buf(), backup));
    }

    /// The draft of `path` is saved, or thrown away: forget it.
    pub fn remove(&self, root: &Path, path: &str) {
        let _ = self
            .jobs
            .send(Job::Remove(root.to_path_buf(), path.to_string()));
    }

    /// Every draft kept for the project at `root`, read after everything
    /// asked before, and handed to `then` on the keeper's thread.
    pub fn read(&self, root: &Path, then: impl FnOnce(Vec<Backup>) + Send + 'static) {
        let root = root.to_path_buf();
        let _ = self.jobs.send(Job::Then(Box::new(move |dir| {
            then(if dir.as_os_str().is_empty() {
                Vec::new()
            } else {
                for_project_in(dir, &root)
            })
        })));
    }

    /// Wait until everything asked so far is on disk — for the window
    /// closing, after which nothing would be. False when it took too long.
    pub fn flush(&self, wait: Duration) -> bool {
        let (done, finished) = mpsc::channel();
        let _ = self.jobs.send(Job::Then(Box::new(move |_| {
            let _ = done.send(());
        })));
        finished.recv_timeout(wait).is_ok()
    }
}

fn name(root: &Path, path: &str) -> String {
    let mut hasher = DefaultHasher::new();
    spelled(root).hash(&mut hasher);
    path.hash(&mut hasher);
    format!("{:016x}.txt", hasher.finish())
}

/// A project's root as a backup names it: forward slashes, so the same
/// directory typed either way round is one project.
fn spelled(root: &Path) -> String {
    root.to_string_lossy().replace('\\', "/")
}

fn write_in(dir: &Path, root: &Path, backup: &Backup) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let body = format!(
        "{HEADER}\n{}\n{}\n{}\n{}\n{}",
        spelled(root),
        backup.path,
        backup.charset.label(),
        backup.ending.label(),
        backup.text
    );
    let file = dir.join(name(root, &backup.path));
    let temporary = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, body)?;
    std::fs::rename(&temporary, file)
}

fn for_project_in(dir: &Path, root: &Path) -> Vec<Backup> {
    let root = spelled(root);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Backup> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "txt"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|body| read(&body, &root))
        .collect();
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn read(body: &str, root: &str) -> Option<Backup> {
    let mut parts = body.splitn(6, '\n');
    if parts.next()? != HEADER || parts.next()? != root {
        return None;
    }
    let path = parts.next()?.to_string();
    let charset = Charset::from_label(parts.next()?)?;
    let ending = LineEnding::from_label(parts.next()?)?;
    let text = parts.next().unwrap_or_default().to_string();
    Some(Backup {
        path,
        text,
        charset,
        ending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(text: &str) -> Backup {
        Backup {
            path: "core/src/lib.rs".into(),
            text: text.into(),
            charset: Charset::Gbk,
            ending: LineEnding::Crlf,
        }
    }

    #[test]
    fn a_draft_comes_back_whole_for_its_own_project_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("E:/work/rustfly");
        let kept = draft("// 草稿\nfn a() {}\n\n");
        write_in(dir.path(), root, &kept).unwrap();
        assert_eq!(
            for_project_in(dir.path(), root),
            std::slice::from_ref(&kept)
        );
        assert!(for_project_in(dir.path(), Path::new("E:/work/other")).is_empty());
        // The same root spelled with backslashes is the same project.
        assert_eq!(
            for_project_in(dir.path(), Path::new(r"E:\work\rustfly")).len(),
            1
        );
        // Written again, it replaces itself rather than piling up.
        write_in(dir.path(), root, &kept).unwrap();
        assert_eq!(for_project_in(dir.path(), root).len(), 1);
    }

    /// The race the keeper exists for: a backup asked for, then the save
    /// that makes it stale. Whatever the threads do, the removal wins.
    #[test]
    fn a_removal_asked_after_a_write_always_lands_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("E:/work/rustfly");
        let keeper = Keeper::start(Some(dir.path().to_path_buf()), |reason| {
            panic!("a backup failed: {reason}")
        });
        for round in 0..50 {
            keeper.write(root, draft(&"x".repeat(round * 1000)));
            keeper.remove(root, "core/src/lib.rs");
        }
        let (found, read) = mpsc::channel();
        keeper.read(root, move |kept| found.send(kept).unwrap());
        assert_eq!(read.recv_timeout(Duration::from_secs(10)).unwrap(), []);
        keeper.write(root, draft("kept"));
        assert!(keeper.flush(Duration::from_secs(10)));
        assert_eq!(for_project_in(dir.path(), root), [draft("kept")]);
    }
}
