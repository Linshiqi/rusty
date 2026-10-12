//! Definitions in layers — what rusty ships, then the user's, then the
//! project's — where a later definition replaces an earlier one with the same
//! id. The chip catalogue, the parts library and the symbol library all load
//! this way (`docs/extensibility.md`), and so did each with its own copy of
//! these two steps.

use std::path::{Path, PathBuf};

/// Every file in `dir` with this extension, in name order, so a directory of
/// definitions layers the same way on every machine rather than however the
/// filesystem listed it. A directory that cannot be read holds none.
pub(crate) fn files_in(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    files.sort();
    files
}

/// `item` in place of the first one `same` says it redefines, or after them
/// all when it redefines nothing.
pub(crate) fn replace_or_push<T>(list: &mut Vec<T>, item: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|held| same(held, &item)) {
        Some(held) => *held = item,
        None => list.push(item),
    }
}
