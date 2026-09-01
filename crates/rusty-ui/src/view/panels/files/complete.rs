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
    state.editor.draft.set(new.clone());
    area.set_value(&new);
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
    let shown: Vec<&CompletionItem> = popup
        .items
        .iter()
        .filter(|item| {
            word.is_empty() || item.label.to_lowercase().starts_with(&word.to_lowercase())
        })
        .collect();
    let Some(item) = shown.get(index.min(shown.len().saturating_sub(1))).copied() else {
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
    state.editor.draft.set(text.clone());
    area.set_value(&text);
    let caret = utf16_offset_of(&text, start_line, start_col) + utf16_len(&item.insert);
    let _ = area.set_selection_start(Some(caret));
    let _ = area.set_selection_end(Some(caret));
    state.editor.completion.set(None);
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

/// Put `insert` at the caret and leave the caret after it.
pub(super) fn insert_at_caret(area: &web_sys::HtmlTextAreaElement, state: AppState, insert: &str) {
    record_edit(state);
    let start_units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    let end_units = area.selection_end().ok().flatten().unwrap_or(0) as usize;
    let mut text = state.editor.draft.get_untracked();
    let start = byte_of_utf16(&text, start_units);
    let end = byte_of_utf16(&text, end_units).max(start);

    text.replace_range(start..end, insert);
    echo_edit(state, &text);
    state.editor.draft.set(text.clone());
    area.set_value(&text);
    let at = start_units as u32 + utf16_len(insert);
    let _ = area.set_selection_start(Some(at));
    let _ = area.set_selection_end(Some(at));
    controller::schedule_pulse(state);
}

/// What pressing Enter at `caret` should insert: a newline and the indentation
/// the next line starts with.
///
/// The current line's leading whitespace, plus one level when the caret sits
/// right after an opening bracket — the two rules that cover nearly every
/// Enter press in Rust. Anything cleverer belongs to the language server.
pub(super) fn newline_indent(text: &str, caret: usize) -> String {
    let before = &text[..caret.min(text.len())];
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let line = &before[line_start..];
    let indent: String = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();

    let deeper = matches!(line.trim_end().chars().last(), Some('{' | '(' | '['));
    if deeper {
        format!("\n{indent}    ")
    } else {
        format!("\n{indent}")
    }
}

#[cfg(test)]
mod tests {
    use super::newline_indent;

    #[test]
    fn enter_copies_the_indent_and_deepens_after_an_opener() {
        let text = "fn main() {\n    let x = 1;\n";
        // After the opening brace: one level deeper.
        assert_eq!(newline_indent(text, 11), "\n    ");
        // After the statement: same level.
        assert_eq!(newline_indent(text, text.len()), "\n");
        let nested = "    if x {\n";
        assert_eq!(newline_indent(nested, nested.len() - 1), "\n        ");
    }
}
