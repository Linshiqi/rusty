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
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    let lines = (count > SEMANTIC_WHOLE_LINES).then(|| {
        let (from, to) = state.editor.drawn_lines.get_value();
        (
            from.saturating_sub(SEMANTIC_MARGIN),
            to.saturating_add(SEMANTIC_MARGIN).min(count),
        )
    });
    let args = Args {
        path: path.clone(),
        lines,
    };
    spawn_local(async move {
        // Errors and empties are the warm-up talking; the lexical base colour
        // stays up either way, so there is nothing to report.
        let Ok(mut spans) =
            ipc::call::<_, Vec<rusty_lsp::SemanticSpan>>(cmd::lsp::SEMANTIC, &args).await
        else {
            return;
        };
        // In order, which the echo finds a line's spans by halving.
        spans.sort_by_key(|span| (span.line, span.start_col));
        let current = state.active_path_now();
        if current.as_deref() == Some(path.as_str()) && !spans.is_empty() {
            state.editor.semantic.set(Some((path, spans)));
            state.editor.semantic_lines.set_value(lines);
        }
    });
}

/// Ask for the inlay hints over the lines of this group's file on screen —
/// the whole file when it is short enough to be asked about whole, as its
/// semantic colours are — when hints are on.
pub fn request_hints(state: AppState, path: String) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        from: u32,
        to: u32,
    }

    if !path.ends_with(".rs")
        || state.lsp.status.get_untracked() != LspStatus::Ready
        || !state.editor.view.with_untracked(|view| view.inlay_hints)
    {
        return;
    }
    let count = state.editor.highlighted.with_untracked(Vec::len) as u32;
    let (from, to) = if count > SEMANTIC_WHOLE_LINES {
        let (from, to) = state.editor.drawn_lines.get_value();
        (
            from.saturating_sub(SEMANTIC_MARGIN),
            to.saturating_add(SEMANTIC_MARGIN).min(count),
        )
    } else {
        (0, count)
    };
    let args = Args {
        path: path.clone(),
        from,
        to,
    };
    // What the server was asked about: an answer that lands after more
    // typing is about this text, and is carried to the one on screen.
    let asked = state.editor.draft.get_untracked();
    spawn_local(async move {
        // The warm-up answers with errors and empties; the hints on screen
        // stay until an answer replaces them.
        let Ok(mut hints) =
            ipc::call::<_, Vec<rusty_lsp::InlayHint>>(cmd::lsp::INLAY_HINTS, &args).await
        else {
            return;
        };
        if state.active_path_now().as_deref() != Some(path.as_str()) {
            return;
        }
        let Some(now) = state.editor.draft.try_get_untracked() else {
            return;
        };
        crate::inlay::follow(&mut hints, &asked, &now);
        hints.sort_by_key(|hint| (hint.line, hint.col));
        let _ = state.editor.hints.try_set(Some(crate::state::HintSet {
            path,
            lines: (from, to),
            hints,
        }));
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
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    static ASKED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    if !path.ends_with(".rs") || state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let asked = ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let args = Args {
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
