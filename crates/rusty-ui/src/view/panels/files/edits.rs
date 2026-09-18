//! Changing the buffer: echo, undo history, comments, paste, format-on-save.
//!
//! Programmatic `.value` writes destroy a textarea's native undo stack, and
//! this editor writes value on every echo, completion accept and format — so
//! the history here is not a convenience, it is the only undo there is.

use leptos::{html, prelude::*};

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

// ─── copy, cut and paste: the selection, or the whole line ───────────────────

/// Vim's normal and visual modes, where the textarea is read-only: the
/// browser inserts and deletes nothing there, so every change is ours.
fn vim_modal(state: AppState) -> bool {
    state.editor.vim_on.get_untracked()
        && state
            .editor
            .vim
            .with_untracked(|vim| vim.mode != crate::vim::Mode::Insert)
}

/// Ctrl+C or Ctrl+X: the selection when there is one, the whole line
/// (`clip.rs`) when there is not — VS Code's rule, in every mode. Vim's
/// normal mode has no selection of its own (its cursor is drawn, not
/// selected), so a selection there is one somebody made, with the mouse or
/// a double-click, and it is what they meant to copy. A cut of a selection
/// in Vim's modes is taken here, because the read-only textarea would copy
/// it and delete nothing; a selection anywhere else is the browser's, as it
/// always was. Whether the key was taken.
pub(super) fn clipboard_key(
    state: AppState,
    area: &web_sys::HtmlTextAreaElement,
    cut: bool,
    read_only: bool,
) -> bool {
    let (from, to) = doc_selection(area, state);
    let text = state.editor.draft.get_untracked();
    if from == to {
        if cut && !read_only {
            let (line, edit) = clip::cut_line(&text, from);
            copy_to_clipboard(&line);
            state.editor.copied_line.set(Some(line));
            apply_edit(area, state, &edit);
        } else {
            let line = clip::copy_line(&text, from);
            copy_to_clipboard(&line);
            state.editor.copied_line.set(Some(line));
        }
        return true;
    }
    // A selection is going on the clipboard, and it is not a line.
    state.editor.copied_line.set(None);
    if cut && vim_modal(state) && !read_only {
        copy_to_clipboard(&text[from..to]);
        let edit = pairs::Edit {
            range: (from, to),
            text: String::new(),
            caret: from,
            select: None,
        };
        apply_edit(area, state, &edit);
        leave_visual(state);
        return true;
    }
    false
}

/// Back to normal mode after a cut or a paste over a visual selection, as
/// Vim goes back after `d` or `p` there. Nothing to do from normal mode.
fn leave_visual(state: AppState) {
    if state.editor.vim.with_untracked(|vim| vim.mode.is_visual()) {
        state
            .editor
            .vim
            .update(|vim| vim.mode = crate::vim::Mode::Normal);
    }
}

/// Ctrl+V in Vim's normal or visual mode, before the browser acts on it. A
/// browser will not paste into a read-only field — nor tell the page that
/// anybody asked — so the textarea is made writable for exactly this one
/// paste, and [`paste_into`] makes it read-only again the moment the paste
/// arrives. A paste that never arrives (nothing on the clipboard) is covered
/// by the timeout, and so is a mode that changed in between.
pub(super) fn open_for_paste(state: AppState, area: NodeRef<html::Textarea>, read_only: bool) {
    if read_only || !vim_modal(state) {
        return;
    }
    let Some(element) = area.get_untracked() else {
        return;
    };
    element.set_read_only(false);
    set_timeout(
        move || {
            let modal = state.editor.vim_on.try_get_untracked().unwrap_or(false)
                && state
                    .editor
                    .vim
                    .try_with_untracked(|vim| vim.mode != crate::vim::Mode::Insert)
                    .unwrap_or(false);
            if let Some(element) = area.try_get_untracked().flatten() {
                element.set_read_only(modal);
            }
        },
        std::time::Duration::ZERO,
    );
}

/// A paste, with what the clipboard holds. With nothing selected, the line
/// Ctrl+C or Ctrl+X copied goes in whole above the caret's line; over a
/// selection it replaces the selection like any other text, as VS Code's
/// does. In Vim's normal and visual modes every paste is put in here, since
/// the browser was let in only to hand the text over. Anything else is the
/// browser's own paste. Whether it was taken.
pub(super) fn paste_into(
    state: AppState,
    area: &web_sys::HtmlTextAreaElement,
    pasted: &str,
    read_only: bool,
) -> bool {
    let modal = vim_modal(state);
    if modal {
        area.set_read_only(true);
    }
    if read_only {
        return false;
    }
    let (from, to) = doc_selection(area, state);
    let text = state.editor.draft.get_untracked();
    let remembered = state.editor.copied_line.get_untracked();
    if from == to && clip::is_copied_line(pasted, remembered.as_deref()) {
        let line = remembered.unwrap_or_default();
        let edit = clip::paste_line(&text, from, &line);
        apply_edit(area, state, &edit);
        return true;
    }
    if !modal {
        return false;
    }
    let plain = pasted.replace("\r\n", "\n");
    if plain.is_empty() {
        return true;
    }
    // At the cursor, or over the selection — a visual one, or one made with
    // the mouse in normal mode — and back to normal mode after a visual one,
    // as a paste over a selection does.
    let edit = pairs::Edit {
        range: (from, to),
        text: plain.clone(),
        caret: from + plain.len(),
        select: None,
    };
    apply_edit(area, state, &edit);
    leave_visual(state);
    true
}

/// The echo lives with the controller now, which carries an edit to the
/// other view of a file with the same function (`controller::views`).
pub(super) use crate::controller::echo_edit;

/// Snapshot the draft before an edit replaces it.
///
/// Bursts coalesce: pushes within 600ms collapse into one undo step, so
/// Ctrl+Z after typing a word removes the word, not one letter.
pub(super) fn record_edit(state: AppState) {
    const BURST_MS: f64 = 600.0;

    let now = js_sys::Date::now();
    let text = state.editor.draft.get_untracked();
    let Some(path) = state.active_path_now() else {
        return;
    };
    state.editor.with_history(&path, |history| {
        history.redo.clear();
        let burst = now - history.last_push < BURST_MS && !history.undo.is_empty();
        if !burst && history.undo.last() != Some(&text) {
            history.undo.push(text);
            history.trim();
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
    let Some(path) = state.active_path_now() else {
        return;
    };
    let mut target = None;
    state.editor.with_history(&path, |history| {
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
    controller::dismiss_completion(state);
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
    comment_marker(&language)
}

/// The line comment for a grammar, by the name the backend reports it under.
///
/// Those are syntect's names — `Rust`, `C++`, `TOML`, `Bourne Again Shell
/// (bash)` — so they are compared without regard to case. This matched
/// `"rust"` alone for as long as it existed, which is what `mock.js` calls
/// the language and nothing the app ever sends: Ctrl+/ and Vim's `gc` did
/// nothing in a Rust file, in the app, while working in the browser.
fn comment_marker(language: &str) -> Option<&'static str> {
    match language.to_ascii_lowercase().as_str() {
        "rust" | "c" | "c++" | "cpp" | "javascript" | "json" | "go" | "java" => Some("//"),
        "toml" | "python" | "yaml" | "makefile" | "shell" | "bourne again shell (bash)" => {
            Some("#")
        }
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
mod comment_tests {
    use super::{comment_marker, toggle_comment_lines};

    /// The names the backend sends are syntect's, capitalised as syntect
    /// capitalises them; the browser mock's lower-case `rust` is not one of
    /// them, and was the only name this used to answer.
    #[test]
    fn the_marker_is_found_by_the_grammar_names_the_backend_sends() {
        for (language, marker) in [
            ("Rust", Some("//")),
            ("C", Some("//")),
            ("C++", Some("//")),
            ("TOML", Some("#")),
            ("Python", Some("#")),
            ("Bourne Again Shell (bash)", Some("#")),
            ("rust", Some("//")),
            ("Markdown", None),
        ] {
            assert_eq!(comment_marker(language), marker, "{language}");
        }
    }

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
