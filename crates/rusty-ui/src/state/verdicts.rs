//! What the test runs said (`crate::testrun`): each test's verdict, and
//! where each failing test stopped — the ✓ and ✗ on the lenses, and the
//! mark on the line a test failed at.

use std::collections::HashMap;

use leptos::prelude::*;
use rusty_lsp::{DiagSeverity, FileDiagnostic};

use crate::testrun::{Failure, Reader, Verdict};

/// A failure's mark is a problem the editor draws like any other — the red
/// line, its card — told apart by this `source`, since what it says is a
/// test's and not the compiler's, and a quick fix is not asked for it.
pub const TEST_SOURCE: &str = "test";

#[derive(Clone, Copy)]
pub struct Tests {
    /// Every test's last verdict, by the harness's name. A run replaces the
    /// verdicts of the tests it ran and keeps the rest, as VS Code does:
    /// running one test does not forget how the others came out.
    pub verdicts: RwSignal<HashMap<String, Verdict>>,
    /// Where the failing tests stopped and what they said. A failure's place
    /// is dropped when its file is edited — it was about the text before —
    /// and the failure itself when its test is heard from again.
    pub failures: RwSignal<Vec<Failure>>,
    /// The run being read. Not a signal: a line wakes nothing until it has
    /// said something.
    pub reader: StoredValue<Reader>,
}

impl Tests {
    pub fn fresh() -> Self {
        Tests {
            verdicts: RwSignal::new(HashMap::new()),
            failures: RwSignal::new(Vec::new()),
            reader: StoredValue::new(Reader::default()),
        }
    }

    /// The failures that stopped in `path`, as problems for the editor to
    /// draw — from the column the panic names to the end of its line.
    /// Tracked, for what is drawn.
    pub fn marks_in(&self, path: &str) -> Vec<FileDiagnostic> {
        self.failures.with(|failures| marks(failures, path))
    }

    /// The same, untracked — for a pointer over the text.
    pub fn marks_in_now(&self, path: &str) -> Vec<FileDiagnostic> {
        self.failures
            .with_untracked(|failures| marks(failures, path))
    }
}

fn marks(failures: &[Failure], path: &str) -> Vec<FileDiagnostic> {
    failures
        .iter()
        .filter_map(|failure| {
            let (file, line, col) = failure.at.as_ref()?;
            (file == path).then(|| FileDiagnostic {
                severity: DiagSeverity::Error,
                message: failure.message.clone(),
                source: Some(TEST_SOURCE.to_string()),
                // The test's name, for the card to say whose failure it is.
                code: Some(failure.test.clone()),
                start_line: *line,
                start_col: *col,
                end_line: *line,
                // The rest of the line: `squiggle::drawn_on` stops at its
                // end, so the whole statement the panic names is marked.
                end_col: u32::MAX,
            })
        })
        .collect()
}
