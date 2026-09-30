//! Tab and Shift+Tab over a selection: the lines it touches, moved a level.
//!
//! A textarea's Tab moves the focus, and the editor's typed four spaces —
//! over whatever was selected, so two lines picked out to be pushed in a
//! level were replaced by the indentation meant for them. VS Code's rule is
//! the one here (`TypeOperations.tab`, and `outdentLines` for Shift+Tab):
//!
//! - **Tab** with a selection that reaches more than one line, or holds the
//!   whole of one, indents every line it touches. A selection inside one
//!   line is typed over, and a caret types, as before.
//! - **Shift+Tab** takes a level off every line the selection touches, or
//!   off the caret's line.
//!
//! A level is the next stop, not four more: a line six spaces in goes to
//! eight, and back to four. A line ends the selection without being in it —
//! the selection stops at its first column — and is left alone, and an empty
//! line is not given indentation to carry. The selection keeps what it held.
//!
//! Pure functions over the document and byte offsets, returning the one
//! [`Edit`] that `apply_edit` puts through the same record / echo /
//! `set_buffer` path as every other write.

use super::TAB_SIZE;
use super::pairs::Edit;

/// Whether Tab over `from..to` moves lines rather than typing indentation.
pub(super) fn shifts_lines(text: &str, from: usize, to: usize, back: bool) -> bool {
    let (from, to) = ordered(text, from, to);
    if back {
        return true;
    }
    if from == to {
        return false;
    }
    if text[from..to].contains('\n') {
        return true;
    }
    // One line: only the whole of it.
    let start = line_start(text, from);
    let end = line_end(text, to);
    from == start && to == end
}

/// The lines `from..to` touches, each a level in — or, `back`, a level out —
/// as one replacement with the selection it leaves. `None` where nothing
/// would change: Shift+Tab on lines already at the margin.
pub(super) fn shift(text: &str, from: usize, to: usize, back: bool) -> Option<Edit> {
    let (from, to) = ordered(text, from, to);
    if !text.is_char_boundary(from) || !text.is_char_boundary(to) {
        return None;
    }
    let first = line_start(text, from);
    // A selection that stops at a line's first column does not hold the
    // line: dragging down to the start of the next one selects these.
    let stops_short = to > from && to > first && text[..to].ends_with('\n');
    let last_end = if stops_short {
        to - 1
    } else {
        line_end(text, to)
    };
    let tabs = file_uses_tabs(text);

    let mut out = String::with_capacity(last_end - first + 16);
    let mut changed = false;
    // Where the two ends of the selection stand in the text after.
    let (mut new_from, mut new_to) = (from, to);
    let mut line_at = first;
    for line in text[first..last_end].split('\n') {
        let old = line
            .find(|c: char| c != ' ' && c != '\t')
            .unwrap_or_else(|| line.trim_end_matches('\r').len());
        let body = &line[old..];
        let empty = line.trim_end_matches('\r').is_empty();
        let indent = if empty && !back {
            // Nothing on the line to indent, and nothing to carry it.
            line[..old].to_string()
        } else {
            moved(&line[..old], back, tabs)
        };
        changed |= indent != line[..old];

        // An end inside the indentation stays where it is, as far as the
        // indentation still reaches; one in the text moves with the text.
        // The selection's start on the indentation's last column stays too,
        // so a line picked from its first column keeps its new indentation.
        let shifted = out.len() + first;
        let place = |at: usize, holds: bool| -> usize {
            let col = at - line_at;
            let in_indent = if holds { col <= old } else { col < old };
            shifted
                + if in_indent {
                    col.min(indent.len())
                } else {
                    col - old + indent.len()
                }
        };
        let here = line_at..=line_at + line.len();
        if here.contains(&from) {
            new_from = place(from, from < to);
        }
        if here.contains(&to) && !stops_short {
            new_to = place(to, false);
        }

        out.push_str(&indent);
        out.push_str(body);
        out.push('\n');
        line_at += line.len() + 1;
    }
    out.pop();
    if !changed {
        return None;
    }
    if stops_short {
        // Past the last line moved: by as much as they all grew or shrank.
        new_to = to + out.len() - (last_end - first);
    }
    if from == to {
        new_to = new_from;
    }
    Some(Edit {
        range: (first, last_end),
        text: out,
        caret: new_to,
        select: (from < to).then_some((new_from, new_to)),
    })
}

/// Indentation one level on from `indent`, or one back: to the next stop,
/// or the one before. Written in what the line is indented with — tabs
/// where it has one, so a Makefile's recipe stays a recipe, and spaces
/// otherwise; a line with none takes what the file uses.
fn moved(indent: &str, back: bool, file_tabs: bool) -> String {
    let size = TAB_SIZE as usize;
    let width = indent.chars().fold(0, |width, c| match c {
        '\t' => (width / size + 1) * size,
        _ => width + 1,
    });
    let target = if back {
        width.saturating_sub(1) / size * size
    } else {
        (width / size + 1) * size
    };
    if indent.contains('\t') || (indent.is_empty() && file_tabs) {
        "\t".repeat(target / size)
    } else {
        " ".repeat(target)
    }
}

/// Whether the file indents with tabs: what its first indented line starts
/// with.
fn file_uses_tabs(text: &str) -> bool {
    text.split('\n')
        .find(|line| line.starts_with([' ', '\t']))
        .is_some_and(|line| line.starts_with('\t'))
}

fn ordered(text: &str, from: usize, to: usize) -> (usize, usize) {
    (from.min(to).min(text.len()), from.max(to).min(text.len()))
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |at| at + 1)
}

/// Where the line holding `at` ends: before its break, and before the
/// carriage return of a Windows one.
fn line_end(text: &str, at: usize) -> usize {
    let end = text[at..].find('\n').map_or(text.len(), |found| at + found);
    if end > at && text[..end].ends_with('\r') {
        end - 1
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply an edit the way `apply_edit` does, and show the selection it
    /// leaves as `[` and `]` in the text.
    fn after(text: &str, from: usize, to: usize, back: bool) -> String {
        let Some(edit) = shift(text, from, to, back) else {
            return "unchanged".to_string();
        };
        let mut out = text.to_string();
        out.replace_range(edit.range.0..edit.range.1, &edit.text);
        let (a, b) = edit.select.unwrap_or((edit.caret, edit.caret));
        out.insert(b, ']');
        out.insert(a, '[');
        out
    }

    /// A text with `[` and `]` marking the selection: the text without
    /// them, and the two offsets.
    fn marked(text: &str) -> (String, usize, usize) {
        let from = text.find('[').expect("a [");
        let to = text.find(']').expect("a ]") - 1;
        (text.replace(['[', ']'], ""), from, to)
    }

    fn tab(text: &str) -> String {
        let (text, from, to) = marked(text);
        after(&text, from, to, false)
    }

    fn back(text: &str) -> String {
        let (text, from, to) = marked(text);
        after(&text, from, to, true)
    }

    /// The report: `y` and `z` picked out under `x` and pushed in to it.
    /// They were replaced by four spaces.
    #[test]
    fn selected_lines_are_indented_not_replaced() {
        let text = "    x: 0.0,\n[y: 0.0,\nz: 0.0,]\n}";
        assert_eq!(tab(text), "    x: 0.0,\n[    y: 0.0,\n    z: 0.0,]\n}");
        let (plain, from, to) = marked(text);
        assert!(shifts_lines(&plain, from, to, false));
    }

    #[test]
    fn shift_tab_takes_a_level_off_every_line_touched() {
        assert_eq!(
            back("        [a,\n        b,]\n    c"),
            "    [a,\n    b,]\n    c"
        );
    }

    /// A selection from the middle of one line to the middle of another
    /// moves both lines whole, and still holds the text it held.
    #[test]
    fn a_selection_inside_the_text_keeps_what_it_held() {
        assert_eq!(
            tab("let [a = 1;\nlet b] = 2;"),
            "    let [a = 1;\n    let b] = 2;"
        );
        assert_eq!(
            back("    let [a = 1;\n    let b] = 2;"),
            "let [a = 1;\nlet b] = 2;"
        );
    }

    /// Dragged down to the start of the next line: that line is not in the
    /// selection, and is not moved.
    #[test]
    fn a_line_the_selection_only_reaches_is_left_alone() {
        assert_eq!(tab("[a\nb\n]c"), "[    a\n    b\n]c");
        assert_eq!(back("[    a\n    b\n]    c"), "[a\nb\n]    c");
    }

    /// A level is the next stop, not four more columns.
    #[test]
    fn indentation_goes_to_the_next_stop() {
        assert_eq!(tab("[      a\n  b]"), "[        a\n    b]");
        assert_eq!(back("[      a\n  b]"), "[    a\nb]");
    }

    #[test]
    fn an_empty_line_is_not_given_indentation() {
        assert_eq!(tab("[a\n\nb]"), "[    a\n\n    b]");
    }

    /// Shift+Tab with no selection is the caret's line, and the caret stays
    /// on the character it was on.
    #[test]
    fn a_caret_takes_its_own_line_back() {
        assert_eq!(back("        let a[] = 1;"), "    let a[] = 1;");
        // In the indentation it stays, as far as the indentation reaches.
        assert_eq!(back("  []      a"), "  []  a");
        assert_eq!(back("      []  a"), "    []a");
    }

    #[test]
    fn nothing_to_take_off_changes_nothing() {
        assert_eq!(back("[a\nb]"), "unchanged");
        assert_eq!(back("a[]"), "unchanged");
    }

    /// A caret types, and a selection inside one line is typed over, as VS
    /// Code's is; the whole of a line is indented.
    #[test]
    fn tab_moves_lines_only_where_the_selection_holds_lines() {
        assert!(!shifts_lines("let a = 1;", 4, 4, false));
        assert!(!shifts_lines("let a = 1;", 4, 5, false));
        assert!(shifts_lines("let a = 1;", 0, 10, false));
        assert!(shifts_lines("a\nlet a = 1;\nb", 2, 12, false));
        assert!(shifts_lines("let a = 1;\nlet b", 4, 14, false));
        // Shift+Tab is always the lines.
        assert!(shifts_lines("let a = 1;", 4, 4, true));
        assert!(shifts_lines("let a = 1;", 4, 5, true));
    }

    /// A line indented with tabs moves by a tab, so a recipe stays one.
    #[test]
    fn a_tab_indented_line_moves_by_a_tab() {
        assert_eq!(
            tab("all:\n[\tcc a.c\n\tcc b.c]"),
            "all:\n[\t\tcc a.c\n\t\tcc b.c]"
        );
        assert_eq!(
            back("all:\n[\t\tcc a.c\n\tcc b.c]"),
            "all:\n[\tcc a.c\ncc b.c]"
        );
        // An unindented line in such a file takes a tab too.
        assert_eq!(tab("all:\n\tcc a.c\n[x\ny]"), "all:\n\tcc a.c\n[\tx\n\ty]");
    }

    #[test]
    fn a_multibyte_line_moves_like_any_other() {
        assert_eq!(
            tab("[// 中文\nlet 名 = 1;]"),
            "[    // 中文\n    let 名 = 1;]"
        );
        assert_eq!(
            back("    // 中[文\n    let 名] = 1;"),
            "// 中[文\nlet 名] = 1;"
        );
    }

    /// Windows line endings: the carriage return is the line's, and an
    /// empty line is still empty with one.
    #[test]
    fn carriage_returns_stay_at_the_ends_of_their_lines() {
        assert_eq!(tab("[a\r\n\r\nb]\r\n"), "[    a\r\n\r\n    b]\r\n");
        assert!(shifts_lines("let a;\r\nb", 0, 6, false));
    }

    #[test]
    fn a_backward_pair_of_offsets_is_the_same_selection() {
        let (text, from, to) = marked("[a\nb]");
        assert_eq!(after(&text, to, from, false), "[    a\n    b]");
    }
}
