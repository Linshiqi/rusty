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

mod caret;
mod complete;
mod editor;
mod edits;
mod find;
mod folding;
mod highlight;
mod modal;
mod rename;
mod surface;
mod tabs;
mod tree;

pub(crate) use editor::Editor;
pub use tree::FilesPanel;

// Reached from most of these modules; declared once here, where every child's
// `use super::*` picks it up.
use crate::view::components::{copy_to_clipboard, register_toolbar};

use caret::*;
use complete::*;
use edits::*;
use find::*;
use folding::*;
use highlight::*;
use modal::*;
use rename::*;
use surface::*;
use tabs::*;

/// Shared by both layers. They must agree exactly or the caret drifts from the
/// character it is over, a column at a time, all the way across the line.
const FONT_SIZE: f64 = 12.5;
const LINE_HEIGHT: f64 = 19.0;

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
