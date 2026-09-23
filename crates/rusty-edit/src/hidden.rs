//! The one rule for what the workbench does not look at.
//!
//! Three walkers used to carry three versions of it: the file tree hid every
//! dot entry, search excluded only `.git`, and the watcher ignored every dot
//! component. So search surfaced `.cargo/config.toml` and `.rusty/sim.toml` —
//! files the tree would never show — and a replace could rewrite them, while
//! the module doc above it promised the opposite. One predicate, three
//! callers, and a test beside each caller that the answer agrees.
//!
//! The walk is here too: the tree, the module scan and search all start from
//! [`project_walk`], so the ignore rules cannot differ between them either.

use std::path::Path;

use ignore::WalkBuilder;

/// Whether a directory entry is one the workbench never shows, searches or
/// watches: anything dot-named.
///
/// `.git` alone is thousands of files nobody is text-searching; `.cargo` and
/// `.rusty` are edited through their own panels, not by hand. There was a
/// toggle to reveal them once and it earned its keep for nobody.
pub(crate) fn hidden_entry(name: &str) -> bool {
    name.starts_with('.')
}

/// The walk every lister of the project's files starts from: ripgrep's
/// walker over the project's own ignore files — nothing above the root,
/// nothing from the user's global git config — with [`hidden_entry`]
/// deciding the dot entries. Each caller adds only what is its own: a depth
/// for the tree and the module scan, include and exclude globs for search.
pub(crate) fn project_walk(root: &Path) -> WalkBuilder {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false) // our own filter below decides
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        // Without this, `.gitignore` is only honoured inside a git repository —
        // and a freshly generated project has a .gitignore and no .git, so
        // `target/` and its tens of thousands of files would land in the tree
        // the first time anyone built.
        .require_git(false)
        // Dot entries never show — the rule the watcher applies as well — so
        // no panel can name a file the tree cannot open. `.git` alone drowned
        // every search the moment a project had history.
        .filter_entry(|entry| {
            entry.depth() == 0 || !hidden_entry(&entry.file_name().to_string_lossy())
        });
    walk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_entries_are_hidden_and_nothing_else_is() {
        for hidden in [".git", ".cargo", ".rusty", ".gitignore", ".hidden.rs"] {
            assert!(hidden_entry(hidden), "{hidden}");
        }
        for shown in ["src", "Cargo.toml", "target", "a.b.c"] {
            assert!(!hidden_entry(shown), "{shown}");
        }
    }
}
