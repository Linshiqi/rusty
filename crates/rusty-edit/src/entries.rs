//! Moving, copying, renaming and deleting what the tree shows.
//!
//! The operations behind drag-and-drop and the tree's right-click menu —
//! the things VS Code's explorer does with a file. Every one takes paths
//! relative to the project root and refuses anything that leaves it, like
//! every other write in this crate, and every one answers with the relative
//! path it produced, because the window has tabs to move along with the
//! file.
//!
//! What is refused rather than guessed: a move or rename onto a name that
//! exists (an editor that silently replaces a file is one that eats work), a
//! directory moved into itself (a rename that can never finish), a name with
//! a separator in it (a rename that is really a move to somewhere unseen).
//! A copy pasted where its name is taken gets ` copy`, ` copy 2`… as VS Code
//! gives it, because "copy" is the one operation whose destination existing
//! is the ordinary case — pasting into the folder the original is in.
//!
//! Delete goes to the platform's recycle bin, never `remove_dir_all`: a tree
//! click is not a place to lose a directory for ever.

use std::path::{Path, PathBuf};

use crate::{
    document::resolve,
    error::{Error, Result},
};

/// Move `from` into the directory `into` (`""` for the root), keeping its
/// name. Answers with the new relative path.
pub fn move_entry(root: &Path, from: &str, into: &str) -> Result<String> {
    let name = base_name(from)?;
    let source = existing(root, from)?;
    let target_dir = resolve(root, into)?;
    if !into.is_empty() && !target_dir.is_dir() {
        return Err(Error::NotFound {
            path: into.to_string(),
        });
    }
    let destination = joined(into, name);
    if destination == from {
        return Ok(from.to_string());
    }
    refuse_inside_itself(from, into, source.is_dir())?;
    let target = target_dir.join(name);
    if target.exists() {
        return Err(Error::Exists {
            path: destination.clone(),
        });
    }
    std::fs::rename(&source, &target).map_err(|source| Error::Write {
        path: destination.clone(),
        source,
    })?;
    Ok(destination)
}

/// Give `from` a new name in the directory it is in. Answers with the new
/// relative path.
pub fn rename_entry(root: &Path, from: &str, new_name: &str) -> Result<String> {
    let new_name = new_name.trim();
    if name_problem(new_name).is_some() {
        return Err(Error::BadName {
            name: new_name.to_string(),
        });
    }
    let source = existing(root, from)?;
    let parent = parent_of(from);
    let destination = joined(parent, new_name);
    if destination == from {
        return Ok(from.to_string());
    }
    let target = resolve(root, &destination)?;
    // Case-only renames on a case-insensitive disk: `exists` says yes about
    // the file itself, and the rename is exactly what was asked for.
    if target.exists() && !same_file_case_insensitively(from, &destination) {
        return Err(Error::Exists { path: destination });
    }
    std::fs::rename(&source, &target).map_err(|source| Error::Write {
        path: destination.clone(),
        source,
    })?;
    Ok(destination)
}

/// Copy `from` into the directory `into`, taking a free name when its own
/// is taken. Answers with the relative path of the copy.
pub fn copy_entry(root: &Path, from: &str, into: &str) -> Result<String> {
    let name = base_name(from)?;
    let source = existing(root, from)?;
    let target_dir = resolve(root, into)?;
    if !into.is_empty() && !target_dir.is_dir() {
        return Err(Error::NotFound {
            path: into.to_string(),
        });
    }
    if source.is_dir() {
        refuse_inside_itself(from, into, true)?;
    }
    let free = free_name(&target_dir, name);
    let destination = joined(into, &free);
    let target = target_dir.join(&free);
    let failed = |source| Error::Write {
        path: destination.clone(),
        source,
    };
    if source.is_dir() {
        copy_dir(&source, &target).map_err(failed)?;
    } else {
        std::fs::copy(&source, &target).map_err(failed)?;
    }
    Ok(destination)
}

/// Move `path` to the platform's recycle bin.
///
/// On a thread of its own. On Windows the recycle bin is a COM call, and
/// `trash` initialises COM on the calling thread in apartment mode — which
/// fails with `RPC_E_CHANGED_MODE` on a thread something else in the process
/// already initialised the other way, and `trash` *panics* rather than
/// returning the error. The app runs this on a runtime's pooled worker, so
/// whether a Delete worked depended on which thread it landed on: measured
/// as "task panicked: Call to CoInitializeEx failed. HRESULT(0x80010106)".
/// A fresh thread has no such history, and a panic on it is an error here
/// rather than a dead task.
pub fn delete_entry(root: &Path, path: &str) -> Result<()> {
    let source = existing(root, path)?;
    if path.is_empty() {
        return Err(Error::Outside {
            path: path.to_string(),
        });
    }
    let failed =
        |why: String| Error::Io(format!("could not move {path} to the recycle bin: {why}"));
    std::thread::spawn(move || trash::delete(&source).map_err(|error| error.to_string()))
        .join()
        .map_err(|_| failed("the system call failed".to_string()))?
        .map_err(failed)
}

/// The absolute path of an entry, for handing to the platform's file
/// manager. Refuses what is not there, so "reveal" never opens a folder
/// with nothing selected.
pub fn absolute(root: &Path, path: &str) -> Result<PathBuf> {
    existing(root, path)
}

// ─── the rules, pure ─────────────────────────────────────────────────────────

/// Why a name typed for an entry is not one. A name is one segment: a
/// rename is a name, and a path is a move to somewhere the tree did not
/// show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProblem {
    /// Nothing but space.
    Empty,
    /// A `/` or a `\` in it.
    Separator,
    /// `.` or `..`, which name a directory that is already there.
    Dots,
    /// What this system will not take for a file's name: on Windows a
    /// character Explorer refuses (`<>:"|?*` and the control range), a
    /// name ending in a dot or a space, or a device name such as `CON` or
    /// `nul.txt`. Said before the write rather than after it, as the
    /// operating system's own error, which names none of this.
    Invalid,
}

/// What is wrong with `name` as the name of an entry on this system, or
/// `None` when it is one. Leading and trailing space is the caller's to
/// trim; a name that is only space is [`NameProblem::Empty`].
pub fn name_problem(name: &str) -> Option<NameProblem> {
    name_problem_on(name, cfg!(windows))
}

/// [`name_problem`] with the system named, so both are tested anywhere.
pub fn name_problem_on(name: &str, windows: bool) -> Option<NameProblem> {
    if name.trim().is_empty() {
        return Some(NameProblem::Empty);
    }
    if name.contains(['/', '\\']) {
        return Some(NameProblem::Separator);
    }
    if name == "." || name == ".." {
        return Some(NameProblem::Dots);
    }
    if windows {
        const DEVICES: [&str; 22] = [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        let refused =
            |c: char| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*');
        let stem = name.split('.').next().unwrap_or(name).trim_end();
        if name.chars().any(refused)
            || name.ends_with(['.', ' '])
            || DEVICES
                .iter()
                .any(|device| stem.eq_ignore_ascii_case(device))
        {
            return Some(NameProblem::Invalid);
        }
    }
    None
}

/// Where a drop of `carried` (a directory when `carried_dir`) would put it:
/// the folder under the pointer, a file's own folder — the row of a file
/// stands for the folder it is in — or the root (`""`) when `over` is the
/// empty space below the rows. `None` where nothing would move: the folder
/// it is already in, or a folder into itself or anywhere below itself. The
/// same refusals [`move_entry`] makes, said before the drop so that the
/// highlight never promises a move the library refuses.
pub fn drop_target(carried: &str, carried_dir: bool, over: Option<(&str, bool)>) -> Option<String> {
    let into = match over {
        None => String::new(),
        Some((path, true)) => path.to_string(),
        Some((path, false)) => parent_of(path).to_string(),
    };
    if parent_of(carried) == into {
        return None;
    }
    if carried_dir && (into == carried || into.starts_with(&format!("{carried}/"))) {
        return None;
    }
    Some(into)
}

/// Where `path` is once `from` has become `to`: `to` itself, or the same
/// place under `to` — and `None` for a path that is not `from` or below
/// it. Below by the separator, not the prefix: `src2/a.rs` is not under
/// `src`. What the window carries its tabs, documents and folders with.
pub fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    let rest = path.strip_prefix(from)?.strip_prefix('/')?;
    Some(joined(to, rest))
}

/// Whether `path` is `dir` or anything below it — by the separator, not
/// the prefix.
pub fn is_within(path: &str, dir: &str) -> bool {
    dir.is_empty() || moved_path(path, dir, dir).is_some()
}

/// The directory a relative path is in, `""` at the root.
pub fn parent_dir(path: &str) -> &str {
    parent_of(path)
}

/// The last segment of a relative path, which is what a move keeps.
fn base_name(path: &str) -> Result<&str> {
    match path.rsplit('/').next() {
        Some(name) if !name.is_empty() && !path.is_empty() => Ok(name),
        _ => Err(Error::BadName {
            name: path.to_string(),
        }),
    }
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn joined(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn same_file_case_insensitively(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// A directory cannot go into itself or below itself: the rename would be
/// asked to put a folder inside a folder that is being moved.
fn refuse_inside_itself(from: &str, into: &str, is_dir: bool) -> Result<()> {
    if is_dir && (into == from || into.starts_with(&format!("{from}/"))) {
        return Err(Error::IntoItself {
            path: from.to_string(),
        });
    }
    Ok(())
}

/// The name a copy takes in `dir`: its own when free, else ` copy`, then
/// ` copy 2`, ` copy 3`… before the extension, as VS Code numbers them.
fn free_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            format!("{stem} copy{ext}")
        } else {
            format!("{stem} copy {n}{ext}")
        };
        if !dir.join(&candidate).exists() {
            return candidate;
        }
        n += 1;
    }
}

/// `main.rs` → (`main`, `.rs`); `Makefile` → (`Makefile`, ``); `.env` → (`.env`, ``).
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) if at > 0 => name.split_at(at),
        _ => (name, ""),
    }
}

fn existing(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = resolve(root, relative)?;
    if !path.exists() {
        return Err(Error::NotFound {
            path: relative.to_string(),
        });
    }
    Ok(path)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/驱动")).unwrap();
        std::fs::create_dir_all(dir.path().join("firmware")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("src/驱动/mod.rs"), "// 驱动").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi").unwrap();
        dir
    }

    #[test]
    fn a_directory_moves_with_everything_in_it_and_answers_with_its_new_path() {
        let dir = project();
        let moved = move_entry(dir.path(), "src", "firmware").unwrap();
        assert_eq!(moved, "firmware/src");
        assert!(dir.path().join("firmware/src/驱动/mod.rs").is_file());
        assert!(!dir.path().join("src").exists());

        // And back out to the root, which is spelled as the empty directory.
        let back = move_entry(dir.path(), "firmware/src", "").unwrap();
        assert_eq!(back, "src");
        assert!(dir.path().join("src/main.rs").is_file());
    }

    #[test]
    fn a_move_onto_its_own_name_is_refused_rather_than_replacing() {
        let dir = project();
        std::fs::write(dir.path().join("firmware/README.md"), "theirs").unwrap();
        let refused = move_entry(dir.path(), "README.md", "firmware").unwrap_err();
        assert!(matches!(refused, Error::Exists { ref path } if path == "firmware/README.md"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("firmware/README.md")).unwrap(),
            "theirs",
            "nothing was overwritten"
        );
        assert!(dir.path().join("README.md").is_file(), "nothing was lost");
    }

    #[test]
    fn a_directory_cannot_move_into_itself_and_a_move_in_place_is_nothing() {
        let dir = project();
        assert!(matches!(
            move_entry(dir.path(), "src", "src/驱动").unwrap_err(),
            Error::IntoItself { .. }
        ));
        assert!(matches!(
            move_entry(dir.path(), "src", "src").unwrap_err(),
            Error::IntoItself { .. }
        ));
        // Already where it is: an answer, not an error, and no rename.
        assert_eq!(
            move_entry(dir.path(), "src/main.rs", "src").unwrap(),
            "src/main.rs"
        );
        assert!(matches!(
            move_entry(dir.path(), "../elsewhere", "src").unwrap_err(),
            Error::Outside { .. }
        ));
        assert!(matches!(
            move_entry(dir.path(), "missing.rs", "src").unwrap_err(),
            Error::NotFound { .. }
        ));
    }

    #[test]
    fn a_rename_keeps_the_directory_and_refuses_a_path_for_a_name() {
        let dir = project();
        assert_eq!(
            rename_entry(dir.path(), "src/main.rs", "lib.rs").unwrap(),
            "src/lib.rs"
        );
        assert!(dir.path().join("src/lib.rs").is_file());
        for bad in ["", "a/b", "..", "."] {
            assert!(
                matches!(
                    rename_entry(dir.path(), "src/lib.rs", bad),
                    Err(Error::BadName { .. })
                ),
                "{bad:?} is not a name",
            );
        }
        assert!(matches!(
            rename_entry(dir.path(), "src/lib.rs", "驱动").unwrap_err(),
            Error::Exists { .. }
        ));
        // The same name is nothing to do.
        assert_eq!(
            rename_entry(dir.path(), "src/lib.rs", "lib.rs").unwrap(),
            "src/lib.rs"
        );
    }

    #[test]
    fn a_copy_takes_a_free_name_where_its_own_is_taken() {
        let dir = project();
        // Into its own folder: ` copy`, then ` copy 2`, before the extension.
        assert_eq!(
            copy_entry(dir.path(), "src/main.rs", "src").unwrap(),
            "src/main copy.rs"
        );
        assert_eq!(
            copy_entry(dir.path(), "src/main.rs", "src").unwrap(),
            "src/main copy 2.rs"
        );
        // Elsewhere, its own name.
        assert_eq!(
            copy_entry(dir.path(), "src/main.rs", "firmware").unwrap(),
            "firmware/main.rs"
        );
        // A directory, recursively, and the original untouched.
        assert_eq!(
            copy_entry(dir.path(), "src", "firmware").unwrap(),
            "firmware/src"
        );
        assert!(dir.path().join("firmware/src/驱动/mod.rs").is_file());
        assert!(dir.path().join("src/驱动/mod.rs").is_file());
        assert!(matches!(
            copy_entry(dir.path(), "src", "src/驱动").unwrap_err(),
            Error::IntoItself { .. }
        ));
    }

    #[test]
    fn names_split_before_the_extension_and_a_dotfile_has_none() {
        assert_eq!(split_extension("main.rs"), ("main", ".rs"));
        assert_eq!(split_extension("archive.tar.gz"), ("archive.tar", ".gz"));
        assert_eq!(split_extension("Makefile"), ("Makefile", ""));
        assert_eq!(split_extension(".env"), (".env", ""));
    }

    #[test]
    fn a_name_is_one_segment_this_system_takes() {
        for (name, windows, problem) in [
            ("", false, Some(NameProblem::Empty)),
            ("   ", true, Some(NameProblem::Empty)),
            ("a/b.rs", false, Some(NameProblem::Separator)),
            (r"a\b.rs", false, Some(NameProblem::Separator)),
            ("..", false, Some(NameProblem::Dots)),
            (".", true, Some(NameProblem::Dots)),
            ("a:b.rs", true, Some(NameProblem::Invalid)),
            ("a:b.rs", false, None),
            ("what?.md", true, Some(NameProblem::Invalid)),
            ("trailing.", true, Some(NameProblem::Invalid)),
            ("nul.txt", true, Some(NameProblem::Invalid)),
            ("COM1", true, Some(NameProblem::Invalid)),
            ("console.rs", true, None),
            ("驱动.rs", true, None),
            (".env", true, None),
            ("main copy 2.rs", true, None),
        ] {
            assert_eq!(
                name_problem_on(name, windows),
                problem,
                "{name:?} on windows={windows}"
            );
        }
    }

    #[test]
    fn a_drop_lands_in_the_folder_under_the_pointer_or_a_files_folder_or_the_root() {
        let file = "src/main.rs";
        assert_eq!(
            drop_target(file, false, Some(("firmware", true))).as_deref(),
            Some("firmware")
        );
        assert_eq!(
            drop_target(file, false, Some(("firmware/Cargo.toml", false))).as_deref(),
            Some("firmware")
        );
        assert_eq!(drop_target(file, false, None).as_deref(), Some(""));
        // Already there: nothing to do, so nothing to offer.
        assert_eq!(drop_target(file, false, Some(("src", true))), None);
        assert_eq!(drop_target(file, false, Some(("src/lib.rs", false))), None);
        assert_eq!(drop_target("Cargo.toml", false, None), None);
    }

    #[test]
    fn a_folder_never_drops_into_itself_or_below() {
        assert_eq!(drop_target("src", true, Some(("src", true))), None);
        assert_eq!(drop_target("src", true, Some(("src/驱动", true))), None);
        assert_eq!(drop_target("src", true, Some(("src/main.rs", false))), None);
        assert_eq!(
            drop_target("src", true, Some(("firmware", true))).as_deref(),
            Some("firmware")
        );
        // `src2` is beside `src`, not inside it.
        assert_eq!(
            drop_target("src", true, Some(("src2", true))).as_deref(),
            Some("src2")
        );
    }

    #[test]
    fn a_path_follows_a_move_by_its_separator_not_its_prefix() {
        assert_eq!(
            moved_path("src", "src", "core/src").as_deref(),
            Some("core/src")
        );
        assert_eq!(
            moved_path("src/驱动/mod.rs", "src", "core/src").as_deref(),
            Some("core/src/驱动/mod.rs")
        );
        assert_eq!(moved_path("src2/a.rs", "src", "core/src"), None);
        assert_eq!(moved_path("README.md", "src", "lib"), None);
        // Out to the root.
        assert_eq!(
            moved_path("a/b/c.rs", "a/b", "b").as_deref(),
            Some("b/c.rs")
        );
        assert!(is_within("src/main.rs", "src"));
        assert!(!is_within("src2/main.rs", "src"));
        assert!(is_within("anything", ""));
        assert_eq!(parent_dir("src/main.rs"), "src");
        assert_eq!(parent_dir("main.rs"), "");
    }

    #[test]
    fn the_root_itself_is_never_deleted_or_moved() {
        let dir = project();
        assert!(matches!(
            delete_entry(dir.path(), "").unwrap_err(),
            Error::Outside { .. }
        ));
        assert!(matches!(
            move_entry(dir.path(), "", "src").unwrap_err(),
            Error::BadName { .. }
        ));
    }
}
