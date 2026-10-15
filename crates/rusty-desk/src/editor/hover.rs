//! The card under the pointer — VS Code's hover: the problems where it
//! rests, worst first, and what the server says about the name there, its
//! signature painted as the editor paints code and its documentation as
//! the Markdown it is written in.
//!
//! Shown a moment after the pointer settles, kept while the pointer moves
//! within the same word or onto the card, put away a moment after it
//! leaves — and at once by any key, since the card is about the text as it
//! was. An answer that lands after the pointer moved on is dropped.

use std::{cell::Cell, rc::Rc, time::Duration};

use rusty_lsp::{FileDiagnostic, HoverInfo};
use slint::{ComponentHandle, ModelRc, StyledText, Timer, TimerMode, VecModel};

use super::{Surface, is_problem, rows};
use crate::{Code, HoverProblem, Row, language};

const SHOW_AFTER: Duration = Duration::from_millis(350);
const HIDE_AFTER: Duration = Duration::from_millis(300);

/// The hover's timers and what it is showing or about to.
#[derive(Default)]
pub struct Hovering {
    show: Timer,
    hide: Timer,
    /// Numbers every ask; an answer to an older one is dropped.
    asked: Cell<u64>,
    /// The line and the byte span the card is about, shown or pending.
    over: Cell<Option<(usize, usize, usize)>>,
    /// Asked for from the keyboard: kept until a key or a press, whatever
    /// the pointer is over — Slint reports a pointer that never moved when
    /// the content under it changes, and the card is such a change.
    from_keys: Cell<bool>,
}

/// The problems whose squiggles cover column `col` of line `line`, whose
/// text is `text`, worst first — found where they are drawn
/// (`rusty_lsp::squiggle`), so a problem at the end of a line is found
/// where its squiggle is.
pub fn problems_at<'a>(
    items: &'a [FileDiagnostic],
    line: u32,
    text: &str,
    col: u32,
) -> Vec<&'a FileDiagnostic> {
    let mut found: Vec<&FileDiagnostic> = items
        .iter()
        .filter(|d| is_problem(d))
        .filter(|d| {
            rusty_lsp::squiggle::drawn_on(d, line, text)
                .is_some_and(|(from, to)| from <= col && col < to)
        })
        .collect();
    found.sort_by_key(|d| d.severity);
    found
}

/// A hover's Markdown taken apart: its code blocks, with the language each
/// is fenced as, and the prose between them.
pub fn split(markdown: &str) -> (Vec<(String, String)>, String) {
    let mut code = Vec::new();
    let mut prose = String::new();
    let mut fence: Option<(String, String)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        match &mut fence {
            Some(_) if trimmed.starts_with("```") => {
                let (lang, body) = fence.take().unwrap_or_default();
                code.push((lang, body.trim_end_matches('\n').to_string()));
            }
            Some((_, body)) => {
                body.push_str(line);
                body.push('\n');
            }
            None if trimmed.starts_with("```") => {
                fence = Some((
                    trimmed.trim_start_matches('`').trim().to_string(),
                    String::new(),
                ));
            }
            None => {
                prose.push_str(line);
                prose.push('\n');
            }
        }
    }
    if let Some((lang, body)) = fence {
        code.push((lang, body.trim_end_matches('\n').to_string()));
    }
    // rust-analyzer rules the signature off from the docs with `---`.
    let prose = prose
        .lines()
        .filter(|line| line.trim() != "---")
        .collect::<Vec<_>>()
        .join("\n");
    (code, prose.trim().to_string())
}

impl Surface {
    pub(super) fn wire_hover(self: &Rc<Self>, code: &Code) {
        let weak = Rc::downgrade(self);
        code.on_hovered(move |row, x, y| {
            if let Some(this) = weak.upgrade() {
                this.hovered(row, x, y);
            }
        });
    }

    /// The pointer moved over row `row` (or left the rows, at `-1`).
    fn hovered(self: &Rc<Self>, row: i32, x: f32, y: f32) {
        if self.hovering.from_keys.get() {
            return;
        }
        let spot = (row >= 0)
            .then(|| self.spot_at(row as usize, x, y))
            .flatten();
        let over = self.hovering.over.get();
        match spot {
            // Still on what the card is about.
            Some((line, at))
                if over.is_some_and(|(l, from, to)| l == line && from <= at && at <= to) =>
            {
                self.hovering.hide.stop();
            }
            Some((line, at)) => {
                self.hide_hover_soon();
                let weak = Rc::downgrade(self);
                self.hovering
                    .show
                    .start(TimerMode::SingleShot, SHOW_AFTER, move || {
                        if let Some(this) = weak.upgrade() {
                            this.hover_at(line, at);
                        }
                    });
            }
            None => {
                self.hovering.show.stop();
                self.hide_hover_soon();
            }
        }
    }

    /// "Show hover": the card for where the caret is, at once.
    pub fn hover_caret(self: &Rc<Self>) {
        let Some(doc) = self.doc() else { return };
        let (line, at) = {
            let d = doc.borrow();
            let head = d.primary().head;
            (d.lines.line_of(head), head)
        };
        self.hover_at(line, at);
        self.hovering.from_keys.set(true);
    }

    /// The line and the byte under the pointer, when it is over something
    /// written there or a squiggle drawn past the end of the line.
    fn spot_at(&self, row: usize, x: f32, y: f32) -> Option<(usize, usize)> {
        let doc = self.doc()?;
        let window = self.window.upgrade()?;
        let row_height = window.global::<Code>().get_row_height();
        let d = doc.borrow();
        let screen = row as isize + (y / row_height).floor() as isize;
        if screen < 0 || screen as usize >= d.rows() || x < 0.0 {
            return None;
        }
        let line = d.line_at_row(screen as usize);
        let text = d.lines.text(&d.text, line);
        let metrics = self.metrics.borrow();
        // The character under the pointer, not the boundary nearest it.
        let mut col = super::inlays::column_on(&metrics, &d, line, x);
        if col > 0 && x < super::inlays::char_on(&metrics, &d, line, col) {
            col -= 1;
        }
        let past = x > super::inlays::end_on(&metrics, &d, line) + metrics.ascii();
        let squiggled = self
            .problems
            .borrow()
            .get(&d.path)
            .is_some_and(|items| !problems_at(items, line as u32, text, col as u32).is_empty());
        if past && !squiggled {
            return None;
        }
        Some((line, d.lines.byte(&d.text, line, col)))
    }

    /// Show the card for the byte `at` on `line`: the problems there at
    /// once, and the server's account of the name when it comes.
    fn hover_at(self: &Rc<Self>, line: usize, at: usize) {
        let Some(doc) = self.doc() else { return };
        let (path, col, problems, word, version, draft_text) = {
            let d = doc.borrow();
            let text = d.lines.text(&d.text, line);
            let col = d.lines.position(&d.text, at).1;
            let problems: Vec<FileDiagnostic> = self
                .problems
                .borrow()
                .get(&d.path)
                .map(|items| {
                    problems_at(items, line as u32, text, col as u32)
                        .into_iter()
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let word = super::cursors::word_at(&d.text, at)
                .filter(|&(from, to)| from <= at && at < to.max(from + 1));
            (
                d.path.clone(),
                col,
                problems,
                word,
                d.version,
                d.text.clone(),
            )
        };
        let asked = self.hovering.asked.get() + 1;
        self.hovering.asked.set(asked);
        let (from, to) = word.unwrap_or((at, at));
        self.hovering.over.set(Some((line, from, to)));
        self.hovering.hide.stop();
        // The problems now; the server's account joins them when it comes.
        if problems.is_empty() {
            self.close_card();
        } else {
            self.show_card(line, from, &problems, None);
        }
        if word.is_some() {
            let draft = self.servers.draft(&draft_text);
            self.servers.ask(
                &path,
                Some(draft),
                move |client, path| client.hover(path, line as u32, col as u32),
                move |app, info| app.editor().hover_answer(asked, version, line, info),
            );
        } else if problems.is_empty() {
            self.hide_hover();
        }
    }

    /// The server's answer, shown when it is still the card being asked
    /// for, over the same text.
    pub fn hover_answer(&self, asked: u64, version: u64, line: usize, info: Option<HoverInfo>) {
        let Some(doc) = self.doc() else { return };
        if asked != self.hovering.asked.get() || doc.borrow().version != version {
            return;
        }
        let Some((_, from, _)) = self.hovering.over.get() else {
            return;
        };
        // The card is drawn again with the answer beside the problems
        // already on it, which are read afresh: a server may have spoken
        // since.
        let problems: Vec<FileDiagnostic> = {
            let d = doc.borrow();
            let text = d.lines.text(&d.text, line);
            let col = d.lines.position(&d.text, from).1;
            self.problems
                .borrow()
                .get(&d.path)
                .map(|items| {
                    problems_at(items, line as u32, text, col as u32)
                        .into_iter()
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        match info.filter(|info| !info.text.trim().is_empty()) {
            Some(info) => self.show_card(line, from, &problems, Some(&info.text)),
            None if problems.is_empty() => self.hide_hover(),
            None => {}
        }
    }

    fn show_card(&self, line: usize, from: usize, problems: &[FileDiagnostic], said: Option<&str>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(doc) = self.doc() else { return };
        let code = window.global::<Code>();
        let rows_of_problems: Vec<HoverProblem> = problems
            .iter()
            .map(|d| HoverProblem {
                kind: match d.severity {
                    rusty_lsp::DiagSeverity::Error => 1,
                    rusty_lsp::DiagSeverity::Warning => 2,
                    _ => 3,
                },
                text: d.message.as_str().into(),
                detail: match (&d.source, &d.code) {
                    (Some(source), Some(c)) => format!("{source}({c})"),
                    (Some(source), None) => source.clone(),
                    (None, Some(c)) => c.clone(),
                    (None, None) => String::new(),
                }
                .into(),
            })
            .collect();
        let (blocks, prose) = said.map(split).unwrap_or_default();
        let metrics = self.metrics.borrow();
        let mut widest: f32 = 0.0;
        let mut code_rows: Vec<Row> = Vec::new();
        for (lang, body) in &blocks {
            // A block fenced as nothing is the document's own language.
            let lang = if lang.is_empty() {
                language::of(&doc.borrow().path).map_or("text", |l| l.name)
            } else {
                lang.as_str()
            };
            for line in self
                .files
                .highlight_snippet(lang, body)
                .into_iter()
                .take(12)
            {
                let text: String = line.spans.iter().map(|s| s.text.as_str()).collect();
                widest = widest.max(metrics.x_of(&text, usize::MAX));
                code_rows.push(rows::painted_row(&metrics, &line));
            }
        }
        let docs = if prose.is_empty() {
            StyledText::default()
        } else {
            StyledText::from_markdown(&prose)
                .unwrap_or_else(|_| StyledText::from_plain_text(&prose))
        };
        let (line_text_x, has_any, row) = {
            let d = doc.borrow();
            let col = d.lines.position(&d.text, from).1;
            (
                super::inlays::char_on(&metrics, &d, line, col),
                !rows_of_problems.is_empty() || !code_rows.is_empty() || !prose.is_empty(),
                d.row_of(line),
            )
        };
        if !has_any {
            self.close_card();
            return;
        }
        code.set_hover_line(row as i32);
        code.set_hover_x(line_text_x);
        code.set_hover_width((widest + 24.0).clamp(320.0, 620.0));
        code.set_hover_problems(ModelRc::new(VecModel::from(rows_of_problems)));
        code.set_hover_code(ModelRc::new(VecModel::from(code_rows)));
        code.set_hover_has_docs(!prose.is_empty());
        code.set_hover_docs(docs);
        code.set_hover_open(true);
    }

    fn hide_hover_soon(self: &Rc<Self>) {
        if self.hovering.over.get().is_none() {
            return;
        }
        let weak = Rc::downgrade(self);
        self.hovering
            .hide
            .start(TimerMode::SingleShot, HIDE_AFTER, move || {
                if let Some(this) = weak.upgrade() {
                    let on_card = this
                        .window
                        .upgrade()
                        .is_some_and(|w| w.global::<Code>().get_on_card());
                    if !on_card {
                        this.hide_hover();
                    }
                }
            });
    }

    /// The card away, and whatever it was waiting for forgotten.
    pub fn hide_hover(&self) {
        self.hovering.show.stop();
        self.hovering.hide.stop();
        self.hovering.asked.set(self.hovering.asked.get() + 1);
        self.hovering.over.set(None);
        self.hovering.from_keys.set(false);
        self.close_card();
    }

    /// The card off the screen, leaving any question it waits on standing.
    fn close_card(&self) {
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            code.set_hover_open(false);
            code.set_on_card(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_lsp::DiagSeverity;

    fn problem(severity: DiagSeverity, from: u32, to: u32) -> FileDiagnostic {
        FileDiagnostic {
            severity,
            message: format!("{from}..{to}"),
            source: None,
            code: None,
            start_line: 3,
            start_col: from,
            end_line: 3,
            end_col: to,
        }
    }

    /// Worst first, found where drawn: a point past the end of the line is
    /// asked about there, and a hint is nobody's problem.
    #[test]
    fn the_problems_under_the_pointer_are_the_ones_drawn_there() {
        let text = "    let x = 1";
        let items = [
            problem(DiagSeverity::Warning, 8, 9),
            problem(DiagSeverity::Error, 4, 13),
            problem(DiagSeverity::Hint, 8, 9),
            problem(DiagSeverity::Error, 13, 13),
        ];
        let at = |col| {
            problems_at(&items, 3, text, col)
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(at(8), ["4..13", "8..9"]);
        assert_eq!(at(13), ["13..13"]);
        assert!(at(2).is_empty());
        assert!(problems_at(&items, 4, text, 8).is_empty());
    }

    #[test]
    fn a_hover_is_its_code_and_its_prose() {
        let said =
            "```rust\nprobe::Gain\n```\n\n```rust\nkp: f32\n```\n\n---\n\nThe *proportional* gain.";
        let (code, prose) = split(said);
        assert_eq!(
            code,
            [
                ("rust".to_string(), "probe::Gain".to_string()),
                ("rust".to_string(), "kp: f32".to_string())
            ]
        );
        assert_eq!(prose, "The *proportional* gain.");
        let (code, prose) = split("just words");
        assert!(code.is_empty());
        assert_eq!(prose, "just words");
    }
}
