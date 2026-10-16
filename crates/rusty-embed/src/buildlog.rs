//! Where a build's errors are, read off what the compiler printed.
//!
//! rustc says what went wrong on one line and where on the next —
//! `error[E0425]: cannot find value` and then ` --> src/main.rs:12:5` —
//! while GCC and Clang say both on one: `src/main.c:12:5: error: …`. A
//! build's errors are what a language server's check does not always see:
//! the C compiled by a build script, a CMake project before its compile
//! database exists, a linker refusing a symbol in a file it names. So the
//! window lists them in Problems after a build, and every line of output
//! that names a place can be clicked to go there.
//!
//! Pure: lines in, places out. Paths are as the compiler printed them —
//! relative to wherever it ran, or absolute — and the caller knows which
//! directory that was.

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A diagnostic the compiler placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Located {
    pub path: String,
    /// Counted from one, as printed.
    pub line: u32,
    /// Counted from one; one when the compiler gave no column.
    pub col: u32,
    pub severity: Severity,
    pub message: String,
}

/// A place a line of output names: `path:line[:col]`, counted from one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub path: String,
    pub line: u32,
    pub col: u32,
}

/// Reads a build's output a line at a time and answers each diagnostic
/// once its place is known.
#[derive(Debug, Default)]
pub struct Reader {
    /// rustc's headline, waiting for the `-->` line under it.
    pending: Option<(Severity, String)>,
}

impl Reader {
    pub fn new() -> Reader {
        Reader::default()
    }

    pub fn read(&mut self, line: &str) -> Option<Located> {
        // rustc puts the place on the line straight after the headline, so
        // a headline waits one line and no longer: a link error's headline
        // followed by cargo's `Finished` must not lend its words to a place
        // printed later.
        let pending = self.pending.take();
        if let Some(rest) = line.trim_start().strip_prefix("--> ") {
            let (severity, message) = pending?;
            let place = place_in(rest.trim())?;
            return Some(Located {
                path: place.path,
                line: place.line,
                col: place.col,
                severity,
                message,
            });
        }
        if let Some(located) = one_line(line) {
            return Some(located);
        }
        // A rustc headline: `error[E0425]: …`, `error: …`, `warning: …`.
        // Cargo's own summaries look the same and are followed by no place.
        self.pending = rustc_headline(line);
        None
    }
}

fn rustc_headline(line: &str) -> Option<(Severity, String)> {
    let (severity, rest) = match line.strip_prefix("error") {
        Some(rest) => (Severity::Error, rest),
        None => (Severity::Warning, line.strip_prefix("warning")?),
    };
    // `error[E0425]: msg` or `error: msg`.
    let rest = match rest.strip_prefix('[') {
        Some(code) => &code[code.find(']')? + 1..],
        None => rest,
    };
    let message = rest.strip_prefix(": ")?.trim().to_string();
    // Cargo's summaries are not about a place in the code.
    if message.starts_with("could not compile") || message.contains("generated ") {
        return None;
    }
    Some((severity, message))
}

/// GCC's and Clang's `path:line:col: error: message` (`fatal error`
/// counts as an error; a `note` is not a problem).
fn one_line(line: &str) -> Option<Located> {
    for (marker, severity) in [
        (": fatal error: ", Severity::Error),
        (": error: ", Severity::Error),
        (": warning: ", Severity::Warning),
    ] {
        if let Some(at) = line.find(marker) {
            let place = place_in(line[..at].trim())?;
            return Some(Located {
                path: place.path,
                line: place.line,
                col: place.col,
                severity,
                message: line[at + marker.len()..].trim().to_string(),
            });
        }
    }
    None
}

/// `path:line:col` or `path:line`, with a Windows drive letter's colon
/// left in the path.
pub fn place_in(text: &str) -> Option<Place> {
    let mut parts = text.rsplitn(3, ':');
    let last = parts.next()?;
    let middle = parts.next()?;
    let number = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    let (path, line, col) = match (number(middle), number(last)) {
        (Some(line), Some(col)) => (parts.next()?, line, col),
        (None, Some(line)) => {
            // `path:line`: the path is everything before the last colon.
            let path = &text[..text.len() - last.len() - 1];
            (path, line, 1)
        }
        _ => return None,
    };
    let path = path.trim();
    // A drive letter alone, or nothing, is not a file.
    if path.is_empty()
        || path.len() == 1
        || path.contains(char::is_whitespace) && !path.contains('.')
    {
        return None;
    }
    Some(Place {
        path: path.to_string(),
        line,
        col,
    })
}

/// The place a line of output names, for the dock to make it a link: the
/// `-->` under a rustc diagnostic, the head of a GCC one, or where a Rust
/// program panicked.
pub fn place_on(line: &str) -> Option<Place> {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("--> ") {
        return place_in(rest.trim());
    }
    if let Some(place) = panic_place(trimmed) {
        return Some(place);
    }
    one_line(line).map(|located| Place {
        path: located.path,
        line: located.line,
        col: located.col,
    })
}

/// Where a panic says it happened: `thread 'tests::it' (1234) panicked at
/// core\src\lib.rs:20:9:` — the path as rustc was handed the file, which for
/// cargo is relative to the workspace's root — or, before Rust 1.73, after
/// the quoted message: `panicked at 'boom', src/lib.rs:5:9`.
fn panic_place(trimmed: &str) -> Option<Place> {
    let rest = trimmed.strip_prefix("thread '")?;
    let (_, rest) = rest.split_once("panicked at ")?;
    let place = match rest.strip_prefix('\'') {
        Some(quoted) => quoted.rsplit_once("', ")?.1,
        None => rest,
    };
    place_in(place.trim().trim_end_matches(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test's panic names its place, which the dock makes a link — in a
    /// run's output and a debugged test's alike — as libtest printed it on
    /// Rust 1.98, and as Rust before 1.73 did.
    #[test]
    fn a_panic_names_its_place() {
        assert_eq!(
            place_on(
                r"thread 'tests::the_dot_of_two_axes' (22376) panicked at core\src\lib.rs:20:9:"
            ),
            Some(Place {
                path: r"core\src\lib.rs".to_string(),
                line: 20,
                col: 9,
            })
        );
        assert_eq!(
            place_on("thread 'main' panicked at 'boom: a, b', src/main.rs:5:9"),
            Some(Place {
                path: "src/main.rs".to_string(),
                line: 5,
                col: 9,
            })
        );
        // The message under the header names no place.
        assert_eq!(place_on("assertion `left == right` failed"), None);
    }

    fn read_all(lines: &[&str]) -> Vec<Located> {
        let mut reader = Reader::new();
        lines.iter().filter_map(|line| reader.read(line)).collect()
    }

    /// rustc's headline and its place are two lines; cargo's summary after
    /// them is no diagnostic.
    #[test]
    fn a_rustc_error_is_placed_by_the_line_under_it() {
        let found = read_all(&[
            "   Compiling app v0.1.0 (E:\\proj)",
            "error[E0425]: cannot find value `x` in this scope",
            "  --> src/main.rs:12:5",
            "   |",
            "12 |     x + 1",
            "   |     ^ not found in this scope",
            "",
            "warning: unused variable: `y`",
            " --> src\\lib.rs:3:9",
            "error: could not compile `app` (bin \"app\") due to 1 previous error; 1 warning emitted",
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, "src/main.rs");
        assert_eq!((found[0].line, found[0].col), (12, 5));
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[0].message, "cannot find value `x` in this scope");
        assert_eq!(found[1].path, "src\\lib.rs");
        assert_eq!(found[1].severity, Severity::Warning);
    }

    /// GCC says it on one line, a Windows path with its drive letter
    /// included; a note is not a problem.
    #[test]
    fn a_gcc_error_is_one_line_drive_letter_and_all() {
        let found = read_all(&[
            "E:\\proj\\src\\main.c:7:3: error: 'led' undeclared (first use in this function)",
            "E:\\proj\\src\\main.c:7:3: note: each undeclared identifier is reported only once",
            "src/startup.c:40: warning: implicit declaration of function 'init'",
            "In file included from src/main.c:2:",
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, "E:\\proj\\src\\main.c");
        assert_eq!((found[0].line, found[0].col), (7, 3));
        assert_eq!(
            found[0].message,
            "'led' undeclared (first use in this function)"
        );
        assert_eq!(found[1].path, "src/startup.c");
        assert_eq!((found[1].line, found[1].col), (40, 1));
    }

    /// A headline whose block ends before a place is forgotten, so the next
    /// place is not given the wrong message.
    #[test]
    fn a_headline_with_no_place_is_forgotten() {
        let found = read_all(&[
            "error: linking with `rust-lld` failed: exit code: 1",
            "    Finished `release` profile",
            "  --> src/main.rs:1:1",
        ]);
        assert!(found.is_empty());
    }

    #[test]
    fn a_line_that_names_a_place_is_a_link() {
        assert_eq!(
            place_on("  --> src/main.rs:12:5"),
            Some(Place {
                path: "src/main.rs".into(),
                line: 12,
                col: 5
            })
        );
        assert!(place_on("   Compiling app v0.1.0").is_none());
        assert!(place_on("note: C:12:5").is_none());
    }
}
