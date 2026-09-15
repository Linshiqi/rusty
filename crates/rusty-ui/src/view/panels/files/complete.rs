//! Accepting what the language server offered — completions and quick fixes
//! — and deciding when to ask it for completions at all.
//!
//! Both kinds of acceptance splice text at a position the server named, so
//! both convert that position into the textarea's own units before touching
//! the buffer.
//!
//! **When to ask is VS Code's rule** ([`ask_for`]): the first letter of a
//! word asks, `.` and `::` ask about what follows them, and the word is asked
//! about again as it grows while the server says its answer is incomplete —
//! which rust-analyzer always does, because its imports are searched by the
//! word typed. It used to ask once per word, on the second letter, and never
//! again: an ask that failed or came back empty while the index warmed up
//! meant no completion for that word at all, and the list stayed frozen at
//! what two letters had found.
//!
//! **What is shown is ranked here** ([`ranked`]): names that start with the
//! word first, in the server's order, then names the word only fuzzily
//! matches — `itr` finds `iter`, `hm` finds `HashMap`. The filter was a
//! prefix on the label alone, and neither found anything.

use leptos::prelude::*;

use rusty_lsp::CompletionItem;

use super::*;
use crate::{controller, state::AppState};

/// How many ranked rows the keyboard can reach. Nine are drawn at a time.
const SHOWN: usize = 50;

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

// ─── when to ask ─────────────────────────────────────────────────────────────

/// What a change to the text asks of the completion popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ask {
    /// Leave it as it is.
    Keep,
    /// Close it, and drop any answer on its way.
    Close,
    /// Ask the server about the word that starts at `word_start`: `now` for
    /// the first ask about a word, after a pause in the typing for a word
    /// that is already showing.
    Complete { word_start: u32, now: bool },
}

/// The popup — or the ask still waiting for its first answer — as the
/// decision needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Showing {
    pub line: u32,
    pub word_start: u32,
    /// The server said its answer was incomplete, or has not answered yet.
    pub incomplete: bool,
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// What typing — or deleting, when `typing` is false — asks of the popup,
/// judged by the text before the caret on its `line`.
///
/// The first letter of a word asks, unless the word is a number; `.` and
/// `::` ask about what follows them. A word already showing is asked about
/// again as it changes while its answer is incomplete, and only narrowed
/// when it is complete. Deleting never opens a popup, but widens one that
/// is open. The popup closes when the caret leaves its word: onto another
/// line, back past where the word starts, or past a character that ends it.
pub(super) fn ask_for(before: &[char], line: u32, showing: Option<Showing>, typing: bool) -> Ask {
    let col = before.len() as u32;
    let word = before.iter().rev().take_while(|ch| is_word(**ch)).count() as u32;
    let start = col - word;
    if let Some(showing) = showing {
        if showing.line != line || col < showing.word_start {
            return Ask::Close;
        }
        if start == showing.word_start {
            return if showing.incomplete {
                Ask::Complete {
                    word_start: start,
                    now: false,
                }
            } else {
                Ask::Keep
            };
        }
    }
    let last = before.last().copied();
    let prev = before.len().checked_sub(2).map(|at| before[at]);
    let opens = match last {
        Some('.') => typing,
        Some(':') => typing && prev == Some(':'),
        Some(ch) if is_word(ch) => typing && !before[start as usize].is_ascii_digit(),
        _ => false,
    };
    if opens {
        let word_start = if word == 0 { col } else { start };
        Ask::Complete {
            word_start,
            now: true,
        }
    } else if showing.is_some() {
        Ask::Close
    } else {
        Ask::Keep
    }
}

// ─── what to show ────────────────────────────────────────────────────────────

/// Where a word starts inside a name: the first character, one after `_`
/// or `:`, or an upper-case letter after a lower-case one — `HashMap`'s `M`.
fn starts_word(chars: &[char], at: usize) -> bool {
    at == 0
        || matches!(chars[at - 1], '_' | ':' | '.' | ' ' | '(' | '<' | '!')
        || (chars[at].is_uppercase() && chars[at - 1].is_lowercase())
}

/// How well `typed` — lower-case — matches `name`, or `None` when it does
/// not. Every typed character must appear in order, the first where a word
/// starts: `itr` finds `iter`, `hm` finds `HashMap`, `iter` finds
/// `into_iter`, and `ln` does not find `println`. A character that follows
/// the one before it scores three, one that starts a word two, one found
/// across a gap nothing, and the best alignment counts.
pub(super) fn fuzzy_score(typed: &[char], name: &str) -> Option<u32> {
    let Some(first) = typed.first() else {
        return Some(0);
    };
    let chars: Vec<char> = name.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|ch| ch.to_lowercase().next().unwrap_or(*ch))
        .collect();
    (0..lower.len())
        .filter(|&at| lower[at] == *first && starts_word(&chars, at))
        .filter_map(|at| follow(&typed[1..], &chars, &lower, at).map(|rest| 2 + rest))
        .max()
}

/// The rest of a fuzzy match, after its first character matched at `at`.
fn follow(rest: &[char], chars: &[char], lower: &[char], at: usize) -> Option<u32> {
    let mut score = 0;
    let mut last = at;
    for want in rest {
        last = if lower.get(last + 1) == Some(want) {
            score += 3;
            last + 1
        } else if let Some(next) =
            (last + 1..lower.len()).find(|&next| lower[next] == *want && starts_word(chars, next))
        {
            score += 2;
            next
        } else {
            (last + 1..lower.len()).find(|&next| lower[next] == *want)?
        };
    }
    Some(score)
}

/// The indices of the items `typed` keeps, best first. Names that start
/// with it come first, in the server's order — rust-analyzer's ranking is
/// good, and a prefix is what most typing is — an exact-case start ahead of
/// another, so `Str` puts `String` before `str`; then the names it only
/// fuzzily matches, the best match first. An item's `filter` stands in for
/// its label when the server gave one.
pub(super) fn ranked(items: &[CompletionItem], typed: &str) -> Vec<usize> {
    if typed.is_empty() {
        return (0..items.len()).collect();
    }
    let lower = typed.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut kept: Vec<((u8, u8, u32), usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(order, item)| {
            let name = item.filter.as_deref().unwrap_or(&item.label);
            if name.to_lowercase().starts_with(&lower) {
                Some(((0, u8::from(!name.starts_with(typed)), 0), order))
            } else {
                fuzzy_score(&chars, name).map(|score| ((1, 0, u32::MAX - score), order))
            }
        })
        .collect();
    kept.sort_unstable();
    kept.into_iter().map(|(_, order)| order).collect()
}

/// The rows the popup shows for the word typed so far, best first, each
/// with its index into that list.
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
    let word = typed_word(draft, popup.line, popup.word_start);
    ranked(&popup.items, &word)
        .into_iter()
        .take(SHOWN)
        .map(|at| popup.items[at].clone())
        .enumerate()
        .collect()
}

// ─── snippets ────────────────────────────────────────────────────────────────

/// A snippet as the text it inserts, and what to select afterwards: the
/// first placeholder — `${1:x}` — so typing replaces it, else the caret at
/// `$0`, else the end. Offsets are chars into `text`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Expanded {
    pub text: String,
    pub select: (usize, usize),
}

/// Expand an LSP snippet: `$1`, `${1}`, `${1:default}` (nested), `${1|a,b|}`,
/// `$NAME` and `${NAME:default}`, with `\` escaping `$`, `}` and itself.
///
/// Only the first tabstop is honoured — the editor does not walk the rest
/// with Tab — which is why functions are asked for as `name($0)` rather than
/// with their arguments filled in (the client's `callable.snippets`).
/// Continuation lines take `indent`, the indentation of the line the snippet
/// lands on, and a tab is four spaces, as everywhere else in this editor.
pub(super) fn expand_snippet(snippet: &str, indent: &str) -> Expanded {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = Out {
        text: String::new(),
        len: 0,
        indent,
        stops: Vec::new(),
    };
    let mut at = 0;
    parse(&chars, &mut at, &mut out, false);
    let first = out
        .stops
        .iter()
        .filter(|stop| stop.0 > 0)
        .min_by_key(|stop| (stop.0, stop.1))
        .copied();
    let last = out.stops.iter().find(|stop| stop.0 == 0).copied();
    let select = first
        .or(last)
        .map_or((out.len, out.len), |(_, start, end)| (start, end));
    Expanded {
        text: out.text,
        select,
    }
}

/// The text being built, its length in chars, and every tabstop met:
/// its number, where it starts and where it ends.
struct Out<'a> {
    text: String,
    len: usize,
    indent: &'a str,
    stops: Vec<(u32, usize, usize)>,
}

impl Out<'_> {
    fn push(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.text.push('\n');
                self.len += 1;
                for ch in self.indent.chars() {
                    self.text.push(ch);
                    self.len += 1;
                }
            }
            '\t' => {
                self.text.push_str("    ");
                self.len += 4;
            }
            ch => {
                self.text.push(ch);
                self.len += 1;
            }
        }
    }
}

/// Read to the end — or, inside a placeholder, to the `}` that closes it.
fn parse(chars: &[char], at: &mut usize, out: &mut Out, nested: bool) {
    while *at < chars.len() {
        match chars[*at] {
            '\\' if chars
                .get(*at + 1)
                .is_some_and(|next| matches!(next, '$' | '}' | '\\')) =>
            {
                out.push(chars[*at + 1]);
                *at += 2;
            }
            '}' if nested => {
                *at += 1;
                return;
            }
            '$' => {
                *at += 1;
                dollar(chars, at, out);
            }
            ch => {
                out.push(ch);
                *at += 1;
            }
        }
    }
}

/// After a `$`: a tabstop, a placeholder, a choice, a variable — or a dollar
/// that starts none of them, which is a dollar.
fn dollar(chars: &[char], at: &mut usize, out: &mut Out) {
    match chars.get(*at) {
        Some(ch) if ch.is_ascii_digit() => {
            let number = digits(chars, at).unwrap_or(0);
            out.stops.push((number, out.len, out.len));
        }
        Some('{') => {
            *at += 1;
            if let Some(number) = digits(chars, at) {
                let start = out.len;
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        parse(chars, at, out, true);
                    }
                    Some('|') => {
                        *at += 1;
                        choice(chars, at, out);
                    }
                    Some('}') => *at += 1,
                    _ => {}
                }
                out.stops.push((number, start, out.len));
            } else if variable(chars, at) {
                // rusty knows no variables; one with a default is its default.
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        parse(chars, at, out, true);
                    }
                    Some('}') => *at += 1,
                    _ => {}
                }
            } else {
                out.push('$');
                out.push('{');
            }
        }
        Some(ch) if ch.is_ascii_alphabetic() || *ch == '_' => {
            variable(chars, at);
        }
        _ => out.push('$'),
    }
}

/// `${1|one,two|}`: the first choice, as text.
fn choice(chars: &[char], at: &mut usize, out: &mut Out) {
    let mut first = true;
    while *at < chars.len() {
        match chars[*at] {
            '\\' if *at + 1 < chars.len() => {
                if first {
                    out.push(chars[*at + 1]);
                }
                *at += 2;
            }
            ',' => {
                first = false;
                *at += 1;
            }
            '|' if chars.get(*at + 1) == Some(&'}') => {
                *at += 2;
                return;
            }
            ch => {
                if first {
                    out.push(ch);
                }
                *at += 1;
            }
        }
    }
}

fn digits(chars: &[char], at: &mut usize) -> Option<u32> {
    let from = *at;
    while *at < chars.len() && chars[*at].is_ascii_digit() {
        *at += 1;
    }
    if *at == from {
        return None;
    }
    chars[from..*at].iter().collect::<String>().parse().ok()
}

fn variable(chars: &[char], at: &mut usize) -> bool {
    let from = *at;
    while *at < chars.len() && (chars[*at].is_ascii_alphanumeric() || chars[*at] == '_') {
        *at += 1;
    }
    *at > from
}

// ─── accepting ───────────────────────────────────────────────────────────────

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

    // A snippet is expanded here — `push($0)` becomes `push()` with the caret
    // between the parentheses — its continuation lines indented like the line
    // it lands on.
    let indent: String = draft
        .split('\n')
        .nth(start_line as usize)
        .unwrap_or_default()
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    let (insert, select) = if item.snippet {
        let expanded = expand_snippet(&item.insert, &indent);
        (expanded.text, expanded.select)
    } else {
        let end = item.insert.chars().count();
        (item.insert.clone(), (end, end))
    };

    record_edit(state);
    let start = byte_of_utf16(
        &draft,
        utf16_offset_of(&draft, start_line, start_col) as usize,
    );
    let end = byte_of_utf16(&draft, utf16_offset_of(&draft, end_line, end_col) as usize);
    let mut text = draft;
    text.replace_range(start.min(end)..end.max(start), &insert);

    echo_edit(state, &text);
    set_buffer(state, area, &text);
    let base = utf16_offset_of(&text, start_line, start_col);
    let at = |chars: usize| base + utf16_len(&insert.chars().take(chars).collect::<String>());
    let _ = area.set_selection_start(Some(at(select.0)));
    let _ = area.set_selection_end(Some(at(select.1)));
    controller::dismiss_completion(state);
    controller::schedule_pulse(state);

    // A call's parentheses were just opened: say what goes in them, as
    // typing the `(` would have.
    let before_caret: String = insert.chars().take(select.0).collect();
    if before_caret.ends_with('(') && !before_caret.contains('\n') {
        controller::request_signature(
            state,
            popup.path.clone(),
            start_line,
            start_col + select.0 as u32,
        );
    }

    // An item that was not in scope brings its `use` line — fetched now,
    // applied when it lands. The import goes above the caret, so the caret
    // moves down by what was inserted and stays on the same text.
    let (path, reply, index, element) = (popup.path.clone(), popup.reply, item.index, area.clone());
    controller::resolve_completion(state, path.clone(), reply, index, move |edits| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            kind: None,
            detail: None,
            insert: label.into(),
            edit: None,
            index: 0,
            label_detail: None,
            filter: None,
            snippet: false,
            description: None,
        }
    }

    fn names(items: &[CompletionItem], typed: &str) -> Vec<String> {
        ranked(items, typed)
            .into_iter()
            .map(|at| items[at].label.clone())
            .collect()
    }

    #[test]
    fn the_first_letter_asks_and_the_word_keeps_asking_while_incomplete() {
        let now = |word_start| Ask::Complete {
            word_start,
            now: true,
        };
        let later = |word_start| Ask::Complete {
            word_start,
            now: false,
        };
        // Nothing showing: a letter opens it, `.` and `::` open it after them.
        assert_eq!(ask_for(&chars("    l"), 3, None, true), now(4));
        assert_eq!(ask_for(&chars("v."), 3, None, true), now(2));
        assert_eq!(ask_for(&chars("std::"), 3, None, true), now(5));
        assert_eq!(ask_for(&chars("std:"), 3, None, true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("x = 1"), 3, None, true),
            Ask::Keep,
            "a number"
        );
        assert_eq!(ask_for(&chars("let "), 3, None, true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("pri"), 3, None, false),
            Ask::Keep,
            "deleting never opens one"
        );

        // Showing: the same word asks again after a pause while incomplete,
        // and is only narrowed once complete.
        let open = Showing {
            line: 3,
            word_start: 4,
            incomplete: true,
        };
        assert_eq!(ask_for(&chars("    pr"), 3, Some(open), true), later(4));
        let done = Showing {
            incomplete: false,
            ..open
        };
        assert_eq!(ask_for(&chars("    pr"), 3, Some(done), true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("    p"), 3, Some(open), false),
            later(4),
            "deleting inside the word widens it"
        );

        // The caret leaves the word: back past its start, onto another line,
        // or past a character that ends it.
        assert_eq!(ask_for(&chars("   "), 3, Some(open), false), Ask::Close);
        assert_eq!(ask_for(&chars("    pr"), 4, Some(open), true), Ask::Close);
        assert_eq!(ask_for(&chars("    pr "), 3, Some(open), true), Ask::Close);
        assert_eq!(ask_for(&chars("    pr("), 3, Some(open), true), Ask::Close);
        // A `.` after the word asks about what follows it instead.
        assert_eq!(ask_for(&chars("    pr."), 3, Some(open), true), now(7));
        // Right after the `.`, the popup's word is empty and still its own.
        let dot = Showing {
            line: 0,
            word_start: 2,
            incomplete: false,
        };
        assert_eq!(ask_for(&chars("v."), 0, Some(dot), false), Ask::Keep);
    }

    #[test]
    fn a_prefix_keeps_the_servers_order_and_fuzzy_matches_follow_it() {
        let items: Vec<CompletionItem> = [
            "print",
            "println",
            "eprintln",
            "iter",
            "into_iter",
            "HashMap",
            "hash",
            "String",
            "str",
        ]
        .into_iter()
        .map(item)
        .collect();
        assert_eq!(
            names(&items, "pr"),
            ["print", "println"],
            "`p` inside a word is not where a match may start"
        );
        assert_eq!(names(&items, "iter"), ["iter", "into_iter"]);
        assert_eq!(names(&items, "itr"), ["iter", "into_iter"]);
        assert_eq!(names(&items, "hm"), ["HashMap"]);
        assert_eq!(
            names(&items, "Str"),
            ["String", "str"],
            "an exact-case start first"
        );
        assert_eq!(names(&items, "str"), ["str", "String"]);
        assert!(names(&items, "ln").is_empty());
        assert_eq!(
            names(&items, "").len(),
            items.len(),
            "no word keeps everything"
        );
    }

    #[test]
    fn an_item_is_matched_by_its_filter_text_when_it_has_one() {
        let mut postfix = item("if expr {}");
        postfix.filter = Some("if".into());
        let items = vec![item("iter"), postfix];
        assert_eq!(names(&items, "if"), ["if expr {}"]);
    }

    #[test]
    fn a_snippet_expands_to_its_text_with_the_first_placeholder_selected() {
        let e = expand_snippet("push($0)", "");
        assert_eq!((e.text.as_str(), e.select), ("push()", (5, 5)));
        let e = expand_snippet("if ${1:cond} {\n\t$0\n}", "    ");
        assert_eq!(e.text, "if cond {\n        \n    }");
        assert_eq!(e.select, (3, 7), "selected, so typing replaces it");
        let e = expand_snippet("${1:foo(${2:x})}$0", "");
        assert_eq!((e.text.as_str(), e.select), ("foo(x)", (0, 6)));
        let e = expand_snippet("${2:b}${1:a}", "");
        assert_eq!(
            (e.text.as_str(), e.select),
            ("ba", (1, 2)),
            "the lowest number first, wherever it is"
        );
        let e = expand_snippet("${1|one,two|}", "");
        assert_eq!((e.text.as_str(), e.select), ("one", (0, 3)));
        let e = expand_snippet(r"cost \$5 \} done", "");
        assert_eq!(
            (e.text.as_str(), e.select),
            ("cost $5 } done", (14, 14)),
            "escapes are text, and no stop is the end"
        );
        let e = expand_snippet("${TM_SELECTED_TEXT:it}$0", "");
        assert_eq!((e.text.as_str(), e.select), ("it", (2, 2)));
        assert_eq!(expand_snippet("a $ b", "").text, "a $ b");
    }
}
