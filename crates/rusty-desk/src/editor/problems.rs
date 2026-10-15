//! What the servers found, on the surface: the rows a new answer changes,
//! F8 through a file's problems, and the documents told to a server that
//! started after they were opened.

use rusty_lsp::{DiagSeverity, FileDiagnostic};

use super::{Cursor, Surface};

/// Whether a diagnostic is a problem: hints are the server's asides — a
/// `#[cfg]` branch that is off — and are neither drawn nor counted.
pub fn is_problem(d: &FileDiagnostic) -> bool {
    d.severity != DiagSeverity::Hint
}

/// The problem after `(line, col)` in `items`, or before it, wrapping at
/// the ends: by where each starts, the one at the caret itself excluded.
pub fn next(items: &[FileDiagnostic], at: (u32, u32), forward: bool) -> Option<&FileDiagnostic> {
    let mut problems: Vec<&FileDiagnostic> = items.iter().filter(|d| is_problem(d)).collect();
    problems.sort_by_key(|d| (d.start_line, d.start_col));
    let start = |d: &&FileDiagnostic| (d.start_line, d.start_col);
    if forward {
        problems
            .iter()
            .find(|d| start(d) > at)
            .or(problems.first())
            .copied()
    } else {
        problems
            .iter()
            .rev()
            .find(|d| start(d) < at)
            .or(problems.last())
            .copied()
    }
}

impl Surface {
    /// What a server found in `path` changed: the rows its squiggles were
    /// on and are on now are asked for again, and no others.
    pub fn problems_changed(&self, path: &str) {
        self.shade_follows_path(path);
        let Some(doc) = self.doc() else { return };
        if doc.borrow().path != path {
            return;
        }
        let now: Vec<usize> = {
            let problems = self.problems.borrow();
            let mut lines: Vec<usize> = problems
                .get(path)
                .into_iter()
                .flatten()
                .filter(|d| is_problem(d))
                .flat_map(|d| d.start_line as usize..=d.end_line as usize)
                .collect();
            lines.sort_unstable();
            lines.dedup();
            lines
        };
        let was = self.squiggled.replace(now.clone());
        if was.len() + now.len() > 400 {
            self.rows.reset();
        } else {
            for line in was.into_iter().chain(now) {
                self.rows.changed(line, line + 1);
            }
        }
    }

    /// F8 and Shift+F8: the caret to the next problem in the file on
    /// screen, wrapping, and what it says.
    pub fn next_problem(&self, forward: bool) -> Option<FileDiagnostic> {
        let doc = self.doc()?;
        let (path, caret) = {
            let d = doc.borrow();
            let (line, col) = d.lines.position(&d.text, d.primary().head);
            (d.path.clone(), (line as u32, col as u32))
        };
        let found = {
            let problems = self.problems.borrow();
            next(problems.get(&path)?, caret, forward)?.clone()
        };
        let at = {
            let d = doc.borrow();
            d.lines
                .byte(&d.text, found.start_line as usize, found.start_col as usize)
        };
        self.set_cursors(vec![Cursor::caret(at)]);
        Some(found)
    }

    /// Every open document, told to the servers — which have just started,
    /// after the documents were opened.
    pub fn announce_all(&self) {
        for doc in self.open.borrow().values() {
            let d = doc.borrow();
            self.servers.opened(&d.path, &d.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, col: u32, severity: DiagSeverity) -> FileDiagnostic {
        FileDiagnostic {
            severity,
            message: format!("{line}:{col}"),
            source: None,
            code: None,
            start_line: line,
            start_col: col,
            end_line: line,
            end_col: col + 1,
        }
    }

    #[test]
    fn f8_steps_through_the_problems_and_wraps_past_hints() {
        let items = [
            at(9, 0, DiagSeverity::Warning),
            at(2, 4, DiagSeverity::Error),
            at(5, 0, DiagSeverity::Hint),
        ];
        let message = |d: Option<&FileDiagnostic>| d.map(|d| d.message.clone());
        assert_eq!(message(next(&items, (0, 0), true)), Some("2:4".into()));
        assert_eq!(message(next(&items, (2, 4), true)), Some("9:0".into()));
        assert_eq!(message(next(&items, (9, 0), true)), Some("2:4".into()));
        assert_eq!(message(next(&items, (2, 4), false)), Some("9:0".into()));
        assert_eq!(message(next(&items, (7, 0), false)), Some("2:4".into()));
        assert!(next(&[at(1, 0, DiagSeverity::Hint)], (0, 0), true).is_none());
    }
}
