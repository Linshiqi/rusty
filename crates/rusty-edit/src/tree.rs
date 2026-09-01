//! The project's files.
//!
//! Walked with ripgrep's `ignore`, which reads `.gitignore` — so `target/`
//! disappears without this crate having to keep its own list of what counts as
//! build output. An embedded project's target directory holds tens of thousands
//! of files, and a tree that showed them would be unusable and slow in the same
//! breath.

use std::path::Path;

use ignore::WalkBuilder;

use crate::{error::Result, model::Entry};

/// How deep to walk.
///
/// Deep enough for any project layout anyone actually uses, shallow enough that
/// a stray symlink into a filesystem root cannot hang the window.
const MAX_DEPTH: usize = 12;

/// Everything in the project worth showing, as one tree.
///
/// Built whole rather than a level at a time. A source tree with `target/`
/// excluded is a few hundred entries — small enough that lazily expanding
/// directories would add a round trip per click and save nothing.
pub fn read(root: &Path) -> Result<Vec<Entry>> {
    let mut top = Vec::new();

    let walk = WalkBuilder::new(root)
        .max_depth(Some(MAX_DEPTH))
        .hidden(false) // our own filter below decides
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        // Without this, `.gitignore` is only honoured inside a git repository —
        // and a freshly generated project has a .gitignore and no .git, so
        // `target/` and its tens of thousands of files would land in the tree
        // the first time anyone built.
        .require_git(false)
        // Dot-entries never show. There was a toggle once; it earned its keep
        // for nobody, and the files it revealed — .cargo, .rusty — are edited
        // through their own panels, not by hand.
        .filter_entry(move |entry| {
            let name = entry.file_name().to_string_lossy();
            entry.depth() == 0 || !name.starts_with('.')
        })
        .build();

    for found in walk.flatten() {
        let path = found.path();
        if path == root {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        // Forward slashes on every platform: this string is an identity the
        // frontend sends back, and two spellings of one path would look like
        // two files.
        let relative = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if relative.is_empty() {
            continue;
        }

        let is_dir = found.file_type().is_some_and(|t| t.is_dir());
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| relative.clone());

        insert(
            &mut top,
            &relative,
            Entry {
                name,
                path: relative.clone(),
                is_dir,
                children: Vec::new(),
            },
        );
    }

    sort(&mut top);
    Ok(top)
}

/// Place an entry under its parent.
///
/// The walker yields parents before children, so the chain always exists by the
/// time it is needed.
fn insert(level: &mut Vec<Entry>, path: &str, entry: Entry) {
    match path.split_once('/') {
        None => level.push(entry),
        Some((head, rest)) => {
            if let Some(parent) = level.iter_mut().find(|e| e.name == head && e.is_dir) {
                insert(&mut parent.children, rest, entry);
            }
        }
    }
}

/// Directories first, then alphabetical — the order every file browser uses,
/// and the one people scan without thinking.
fn sort(level: &mut Vec<Entry>) {
    level.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for entry in level {
        sort(&mut entry.children);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();

        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::create_dir_all(root.join(".cargo")).unwrap();
        fs::create_dir_all(root.join("target/debug/deps")).unwrap();

        fs::write(root.join("Cargo.toml"), "[package]").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("src/bin/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join(".cargo/config.toml"), "[build]").unwrap();
        fs::write(root.join("target/debug/deps/junk.rlib"), "").unwrap();

        dir
    }

    fn flatten(entries: &[Entry], out: &mut Vec<String>) {
        for entry in entries {
            out.push(entry.path.clone());
            flatten(&entry.children, out);
        }
    }

    #[test]
    fn build_output_is_not_in_the_tree() {
        let dir = scratch();
        let mut paths = Vec::new();
        flatten(&read(dir.path()).unwrap(), &mut paths);

        assert!(
            !paths.iter().any(|p| p.starts_with("target")),
            "`.gitignore` names target/, and a tree with it in is unusable: {paths:?}",
        );
    }

    /// Dot-entries never show — .git's object store alone is thousands of
    /// files no tree should list, and the rest are edited through panels.
    #[test]
    fn dotted_entries_never_show() {
        let dir = scratch();
        std::fs::create_dir_all(dir.path().join(".git/info")).unwrap();
        std::fs::write(dir.path().join(".git/info/exclude"), "# git\n").unwrap();

        let mut paths = Vec::new();
        flatten(&read(dir.path()).unwrap(), &mut paths);
        assert!(
            !paths.iter().any(|p| p.starts_with('.')),
            "dot-entries must stay out of the tree: {paths:?}",
        );
        assert!(paths.contains(&"Cargo.toml".to_string()), "{paths:?}");
    }

    #[test]
    fn nesting_is_preserved_and_directories_come_first() {
        let dir = scratch();
        let tree = read(dir.path()).unwrap();

        let names: Vec<_> = tree.iter().map(|e| e.name.as_str()).collect();
        let first_file = names
            .iter()
            .position(|n| !n.starts_with('.') && n.contains('.'));
        let last_dir = tree.iter().rposition(|e| e.is_dir);
        assert!(
            last_dir < first_file.or(Some(usize::MAX)),
            "directories must sort before files: {names:?}",
        );

        let src = tree.iter().find(|e| e.name == "src").expect("src");
        let bin = src.children.iter().find(|e| e.name == "bin").expect("bin");
        assert_eq!(bin.children[0].path, "src/bin/main.rs");
    }
}
