//! Accepting what the language server offered: completions and quick fixes.
//!
//! Both splice text at a position the server named, so both have to convert
//! that position into the textarea's own units before touching the buffer.

use leptos::prelude::*;

use rusty_lsp::CompletionItem;

use super::*;
use crate::{controller, state::AppState};

/// Apply the chosen quick fix: splice its edits bottom-up so earlier ranges
/// stay valid, through the undo pipeline.
pub(super) fn apply_action(state: AppState, area: &web_sys::HtmlTextAreaElement, index: usize) {
    let Some((_, _, fixes)) = state.editor.actions.get_untracked() else {
        return;
    };
    let Some(fix) = fixes.get(index.min(fixes.len().saturating_sub(1))) else {
        return;
    };

    let text = state.editor.draft.get_untracked();
    let mut edits: Vec<(usize, usize, &str)> = fix
        .edits
        .iter()
        .map(|edit| {
            let from = byte_of_utf16(
                &text,
                utf16_offset_of(&text, edit.range.start_line, edit.range.start_col) as usize,
            );
            let to = byte_of_utf16(
                &text,
                utf16_offset_of(&text, edit.range.end_line, edit.range.end_col) as usize,
            );
            (from, to.max(from), edit.new_text.as_str())
        })
        .collect();
    edits.sort_by_key(|(from, ..)| std::cmp::Reverse(*from));

    record_edit(state);
    let mut new = text.clone();
    for (from, to, replacement) in edits {
        new.replace_range(from..to, replacement);
    }

    echo_edit(state, &new);
    set_buffer(state, area, &new);
    state.editor.actions.set(None);
    controller::schedule_pulse(state);
}

/// Where the identifier under the caret begins, for Ctrl+Space.
pub(super) fn word_start_before(text: &str, line: u32, col: u32) -> u32 {
    let Some(line_text) = text.split('\n').nth(line as usize) else {
        return col;
    };
    let chars: Vec<char> = line_text.chars().take(col as usize).collect();
    let mut start = chars.len();
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    start as u32
}

/// The word typed since the popup opened — what the list narrows against.
pub(super) fn typed_word(text: &str, line: u32, word_start: u32) -> String {
    text.split('\n')
        .nth(line as usize)
        .map(|line_text| {
            line_text
                .chars()
                .skip(word_start as usize)
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect()
        })
        .unwrap_or_default()
}

/// Apply the chosen completion to the draft.
pub(super) fn accept_completion(
    state: AppState,
    area: &web_sys::HtmlTextAreaElement,
    index: usize,
) {
    let Some(popup) = state.editor.completion.get_untracked() else {
        return;
    };
    let draft = state.editor.draft.get_untracked();
    let word = typed_word(&draft, popup.line, popup.word_start);
    let shown = visible_items(&popup, &draft);
    let Some((_, item)) = shown.get(index.min(shown.len().saturating_sub(1))) else {
        return;
    };

    // Where the replacement starts is the server's to say; where it *ends* is
    // not, and taking the server's end was the bug: rust-analyzer computes
    // the range against the text it had when asked, and the popup stays open
    // while more is typed, filtering locally. Ask on `pe`, type `r`, accept —
    // and the stale range replaced `pe` alone, leaving `peripheralsr`.
    //
    // The end is always the word as it stands now.
    let (start_line, start_col) = match &item.edit {
        Some(edit) => (edit.start_line, edit.start_col),
        None => (popup.line, popup.word_start),
    };
    let (end_line, end_col) = (popup.line, popup.word_start + word.chars().count() as u32);

    record_edit(state);
    let start = byte_of_utf16(
        &draft,
        utf16_offset_of(&draft, start_line, start_col) as usize,
    );
    let end = byte_of_utf16(&draft, utf16_offset_of(&draft, end_line, end_col) as usize);
    let mut text = draft;
    text.replace_range(start.min(end)..end.max(start), &item.insert);

    echo_edit(state, &text);
    set_buffer(state, area, &text);
    let caret = utf16_offset_of(&text, start_line, start_col) + utf16_len(&item.insert);
    let _ = area.set_selection_start(Some(caret));
    let _ = area.set_selection_end(Some(caret));
    state.editor.completion.set(None);
    controller::schedule_pulse(state);

    // An item that was not in scope brings its `use` line — fetched now,
    // applied when it lands. The import goes above the caret, so the caret
    // moves down by what was inserted and stays on the same text.
    let (path, index, element) = (popup.path.clone(), item.index, area.clone());
    controller::resolve_completion(state, path.clone(), index, move |edits| {
        if state.active_path_now().as_deref() == Some(path.as_str()) {
            apply_server_edits(state, &element, &edits);
        }
    });
}

/// Splice edits the server computed against the document as it was when
/// asked — a completion's imports — keeping the caret on the text it was on.
/// Applied bottom-up so earlier ranges stay valid; the caret shifts by the
/// length of every edit that lies wholly before it. An edit whose range
/// falls outside the document is refused whole rather than guessed at.
pub(super) fn apply_server_edits(
    state: AppState,
    area: &web_sys::HtmlTextAreaElement,
    edits: &[rusty_lsp::ActionEdit],
) {
    if edits.is_empty() {
        return;
    }
    let text = state.editor.draft.get_untracked();
    let (caret, _) = doc_selection(area, state);
    let mut spans: Vec<(usize, usize, &str)> = Vec::with_capacity(edits.len());
    for edit in edits {
        let from = byte_of_utf16(
            &text,
            utf16_offset_of(&text, edit.range.start_line, edit.range.start_col) as usize,
        );
        let to = byte_of_utf16(
            &text,
            utf16_offset_of(&text, edit.range.end_line, edit.range.end_col) as usize,
        );
        if from > to || to > text.len() {
            return;
        }
        spans.push((from, to, edit.new_text.as_str()));
    }
    spans.sort_by_key(|(from, ..)| std::cmp::Reverse(*from));

    record_edit(state);
    let mut new = text.clone();
    let mut shift: i64 = 0;
    for (from, to, replacement) in spans {
        new.replace_range(from..to, replacement);
        if to <= caret {
            shift += replacement.len() as i64 - (to - from) as i64;
        }
    }
    let caret = (caret as i64 + shift).clamp(0, new.len() as i64) as usize;
    echo_edit(state, &new);
    set_buffer(state, area, &new);
    let at = utf16_len(&new[..caret]);
    let _ = area.set_selection_start(Some(at));
    let _ = area.set_selection_end(Some(at));
    controller::schedule_pulse(state);
}

/// Whether a cell sits inside a hover range.
pub(super) fn within(range: &rusty_lsp::EditRange, line: u32, col: u32) -> bool {
    if line < range.start_line || line > range.end_line {
        return false;
    }
    if line == range.start_line && col < range.start_col {
        return false;
    }
    if line == range.end_line && col >= range.end_col.max(range.start_col + 1) {
        return false;
    }
    true
}

/// The rows the popup shows for the word typed so far, in the server's
/// ranking, each with its index into that list.
///
/// The one filter, shared by the view that draws the rows, the accept that
/// splices the chosen one and the key handler that decides whether the popup
/// is still up. It was three copies, and the third — `Some` alone — was the
/// bug: a popup narrowed to nothing was invisible yet still ate Enter and
/// Tab, so a line ending in `v.xyz` could not be broken.
pub(super) fn visible_items(
    popup: &crate::state::CompletionPopup,
    draft: &str,
) -> Vec<(usize, CompletionItem)> {
    let word = typed_word(draft, popup.line, popup.word_start).to_lowercase();
    popup
        .items
        .iter()
        .filter(|item| word.is_empty() || item.label.to_lowercase().starts_with(&word))
        .take(50)
        .cloned()
        .enumerate()
        .collect()
}

/// Apply one replacement to the document and place the caret — the path
/// every keystroke the editor types on the browser's behalf goes through:
/// Enter, Tab, a bracket pair, a step over a closer. Offsets are document
/// bytes, as [`doc_selection`] reports them, so the edit is right while
/// something is folded too; `set_buffer` then unfolds, which is what lets
/// the caret be placed on the same text the edit was computed against.
pub(super) fn apply_edit(area: &web_sys::HtmlTextAreaElement, state: AppState, edit: &pairs::Edit) {
    record_edit(state);
    let mut text = state.editor.draft.get_untracked();
    let (from, to) = edit.range;
    if from > to || to > text.len() || !text.is_char_boundary(from) || !text.is_char_boundary(to) {
        return;
    }
    text.replace_range(from..to, &edit.text);
    echo_edit(state, &text);
    set_buffer(state, area, &text);
    let (start, end) = match edit.select {
        Some((a, b)) => (a, b),
        None => (edit.caret, edit.caret),
    };
    let _ = area.set_selection_start(Some(utf16_len(&text[..start.min(text.len())])));
    let _ = area.set_selection_end(Some(utf16_len(&text[..end.min(text.len())])));
    controller::schedule_pulse(state);
}

/// Put `insert` at the caret, replacing any selection, and leave the caret
/// after it.
pub(super) fn insert_at_caret(area: &web_sys::HtmlTextAreaElement, state: AppState, insert: &str) {
    let (from, to) = doc_selection(area, state);
    apply_edit(
        area,
        state,
        &pairs::Edit {
            range: (from, to),
            text: insert.to_string(),
            caret: from + insert.len(),
            select: None,
        },
    );
}
