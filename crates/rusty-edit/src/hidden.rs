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

/// Whether a directory is a cache by the cache-directory standard: it holds
/// a `CACHEDIR.TAG`, which cargo writes into every build directory it makes —
/// `target/`, or wherever `build.target-dir` put it.
///
/// Asked because the ignore files cannot be trusted to: a project with no
/// `.gitignore` of its own — an example inside a repository whose ignore file
/// sits above it, where the walk does not look, or one nobody wrote one for —
/// had its `target/` listed whole. The assistant's `list_files` handed a model
/// seven hundred build artifacts for a project of three source files, about
/// seven thousand tokens a call, and cut the real files off the list.
pub(crate) fn cache_directory(dir: &Path) -> bool {
    dir.join("CACHEDIR.TAG").is_file()
}

/// The walk every lister of the project's files starts from: ripgrep's
/// walker over the project's own ignore files — nothing above the root,
/// nothing from the user's global git config — no deeper than
/// [`MAX_DEPTH`], with [`hidden_entry`] deciding the dot entries and
/// [`cache_directory`] the build directories. Each caller adds only what is
/// its own: include and exclude globs for search and replace.
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
            if entry.depth() == 0 {
                return true;
            }
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            !hidden_entry(&entry.file_name().to_string_lossy())
                && !(is_dir && cache_directory(entry.path()))
        });
    walk
}

/// `path` relative to `root`, its components joined with `/`: the name the
/// tree, search and the module scan all give what the walk finds. `None`
/// outside the root; the root itself is the empty string.
///
/// Joined by component, not by turning every `\` into `/`. On Windows the
/// two agree, but on Linux and macOS a backslash is an ordinary character
/// in a file name: `a\b.rs` was reported as `a/b.rs`, a file that does not
/// exist, and a replace did not know it for the draft the editor held.
pub(crate) fn relative_slashed(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    Some(
        relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
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

    /// A build directory stays out of every listing with no ignore file to
    /// say so — the project nobody wrote a `.gitignore` for, or one inside a
    /// repository whose ignore file sits above it — while a directory that
    /// only shares the name is source like any other.
    #[test]
    fn a_build_directory_is_left_out_whatever_the_ignore_files_say() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "").unwrap();
        std::fs::write(root.join("src/target/mod.rs"), "").unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(
            root.join("target/CACHEDIR.TAG"),
            "Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        std::fs::write(root.join("target/debug/app.exe"), "").unwrap();
        std::fs::create_dir_all(root.join("build")).unwrap();
        std::fs::write(root.join("build/CACHEDIR.TAG"), "").unwrap();
        std::fs::write(root.join("build/out.rlib"), "").unwrap();

        let found: Vec<String> = project_walk(root)
            .build()
            .flatten()
            .filter_map(|entry| relative_slashed(root, entry.path()))
            .filter(|path| !path.is_empty())
            .collect();
        assert!(
            found
                .iter()
                .all(|p| !p.starts_with("target") && !p.starts_with("build")),
            "cargo's build directories, by their tag: {found:?}"
        );
        assert!(found.contains(&"src/target/mod.rs".to_string()));
        assert!(found.contains(&"src/main.rs".to_string()));
    }

    /// The name the tree, search and the module scan give a file: forward
    /// slashes on every platform, and nothing for a path outside the root.
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

    /// A backslash separates on Windows and is part of the name everywhere
    /// else, and the name given is the components the path really has.
    /// Turning every `\` into `/` named a Linux file `a\b.rs` as `a/b.rs`.
    #[test]
    fn a_walked_path_is_named_by_its_own_components() {
        let root = Path::new("project");
        let path = root.join("src").join(r"a\b.rs");
        let named = if cfg!(windows) {
            "src/a/b.rs"
        } else {
            r"src/a\b.rs"
        };
        assert_eq!(relative_slashed(root, &path).as_deref(), Some(named));
    }
}
