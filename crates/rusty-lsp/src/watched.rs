//! What rust-analyzer is told about files changing on disk.
//!
//! **The client watches, so the server does not.** Left to watch for itself,
//! rust-analyzer holds a handle on the directories of the workspace, and on
//! Windows a directory with an open handle below it cannot be renamed or
//! moved: measured with `diag_probe`, `src` refused to be renamed — access
//! denied, every attempt — for as long as the server ran, while `src/math`,
//! which has no directory below it, renamed freely. The tree's rename and
//! move met exactly that, and so did moving an empty `.git` to the recycle
//! bin. A client that declares `didChangeWatchedFiles` with dynamic
//! registration is asked to watch instead (rust-analyzer's `files.watcher`,
//! `client` by default), and rusty already has a watcher: the one that keeps
//! the tree and the open files current. Its batches become the events here.
//!
//! Pure: which paths the server cares about, and what two listings and a
//! batch of changed files amount to. The listing is the tree the watcher
//! already refreshes on a structural change, so a directory renamed, moved
//! or deleted arrives as every file in it created or deleted — which a single
//! event for the directory would not be: rust-analyzer reads a path it is
//! told about as one file.

use std::collections::BTreeSet;

/// LSP's `FileChangeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    Created = 1,
    Changed = 2,
    Deleted = 3,
}

/// Whether rust-analyzer reads this file: Rust sources, the manifests and
/// lockfiles that shape the workspace, and its own configuration. What it
/// registers to watch is the same set, spelled as globs.
pub fn relevant(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.ends_with(".rs")
        || matches!(
            name,
            "Cargo.toml"
                | "Cargo.lock"
                | "rust-analyzer.toml"
                | "rust-toolchain.toml"
                | "rust-toolchain"
        )
}

/// The events for one batch: files in `after` and not in `before` were
/// created, the reverse deleted, and the contents of `changed` changed.
///
/// `before` and `after` are the relevant files of two listings, taken either
/// side of a structural change; with no structural change they are the same
/// set, and only `changed` says anything. A file both changed and created in
/// one batch is created — the server reads it whole either way.
pub fn file_events(
    before: &BTreeSet<String>,
    after: &BTreeSet<String>,
    changed: &[String],
) -> Vec<(String, FileChange)> {
    let mut events: Vec<(String, FileChange)> = after
        .difference(before)
        .map(|path| (path.clone(), FileChange::Created))
        .chain(
            before
                .difference(after)
                .map(|path| (path.clone(), FileChange::Deleted)),
        )
        .collect();
    for path in changed {
        if relevant(path) && after.contains(path) && before.contains(path) {
            events.push((path.clone(), FileChange::Changed));
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn the_server_hears_about_sources_manifests_and_its_own_config_only() {
        for path in [
            "src/main.rs",
            "firmware/Cargo.toml",
            "Cargo.lock",
            "rust-analyzer.toml",
            "rust-toolchain.toml",
        ] {
            assert!(relevant(path), "{path}");
        }
        for path in [
            "README.md",
            "src/data.json",
            "notes/Cargo.toml.bak",
            ".rs/x",
        ] {
            assert!(!relevant(path), "{path}");
        }
    }

    /// The case this exists for: `src/math` renamed to `src/geometry` is two
    /// files deleted and two created, not one directory moved.
    #[test]
    fn a_renamed_directory_is_every_file_in_it_deleted_and_created() {
        let before = set(&["src/lib.rs", "src/math/mod.rs", "src/math/vector.rs"]);
        let after = set(&[
            "src/lib.rs",
            "src/geometry/mod.rs",
            "src/geometry/vector.rs",
        ]);
        let events = file_events(&before, &after, &[]);
        assert_eq!(
            events,
            vec![
                ("src/geometry/mod.rs".to_string(), FileChange::Created),
                ("src/geometry/vector.rs".to_string(), FileChange::Created),
                ("src/math/mod.rs".to_string(), FileChange::Deleted),
                ("src/math/vector.rs".to_string(), FileChange::Deleted),
            ]
        );
    }

    /// Content changes travel only for files the server reads and that
    /// exist on both sides; a new file is announced once, as created.
    #[test]
    fn changed_contents_are_changes_and_nothing_is_said_twice() {
        let before = set(&["src/lib.rs", "Cargo.toml"]);
        let after = set(&["src/lib.rs", "Cargo.toml", "src/new.rs"]);
        let changed = vec![
            "src/lib.rs".to_string(),
            "README.md".to_string(),
            "src/new.rs".to_string(),
        ];
        assert_eq!(
            file_events(&before, &after, &changed),
            vec![
                ("src/new.rs".to_string(), FileChange::Created),
                ("src/lib.rs".to_string(), FileChange::Changed),
            ]
        );
    }
}
