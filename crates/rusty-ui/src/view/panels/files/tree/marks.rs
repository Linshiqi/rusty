//! What the check found in a file, drawn on its row: red with the count
//! for errors, amber for warnings, and a folder in the colour of the worst
//! thing inside it.

/// How much is wrong in one tree row: its errors and warnings, or those of
/// everything under it for a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Mark {
    pub errors: usize,
    pub warnings: usize,
}

/// The mark for `path`, or `None` when nothing there is an error or a
/// warning. Hints and information are not problems — the Problems panel's
/// rule — so a crate full of `#[cfg]`-inactive code is not painted amber.
/// A folder counts what is under it by path, with the separator: `src2` is
/// not under `src`.
pub(super) fn problem_mark(
    by_file: &std::collections::HashMap<String, Vec<rusty_lsp::FileDiagnostic>>,
    path: &str,
    is_dir: bool,
) -> Option<Mark> {
    use rusty_lsp::DiagSeverity;

    let prefix = format!("{}/", path.trim_end_matches('/'));
    let mut mark = Mark {
        errors: 0,
        warnings: 0,
    };
    for (file, items) in by_file {
        let inside = if is_dir {
            path.is_empty() || file.starts_with(&prefix)
        } else {
            file == path
        };
        if !inside {
            continue;
        }
        for item in items {
            match item.severity {
                DiagSeverity::Error => mark.errors += 1,
                DiagSeverity::Warning => mark.warnings += 1,
                _ => {}
            }
        }
    }
    (mark.errors + mark.warnings > 0).then_some(mark)
}

#[cfg(test)]
mod mark_tests {
    use std::collections::HashMap;

    use rusty_lsp::{DiagSeverity, FileDiagnostic};

    use super::*;

    fn items(severities: &[DiagSeverity]) -> Vec<FileDiagnostic> {
        severities
            .iter()
            .map(|severity| FileDiagnostic {
                severity: *severity,
                message: String::new(),
                source: Some("rustc".into()),
                code: None,
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 0,
            })
            .collect()
    }

    /// The report: an error in `core/src/math/quaternion.rs` marks the file
    /// and each folder above it, and nothing beside it.
    #[test]
    fn an_error_marks_its_file_and_every_folder_above_it() {
        let by_file = HashMap::from([(
            "core/src/math/quaternion.rs".to_string(),
            items(&[
                DiagSeverity::Error,
                DiagSeverity::Warning,
                DiagSeverity::Hint,
            ]),
        )]);
        let file = problem_mark(&by_file, "core/src/math/quaternion.rs", false);
        assert_eq!(
            file,
            Some(Mark {
                errors: 1,
                warnings: 1
            }),
            "a hint is not a problem"
        );
        for folder in ["core", "core/src", "core/src/math"] {
            assert_eq!(
                problem_mark(&by_file, folder, true).map(|m| m.errors),
                Some(1),
                "{folder}"
            );
        }
        assert_eq!(
            problem_mark(&by_file, "core/src/math/vector.rs", false),
            None
        );
        assert_eq!(problem_mark(&by_file, "firmware", true), None);
    }

    /// A folder whose name another begins with is not its parent, and a file
    /// with only hints is unmarked.
    #[test]
    fn a_prefix_is_not_a_parent_and_hints_mark_nothing() {
        let by_file = HashMap::from([
            ("src2/lib.rs".to_string(), items(&[DiagSeverity::Error])),
            (
                "src/main.rs".to_string(),
                items(&[DiagSeverity::Hint, DiagSeverity::Info]),
            ),
        ]);
        assert_eq!(problem_mark(&by_file, "src", true), None);
        assert_eq!(problem_mark(&by_file, "src/main.rs", false), None);
        assert_eq!(
            problem_mark(&by_file, "src2", true).map(|m| m.errors),
            Some(1)
        );
    }
}
