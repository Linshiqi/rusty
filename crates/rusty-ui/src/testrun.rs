//! What a test harness says, read a line at a time: each test's verdict,
//! and where each failing test panicked and what it said.
//!
//! Read off libtest's own text, the lines `cargo test` prints, as captured
//! from a real run (the tests below): `test <name> ... ok`, then — with
//! output captured, as the lens runs it — one `---- <name> stdout ----`
//! section per test that printed or failed, the panic inside it:
//!
//! ```text
//! ---- math::vector::tests::the_sum_is_four stdout ----
//!
//! thread 'math::vector::tests::the_sum_is_four' (28312) panicked at core\src\math\vector.rs:17:9:
//! assertion `left == right` failed
//!   left: 3
//!  right: 4
//! ```
//!
//! A panic is a failure only where the verdict says so: a `#[should_panic]`
//! test that passed prints its panic too, under `successes:`, and marking
//! it would be marking a test that did what it was written to do.

use std::collections::HashMap;

/// How one test came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    Failed,
    Ignored,
}

/// Where a failing test stopped, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The harness's name for the test: `math::vector::tests::the_sum`.
    pub test: String,
    /// Project-relative with forward slashes, zero-based line and column —
    /// `None` for a place outside the project (the standard library, a
    /// dependency) and for a test that failed without panicking.
    pub at: Option<(String, u32, u32)>,
    /// The panic's message, every line of it.
    pub message: String,
}

/// What one line said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heard {
    Verdict(String, Verdict),
    Failure(Failure),
}

/// A harness's verdict line: `test <name> ... ok`, `... FAILED`,
/// `... ignored, <reason>`, with the name as the harness knows the test
/// (` - should panic` is how the line says so, not part of the name) and
/// the byte range of the verdict's word, for colouring it.
pub fn verdict_line(text: &str) -> Option<(&str, Verdict, (usize, usize))> {
    let rest = text.strip_prefix("test ")?;
    let (name, outcome) = rest.rsplit_once(" ... ")?;
    let word_len = outcome
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(outcome.len());
    let verdict = verdict_of(&outcome[..word_len])?;
    let start = text.len() - outcome.len();
    let name = name.strip_suffix(" - should panic").unwrap_or(name);
    Some((name, verdict, (start, start + word_len)))
}

/// The word of a verdict, as the harness prints it and colours it.
fn verdict_of(word: &str) -> Option<Verdict> {
    match word {
        "ok" => Some(Verdict::Passed),
        "FAILED" => Some(Verdict::Failed),
        "ignored" => Some(Verdict::Ignored),
        _ => None,
    }
}

/// The word a line is coloured by, where a terminal would colour it: the
/// verdict of one test, or of a whole binary (`test result: FAILED. …`).
pub fn verdict_word(text: &str) -> Option<(usize, usize, Verdict)> {
    if let Some((_, verdict, (from, to))) = verdict_line(text) {
        return Some((from, to, verdict));
    }
    let rest = text.strip_prefix("test result: ")?;
    let word = rest.split('.').next()?;
    let verdict = verdict_of(word)?;
    let from = text.len() - rest.len();
    Some((from, from + word.len(), verdict))
}

/// Whether a line is the start of a panic — the one line of a failure that
/// says where it happened, which the dock paints as an error.
pub fn is_panic_header(text: &str) -> bool {
    panic_header(text).is_some()
}

/// `thread '<name>' panicked at …` — with the thread's number between the
/// two since Rust 1.91 — as the test it names and the rest of the line.
fn panic_header(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("thread '")?;
    let (thread, rest) = rest.split_once('\'')?;
    let (_, after) = rest.split_once("panicked at ")?;
    Some((thread, after))
}

/// A location as a panic prints it — `core\src\lib.rs:5:20` — made the
/// editor's: project-relative with forward slashes, zero-based. `None`
/// outside the project: a path with a drive letter or a leading separator
/// is the standard library's or a dependency's.
fn location(text: &str) -> Option<(String, u32, u32)> {
    let mut parts = text.trim_end_matches(':').rsplitn(3, ':');
    let col: u32 = parts.next()?.parse().ok()?;
    let line: u32 = parts.next()?.parse().ok()?;
    let path = parts.next()?;
    let absolute = path.starts_with('/')
        || path.starts_with('\\')
        || path.as_bytes().get(1) == Some(&b':')
        || path.is_empty();
    if absolute || line == 0 {
        return None;
    }
    Some((path.replace('\\', "/"), line - 1, col.saturating_sub(1)))
}

/// Reads a run's lines in order, remembering what a line cannot say alone:
/// whose output this is, which verdicts have been given, and a panic whose
/// message is still arriving.
#[derive(Debug, Default)]
pub struct Reader {
    /// The test whose captured output is being read.
    section: Option<String>,
    /// A panic whose message has not ended yet.
    open: Option<Failure>,
    /// This binary's verdicts so far: which panics are failures.
    verdicts: HashMap<String, Verdict>,
    /// Panics read before their test's verdict — `--nocapture` prints them
    /// as they happen, ahead of the line that says how the test came out.
    waiting: Vec<Failure>,
}

impl Reader {
    /// One line of the run.
    pub fn read(&mut self, line: &str) -> Vec<Heard> {
        let mut heard = Vec::new();
        let text = line.trim_end_matches('\r');
        if let Some(open) = &mut self.open {
            if !ends_a_message(text) {
                if !open.message.is_empty() {
                    open.message.push('\n');
                }
                open.message.push_str(text);
                return heard;
            }
            self.close(&mut heard);
        }

        if let Some(name) = text
            .strip_prefix("---- ")
            .and_then(|rest| rest.strip_suffix(" stdout ----"))
        {
            self.section = Some(name.to_string());
        } else if matches!(text, "failures:" | "successes:") {
            self.section = None;
        } else if starts_a_binary(text) || text.starts_with("test result: ") {
            // One binary's run begins or ends: its names are its own.
            self.section = None;
            self.verdicts.clear();
            self.waiting.clear();
        } else if let Some((name, verdict, _)) = verdict_line(text) {
            self.verdicts.insert(name.to_string(), verdict);
            heard.push(Heard::Verdict(name.to_string(), verdict));
            let waiting = std::mem::take(&mut self.waiting);
            for failure in waiting {
                if failure.test != name {
                    self.waiting.push(failure);
                } else if verdict == Verdict::Failed {
                    heard.push(Heard::Failure(failure));
                }
            }
        } else if let Some((thread, rest)) = panic_header(text) {
            let test = self.section.clone().unwrap_or_else(|| thread.to_string());
            // Before Rust 1.73 the message came first, quoted, and the
            // place after it: `panicked at 'boom', src/lib.rs:5:9`.
            if let Some(quoted) = rest.strip_prefix('\'')
                && let Some((message, place)) = quoted.rsplit_once("', ")
            {
                self.settle(
                    Failure {
                        test,
                        at: location(place),
                        message: message.to_string(),
                    },
                    &mut heard,
                );
            } else {
                self.open = Some(Failure {
                    test,
                    at: location(rest),
                    message: String::new(),
                });
            }
        } else if let Some(section) = self.section.clone() {
            // A failure the harness reports without a panic: a
            // `#[should_panic]` test that returned, and a test that
            // returned an `Err`.
            if let Some(rest) = text.strip_prefix("note: test did not panic as expected") {
                let at = rest.strip_prefix(" at ").and_then(location);
                let message = "test did not panic as expected".to_string();
                self.settle(
                    Failure {
                        test: section,
                        at,
                        message,
                    },
                    &mut heard,
                );
            } else if text.starts_with("Error: ") {
                self.settle(
                    Failure {
                        test: section,
                        at: None,
                        message: text.to_string(),
                    },
                    &mut heard,
                );
            }
        }
        heard
    }

    /// The run is over: a panic still open ends here.
    pub fn finish(&mut self) -> Vec<Heard> {
        let mut heard = Vec::new();
        self.close(&mut heard);
        heard
    }

    fn close(&mut self, heard: &mut Vec<Heard>) {
        if let Some(mut failure) = self.open.take() {
            failure.message.truncate(failure.message.trim_end().len());
            self.settle(failure, heard);
        }
    }

    /// Said now when the test's verdict was a failure, dropped when it was
    /// not, and kept for the verdict when it has not come yet.
    fn settle(&mut self, failure: Failure, heard: &mut Vec<Heard>) {
        match self.verdicts.get(&failure.test) {
            Some(Verdict::Failed) => heard.push(Heard::Failure(failure)),
            Some(_) => {}
            None => self.waiting.push(failure),
        }
    }
}

/// `running 17 tests`, `running 1 test`: a test binary starting. Exactly
/// that — a test's own output saying "running the loop" is not one, and
/// taken for one it would forget this binary's verdicts halfway through.
fn starts_a_binary(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("running ") else {
        return false;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && matches!(&rest[digits..], " test" | " tests")
}

/// Whether a line is past the end of a panic's message: a blank line, the
/// hook's note, a backtrace, or anything else of the harness's own. A
/// message with a blank line inside it is cut there — rare, and the whole
/// of it is still in the dock.
fn ends_a_message(text: &str) -> bool {
    text.trim().is_empty()
        || text.starts_with("note: ")
        || text.starts_with("stack backtrace:")
        || text.starts_with("---- ")
        || text.starts_with("thread '")
        || text.starts_with("test result: ")
        || text.starts_with("error: ")
        || text.starts_with("[rusty:")
        || matches!(text, "failures:" | "successes:")
        || verdict_line(text).is_some()
}

/// The module a file is, in the harness's names: `core/src/math/vector.rs`
/// is `math::vector`, a crate's root is `""`. `None` where the path says
/// nothing about it — no `src/` and no `tests/`, `benches/` or `examples/`
/// to count from. Read off the path, as the compiler finds a module's file,
/// so a `#[path]` attribute can make it wrong; a name that matches no test
/// then marks nothing, which is the failure this can afford.
pub fn module_of(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    // Counted from the package's `src/` — the first, so a module named
    // `tests` inside it stays a module — or else from the last `tests/`,
    // `benches/` or `examples/`, so a package kept under a directory named
    // `examples/` is still read from its own.
    let root = parts.iter().position(|part| *part == "src").or_else(|| {
        parts
            .iter()
            .rposition(|part| matches!(*part, "tests" | "benches" | "examples"))
    })?;
    let (file, dirs) = parts[root + 1..].split_last()?;
    let stem = file.strip_suffix(".rs")?;
    // Every file under `tests/`, `benches/`, `examples/` and `src/bin/` is
    // a target of its own, and so is a directory there with a `main.rs`,
    // whose other files are its modules: `tests/flight/wind.rs` is `wind`.
    // `tests/common/mod.rs` is the module `common` of whichever declares it.
    let (own, dirs) = match (parts[root], dirs) {
        ("src", ["bin", rest @ ..]) => (true, rest),
        ("src", dirs) => (false, dirs),
        (_, dirs) => (true, dirs),
    };
    let dirs: &[&str] = if !own || (stem == "mod" && dirs.len() == 1) {
        dirs
    } else if dirs.is_empty() {
        return Some(String::new());
    } else {
        &dirs[1..]
    };
    let mut segments = dirs.to_vec();
    match stem {
        "mod" => {}
        "lib" | "main" if segments.is_empty() => {}
        other => segments.push(other),
    }
    Some(segments.join("::"))
}

/// How the test or module a lens runs came out last time, by the harness's
/// names for them: a test by its whole name, a module by every test under
/// it — any failure fails it, any pass passes it, and one that only holds
/// ignored tests has no verdict to show.
pub fn verdict_of_lens(
    verdicts: &HashMap<String, Verdict>,
    file: &str,
    filter: &str,
    module: bool,
) -> Option<Verdict> {
    let prefix = module_of(file)?;
    let full = if prefix.is_empty() {
        filter.to_string()
    } else {
        format!("{prefix}::{filter}")
    };
    if !module {
        return verdicts.get(&full).copied();
    }
    let under = format!("{full}::");
    let mut passed = false;
    for (name, verdict) in verdicts {
        if !name.starts_with(&under) {
            continue;
        }
        match verdict {
            Verdict::Failed => return Some(Verdict::Failed),
            Verdict::Passed => passed = true,
            Verdict::Ignored => {}
        }
    }
    passed.then_some(Verdict::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim from `cargo test -- --show-output` on a two-module crate in
    /// a workspace, Rust 1.98 on Windows — what the lens now runs.
    const CAPTURED: &str = "
running 10 tests
test math::vector::tests::it_is_ignored ... ignored, slow
test math::vector::tests::it_does_not_panic - should panic ... FAILED
test math::vector::tests::it_prints_and_passes ... ok
test math::vector::tests::it_panics_with_a_message_over_lines ... FAILED
test math::vector::tests::a_quarter_turn_is_positive ... FAILED
test math::vector::tests::the_sum_is_four ... FAILED
test math::vector::tests::it_returns_an_error ... FAILED
test math::vector::tests::it_panics_as_expected - should panic ... ok
test tests::a_value_is_held ... ok
test tests::the_library_panics_for_it ... FAILED

successes:

---- math::vector::tests::it_prints_and_passes stdout ----
hello from a passing test
[rusty:draw] scene 1 demo

---- math::vector::tests::it_panics_as_expected stdout ----

thread 'math::vector::tests::it_panics_as_expected' (29272) panicked at core\\src\\math\\vector.rs:29:9:
boom


successes:
    math::vector::tests::it_panics_as_expected
    math::vector::tests::it_prints_and_passes
    tests::a_value_is_held

failures:

---- math::vector::tests::it_does_not_panic stdout ----
note: test did not panic as expected at core\\src\\math\\vector.rs:34:8
---- math::vector::tests::it_panics_with_a_message_over_lines stdout ----

thread 'math::vector::tests::it_panics_with_a_message_over_lines' (6152) panicked at core\\src\\math\\vector.rs:47:9:
first line
second line

---- math::vector::tests::a_quarter_turn_is_positive stdout ----

thread 'math::vector::tests::a_quarter_turn_is_positive' (29240) panicked at core\\src\\math\\vector.rs:12:9:
-1.5707963
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- math::vector::tests::the_sum_is_four stdout ----

thread 'math::vector::tests::the_sum_is_four' (28312) panicked at core\\src\\math\\vector.rs:17:9:
assertion `left == right` failed
  left: 3
 right: 4

---- math::vector::tests::it_returns_an_error stdout ----
Error: \"no such thing\"

---- tests::the_library_panics_for_it stdout ----

thread 'tests::the_library_panics_for_it' (3624) panicked at core\\src\\lib.rs:5:20:
called `Option::unwrap()` on a `None` value


failures:
    math::vector::tests::a_quarter_turn_is_positive
    math::vector::tests::it_does_not_panic
    math::vector::tests::it_panics_with_a_message_over_lines
    math::vector::tests::it_returns_an_error
    math::vector::tests::the_sum_is_four
    tests::the_library_panics_for_it

test result: FAILED. 3 passed; 6 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s
";

    fn read_all(text: &str) -> Vec<Heard> {
        let mut reader = Reader::default();
        let mut heard: Vec<Heard> = text.lines().flat_map(|line| reader.read(line)).collect();
        heard.extend(reader.finish());
        heard
    }

    fn failures(heard: &[Heard]) -> Vec<&Failure> {
        heard
            .iter()
            .filter_map(|h| match h {
                Heard::Failure(failure) => Some(failure),
                Heard::Verdict(..) => None,
            })
            .collect()
    }

    fn at(path: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
        Some((path.to_string(), line, col))
    }

    #[test]
    fn every_verdict_is_heard_by_the_tests_own_name() {
        let heard = read_all(CAPTURED);
        let verdicts: Vec<(&str, Verdict)> = heard
            .iter()
            .filter_map(|h| match h {
                Heard::Verdict(name, verdict) => Some((name.as_str(), *verdict)),
                Heard::Failure(_) => None,
            })
            .collect();
        assert_eq!(verdicts.len(), 10);
        assert!(verdicts.contains(&("math::vector::tests::it_is_ignored", Verdict::Ignored)));
        assert!(verdicts.contains(&("math::vector::tests::it_does_not_panic", Verdict::Failed)));
        assert!(verdicts.contains(&(
            "math::vector::tests::it_panics_as_expected",
            Verdict::Passed
        )));
    }

    #[test]
    fn each_failure_is_where_it_panicked_with_what_it_said() {
        let heard = read_all(CAPTURED);
        let found = failures(&heard);
        let by_test = |name: &str| {
            found
                .iter()
                .find(|f| f.test == name)
                .copied()
                .unwrap_or_else(|| panic!("{name} failed"))
        };
        let sum = by_test("math::vector::tests::the_sum_is_four");
        assert_eq!(sum.at, at("core/src/math/vector.rs", 16, 8));
        assert_eq!(
            sum.message,
            "assertion `left == right` failed\n  left: 3\n right: 4"
        );
        let turn = by_test("math::vector::tests::a_quarter_turn_is_positive");
        assert_eq!(turn.at, at("core/src/math/vector.rs", 11, 8));
        assert_eq!(
            turn.message, "-1.5707963",
            "the hook's note is not the message"
        );
        let lines = by_test("math::vector::tests::it_panics_with_a_message_over_lines");
        assert_eq!(lines.message, "first line\nsecond line");
        // Where the library panicked, from the test that called it.
        let library = by_test("tests::the_library_panics_for_it");
        assert_eq!(library.at, at("core/src/lib.rs", 4, 19));
        // Failures that are no panic.
        let returned = by_test("math::vector::tests::it_does_not_panic");
        assert_eq!(returned.at, at("core/src/math/vector.rs", 33, 7));
        assert_eq!(returned.message, "test did not panic as expected");
        let error = by_test("math::vector::tests::it_returns_an_error");
        assert_eq!(error.at, None);
        assert_eq!(error.message, "Error: \"no such thing\"");
        assert_eq!(found.len(), 6, "one per failed test: {found:#?}");
    }

    /// `#[should_panic]` passed: its panic is under `successes:`, and a
    /// mark for it would be a mark on a test that did what it should.
    #[test]
    fn a_panic_the_test_expected_is_not_a_failure() {
        let heard = read_all(CAPTURED);
        assert!(
            failures(&heard)
                .iter()
                .all(|f| f.test != "math::vector::tests::it_panics_as_expected")
        );
    }

    /// Verbatim from the same run with `--nocapture`: each panic on stderr
    /// as it happens, ahead of the verdict that makes it a failure.
    #[test]
    fn a_panic_heard_before_its_verdict_waits_for_it() {
        let run = "
running 3 tests

thread 'math::vector::tests::a_quarter_turn_is_positive' (2408) panicked at core\\src\\math\\vector.rs:12:9:
-1.5707963
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

thread 'math::vector::tests::it_panics_as_expected' (28020) panicked at core\\src\\math\\vector.rs:29:9:
boom
test math::vector::tests::a_quarter_turn_is_positive ... FAILED
test math::vector::tests::it_panics_as_expected - should panic ... ok
test tests::a_value_is_held ... ok
";
        let heard = read_all(run);
        let found = failures(&heard);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].test,
            "math::vector::tests::a_quarter_turn_is_positive"
        );
        assert_eq!(found[0].at, at("core/src/math/vector.rs", 11, 8));
    }

    /// A scene a failing test was drawing goes out as it unwinds, after the
    /// panic's message — with no note after a process's first panic to end
    /// the message. Protocol lines are not the message.
    #[test]
    fn a_drawing_after_a_panic_is_not_its_message() {
        let run = "
test tests::draws ... FAILED
failures:
---- tests::draws stdout ----
thread 'tests::draws' (1) panicked at src/lib.rs:9:5:
angle is off
[rusty:draw] scene 1 cross
[rusty:draw] vector 1 0 0 a
";
        let heard = read_all(run);
        assert_eq!(failures(&heard)[0].message, "angle is off");
    }

    #[test]
    fn a_place_outside_the_project_is_no_place() {
        assert_eq!(
            location(r"C:\Users\x\.cargo\registry\src\index\libm-0.2.16\src\lib.rs:5:9:"),
            None
        );
        assert_eq!(
            location("/rustc/48a229cea/library/core/src/option.rs:1:2:"),
            None
        );
        assert_eq!(
            location(r"core\src\lib.rs:5:20:"),
            at("core/src/lib.rs", 4, 19)
        );
    }

    /// The format before Rust 1.73: the message quoted, the place after it.
    #[test]
    fn the_old_panic_line_is_read_too() {
        let run = "test tests::old ... FAILED\n---- tests::old stdout ----\n\
                   thread 'tests::old' panicked at 'boom', src/lib.rs:5:9\n";
        let heard = read_all(run);
        let found = failures(&heard);
        assert_eq!(found[0].message, "boom");
        assert_eq!(found[0].at, at("src/lib.rs", 4, 8));
    }

    /// A passing test that prints "running …" has not started a binary:
    /// taken for one, the verdicts before it were forgotten and the failure
    /// after it never marked.
    #[test]
    fn a_test_saying_running_is_not_a_new_binary() {
        let run = "
running 2 tests
test tests::loud ... ok
test tests::quiet ... FAILED
successes:
---- tests::loud stdout ----
running the loop 3 times
failures:
---- tests::quiet stdout ----
thread 'tests::quiet' (7) panicked at src/lib.rs:12:9:
too quiet
";
        let heard = read_all(run);
        assert_eq!(failures(&heard).len(), 1);
        assert!(starts_a_binary("running 1 test"));
        assert!(starts_a_binary("running 17 tests"));
        assert!(!starts_a_binary("running the loop 3 times"));
        assert!(!starts_a_binary("running 3 times"));
    }

    /// The names of one binary are its own: `tests::it_works` in a library
    /// and in its binary are two tests, and a verdict from the first must
    /// not make a panic in the second a failure.
    #[test]
    fn each_binary_is_read_on_its_own() {
        let run = "
running 1 test
test tests::it_works ... FAILED
test result: FAILED. 0 passed; 1 failed
running 1 test
---- tests::it_works stdout ----
thread 'tests::it_works' (2) panicked at src/main.rs:3:5:
passed anyway
";
        assert!(failures(&read_all(run)).is_empty());
    }

    #[test]
    fn a_verdict_word_is_found_where_a_terminal_colours_it() {
        let line = "test math::vector::tests::the_sum_is_four ... FAILED";
        let (from, to, verdict) = verdict_word(line).unwrap();
        assert_eq!((&line[from..to], verdict), ("FAILED", Verdict::Failed));
        let line = "test math::vector::tests::it_is_ignored ... ignored, slow";
        let (from, to, _) = verdict_word(line).unwrap();
        assert_eq!(&line[from..to], "ignored");
        let line = "test tests::it_panics_as_expected - should panic ... ok";
        let (from, to, verdict) = verdict_word(line).unwrap();
        assert_eq!((&line[from..to], verdict), ("ok", Verdict::Passed));
        let line = "test result: FAILED. 3 passed; 6 failed; 1 ignored";
        let (from, to, verdict) = verdict_word(line).unwrap();
        assert_eq!((&line[from..to], verdict), ("FAILED", Verdict::Failed));
        let line = "test result: ok. 12 passed; 0 failed";
        let (from, to, _) = verdict_word(line).unwrap();
        assert_eq!(&line[from..to], "ok");
        assert_eq!(verdict_word("running 17 tests"), None);
        assert_eq!(
            verdict_word("test bench_add ... bench:   1,234 ns/iter"),
            None
        );
    }

    #[test]
    fn a_files_module_is_read_off_its_path() {
        assert_eq!(
            module_of("core/src/math/vector.rs").as_deref(),
            Some("math::vector")
        );
        assert_eq!(module_of("core/src/math/mod.rs").as_deref(), Some("math"));
        assert_eq!(module_of("core/src/lib.rs").as_deref(), Some(""));
        assert_eq!(module_of("src/main.rs").as_deref(), Some(""));
        assert_eq!(module_of("src/util.rs").as_deref(), Some("util"));
        assert_eq!(module_of("src/bin/tool.rs").as_deref(), Some(""));
        assert_eq!(module_of("src/bin/tool/main.rs").as_deref(), Some(""));
        assert_eq!(module_of("src/bin/tool/parse.rs").as_deref(), Some("parse"));
        assert_eq!(module_of("core/tests/flight.rs").as_deref(), Some(""));
        assert_eq!(module_of("tests/common/mod.rs").as_deref(), Some("common"));
        assert_eq!(module_of("examples/cross.rs").as_deref(), Some(""));
        assert_eq!(module_of("build.rs"), None);
        assert_eq!(module_of("core/src/data.toml"), None);
        // A package kept under `examples/`, as this repository keeps one,
        // and a module named `tests` inside `src/`.
        assert_eq!(
            module_of("examples/draw-vectors/src/lib.rs").as_deref(),
            Some("")
        );
        assert_eq!(
            module_of("examples/draw-vectors/examples/cross.rs").as_deref(),
            Some("")
        );
        assert_eq!(module_of("src/tests/mod.rs").as_deref(), Some("tests"));
        assert_eq!(
            module_of("src/tests/wind.rs").as_deref(),
            Some("tests::wind")
        );
    }

    #[test]
    fn a_lens_shows_how_its_tests_came_out() {
        let verdicts = HashMap::from([
            ("math::vector::tests::a".to_string(), Verdict::Passed),
            ("math::vector::tests::b".to_string(), Verdict::Failed),
            ("math::quat::tests::a".to_string(), Verdict::Passed),
            ("tests::c".to_string(), Verdict::Ignored),
        ]);
        let file = "core/src/math/vector.rs";
        assert_eq!(
            verdict_of_lens(&verdicts, file, "tests::a", false),
            Some(Verdict::Passed)
        );
        assert_eq!(
            verdict_of_lens(&verdicts, file, "tests::b", false),
            Some(Verdict::Failed)
        );
        assert_eq!(
            verdict_of_lens(&verdicts, file, "tests", true),
            Some(Verdict::Failed),
            "a module with a failure in it failed",
        );
        assert_eq!(
            verdict_of_lens(&verdicts, "core/src/math/quat.rs", "tests", true),
            Some(Verdict::Passed)
        );
        // Another file's test of the same name is not this one's.
        assert_eq!(
            verdict_of_lens(&verdicts, "core/src/util.rs", "tests::a", false),
            None
        );
        assert_eq!(
            verdict_of_lens(&verdicts, "core/src/lib.rs", "tests", true),
            None,
            "only ignored tests under it: nothing to show",
        );
    }
}
