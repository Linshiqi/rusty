//! Where a breakpoint clicked on a line is placed.
//!
//! On the line it was clicked on, but for one shape. A multi-line
//! `assert!(` or `debug_assert!(` has no code of its own on its first line
//! except the branch that panics: the condition is evaluated on the lines
//! below it, and what the compiler attributes to the macro's own line is the
//! call to the panic handler, which runs only when the assertion fails.
//! Measured in the line table of a Windows test binary: line 139, `assert!(`,
//! had four addresses, all after line 142's `r.norm()` on the failing path,
//! and line 140's condition came before them. A debugger binds the
//! breakpoint there faithfully, the assertion holds, and the test runs to
//! its end without stopping — which read as "Debug does not start".
//! `assert_eq!(`, `assert_ne!(`, `println!(` and `vec![` compare, print or
//! allocate on their own line and stop there as clicked, so they are left
//! alone.
//!
//! The breakpoint goes where the statement begins to run instead: the first
//! line of the condition. Both debuggers report where a breakpoint landed
//! beside the line that was clicked (`Breakpoint::requested`), so the dot
//! moves to it when the session starts, as it does for a line with no code.

/// The zero-based line a breakpoint on `line` of `source` is placed on.
pub fn placed_line(source: &str, line: u32) -> u32 {
    let mut lines = source.split('\n').skip(line as usize);
    let Some(clicked) = lines.next() else {
        return line;
    };
    if !opens_an_assertion(clicked) {
        return line;
    }
    // The condition's first line: past blank lines and comments, which a
    // debugger would move off anyway, but a comment is text this rule has
    // to step over to know where the condition is.
    lines
        .position(|text| {
            let text = text.trim();
            !text.is_empty() && !text.starts_with("//")
        })
        .map_or(line, |after| line + 1 + after as u32)
}

/// A line that is nothing but `assert!(` or `debug_assert!(` — a path to
/// either is allowed, `std::assert!(` — and a comment after it. Anything
/// of the condition on the line is code on the line, and the breakpoint
/// stays where it was clicked.
fn opens_an_assertion(text: &str) -> bool {
    let code = text.split_once("//").map_or(text, |(code, _)| code).trim();
    let Some(name) = code.strip_suffix("!(") else {
        return false;
    };
    let name = name.strip_prefix("::").unwrap_or(name);
    let (path, last) = name.rsplit_once("::").unwrap_or(("", name));
    let path_is_names = path.is_empty()
        || path.split("::").all(|segment| {
            !segment.is_empty() && segment.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
    path_is_names && matches!(last, "assert" | "debug_assert")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The report, verbatim from the file it came from: a breakpoint on
    /// line 139 never stopped, and the one this places on 140 does.
    const TEST: &str = "    #[test]
    fn the_rotation_between_two_vectors_is_an_axis_times_an_angle() {
        let r = Vector::rotation_vector_between(Vector::UP, Vector::new(1.0, 0.0, 0.0));
        println!(\"r: {:?}\", r);
        assert!(
            (r.norm() - core::f32::consts::FRAC_PI_2).abs() < EPS,
            \"expected a quarter turn, got {} rad\",
            r.norm()
        );
        // Z onto X turns about +Y.
        assert!(r.y > 0.0 && r.x.abs() < EPS && r.z.abs() < EPS, \"{r:?}\");
    }
";

    #[test]
    fn a_breakpoint_on_a_multi_line_assert_goes_to_its_condition() {
        assert_eq!(placed_line(TEST, 4), 5);
    }

    #[test]
    fn every_other_line_keeps_its_breakpoint() {
        for line in [0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11] {
            assert_eq!(placed_line(TEST, line), line, "line {line}");
        }
    }

    /// A single-line assertion evaluates its condition on its own line.
    #[test]
    fn an_assertion_on_one_line_is_left_where_it_is() {
        assert_eq!(
            placed_line("    assert!(ok, \"why\");\n    next();\n", 0),
            0
        );
    }

    /// The compiler's own assertions only: the rest compare, print or
    /// allocate on their first line, and stop there as clicked.
    #[test]
    fn macros_with_code_on_their_first_line_are_left_alone() {
        for opener in ["assert_eq!(", "assert_ne!(", "println!(", "let v = vec!["] {
            let source = format!("    {opener}\n        a,\n        b\n    );\n");
            assert_eq!(placed_line(&source, 0), 0, "{opener}");
        }
    }

    #[test]
    fn debug_assert_and_a_path_to_either_count() {
        for opener in [
            "debug_assert!(",
            "std::assert!(",
            "core::debug_assert!(",
            "::core::assert!(",
            "assert!( // the angle",
        ] {
            let source = format!("    {opener}\n        a < b,\n    );\n");
            assert_eq!(placed_line(&source, 0), 1, "{opener}");
        }
    }

    /// Blank lines and comments between the opener and the condition hold
    /// no code, so the breakpoint goes past them to the condition.
    #[test]
    fn the_condition_is_found_past_blank_lines_and_comments() {
        let source = "assert!(\n\n    // why this must hold\n    a < b,\n);\n";
        assert_eq!(placed_line(source, 0), 3);
    }

    #[test]
    fn a_line_past_the_end_or_an_opener_at_the_end_stays() {
        assert_eq!(placed_line(TEST, 99), 99);
        assert_eq!(placed_line("assert!(", 0), 0);
    }

    /// `\r\n` files: the carriage return is whitespace the trim takes.
    #[test]
    fn windows_line_endings_read_the_same() {
        let source = TEST.replace('\n', "\r\n");
        assert_eq!(placed_line(&source, 4), 5);
    }

    #[test]
    fn a_call_ending_in_assert_is_not_an_assertion() {
        assert_eq!(placed_line("    my_assert!(\n        a,\n    );\n", 0), 0);
        assert_eq!(placed_line("    x.assert!(\n        a,\n    );\n", 0), 0);
    }
}
