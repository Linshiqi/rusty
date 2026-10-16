//! A failed test's message, on the line where it failed — VS Code's test
//! message: the red line every problem gets under the place it panicked,
//! the message's first line after everything drawn on that line, dim, and
//! the whole message on the hover card.
//!
//! **A mark is about the text as it was when the test ran**, so it does not
//! follow an edit, as a problem's squiggle does: an edit to the file takes
//! the file's marks away, and a file edited while a run is going — after
//! its drafts were written, while cargo built what the disk held — is
//! given none by that run, since the place it names may be a line that has
//! moved. A new run takes away what an earlier run marked for each test it
//! gives a verdict, and keeps the rest, as the lenses keep their verdicts:
//! a test that passes now loses its mark, a test this run did not run keeps
//! its own.
//!
//! **Not a problem.** The marks are kept here, beside the lenses' verdicts
//! in the margin every view shares, and never in what the servers found,
//! which the Problems panel lists: Problems is what stops the build.

use std::collections::HashSet;

use rusty_lsp::{DiagSeverity, FileDiagnostic};

use super::{Document, Metrics, Surface, inlays};

/// Where a failed test stopped, in a file as the project names it, and
/// what it said there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    /// The harness's name for the test.
    pub test: String,
    pub path: String,
    /// Zero-based.
    pub line: u32,
    pub col: u32,
    /// Every line of it.
    pub message: String,
}

impl Failed {
    /// The message's first line, drawn after the code.
    pub fn first_line(&self) -> &str {
        self.message.lines().next().unwrap_or_default().trim_end()
    }

    /// The mark as a problem of no width at the place — drawn, and found
    /// under the pointer, where every other problem's squiggle is
    /// (`rusty_lsp::squiggle::drawn_on`): on the word the place touches.
    pub fn diagnostic(&self) -> FileDiagnostic {
        FileDiagnostic {
            severity: DiagSeverity::Error,
            message: self.message.clone(),
            source: None,
            code: None,
            start_line: self.line,
            start_col: self.col,
            end_line: self.line,
            end_col: self.col,
        }
    }
}

/// Every failed test's mark, and what decides which stay.
#[derive(Debug, Default)]
pub struct Failures {
    /// Each with the run that marked it.
    marks: Vec<(Failed, u64)>,
    /// The run whose verdicts are being read.
    run: u64,
    /// The files edited since that run began.
    edited: HashSet<String>,
}

impl Failures {
    /// A run begins: what it says is about the text as it is now.
    pub fn began(&mut self) {
        self.run += 1;
        self.edited.clear();
    }

    /// `test` came out somehow in this run: what an earlier run marked for
    /// it goes. Whether anything did.
    pub fn verdict(&mut self, test: &str) -> bool {
        let run = self.run;
        let before = self.marks.len();
        self.marks
            .retain(|(mark, marked)| mark.test != test || *marked == run);
        self.marks.len() != before
    }

    /// `mark` failed in this run, unless its file was edited since the run
    /// began. Whether it was marked.
    pub fn failed(&mut self, mark: Failed) -> bool {
        if self.edited.contains(&mark.path) || self.marks.iter().any(|(m, _)| *m == mark) {
            return false;
        }
        self.marks.push((mark, self.run));
        true
    }

    /// `path` was edited: its marks go, and a run going now marks it no
    /// more. Whether any went.
    pub fn edited(&mut self, path: &str) -> bool {
        // Every keystroke comes here; the path is kept once.
        if !self.edited.contains(path) {
            self.edited.insert(path.to_string());
        }
        self.forget(|mark| mark.path == path)
    }

    /// Paths that moved: their marks with them, the text being the same.
    pub fn moved(&mut self, from: &str, to: &str) -> bool {
        let mut any = false;
        for (mark, _) in &mut self.marks {
            if let Some(new) = rusty_edit::moved_path(&mark.path, from, to) {
                mark.path = new;
                any = true;
            }
        }
        any
    }

    /// Every mark `gone` names goes. Whether any did.
    pub fn forget(&mut self, gone: impl Fn(&Failed) -> bool) -> bool {
        let before = self.marks.len();
        self.marks.retain(|(mark, _)| !gone(mark));
        self.marks.len() != before
    }

    /// The marks on `line` of `path`, in the order they were made.
    pub fn on<'a>(&'a self, path: &'a str, line: u32) -> impl Iterator<Item = &'a Failed> + 'a {
        self.marks
            .iter()
            .map(|(mark, _)| mark)
            .filter(move |mark| mark.path == path && mark.line == line)
    }
}

/// Where the first message on `line` is drawn and how wide it is: three
/// columns past everything on the line — its text, its hints, and a
/// collapsed block's ellipsis — in the code's own font, so the x the rows
/// draw it at and the x the pointer finds it at are one.
pub fn message_place(metrics: &Metrics, doc: &Document, line: usize, text: &str) -> (f32, f32) {
    let end = inlays::end_on(metrics, doc, line);
    let ascii = metrics.ascii();
    // The ellipsis is a character and ten pixels of padding, an ascii after
    // the end (`fold_x`).
    let after = if doc.folds.is_folded(line as u32) {
        end + ascii + ascii + 10.0
    } else {
        end
    };
    (after + ascii * 3.0, metrics.x_of(text, usize::MAX))
}

impl Surface {
    /// A test run begins, every draft written: one still unsaved — typed
    /// into while the drafts were written — is not the text the run
    /// builds, and is given no mark by it.
    pub fn tests_began(&self) {
        let mut gutter = self.gutter.borrow_mut();
        gutter.failures.began();
        for (path, doc) in self.open.borrow().iter() {
            if doc.borrow().dirty() {
                gutter.failures.edited(path);
            }
        }
    }

    /// A test came out: an earlier run's mark for it goes.
    pub fn test_verdict(&self, test: &str) {
        let changed = self.gutter.borrow_mut().failures.verdict(test);
        if changed {
            self.gutter_changed();
        }
    }

    /// A test failed at a place in the project.
    pub fn test_failed(&self, mark: Failed) {
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, &mark.message);
        }
        let marked = self.gutter.borrow_mut().failures.failed(mark);
        if marked {
            self.gutter_changed();
        }
    }

    /// Every mark goes: another project, whose files these are not.
    pub fn forget_tests(&self) {
        let mut gutter = self.gutter.borrow_mut();
        gutter.verdicts.clear();
        let changed = gutter.failures.forget(|_| true);
        gutter.failures.began();
        drop(gutter);
        if changed {
            self.gutter_changed();
        }
    }

    /// Files the disk changed that no document holds: their marks are
    /// about a text that is not there any more. A document's own follow
    /// the edits made to it.
    pub fn tests_disk_changed(&self, paths: &[String]) {
        // Every path told, not only those up to the first with a mark.
        let mut changed = false;
        {
            let open = self.open.borrow();
            let mut gutter = self.gutter.borrow_mut();
            for path in paths.iter().filter(|path| !open.contains_key(*path)) {
                changed |= gutter.failures.edited(path);
            }
        }
        if changed {
            self.gutter_changed();
        }
    }

    /// `path` was edited: its marks go.
    pub(super) fn failures_edited(&self, path: &str) {
        let changed = self.gutter.borrow_mut().failures.edited(path);
        if changed {
            self.gutter_changed();
        }
    }

    /// The marks under column `col` of `line` in the document on screen,
    /// found where their squiggles are drawn.
    pub(super) fn failures_at(&self, doc: &Document, line: usize, col: usize) -> Vec<Failed> {
        let text = doc.lines.text(&doc.text, line);
        self.gutter
            .borrow()
            .failures
            .on(&doc.path, line as u32)
            .filter(|mark| {
                rusty_lsp::squiggle::drawn_on(&mark.diagnostic(), line as u32, text)
                    .is_some_and(|(from, to)| from as usize <= col && col < to as usize)
            })
            .cloned()
            .collect()
    }

    /// The mark whose message the pointer at `x` is over on `line`, past
    /// the end of it.
    pub(super) fn failure_message_under(
        &self,
        doc: &Document,
        line: usize,
        x: f32,
    ) -> Option<Failed> {
        let gutter = self.gutter.borrow();
        let mark = gutter.failures.on(&doc.path, line as u32).next()?;
        let metrics = self.metrics.borrow();
        let (from, width) = message_place(&metrics, doc, line, mark.first_line());
        (x >= from && x <= from + width).then(|| mark.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(test: &str, path: &str, line: u32) -> Failed {
        Failed {
            test: test.into(),
            path: path.into(),
            line,
            col: 8,
            message: "assertion `left == right` failed\n  left: 2\n right: 3".into(),
        }
    }

    fn lines(failures: &Failures, path: &str) -> Vec<u32> {
        (0..100)
            .filter(|line| failures.on(path, *line).next().is_some())
            .collect()
    }

    #[test]
    fn an_edit_takes_its_files_marks_away_and_no_others() {
        let mut failures = Failures::default();
        failures.began();
        assert!(failures.failed(mark("tests::a", "src/lib.rs", 8)));
        assert!(failures.failed(mark("tests::b", "src/other.rs", 3)));
        assert!(failures.edited("src/lib.rs"));
        assert!(lines(&failures, "src/lib.rs").is_empty());
        assert_eq!(lines(&failures, "src/other.rs"), vec![3]);
        assert!(!failures.edited("src/lib.rs"), "nothing left to take");
    }

    /// The run read the disk before the edit: what it says about that file
    /// is about a text that is gone.
    #[test]
    fn a_file_edited_during_a_run_is_not_marked_by_it() {
        let mut failures = Failures::default();
        failures.began();
        failures.edited("src/lib.rs");
        assert!(!failures.failed(mark("tests::a", "src/lib.rs", 8)));
        assert!(failures.failed(mark("tests::b", "src/other.rs", 3)));
        // The next run is about the text as it is then.
        failures.began();
        assert!(failures.failed(mark("tests::a", "src/lib.rs", 8)));
    }

    #[test]
    fn a_test_that_passes_now_loses_its_mark_and_the_rest_stay() {
        let mut failures = Failures::default();
        failures.began();
        failures.failed(mark("tests::a", "src/lib.rs", 8));
        failures.failed(mark("tests::b", "src/lib.rs", 20));
        // A run of `tests::a` alone, which passes.
        failures.began();
        assert!(failures.verdict("tests::a"));
        assert_eq!(lines(&failures, "src/lib.rs"), vec![20]);
        // Failing again: its verdict takes the old mark, the panic gives a
        // new one, and a second binary's test of the same name in the same
        // run does not take that away.
        failures.began();
        failures.verdict("tests::b");
        failures.failed(mark("tests::b", "src/lib.rs", 21));
        assert!(!failures.verdict("tests::b"), "this run's own mark stays");
        assert_eq!(lines(&failures, "src/lib.rs"), vec![21]);
    }

    #[test]
    fn a_mark_follows_its_file_when_it_moves() {
        let mut failures = Failures::default();
        failures.began();
        failures.failed(mark("tests::a", "src/lib.rs", 8));
        assert!(failures.moved("src", "core/src"));
        assert_eq!(lines(&failures, "core/src/lib.rs"), vec![8]);
        assert!(lines(&failures, "src/lib.rs").is_empty());
        // Deleted with the folder it is in.
        assert!(failures.forget(|m| rusty_edit::is_within(&m.path, "core")));
        assert!(lines(&failures, "core/src/lib.rs").is_empty());
    }

    #[test]
    fn the_message_is_its_first_line_after_the_code() {
        let failed = mark("tests::a", "src/lib.rs", 8);
        assert_eq!(failed.first_line(), "assertion `left == right` failed");
        let d = failed.diagnostic();
        assert_eq!((d.start_line, d.start_col, d.end_col), (8, 8, 8));
        // A point is drawn over the word it touches, as a problem's is.
        let text = "        assert_eq!(super::two(), 3);";
        assert_eq!(rusty_lsp::squiggle::drawn_on(&d, 8, text), Some((8, 17)));
    }
}
