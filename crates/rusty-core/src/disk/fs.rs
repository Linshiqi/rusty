//! The filesystem, as the scan reads it and a removal changes it: the space
//! on a volume, how much a directory holds and when it was last written, a
//! directory's entries in a stable order, whether a build holds a tree's
//! lock, and removing a path.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::error::{Error, Result};
use crate::model::Volume;

/// Free and total space on the volume holding `path`.
pub fn volume_of(path: &Path) -> Option<Volume> {
    let free_bytes = fs4::available_space(path).ok()?;
    let total_bytes = fs4::total_space(path).ok()?;
    Some(Volume {
        free_bytes,
        total_bytes,
    })
}

/// Whether a build holds this tree's lock right now. Cargo takes
/// `<tree>/.cargo-lock` for the whole of a build, and so does rust-analyzer's
/// check; nothing in a locked tree is removed.
pub(super) fn tree_locked(tree: &Path) -> bool {
    use fs4::fs_std::FileExt;
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(tree.join(".cargo-lock"))
    else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(true) => {
            let _ = FileExt::unlock(&file);
            false
        }
        Ok(false) | Err(_) => true,
    }
}

/// Bytes and files under `path`, symlinks not followed.
pub(super) fn measure(path: &Path) -> (u64, u64) {
    let mut bytes = 0;
    let mut files = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                files += 1;
            }
        }
    }
    (bytes, files)
}

/// The newest modification time of any file under `dir`, or the directory's
/// own when it holds none.
pub(super) fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if let Ok(modified) = metadata.modified() {
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
    }
    newest.or_else(|| fs::metadata(dir).and_then(|m| m.modified()).ok())
}

pub(super) fn sorted_entries(dir: &Path) -> Vec<fs::DirEntry> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(dir)
        .map(|iter| iter.flatten().collect())
        .unwrap_or_default();
    entries.sort_by_key(fs::DirEntry::file_name);
    entries
}

pub(super) fn remove_path(path: &Path) -> Result<()> {
    let io = |source| Error::Io {
        what: "remove".into(),
        path: path.display().to_string(),
        source,
    };
    let metadata = fs::symlink_metadata(path).map_err(io)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(io)
    } else {
        fs::remove_file(path).map_err(io)
    }
}

/// Whether two paths name the same directory, by canonical form when both
/// exist and by text otherwise.
pub(super) fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}
