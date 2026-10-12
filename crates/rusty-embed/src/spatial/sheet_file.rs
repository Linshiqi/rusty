//! `.rusty/math.toml`: the math toolbox's sheet, kept in the project beside
//! the board, so the arithmetic a flight controller was checked against is
//! diffed and reviewed with the flight controller.
//!
//! The file's own shape, apart from the wire's (`MathSheet`): somebody may
//! write it by hand, and a rename on the frontend must not become a key
//! missing from everybody's file.

use std::path::Path;

use serde::Deserialize;

use super::Frame;
use super::sheet::MathSheet;
use crate::error::{Error, Result};

/// Where the sheet lives, from the project's root.
pub const PATH: &str = ".rusty/math.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    frame: Option<String>,
    #[serde(default)]
    rows: Vec<String>,
}

/// The project's sheet, or `None` when it has none yet.
///
/// A file that is there and does not read is an error naming it — never an
/// empty sheet in its place, because the panel saves as it is typed in and
/// would write the emptiness over whatever the file held.
pub fn load(root: &Path) -> Result<Option<MathSheet>> {
    let path = root.join(PATH);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Read {
                path: PATH.into(),
                source,
            });
        }
    };
    let file: File = toml::from_str(&text).map_err(|source| Error::Toml {
        path: PATH.into(),
        source,
    })?;
    let frame = match file.frame.as_deref() {
        None | Some("z-up") => Frame::ZUp,
        Some("z-down") => Frame::ZDown,
        Some(other) => {
            return Err(Error::refused(format!(
                "{PATH} says frame = \"{other}\", which is neither \"z-up\" nor \"z-down\""
            )));
        }
    };
    Ok(Some(MathSheet {
        frame,
        rows: file.rows,
    }))
}

/// The sheet, written a row a line, so a diff of the file reads as the rows
/// that changed.
pub fn save(root: &Path, sheet: &MathSheet) -> Result<()> {
    let dir = root.join(".rusty");
    std::fs::create_dir_all(&dir).map_err(|source| Error::Write {
        path: ".rusty".into(),
        source,
    })?;
    let frame = match sheet.frame {
        Frame::ZUp => "z-up",
        Frame::ZDown => "z-down",
    };
    let mut text = String::from(
        "# rusty's math toolbox: one definition a row, read top to bottom.\n\
         # `frame` is which way the world's Z points: \"z-up\" or \"z-down\".\n",
    );
    text.push_str(&format!("frame = \"{frame}\"\nrows = [\n"));
    for row in &sheet.rows {
        text.push_str(&format!("  {},\n", toml::Value::String(row.clone())));
    }
    text.push_str("]\n");
    std::fs::write(root.join(PATH), text).map_err(|source| Error::Write {
        path: PATH.into(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sheet_comes_back_as_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).unwrap().is_none());
        let sheet = MathSheet {
            frame: Frame::ZDown,
            rows: vec![
                "roll = 30° # bank".into(),
                "est = euler(tel(\"roll\"), tel(\"pitch\"), 0)".into(),
                String::new(),
                "back\\slash".into(),
            ],
        };
        save(dir.path(), &sheet).unwrap();
        assert_eq!(load(dir.path()).unwrap(), Some(sheet));
        let text = std::fs::read_to_string(dir.path().join(PATH)).unwrap();
        assert!(text.contains("frame = \"z-down\"\n"), "{text}");
        assert!(text.contains("  \"roll = 30° # bank\",\n"), "{text}");
    }

    /// A file that does not read is refused by name, never read as empty —
    /// the panel saves as it is typed in, and would write that over it.
    #[test]
    fn a_file_that_does_not_read_is_refused_not_emptied() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".rusty")).unwrap();
        std::fs::write(dir.path().join(PATH), "rows = [\"unclosed\"\n").unwrap();
        assert!(matches!(load(dir.path()), Err(Error::Toml { .. })));
        std::fs::write(dir.path().join(PATH), "frame = \"sideways\"\n").unwrap();
        let refused = load(dir.path()).unwrap_err().to_string();
        assert!(refused.contains("sideways"), "{refused}");
        std::fs::write(dir.path().join(PATH), "colour = \"red\"\n").unwrap();
        assert!(load(dir.path()).is_err());
    }
}
