//! Reading and writing one file.

use std::path::{Component, Path, PathBuf};

use syntect::parsing::SyntaxSet;

use crate::{
    error::{Error, Result},
    highlight,
    model::Document,
};

/// Files bigger than this are refused rather than shown.
///
/// A linked ELF or a Wi-Fi blob sits in the same tree as the source, and
/// sending one to the frontend would take the window down. The tree lists them;
/// opening one says why it will not.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Holds the grammars.
///
/// `SyntaxSet::load_defaults_newlines` parses a bundled binary dump and takes
/// long enough that doing it per file is noticeable — so it is built once and
/// kept.
pub struct Files {
    syntaxes: SyntaxSet,
}

impl Default for Files {
    fn default() -> Self {
        Self::new()
    }
}

impl Files {
    pub fn new() -> Self {
        Files {
            syntaxes: SyntaxSet::load_defaults_newlines(),
        }
    }

    /// Highlight text that is not (or not yet) what is on disk.
    ///
    /// The editor calls this as the user types, so the colours track the draft
    /// rather than the last save — without it the painted layer under the
    /// caret shows stale text, which reads as corruption.
    pub fn highlight_source(&self, path: &str, text: &str) -> Vec<crate::model::Line> {
        crate::highlight::lines(&self.syntaxes, path, text).0
    }

    /// Read a file under `root`, highlighted.
    pub fn open(&self, root: &Path, relative: &str) -> Result<Document> {
        let path = resolve(root, relative)?;

        let metadata = std::fs::metadata(&path).map_err(|source| Error::Read {
            path: relative.to_string(),
            source,
        })?;
        if metadata.len() > MAX_BYTES {
            return Ok(refused(relative, Refusal::TooLarge));
        }

        let bytes = std::fs::read(&path).map_err(|source| Error::Read {
            path: relative.to_string(),
            source,
        })?;

        // A NUL byte is what every tool uses to decide a file is not text, and
        // it is right often enough. Rendering a firmware image as mojibake
        // helps nobody.
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(refused(relative, Refusal::Binary));
        };
        if text.contains('\0') {
            return Ok(refused(relative, Refusal::Binary));
        }

        let (lines, language, truncated) = highlight::lines(&self.syntaxes, relative, &text);
        Ok(Document {
            path: relative.to_string(),
            lines,
            text,
            language,
            binary: false,
            too_large: false,
            truncated,
            read_only: false,
        })
    }

    /// Open a file outside the project, read-only — where goto-definition
    /// lands when the answer is in a dependency.
    ///
    /// Only paths under the places library source actually lives: the cargo
    /// registry cache, cargo's git checkouts, and rustup's toolchains (which
    /// hold `core` and friends). Everything else is refused — this is the one
    /// door out of the project sandbox, and it opens exactly wide enough for
    /// "show me the definition" and no wider.
    pub fn open_external(&self, absolute: &str) -> Result<Document> {
        let path = std::path::PathBuf::from(absolute);
        if !is_library_source(&path) {
            return Err(Error::Outside {
                path: absolute.to_string(),
            });
        }

        let bytes = std::fs::read(&path).map_err(|source| Error::Read {
            path: absolute.to_string(),
            source,
        })?;
        let Ok(text) = String::from_utf8(bytes) else {
            let mut document = refused(absolute, Refusal::Binary);
            document.read_only = true;
            return Ok(document);
        };

        let (lines, language, truncated) = highlight::lines(&self.syntaxes, absolute, &text);
        Ok(Document {
            path: absolute.replace('\\', "/"),
            lines,
            text,
            language,
            binary: false,
            too_large: false,
            truncated,
            read_only: true,
        })
    }
}

/// Whether a path is somewhere dependency source lives.
fn is_library_source(path: &std::path::Path) -> bool {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else {
        return false;
    };
    let home = std::path::PathBuf::from(home);
    let cargo = std::env::var_os("CARGO_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".cargo"));
    let rustup = std::env::var_os("RUSTUP_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".rustup"));

    [
        cargo.join("registry").join("src"),
        cargo.join("git").join("checkouts"),
        rustup.join("toolchains"),
    ]
    .iter()
    .any(|root| path.starts_with(root))
}

/// Write a file under `root`.
pub fn save(root: &Path, relative: &str, text: &str) -> Result<()> {
    let path = resolve(root, relative)?;
    std::fs::write(&path, text).map_err(|source| Error::Write {
        path: relative.to_string(),
        source,
    })
}

/// Create an empty file or a directory under `root`.
///
/// Refuses to touch anything that already exists — "new file" over an
/// existing name must never become "truncate it". Parent directories are
/// created for a file, so `src/驱动/mod.rs` works in one step.
pub fn create(root: &Path, relative: &str, dir: bool) -> Result<()> {
    let path = resolve(root, relative)?;
    if path.exists() {
        return Err(Error::Exists {
            path: relative.to_string(),
        });
    }
    let write = |source| Error::Write {
        path: relative.to_string(),
        source,
    };
    if dir {
        std::fs::create_dir_all(&path).map_err(write)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(write)?;
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map(|_| ())
            .map_err(write)
    }
}

/// Turn a relative path from the frontend into a real one, refusing anything
/// that leaves the project.
///
/// The frontend echoes back paths this crate gave it, so in normal use this
/// never fires. It exists because "never in normal use" is not a security
/// property: a `..` in one of them would read or overwrite anything the process
/// can reach, and normalising it away would silently open a different file than
/// the caller named.
fn resolve(root: &Path, relative: &str) -> Result<PathBuf> {
    let candidate = Path::new(relative);
    let escapes = candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if escapes || candidate.is_absolute() {
        return Err(Error::Outside {
            path: relative.to_string(),
        });
    }
    Ok(root.join(candidate))
}

/// Why a file is not shown as text.
enum Refusal {
    /// Not text at all — a NUL byte, or bytes that are not UTF-8.
    Binary,
    /// Text, perhaps, but over [`MAX_BYTES`]; nobody read it to find out.
    TooLarge,
}

/// A document with nothing in it and the reason why. `binary` is set for
/// both, because both mean "there are no lines to draw"; `too_large` says
/// which, so a viewer can tell a 3 MB register map from a firmware image.
fn refused(relative: &str, why: Refusal) -> Document {
    Document {
        path: relative.to_string(),
        lines: Vec::new(),
        text: String::new(),
        language: None,
        binary: true,
        too_large: matches!(why, Refusal::TooLarge),
        truncated: false,
        read_only: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            dir.path().join("firmware.elf"),
            [0x7f, b'E', b'L', b'F', 0, 1],
        )
        .unwrap();
        dir
    }

    /// Creation refuses what exists and escapes; a nested new file gets its
    /// parents. The refusal matters most: "new file" over `main.rs` must
    /// error, not hand back an emptied `main.rs`.
    #[test]
    fn create_makes_new_things_and_only_new_things() {
        let dir = scratch();

        create(dir.path(), "src/驱动/mod.rs", false).unwrap();
        assert!(dir.path().join("src/驱动/mod.rs").is_file());
        create(dir.path(), "docs", true).unwrap();
        assert!(dir.path().join("docs").is_dir());

        assert!(
            matches!(
                create(dir.path(), "src/main.rs", false).unwrap_err(),
                Error::Exists { .. },
            ),
            "an existing file must be refused, never truncated",
        );
        assert!(matches!(
            create(dir.path(), "../escape", true).unwrap_err(),
            Error::Outside { .. },
        ));
    }

    #[test]
    fn a_source_file_comes_back_highlighted() {
        let dir = scratch();
        let document = Files::new().open(dir.path(), "src/main.rs").unwrap();

        assert!(!document.binary);
        assert_eq!(document.text, "fn main() {}\n");
        assert_eq!(document.language.as_deref(), Some("Rust"));
        assert!(!document.lines.is_empty());
    }

    /// Binary files sit in the same tree as the source. Rendering one as text
    /// produces a screen of mojibake and, for a real firmware image, a frame
    /// large enough to take the window down.
    #[test]
    fn a_binary_is_reported_rather_than_rendered() {
        let dir = scratch();
        let document = Files::new().open(dir.path(), "firmware.elf").unwrap();

        assert!(document.binary);
        assert!(
            !document.too_large,
            "an ELF is refused for what it is, not its size"
        );
        assert!(document.lines.is_empty());
        assert!(document.text.is_empty());
    }

    /// A file over the size limit is text nobody read, not a binary. The
    /// two used to share one flag, and a generated register map three
    /// megabytes long was reported as a firmware image.
    #[test]
    fn a_file_too_large_to_open_says_so_rather_than_calling_itself_binary() {
        let dir = scratch();
        let big = "// generated\n".repeat((MAX_BYTES as usize / 13) + 1);
        assert!(big.len() as u64 > MAX_BYTES);
        std::fs::write(dir.path().join("regs.rs"), &big).unwrap();

        let document = Files::new().open(dir.path(), "regs.rs").unwrap();
        assert!(document.too_large, "{document:?}");
        assert!(
            document.binary,
            "no lines to draw, so the viewer's refusal still applies"
        );
        assert!(document.lines.is_empty());
        assert!(document.text.is_empty());
    }

    #[test]
    fn a_path_climbing_out_of_the_project_is_refused() {
        let dir = scratch();
        let files = Files::new();

        for escape in ["../secrets", "src/../../secrets", "/etc/passwd"] {
            let error = files.open(dir.path(), escape).unwrap_err();
            assert!(
                matches!(error, Error::Outside { .. }),
                "{escape} should be refused, got {error:?}",
            );
        }
        // And writing, which is the half that would do real damage.
        assert!(matches!(
            save(dir.path(), "../oops", "x").unwrap_err(),
            Error::Outside { .. }
        ));
    }

    #[test]
    fn saving_round_trips() {
        let dir = scratch();
        save(dir.path(), "src/main.rs", "fn main() { todo!() }\n").unwrap();

        let document = Files::new().open(dir.path(), "src/main.rs").unwrap();
        assert_eq!(document.text, "fn main() { todo!() }\n");
    }
}
