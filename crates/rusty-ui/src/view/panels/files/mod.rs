//! The project's files, and an editor for them.
//!
//! The editor is a highlighted `<pre>` with a transparent `<textarea>` laid
//! exactly over it. That is how a text editor is built on the web without
//! pulling in Monaco or CodeMirror — both of which are npm, which this
//! repository does not have. The two layers share a font, a size and a line
//! height, so the caret sits where the glyph under it is.
//!
//! Completion, diagnostics, navigation and the signature card come from
//! rust-analyzer over `rusty-lsp`; saving runs the buffer through rustfmt
//! first.
//!
//! One module per thing the editor does. It was one file of 3,700 lines
//! holding eight concerns — the tree, the tabs, the surface, undo, find,
//! completion, highlighting, modal keys — and the only thing separating them
//! was the order they happened to be written in. The names below are what a
//! reader is actually looking for when they open this directory.

mod brackets;
mod caret;
mod clip;
mod complete;
mod editor;
mod edits;
mod find;
mod folding;
pub(crate) mod highlight;
mod lens;
mod modal;
mod pairs;
mod rename;
mod surface;
mod tabs;
mod tree;
mod window;

pub(crate) use editor::Editor;
use editor::{EditorGroup, SplitGrip};
pub use tree::FilesPanel;

// Reached from most of these modules; declared once here, where every child's
// `use super::*` picks it up.
use crate::view::components::copy_to_clipboard;

use brackets::*;
use caret::*;
use complete::*;
use edits::*;
use find::*;
use folding::*;
use highlight::*;
use lens::*;
use modal::*;
use rename::*;
use surface::*;
use tabs::*;
use window::*;

/// The path the OS knows an entry by: what "Copy path" puts on the
/// clipboard and a tab's tooltip shows. A Windows path is spelled with
/// backslashes, as Explorer spells it, however the root was stored — a root
/// read back from `workbench.toml` can be `E:/work/blinky` as easily as
/// `E:\work\blinky`, and the user's own file had one of each.
fn absolute_path(root: &str, relative: &str) -> String {
    let root = root.trim_end_matches(['/', '\\']);
    if root.contains('\\') || has_drive(root) {
        let root = root.replace('/', "\\");
        if relative.is_empty() {
            return root;
        }
        format!("{root}\\{}", relative.replace('/', "\\"))
    } else if relative.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{relative}")
    }
}

/// Whether a path begins with a drive letter: `E:`.
fn has_drive(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

/// Whether an open file's path is one the OS knows as it stands: a library's
/// source, opened read-only where a definition led, is held by its absolute
/// path, where a project file is held relative to the root.
fn is_outside(path: &str) -> bool {
    path.starts_with('/') || has_drive(path)
}

/// The whole path of an open file, for a tab's tooltip and the clipboard:
/// under the root for a project file, a library's as it came — with a
/// drive letter's own separators, since it arrives with `/` — and a macro
/// expansion, which is no file at all, by its name.
fn full_path(root: &str, path: &str) -> String {
    if rusty_edit::is_expansion(path) {
        path.to_string()
    } else if is_outside(path) {
        if has_drive(path) {
            path.replace('/', "\\")
        } else {
            path.to_string()
        }
    } else {
        absolute_path(root, path)
    }
}

/// Shared by both layers. They must agree exactly or the caret drifts from the
/// character it is over, a column at a time, all the way across the line.
const FONT_SIZE: f64 = 12.5;
const LINE_HEIGHT: f64 = 19.0;
/// How many spaces a tab stop is, for both layers' `tab-size` and for every
/// overlay that measures a column — one number, or the overlays measure a
/// tab as one character while the text draws it as up to four.
const TAB_SIZE: f64 = 4.0;

/// The height of one row at this zoom, in whole pixels.
///
/// Integral on purpose. The gutter draws its rows as flex containers and the
/// echo draws its as blocks, and at a fractional `line-height` the two round
/// differently — about fifteen thousandths of a pixel each, which is
/// invisible on one row and a whole line by row eighty. The numbers walk away
/// from the code they belong to, and the further down the file you look the
/// worse it is.
///
/// So the two layers are never given a fraction to disagree about, and every
/// overlay that positions itself by row uses the same number rather than
/// recomputing `LINE_HEIGHT * zoom` and landing between rows.
fn row_height(zoom: f64) -> f64 {
    (LINE_HEIGHT * zoom).round().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_path_is_spelled_as_its_system_spells_it() {
        assert_eq!(
            absolute_path("E:\\work\\blinky", "src/main.rs"),
            "E:\\work\\blinky\\src\\main.rs"
        );
        assert_eq!(
            absolute_path("/home/a/blinky/", "src"),
            "/home/a/blinky/src"
        );
        assert_eq!(absolute_path("E:\\work\\blinky\\", ""), "E:\\work\\blinky");
        // A Windows root stored with forward slashes is still a Windows path.
        assert_eq!(
            absolute_path("E:/embedded/blinky", "src/main.rs"),
            "E:\\embedded\\blinky\\src\\main.rs"
        );
        assert_eq!(
            absolute_path("E:/embedded/blinky/", ""),
            "E:\\embedded\\blinky"
        );
    }

    /// A tab's whole path: a project file under the root, a library's source
    /// as it came but with its drive's separators, a macro expansion by name.
    #[test]
    fn a_tabs_full_path_is_the_one_the_os_knows_it_by() {
        let root = "E:\\work\\blinky";
        assert_eq!(
            full_path(root, "src/main.rs"),
            "E:\\work\\blinky\\src\\main.rs"
        );
        assert_eq!(
            full_path(root, "C:/Users/a/.cargo/registry/src/gpio.rs"),
            "C:\\Users\\a\\.cargo\\registry\\src\\gpio.rs"
        );
        assert_eq!(
            full_path("/home/a/blinky", "/home/a/.cargo/registry/src/gpio.rs"),
            "/home/a/.cargo/registry/src/gpio.rs"
        );
        let expansion = format!("{}vec!", rusty_edit::EXPANSION_PREFIX);
        assert_eq!(full_path(root, &expansion), expansion);
        assert!(!is_outside("src/main.rs"));
    }
}
