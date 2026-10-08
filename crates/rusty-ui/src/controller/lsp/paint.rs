//! What the server paints over the text: semantic colours, inlay hints and
//! the other places the name under the caret occurs.

use super::*;

/// Ask for the document's semantic colouring, and keep it only if the answer
/// still describes what is on screen.
pub fn request_semantic(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        lines: Option<(u32, u32)>,
    }

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let lines = semantic_lines(state);
    let args = Args {
        path: path.clone(),
        lines,
    };
    spawn_local(async move {
        // Errors and empties are the warm-up talking; the lexical base colour
        // stays up either way, so there is nothing to report.
        if let Ok(spans) =
            ipc::call::<_, Vec<rusty_lsp::SemanticSpan>>(cmd::lsp::SEMANTIC, &args).await
        {
            take_semantic(state, path, lines, spans);
        }
    });
}

/// The lines a semantic ask covers: the whole file (`None`) when it is short
/// enough, the lines on screen and a margin either side when it is not.
fn semantic_lines(state: AppState) -> Option<(u32, u32)> {
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    (count > SEMANTIC_WHOLE_LINES).then(|| around_drawn(state, count))
}

/// The lines on screen and [`SEMANTIC_MARGIN`] either side, within the file.
fn around_drawn(state: AppState, count: u32) -> (u32, u32) {
    let (from, to) = state.editor.drawn_lines.get_value();
    (
        from.saturating_sub(SEMANTIC_MARGIN),
        to.saturating_add(SEMANTIC_MARGIN).min(count),
    )
}

/// Put an answer's semantic colours on screen, while its file is.
fn take_semantic(
    state: AppState,
    path: String,
    lines: Option<(u32, u32)>,
    mut spans: Vec<rusty_lsp::SemanticSpan>,
) {
    // In order, which the echo finds a line's spans by halving.
    spans.sort_by_key(|span| (span.line, span.start_col));
    let current = state.active_path_now();
    if current.as_deref() == Some(path.as_str()) && !spans.is_empty() {
        let _ = state.editor.semantic.try_set(Some((path, spans)));
        state.editor.semantic_lines.set_value(lines);
    }
}

/// Ask for the inlay hints over the lines of this group's file on screen —
/// the whole file when it is short enough to be asked about whole, as its
/// semantic colours are — when hints are on.
///
/// **An answer is drawn only over the text it is about.** A hint is a line
/// and a column; the answer names its text (`InlayHints::about`) and is
/// carried from the text asked over to the one on screen. The server's
/// text is not always the one asked over — a change on its way, typing the
/// pulse has not sent yet — and an answer about another text, drawn anyway,
/// put every hint below an edit a line off its code and left it there until
/// the next edit: reported after two lines were replaced by one. Such an
/// answer is asked for again, this time carrying the text on screen.
pub fn request_hints(state: AppState, path: String) {
    ask_hints(state, path, false);
}

/// [`request_hints`], with whether the ask carries the text on screen
/// (`rusty_lsp::Draft`) — which only the second ask does, so a server that
/// keeps another text cannot be asked for ever, and a scroll does not send
/// the whole file.
fn ask_hints(state: AppState, path: String, carry: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        from: u32,
        to: u32,
        draft: Option<rusty_lsp::Draft>,
    }

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let Some((from, to)) = hint_lines(state) else {
        return;
    };
    // What the server was asked about: an answer that lands after more
    // typing is about this text, and is carried to the one on screen.
    let asked = state.editor.draft.get_untracked();
    let args = Args {
        path: path.clone(),
        from,
        to,
        draft: carry.then(|| draft(asked.clone())),
    };
    spawn_local(async move {
        // The warm-up answers with errors and empties; the hints on screen
        // stay until an answer replaces them.
        if let Ok(answer) =
            ipc::call::<_, rusty_lsp::InlayHints>(cmd::lsp::INLAY_HINTS, &args).await
        {
            take_hints(state, path, (from, to), &asked, answer, !carry);
        }
    });
}

/// The lines a hints ask covers — the whole file when it is short enough to
/// be asked about whole, as its semantic colours are — or `None` with hints
/// off.
fn hint_lines(state: AppState) -> Option<(u32, u32)> {
    if !state.editor.view.with_untracked(|view| view.inlay_hints) {
        return None;
    }
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    Some(if count > SEMANTIC_WHOLE_LINES {
        around_drawn(state, count)
    } else {
        (0, count)
    })
}

/// Put an answer's hints on screen, carried from `asked` to the text there
/// now — when the answer is about `asked`. One about another text is asked
/// for again, carrying the text on screen, when `again` says it may be.
fn take_hints(
    state: AppState,
    path: String,
    lines: (u32, u32),
    asked: &str,
    answer: rusty_lsp::InlayHints,
    again: bool,
) {
    if state.active_path_now().as_deref() != Some(path.as_str()) {
        return;
    }
    if answer.about != rusty_lsp::text_mark(asked) {
        // About another text. The hints on screen have moved with every
        // edit and stand where they should; these would not.
        if again {
            ask_hints(state, path, true);
        }
        return;
    }
    let Some(now) = state.editor.draft.try_get_untracked() else {
        return;
    };
    let mut hints = answer.hints;
    crate::inlay::follow(&mut hints, asked, &now);
    hints.sort_by_key(|hint| (hint.line, hint.col));
    let _ = state
        .editor
        .hints
        .try_set(Some(crate::state::HintSet { path, lines, hints }));
}

/// The pulse's questions, in one command with the edit they follow
/// (`lsp_painted`): the draft, then the colours and the hints over the lines
/// this group is drawing. Two commands — a change, then the questions — were
/// two tasks on the backend, and each group's questions could reach the
/// server ahead of the change, or another flow's older change after it.
pub(super) fn request_painted(state: AppState, path: String, draft: rusty_lsp::Draft) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        draft: rusty_lsp::Draft,
        lines: Option<(u32, u32)>,
        hints: Option<(u32, u32)>,
    }

    let lines = semantic_lines(state);
    let hints = hint_lines(state);
    let asked = draft.text.clone();
    let args = Args {
        path: path.clone(),
        draft,
        lines,
        hints,
    };
    spawn_local(async move {
        let Ok(answer) = ipc::call::<_, rusty_lsp::Painted>(cmd::lsp::PAINTED, &args).await else {
            return;
        };
        if let Some(spans) = answer.semantic {
            take_semantic(state, path.clone(), lines, spans);
        }
        if let (Some(range), Some(found)) = (hints, answer.hints) {
            take_hints(state, path, range, &asked, found, true);
        }
    });
}

/// Whether the hints on hand cover document lines `from..to` of the file on
/// screen — any answer, for a file short enough to be asked about whole.
pub fn hints_cover(state: AppState, from: u32, to: u32) -> bool {
    let path = state.active_path_now();
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    state.editor.hints.with_untracked(|hints| {
        hints.as_ref().is_some_and(|set| {
            Some(&set.path) == path.as_ref()
                && (count <= SEMANTIC_WHOLE_LINES || (set.lines.0 <= from && to <= set.lines.1))
        })
    })
}

/// Where the name at this position occurs in its file, for the editor to
/// mark. Only the latest ask is answered: the caret has moved on from the
/// others.
pub fn request_highlights(state: AppState, path: String, line: u32, col: u32) {
    static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let asked = ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let args = PathAt {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let answer = ipc::call::<_, Vec<rusty_lsp::EditRange>>(cmd::lsp::HIGHLIGHTS, &args).await;
        if ASKED.load(std::sync::atomic::Ordering::Relaxed) != asked
            || state.active_path_now().as_deref() != Some(path.as_str())
        {
            return;
        }
        let ranges = answer.unwrap_or_default();
        let _ = state
            .editor
            .occurrences
            .try_set((!ranges.is_empty()).then_some((path, ranges)));
    });
}

/// Files longer than this are asked for their semantic colours around the
/// lines on screen rather than whole.
const SEMANTIC_WHOLE_LINES: u32 = 3_000;

/// How many lines either side of the drawn ones such a request covers, so a
/// scroll of a screen or two stays inside the answer.
const SEMANTIC_MARGIN: u32 = 400;

/// Whether the semantic colours on hand cover document lines `from..to` —
/// always, for a file short enough to be asked about whole.
pub fn semantic_covers(state: AppState, from: u32, to: u32) -> bool {
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    count <= SEMANTIC_WHOLE_LINES
        || state
            .editor
            .semantic_lines
            .get_value()
            .is_some_and(|(first, end)| first <= from && to <= end)
}
