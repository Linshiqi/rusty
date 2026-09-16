//! Copy, cut and paste with nothing selected: the whole line.
//!
//! VS Code's `editor.emptySelectionClipboard`, on by default there, and the
//! habit it teaches: Ctrl+C on a line copies the line, Ctrl+X takes it out,
//! and Ctrl+V of a line copied that way puts it back *as a line* — above the
//! caret's line, wherever in that line the caret is — instead of splicing it
//! into the middle of whatever is there. The browser's own copy and cut do
//! nothing without a selection, so in this editor the keys did nothing at
//! all, and in Vim's normal mode they copied the one character under the
//! block cursor, which is the selection that draws it.
//!
//! Pure, over the document and a byte offset into it, and every answer that
//! changes the text is a [`pairs::Edit`] — so it reaches the buffer through
//! the same `apply_edit` as a bracket pair, and undo, the echo and the folds
//! cannot disagree with it. The caller maps the caret through the fold table
//! first.

use super::pairs::Edit;

/// `at` moved back onto a character boundary — a caret is never inside a
/// character, and slicing there would panic the window.
fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Where the line holding `at` starts, and where it ends: its line break, or
/// the end of the text on the last line.
fn line_bounds(text: &str, at: usize) -> (usize, usize) {
    let at = boundary(text, at);
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
    (start, end)
}

/// What Ctrl+C with nothing selected puts on the clipboard: the caret's line
/// and a line break after it, whether or not the file has one there. The
/// break is part of what makes it a line when it comes back.
pub(super) fn copy_line(text: &str, caret: usize) -> String {
    let (start, end) = line_bounds(text, caret);
    format!("{}\n", &text[start..end])
}

/// Ctrl+X with nothing selected: the line for the clipboard, and the edit
/// that takes it out, break and all. The caret keeps its column — counted in
/// characters, so a line of 中文 keeps its place — on the line that moves up
/// into the gap, or on the line above when the last line was the one cut.
pub(super) fn cut_line(text: &str, caret: usize) -> (String, Edit) {
    let caret = boundary(text, caret);
    let (start, end) = line_bounds(text, caret);
    let copied = format!("{}\n", &text[start..end]);
    let column = text[start..caret].chars().count();
    let last = end == text.len();
    let range = if !last {
        (start, end + 1)
    } else if start > 0 {
        // No break after the last line, so the one before it goes instead;
        // otherwise the cut would leave an empty line where the text was.
        (start - 1, end)
    } else {
        (start, end)
    };
    let mut after = text.to_string();
    after.replace_range(range.0..range.1, "");
    let landing = if last && start > 0 {
        line_bounds(&after, start - 1).0
    } else {
        start
    };
    let (line_start, line_end) = line_bounds(&after, landing);
    let caret = line_start
        + after[line_start..line_end]
            .chars()
            .take(column)
            .map(char::len_utf8)
            .sum::<usize>();
    (
        copied,
        Edit {
            range,
            text: String::new(),
            caret,
            select: None,
        },
    )
}

/// Whether what is being pasted is the line [`copy_line`] or [`cut_line`]
/// last put on the clipboard. Compared with the line breaks made plain,
/// because the Windows clipboard hands `\n` back as `\r\n`; and it must still
/// be *that* text — something copied in another program since is pasted as
/// what it is.
pub(super) fn is_copied_line(pasted: &str, remembered: Option<&str>) -> bool {
    let plain = |text: &str| text.replace("\r\n", "\n");
    remembered.is_some_and(|line| line.ends_with('\n') && plain(pasted) == plain(line))
}

/// Ctrl+V of a copied line with nothing selected: in whole, above the
/// caret's line, with the caret staying where it was in its own line — which
/// is one line further down. On an empty line that is the line landing where
/// the caret was.
pub(super) fn paste_line(text: &str, caret: usize, line: &str) -> Edit {
    let caret = boundary(text, caret);
    let (start, _) = line_bounds(text, caret);
    Edit {
        range: (start, start),
        text: line.to_string(),
        caret: caret + line.len(),
        select: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: &Edit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range.0..edit.range.1, &edit.text);
        out
    }

    const STRUCT: &str = "pub struct Vector3D {\n    x: f32,\n\n}\n";

    /// The case this was asked for, from the screenshot that asked for it:
    /// the caret at the end of `x: f32,`, Ctrl+C, down to the empty line,
    /// Ctrl+V — and the field is there twice, with nothing spliced into
    /// anything.
    #[test]
    fn a_line_copied_and_pasted_on_an_empty_line_is_a_second_line() {
        let caret = STRUCT.find("f32,").unwrap() + "f32,".len();
        let line = copy_line(STRUCT, caret);
        assert_eq!(line, "    x: f32,\n");

        let empty = STRUCT.find("\n\n}").unwrap() + 1;
        let edit = paste_line(STRUCT, empty, &line);
        let after = apply(STRUCT, &edit);
        assert_eq!(
            after,
            "pub struct Vector3D {\n    x: f32,\n    x: f32,\n\n}\n"
        );
        assert_eq!(
            &after[edit.caret..],
            "\n}\n",
            "the caret stays on its empty line, now one further down",
        );
    }

    /// Wherever the caret is in a line, a copied line goes above that line —
    /// never into the middle of it, which is what a plain paste would do.
    #[test]
    fn a_copied_line_goes_above_the_caret_and_the_caret_keeps_its_place() {
        let text = "let a = 1;\nlet b = 2;\n";
        let caret = text.find("b = 2").unwrap() + 2;
        let edit = paste_line(text, caret, "let z = 0;\n");
        let after = apply(text, &edit);
        assert_eq!(after, "let a = 1;\nlet z = 0;\nlet b = 2;\n");
        assert!(after[edit.caret..].starts_with("= 2;"), "{after:?}");
    }

    /// A line in the middle goes with its break, and the caret keeps its
    /// column on the line that moves up into the gap.
    #[test]
    fn a_cut_takes_the_line_and_its_break() {
        let text = "one\ntwo\nthree\n";
        let caret = text.find("two").unwrap() + 2;
        let (copied, edit) = cut_line(text, caret);
        assert_eq!(copied, "two\n");
        let after = apply(text, &edit);
        assert_eq!(after, "one\nthree\n");
        assert!(after[edit.caret..].starts_with("ree"), "{after:?}");
    }

    /// The last line has no break after it, so the break before it goes —
    /// cutting it must not leave an empty line behind — and the caret lands
    /// on the line above, clamped to how long that line is.
    #[test]
    fn cutting_the_last_line_takes_the_break_before_it() {
        let text = "ab\nlonger";
        let (copied, edit) = cut_line(text, text.len());
        assert_eq!(copied, "longer\n", "a copied line always ends in a break");
        let after = apply(text, &edit);
        assert_eq!(after, "ab");
        assert_eq!(edit.caret, 2, "column six clamps to the end of `ab`");

        let (copied, edit) = cut_line("only", 1);
        assert_eq!(copied, "only\n");
        assert_eq!(apply("only", &edit), "");
        assert_eq!(edit.caret, 0);
    }

    /// Columns are characters: a caret after 中 on one line is after the
    /// first character on the next, not three bytes in.
    #[test]
    fn a_cut_keeps_the_column_in_characters() {
        let text = "中文注释\n世界你好\n";
        let caret = "中".len();
        let (_, edit) = cut_line(text, caret);
        let after = apply(text, &edit);
        assert_eq!(after, "世界你好\n");
        assert_eq!(&after[edit.caret..], "界你好\n");
    }

    /// A caret offset inside a character is walked back, not sliced at.
    #[test]
    fn an_offset_inside_a_character_is_refused_not_panicked() {
        let text = "中\n";
        assert_eq!(copy_line(text, 1), "中\n");
        let (copied, _) = cut_line(text, 2);
        assert_eq!(copied, "中\n");
        let edit = paste_line(text, 1, "x\n");
        assert_eq!(apply(text, &edit), "x\n中\n");
    }

    /// Only the line rusty put there pastes as a line — with the Windows
    /// clipboard's line breaks forgiven — and anything copied since in
    /// another program is pasted as what it is.
    #[test]
    fn only_the_line_that_was_copied_pastes_as_a_line() {
        assert!(is_copied_line("    x: f32,\n", Some("    x: f32,\n")));
        assert!(is_copied_line("    x: f32,\r\n", Some("    x: f32,\n")));
        assert!(!is_copied_line("something else", Some("    x: f32,\n")));
        assert!(!is_copied_line("    x: f32,\n", None));
        assert!(
            !is_copied_line("f32", Some("f32")),
            "a selection copied without a break is not a line"
        );
    }
}
