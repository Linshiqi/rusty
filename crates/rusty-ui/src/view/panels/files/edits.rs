//! Changing the buffer: echo, undo history, comments, paste, format-on-save.
//!
//! Programmatic `.value` writes destroy a textarea's native undo stack, and
//! this editor writes value on every echo, completion accept and format — so
//! the history here is not a convenience, it is the only undo there is.

use leptos::{html, prelude::*};

use rusty_edit::{Line, Span, Token};

use super::*;
use crate::{controller, state::AppState};

/// Format with rustfmt and save, landing the caret where the eye already is.
/// Shared by Ctrl+S and the editor's own menu, so the two cannot drift.
pub(super) fn format_and_save(state: AppState, area: NodeRef<html::Textarea>) {
    let caret = area
        .get_untracked()
        .and_then(|element| caret_line_col(&element, &screen(state)));
    controller::format_then_save(state, caret, move |text, caret| {
        record_edit(state);
        echo_edit(state, text);
        let Some(element) = area.get_untracked() else {
            return;
        };
        // rustfmt hands back a whole new document, so this is a wholesale
        // rewrite like every other: the folds go, and the two texts agree.
        set_buffer(state, &element, text);
        // The old caret's line and column, clamped into the reformatted
        // text. rustfmt moves lines, not the one being typed on, so this
        // lands where the eye already is.
        if let Some((line, col)) = caret {
            let last = text.split('\n').count().saturating_sub(1);
            let line = (line as usize).min(last) as u32;
            let width = text
                .split('\n')
                .nth(line as usize)
                .map(|l| l.chars().count() as u32)
                .unwrap_or(0);
            let unit = utf16_offset_of(text, line, col.min(width));
            let _ = element.set_selection_start(Some(unit));
            let _ = element.set_selection_end(Some(unit));
        }
    });
}

/// Patch the painted lines for an edit, without waiting for the re-highlight.
///
/// A line diff against what the paint currently shows: unchanged lines keep
/// their colours, edited ones are swapped for plain text immediately. The
/// debounced pulse recolours them a beat later — the same catch-up every
/// editor's highlighting does, built from a splice instead of a parser.
pub(super) fn echo_edit(state: AppState, new: &str) {
    let old = state.editor.echo_text.get_untracked();
    if old == new {
        return;
    }

    let (prefix, suffix, replacement) = line_patch(&old, new);
    let old_len = old.split('\n').count();

    state.editor.highlighted.update(|lines| {
        // The paint can be shorter than the text (a truncated open); clamp so
        // a splice out of range cannot panic the whole window.
        let end = (old_len - suffix).min(lines.len());
        let start = prefix.min(end);
        lines.splice(start..end, replacement);
    });
    state.editor.echo_text.set(new.to_string());
}

/// The line diff behind [`echo_edit`]: how many leading and trailing lines
/// `old` and `new` share, and the plain-text lines that replace the middle.
///
/// Pure, so the arithmetic that decides which painted rows survive a
/// keystroke can be tested without a textarea.
fn line_patch(old: &str, new: &str) -> (usize, usize, Vec<Line>) {
    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();

    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let replacement: Vec<Line> = new_lines[prefix..new_lines.len() - suffix]
        .iter()
        .map(|text| Line {
            spans: vec![Span {
                text: (*text).to_string(),
                token: Token::Plain,
            }],
        })
        .collect();
    (prefix, suffix, replacement)
}

/// Snapshot the draft before an edit replaces it.
///
/// Bursts coalesce: pushes within 600ms collapse into one undo step, so
/// Ctrl+Z after typing a word removes the word, not one letter.
pub(super) fn record_edit(state: AppState) {
    const CAP: usize = 200;
    const BURST_MS: f64 = 600.0;

    let now = js_sys::Date::now();
    let text = state.editor.draft.get_untracked();
    state.editor.history.update(|history| {
        history.redo.clear();
        let burst = now - history.last_push < BURST_MS && !history.undo.is_empty();
        if !burst && history.undo.last() != Some(&text) {
            history.undo.push(text);
            if history.undo.len() > CAP {
                history.undo.remove(0);
            }
        }
        history.last_push = now;
    });
}

/// Undo or redo one step.
pub(super) fn apply_history(
    area: &web_sys::HtmlTextAreaElement,
    state: AppState,
    scroller: NodeRef<html::Div>,
    undo: bool,
) {
    let current = state.editor.draft.get_untracked();
    let mut target = None;
    state.editor.history.update(|history| {
        let (from, to) = if undo {
            (&mut history.undo, &mut history.redo)
        } else {
            (&mut history.redo, &mut history.undo)
        };
        while let Some(text) = from.pop() {
            if text != current {
                to.push(current.clone());
                target = Some(text);
                break;
            }
        }
        // The restore itself must not merge into a typing burst.
        history.last_push = 0.0;
    });
    let Some(text) = target else {
        return;
    };

    let caret = caret_after_restore(&text, &current);
    echo_edit(state, &text);
    set_buffer(state, area, &text);
    let _ = area.set_selection_start(Some(caret));
    let _ = area.set_selection_end(Some(caret));
    state.editor.completion.set(None);
    state.editor.signature.set(None);
    keep_caret_in_view(area, state, scroller);
    controller::schedule_pulse(state);
}

/// The word the caret is on, for `*` and `#`.
pub(super) fn word_at(text: &str, cursor: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let at = cursor.min(chars.len().saturating_sub(1));
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if chars.is_empty() || !is_word(chars[at]) {
        return None;
    }
    let mut start = at;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at;
    while end + 1 < chars.len() && is_word(chars[end + 1]) {
        end += 1;
    }
    Some(chars[start..=end].iter().collect())
}

/// The line comment this document uses, or `None` for a language with none
/// that this editor knows — in which case nothing is toggled, rather than
/// `//` being written into a TOML file.
fn line_comment(state: AppState) -> Option<&'static str> {
    let language = state
        .editor
        .document
        .with_untracked(|d| d.as_ref().and_then(|d| d.language.clone()))?;
    match language.as_str() {
        "rust" | "c" | "cpp" | "javascript" | "json" => Some("//"),
        "toml" | "python" | "shell" | "yaml" => Some("#"),
        _ => None,
    }
}

/// Toggle line comments across the lines `from..=to` touch.
///
/// Vim's rule, and every editor's: if *every* non-blank line in the range is
/// already commented, uncomment; otherwise comment them all. A per-line
/// toggle would shred a half-commented block into the other half.
///
/// The marker goes at the first non-blank, not at column zero, so indented
/// code keeps its shape.
pub(super) fn toggle_comments(state: AppState, text: &str, from: usize, to: usize) -> String {
    let Some(marker) = line_comment(state) else {
        return text.to_string();
    };
    toggle_comment_lines(marker, text, from, to)
}

/// [`toggle_comments`] once the marker is known — the whole of the
/// arithmetic, with nothing reactive in it.
fn toggle_comment_lines(marker: &str, text: &str, from: usize, to: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let (mut start, mut at) = (0usize, 0usize);
    while at <= chars.len() {
        if at == chars.len() || chars[at] == '\n' {
            if start <= to && at >= from {
                lines.push((start, at));
            }
            start = at + 1;
        }
        at += 1;
    }

    let body = |(a, b): (usize, usize)| -> String { chars[a..b].iter().collect() };
    let commented = lines
        .iter()
        .map(|span| body(*span))
        .filter(|line| !line.trim().is_empty())
        .all(|line| line.trim_start().starts_with(marker));

    let mut out: Vec<char> = chars.clone();
    // Back to front, so an edit never moves the spans still to be applied.
    for (a, b) in lines.into_iter().rev() {
        let line = body((a, b));
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let replacement: String = if commented {
            let rest = line.trim_start().trim_start_matches(marker);
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            format!("{}{rest}", &line[..indent])
        } else {
            format!("{}{marker} {}", &line[..indent], line.trim_start())
        };
        out.splice(a..b, replacement.chars());
    }
    out.into_iter().collect()
}

/// Comment or uncomment whatever the selection touches, or the caret's line.
///
/// The shared implementation behind Ctrl+/ and Vim's `gc`, so the two cannot
/// come to disagree about what a half-commented block does.
pub(super) fn comment_selection(state: AppState, area: &web_sys::HtmlTextAreaElement) {
    let text = state.editor.draft.get_untracked();
    let from = scalar_of_units(
        &text,
        area.selection_start().ok().flatten().unwrap_or(0) as usize,
    );
    let to = scalar_of_units(
        &text,
        area.selection_end().ok().flatten().unwrap_or(0) as usize,
    );
    let out = toggle_comments(state, &text, from.min(to), from.max(to));
    if out == text {
        return;
    }
    record_edit(state);
    echo_edit(state, &out);
    set_buffer(state, area, &out);
    // Back where the caret was, clamped: the line grew or shrank by the
    // marker's width and a caret past the end would snap to the buffer's.
    let at = units_of_scalar(&out, from.min(out.chars().count()));
    let _ = area.set_selection_start(Some(at));
    let _ = area.set_selection_end(Some(at));
    controller::schedule_pulse(state);
}

#[cfg(test)]
mod history_tests {
    use super::caret_after_restore;

    #[test]
    fn undoing_an_insertion_lands_at_the_insertion_point() {
        // other = after typing "abXc", target = restore "abc"
        assert_eq!(caret_after_restore("abc", "abXc"), 2);
    }

    #[test]
    fn undoing_a_deletion_lands_after_the_restored_text() {
        // other = after deleting X, target restores it
        assert_eq!(caret_after_restore("abXc", "abc"), 3);
    }

    #[test]
    fn cjk_before_the_change_counts_utf16_units() {
        // "中" is one scalar, one UTF-16 unit; the change is after it.
        assert_eq!(caret_after_restore("中aZb", "中ab"), 3);
        // Beyond the BMP: "𝄞" is two UTF-16 units.
        assert_eq!(caret_after_restore("𝄞aZ", "𝄞a"), 4);
    }

    #[test]
    fn identical_texts_land_at_the_end() {
        assert_eq!(caret_after_restore("same", "same"), 4);
    }
}

#[cfg(test)]
mod echo_tests {
    use super::line_patch;

    fn texts(lines: &[rusty_edit::Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.text.as_str()).collect())
            .collect()
    }

    /// Only the edited line is repainted plain; the lines around it keep
    /// their colours. Typing on line two of three touches one row.
    #[test]
    fn an_edit_inside_one_line_replaces_only_that_line() {
        let (prefix, suffix, replaced) = line_patch("a\nb\nc", "a\nbX\nc");
        assert_eq!((prefix, suffix), (1, 1));
        assert_eq!(texts(&replaced), vec!["bX"]);
    }

    /// Enter in the middle of a file inserts a row rather than repainting
    /// everything below it.
    #[test]
    fn a_new_line_is_an_insertion_not_a_repaint_of_the_rest() {
        let (prefix, suffix, replaced) = line_patch("a\nb\nc", "a\nb\n\nc");
        assert_eq!((prefix, suffix), (2, 1));
        assert_eq!(texts(&replaced), vec![""]);
    }

    /// Deleting a line is an empty replacement over one row, and a repeated
    /// line is not mistaken for context: "a\na" minus the second "a" removes
    /// one row rather than claiming both survived.
    #[test]
    fn a_deleted_line_is_an_empty_replacement() {
        let (prefix, suffix, replaced) = line_patch("a\nb\nc", "a\nc");
        assert_eq!((prefix, suffix), (1, 1));
        assert!(replaced.is_empty());

        let (prefix, suffix, replaced) = line_patch("a\na", "a");
        assert_eq!(prefix + suffix, 1);
        assert!(replaced.is_empty());
    }
}

#[cfg(test)]
mod comment_tests {
    use super::toggle_comment_lines;

    /// A block that is not entirely commented gets commented — at the first
    /// non-blank, so indentation keeps its shape — and blank lines are left
    /// alone.
    #[test]
    fn a_mixed_block_is_commented_at_the_indent() {
        let text = "    let a = 1;\n\n    // let b = 2;\n    let c = 3;";
        let out = toggle_comment_lines("//", text, 0, text.chars().count());
        assert_eq!(
            out,
            "    // let a = 1;\n\n    // // let b = 2;\n    // let c = 3;"
        );
    }

    /// A block that is entirely commented is uncommented, and the space the
    /// marker was written with goes too.
    #[test]
    fn a_fully_commented_block_is_uncommented() {
        let text = "  // one\n  //two\n\n  // three";
        let out = toggle_comment_lines("//", text, 0, text.chars().count());
        assert_eq!(out, "  one\n  two\n\n  three");
    }

    /// Only the lines the range touches change: a selection on the second
    /// line leaves the first and third as they were.
    #[test]
    fn only_the_touched_lines_change() {
        let text = "a\nb\nc";
        // Scalar 2 is the "b"; from == to is the caret's line.
        assert_eq!(toggle_comment_lines("#", text, 2, 2), "a\n# b\nc");
    }

    /// Wide characters count as one column each, so a comment marker in a
    /// file of CJK identifiers lands at the indent and not inside a glyph.
    #[test]
    fn scalars_not_bytes_choose_the_lines() {
        let text = "中文\n变量 = 1";
        let start = "中文\n".chars().count();
        assert_eq!(
            toggle_comment_lines("#", text, start, start),
            "中文\n# 变量 = 1"
        );
    }
}
