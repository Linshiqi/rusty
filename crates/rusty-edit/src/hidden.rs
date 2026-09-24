//! The one rule for what the workbench does not look at.
//!
//! Three walkers used to carry three versions of it: the file tree hid every
//! dot entry, search excluded only `.git`, and the watcher ignored every dot
//! component. So search surfaced `.cargo/config.toml` and `.rusty/sim.toml` —
//! files the tree would never show — and a replace could rewrite them, while
//! the module doc above it promised the opposite. One predicate, three
//! callers, and a test beside each caller that the answer agrees.
//!
//! The walk is here too: the tree, the module scan, search and replace all
//! start from [`project_walk`], so neither the ignore rules nor how deep the
//! walk goes can differ between them.

use std::path::Path;

use ignore::WalkBuilder;

/// How deep any walk of the project goes.
///
/// Deep enough for any project layout anyone actually uses, shallow enough that
/// a stray symlink into a filesystem root cannot hang the window. It was the
/// tree's own once, and search and replace walked without it: they listed —
/// and rewrote — files further down than the tree had ever shown.
pub(crate) const MAX_DEPTH: usize = 12;

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
/// nothing from the user's global git config — no deeper than
/// [`MAX_DEPTH`], with [`hidden_entry`] deciding the dot entries. Each
/// caller adds only what is its own: include and exclude globs for search
/// and replace.
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
        .max_depth(Some(MAX_DEPTH))
        // Dot entries never show — the rule the watcher applies as well — so
        // no panel can name a file the tree cannot open. `.git` alone drowned
        // every search the moment a project had history.
        .filter_entry(|entry| {
            entry.depth() == 0 || !hidden_entry(&entry.file_name().to_string_lossy())
        });
    walk
}

/// `path` relative to `root` with every backslash made a forward slash: the
/// name search and the module scan give what the walk finds. `None` outside
/// the root; the root itself is the empty string.
pub(crate) fn relative_slashed(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
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

    /// The name search and the module scan give a file: forward slashes on
    /// every platform, and nothing for a path outside the root.
    #[test]
    fn a_walked_path_is_named_from_the_root_with_forward_slashes() {
        let root = Path::new("project");
        assert_eq!(
            relative_slashed(root, &root.join("src").join("main.rs")).as_deref(),
            Some("src/main.rs")
        );
        assert_eq!(relative_slashed(root, Path::new("elsewhere/main.rs")), None);
        assert_eq!(relative_slashed(root, root).as_deref(), Some(""));
    }
}
