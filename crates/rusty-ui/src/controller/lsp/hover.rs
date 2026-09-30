//! The hover card: the server's words about the name under the pointer,
//! the problem there, and the quick fixes that hang off it.

use super::*;

/// Ask what quick fixes exist at the caret, after syncing the draft — an
/// answer about stale text splices into the wrong place.
pub fn request_actions(state: AppState, path: String, line: u32, col: u32) {
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let sync = PathText {
        path: path.clone(),
        text: state.editor.draft.get_untracked(),
    };
    let ask = PathAt {
        path: path.clone(),
        line,
        col,
    };
    spawn_local(async move {
        let _ = ipc::call::<_, ()>(cmd::lsp::CHANGE, &sync).await;
        let Ok(answer) = ipc::call::<_, rusty_lsp::CodeActions>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        let current = state.active_path_now();
        if current.as_deref() == Some(path.as_str()) {
            if answer.fixes.is_empty() {
                state.push_log(LogLine {
                    stream: LogStream::Stdout,
                    text: t!("misc.no-quick-fixes"),
                    level: None,
                });
            } else {
                state.editor.actions.set(Some((path, line, answer)));
            }
        }
    });
}

/// Write the part of an accepted quick fix that lands in other files — the
/// `mod` line a fix for an undeclared file puts in its parent module. Those
/// edits land on disk, so a file among them with an unsaved draft refuses
/// the whole fix, by name: the next Ctrl+S there would overwrite them with
/// the draft's stale bytes. Says what changed, as a rename does.
pub fn apply_action_elsewhere(
    state: AppState,
    path: String,
    reply: u64,
    index: u32,
    files: Vec<String>,
) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        reply: u64,
        index: u32,
    }
    if let Some(dirty) = files.iter().find(|file| state.is_dirty(file)) {
        state.push_log(LogLine {
            stream: LogStream::Stderr,
            text: t!("misc.save-first", path = dirty.clone()),
            level: Some(LogLevel::Warn),
        });
        return;
    }
    track(
        state,
        async move {
            ipc::call::<_, Vec<String>>(cmd::lsp::APPLY_ACTION, &Args { path, reply, index }).await
        },
        move |changed| {
            state.push_log(LogLine {
                stream: LogStream::Stdout,
                text: t!("misc.edited-elsewhere", count = changed.len().to_string()),
                level: None,
            });
        },
    );
}

/// Ask what the thing at this position is, for the tooltip.
///
/// Silent on failure and on `None`: hover is ambient, and a banner about a
/// hover would be absurd. The reply is dropped if the user has moved to
/// another file by the time it lands.
pub fn request_hover(state: AppState, path: String, line: u32, col: u32) {
    let args = PathAt {
        path: path.clone(),
        line,
        col,
    };

    // What is wrong here, if anything — read before the server is asked and
    // shown above whatever it says. Over a squiggle the error *is* the
    // question; the type of an expression that does not compile is a footnote
    // to it, and showing only the type reads as an editor that cannot see the
    // red line under the cursor.
    let problem = problem_at(state, &path, line, col);

    spawn_local(async move {
        let info = ipc::call::<_, Option<HoverInfo>>(cmd::lsp::HOVER, &args)
            .await
            .ok()
            .flatten();
        // A card for the diagnostic even when the server has nothing to say
        // about the position, which is common at exactly the places that are
        // broken enough to be underlined.
        if problem.is_none() && info.is_none() {
            return;
        }
        let current = state.active_path_now();
        if current.as_deref() != Some(path.as_str()) {
            return;
        }

        let mut text = String::new();
        // A failed test's mark: the test's name, and the panic's message as
        // printed — in a block, since `left:` and `right:` line up.
        let test_failure = problem
            .as_ref()
            .filter(|(problem, _)| problem.source.as_deref() == Some(TEST_SOURCE));
        if let Some((problem, _)) = test_failure {
            let name = problem.code.clone().unwrap_or_default();
            text.push_str(&format!(
                "**{}**\n\n```text\n{}\n```",
                t!("files.test-failed", name = name),
                problem.message,
            ));
            if info.is_some() {
                text.push_str("\n\n---\n\n");
            }
        } else if let Some((problem, _)) = &problem {
            let label = match problem.severity {
                rusty_lsp::DiagSeverity::Error => "error",
                rusty_lsp::DiagSeverity::Warning => "warning",
                _ => "note",
            };
            match &problem.code {
                Some(code) => text.push_str(&format!("**{label}[{code}]**\n\n")),
                None => text.push_str(&format!("**{label}**\n\n")),
            }
            text.push_str(&problem.message);
            if info.is_some() {
                text.push_str("\n\n---\n\n");
            }
        }
        if let Some(info) = &info {
            text.push_str(&info.text);
        }

        // Where the problem's squiggle is drawn on this line when there is
        // one: it is what the reader pointed at, and it is what "moved away"
        // has to be measured against or the card closes while the pointer
        // is still over the red line — a point's own range is no width at
        // all.
        let range = match (&problem, info.as_ref().and_then(|i| i.range)) {
            (Some((_, (from, to))), _) => rusty_lsp::EditRange {
                start_line: line,
                start_col: *from,
                end_line: line,
                end_col: *to,
            },
            (None, Some(range)) => range,
            // No range from the server means "just this cell" — the card
            // still needs one to decide what counts as moving away.
            (None, None) => rusty_lsp::EditRange {
                start_line: line,
                start_col: col,
                end_line: line,
                end_col: col + 1,
            },
        };
        state.editor.hover.set(Some(HoverCard {
            path: path.clone(),
            range,
            text,
            line,
            col,
            fixes: rusty_lsp::CodeActions::default(),
        }));

        // What the server offers to do about the problem, on the card that
        // names it. Asked only where there *is* a problem: that is where a
        // fix exists, and it is the whole of the reason somebody hovers a
        // red line. An `impl Trait for T {}` with no members is the case
        // this exists for — the fix is rust-analyzer's own "Implement
        // missing members", reachable until now only by putting the caret
        // there and pressing Ctrl+.
        //
        // It arrives *after* the card rather than with it: a `codeAction`
        // resolves every offer it did not come with, which is a second or
        // more on a cold index, and a tooltip that waits for it is a
        // tooltip that does not appear. The buttons grow onto the card
        // that is already up, and only if it is still that card — the
        // pointer moves on while this is in flight. Not for a test's
        // failure: the compiler has nothing to fix about an assertion that
        // did not hold.
        if problem.is_none() || test_failure.is_some() {
            return;
        }
        let ask = PathAt {
            path: path.clone(),
            line,
            col,
        };
        let Ok(answer) = ipc::call::<_, rusty_lsp::CodeActions>(cmd::lsp::ACTIONS, &ask).await
        else {
            return;
        };
        if answer.fixes.is_empty() {
            return;
        }
        state.editor.hover.update(|card| {
            if let Some(card) = card
                && card.path == path
                && (card.line, card.col) == (line, col)
            {
                card.fixes = answer;
            }
        });
    });
}

/// The diagnostic under a position, worst first, with the columns its
/// squiggle covers on that line.
///
/// Errors outrank warnings at the same spot: two squiggles overlap often —
/// an unused import that is also a type error — and the one that stops the
/// build is the one being asked about. Found where the echo *draws* it
/// (`squiggle::drawn_on`), read off the same draft: a problem at the end of
/// a line is no columns wide, and matched by its own range it could never
/// be hovered at all.
fn problem_at(
    state: AppState,
    path: &str,
    line: u32,
    col: u32,
) -> Option<(rusty_lsp::FileDiagnostic, (u32, u32))> {
    let text = state
        .editor
        .draft
        .with_untracked(|draft| draft.split('\n').nth(line as usize).map(str::to_string))?;
    // The compiler's problems and where tests failed: both drawn by the
    // echo, so both found here.
    let mut problems = state
        .lsp
        .diagnostics
        .with_untracked(|by_file| by_file.get(path).cloned())
        .unwrap_or_default();
    problems.extend(state.tests.marks_in_now(path));
    worst_at(&problems, line, col, &text).map(|(diagnostic, span)| (diagnostic.clone(), span))
}

/// The pure half of [`problem_at`], so the ranking is pinned by tests rather
/// than by eye — overlapping squiggles are exactly where it would go wrong.
fn worst_at<'d>(
    diagnostics: &'d [rusty_lsp::FileDiagnostic],
    line: u32,
    col: u32,
    text: &str,
) -> Option<(&'d rusty_lsp::FileDiagnostic, (u32, u32))> {
    let mut found: Option<(&rusty_lsp::FileDiagnostic, (u32, u32))> = None;
    for diagnostic in diagnostics {
        let Some((from, to)) = crate::squiggle::drawn_on(diagnostic, line, text) else {
            continue;
        };
        if !(from..to).contains(&col) {
            continue;
        }
        let better = found.is_none_or(|(best, _)| {
            matches!(diagnostic.severity, rusty_lsp::DiagSeverity::Error)
                && !matches!(best.severity, rusty_lsp::DiagSeverity::Error)
        });
        if better {
            found = Some((diagnostic, (from, to)));
        }
    }
    found
}

#[cfg(test)]
mod hover_tests {
    use super::worst_at;
    use rusty_lsp::{DiagSeverity, FileDiagnostic};

    /// Long enough for every column the tests below point at.
    const LINE: &str = "let value = compute(first, second);";

    fn diag(severity: DiagSeverity, line: u32, from: u32, to: u32) -> FileDiagnostic {
        FileDiagnostic {
            severity,
            message: format!("{severity:?} at {from}..{to}"),
            source: None,
            code: None,
            start_line: line,
            start_col: from,
            end_line: line,
            end_col: to,
        }
    }

    #[test]
    fn only_a_diagnostic_covering_the_position_counts() {
        let diagnostics = [diag(DiagSeverity::Error, 3, 4, 9)];
        assert!(
            worst_at(&diagnostics, 3, 4, LINE).is_some(),
            "the first column is inside"
        );
        assert!(worst_at(&diagnostics, 3, 8, LINE).is_some());
        // Half-open: the end column is where the squiggle stops, so hovering
        // there is hovering past it.
        assert!(worst_at(&diagnostics, 3, 9, LINE).is_none());
        assert!(worst_at(&diagnostics, 3, 3, LINE).is_none());
        assert!(
            worst_at(&diagnostics, 2, 5, LINE).is_none(),
            "another line entirely"
        );
    }

    /// The report: `expected SEMICOLON` is a point at the end of a line,
    /// drawn one cell past it, and the pointer past the end of that line —
    /// which the editor reads as the line's last column — finds it there,
    /// with the drawn cell as the card's span. Matched by its own range of
    /// no width it could never be found.
    #[test]
    fn a_point_at_the_end_of_a_line_is_found_where_it_is_drawn() {
        let line = "    pub const UP: Vector = Vector::new(0.0, 0.0, 1.0)";
        let end = line.chars().count() as u32;
        let diagnostics = [diag(DiagSeverity::Error, 15, end, end)];
        let (found, span) = worst_at(&diagnostics, 15, end, line).expect("drawn past the `)`");
        assert_eq!(found.start_col, end);
        assert_eq!(span, (end, end + 1));
        assert!(
            worst_at(&diagnostics, 15, end - 1, line).is_none(),
            "the `)`"
        );
        // Inside a line a point is drawn over the word it touches, and
        // found anywhere on it.
        let inside = [diag(DiagSeverity::Error, 0, 6, 6)];
        for col in [4, 8] {
            let (_, span) = worst_at(&inside, 0, col, LINE).expect("on `value`");
            assert_eq!(span, (4, 9));
        }
        assert!(
            worst_at(&inside, 0, 9, LINE).is_none(),
            "the space after it"
        );
    }

    #[test]
    fn an_error_outranks_a_warning_at_the_same_place() {
        // Two squiggles overlap often — an unused import that is also a type
        // error — and the one that stops the build is the one being asked
        // about. Both orders, because a first-match loop passes one of them
        // by accident.
        let warning_first = [
            diag(DiagSeverity::Warning, 1, 0, 10),
            diag(DiagSeverity::Error, 1, 2, 6),
        ];
        let error_first = [
            diag(DiagSeverity::Error, 1, 2, 6),
            diag(DiagSeverity::Warning, 1, 0, 10),
        ];
        for diagnostics in [warning_first, error_first] {
            let (found, _) = worst_at(&diagnostics, 1, 4, LINE).expect("one covers this");
            assert!(
                matches!(found.severity, DiagSeverity::Error),
                "the warning won at column 4",
            );
        }
    }

    #[test]
    fn a_warning_still_shows_where_no_error_covers_it() {
        let diagnostics = [
            diag(DiagSeverity::Warning, 1, 0, 10),
            diag(DiagSeverity::Error, 1, 2, 6),
        ];
        let (found, _) = worst_at(&diagnostics, 1, 8, LINE).expect("the warning covers this");
        assert!(matches!(found.severity, DiagSeverity::Warning));
    }
}
