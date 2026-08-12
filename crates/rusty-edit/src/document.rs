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

    /// Read a file under `root`, highlighted.
    pub fn open(&self, root: &Path, relative: &str) -> Result<Document> {
        let path = resolve(root, relative)?;

        let metadata = std::fs::metadata(&path).map_err(|source| Error::Read {
            path: relative.to_string(),
            source,
        })?;
        if metadata.len() > MAX_BYTES {
            return Ok(binary(relative));
        }

        let bytes = std::fs::read(&path).map_err(|source| Error::Read {
            path: relative.to_string(),
            source,
        })?;

        // A NUL byte is what every tool uses to decide a file is not text, and
        // it is right often enough. Rendering a firmware image as mojibake
        // helps nobody.
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(binary(relative));
        };
        if text.contains('\0') {
            return Ok(binary(relative));
        }

        let (lines, language, truncated) = highlight::lines(&self.syntaxes, relative, &text);
        Ok(Document {
            path: relative.to_string(),
            lines,
            text,
            language,
            binary: false,
            truncated,
        })
    }
}

/// Write a file under `root`.
pub fn save(root: &Path, relative: &str, text: &str) -> Result<()> {
    let path = resolve(root, relative)?;
    std::fs::write(&path, text).map_err(|source| Error::Write {
        path: relative.to_string(),
        source,
    })
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
        matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
    });
    if escapes || candidate.is_absolute() {
        return Err(Error::Outside {
            path: relative.to_string(),
        });
    }
    Ok(root.join(candidate))
}

fn binary(relative: &str) -> Document {
    Document {
        path: relative.to_string(),
        lines: Vec::new(),
        text: String::new(),
        language: None,
        binary: true,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("firmware.elf"), [0x7f, b'E', b'L', b'F', 0, 1]).unwrap();
        dir
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
