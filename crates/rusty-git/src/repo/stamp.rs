//! The repository's fingerprint, a [`GitStamp`]: the sizes and times of
//! the files git keeps its state in, and where those files are.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::model::GitStamp;

use super::run::run;
use super::{Error, Result};

/// Where a repository keeps its state: its own git directory (HEAD, the
/// index, an operation's markers) and the common one shared by every
/// worktree (the refs, the stash). The same directory in an ordinary
/// checkout; two in a linked worktree.
#[derive(Clone, Debug)]
pub(super) struct Dirs {
    pub(super) git: PathBuf,
    common: PathBuf,
}

/// Found once per root and kept: the stamp is asked for every few seconds,
/// and a `rev-parse` each time would be the process it exists to avoid.
static DIRS: Mutex<Vec<(PathBuf, Dirs)>> = Mutex::new(Vec::new());

pub(super) fn dirs(root: &Path) -> Result<Dirs> {
    let cached = DIRS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .find(|(known, _)| known == root)
        .map(|(_, dirs)| dirs.clone());
    if let Some(dirs) = cached {
        return Ok(dirs);
    }
    // `--path-format` is git 2.31; an older git echoes the unknown option
    // back as a line and prints the directories relative, so lines that
    // look like options are skipped and relative ones joined to the root.
    let text = run(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )?;
    let mut found = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .map(|line| {
            let path = PathBuf::from(line);
            if path.is_absolute() {
                path
            } else {
                root.join(path)
            }
        });
    let git = found.next().ok_or_else(|| Error::NotARepository {
        path: root.display().to_string(),
    })?;
    let common = found.next().unwrap_or_else(|| git.clone());
    let dirs = Dirs { git, common };
    let mut cache = DIRS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= 16 {
        cache.remove(0);
    }
    cache.push((root.to_path_buf(), dirs.clone()));
    Ok(dirs)
}

/// The repository's fingerprint — see [`GitStamp`]. Reads metadata only,
/// once [`dirs`] has found the git directory: the first ask for a root runs
/// its one `rev-parse`.
pub fn stamp(root: &Path) -> Result<GitStamp> {
    let dirs = dirs(root)?;
    let mut head = Fnv::new();
    head.file(&dirs.git.join("HEAD"));
    head.file(&dirs.git.join("logs").join("HEAD"));
    for marker in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        head.file(&dirs.git.join(marker));
    }
    let mut refs = Fnv::new();
    refs.file(&dirs.common.join("packed-refs"));
    let refs_dir = dirs.common.join("refs");
    refs.tree(&refs_dir, &refs_dir.join("stash"));
    let mut index = Fnv::new();
    index.file(&dirs.git.join("index"));
    let mut stash = Fnv::new();
    stash.file(&dirs.common.join("refs").join("stash"));
    stash.file(&dirs.common.join("logs").join("refs").join("stash"));
    let mut config = Fnv::new();
    config.file(&dirs.common.join("config"));
    Ok(GitStamp {
        head: head.finish(),
        refs: refs.finish(),
        index: index.finish(),
        stash: stash.finish(),
        config: config.finish(),
    })
}

/// The largest integer a JavaScript number holds exactly, which is as wide
/// as a stamp can be and still cross the wire — see [`GitStamp`].
pub(super) const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// FNV-1a over names, sizes and modification times.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    /// The hash folded to 53 bits: the top eleven are mixed into the rest
    /// rather than dropped.
    fn finish(&self) -> u64 {
        (self.0 ^ (self.0 >> 53)) & MAX_SAFE_INTEGER
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    /// One file or directory: its name, then its size and time — or that it
    /// is absent, which is a state too (a merge finishing removes a marker).
    fn file(&mut self, path: &Path) {
        self.bytes(path.to_string_lossy().as_bytes());
        match std::fs::metadata(path) {
            Ok(meta) => {
                self.bytes(&meta.len().to_le_bytes());
                let nanos = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_nanos());
                self.bytes(&nanos.to_le_bytes());
            }
            Err(_) => self.bytes(b"absent"),
        }
    }

    /// Every file under `dir`, in name order so the same tree hashes the
    /// same, less `skip`. Names are hashed too, so a ref created or deleted
    /// moves the stamp even where no time does.
    fn tree(&mut self, dir: &Path, skip: &Path) {
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = read.flatten().map(|entry| entry.path()).collect();
        entries.sort();
        for path in entries {
            if path == skip {
                continue;
            }
            if path.is_dir() {
                self.tree(&path, skip);
            } else {
                self.file(&path);
            }
        }
    }
}
