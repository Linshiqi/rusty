//! Project-wide text search, on ripgrep's own engine.
//!
//! Both halves of "why is ripgrep fast" are libraries, and this uses both:
//! `ignore` walks the tree in parallel with the same gitignore rules the
//! file tree shows, and `grep-searcher`/`grep-regex` do the matching — SIMD
//! literal search, real Unicode case folding, `-w` word boundaries, binary
//! detection. The walker and the tree share ignore rules, so search never
//! surfaces a file the tree would hide.
//!
//! Literal by default; regex sits behind an explicit toggle, and a pattern
//! that does not parse is named rather than silently matching nothing.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::Lossy;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::overrides::OverrideBuilder;
use ignore::{WalkBuilder, WalkState};

use crate::model::{ReplaceOutcome, SearchHit, SearchResults, Skipped};

/// Stop after this many hits. The panel says so when it happens.
const MAX_HITS: usize = 500;
/// Files bigger than this are skipped — nobody is text-searching an ELF, and
/// a generated 40MB file would eat the whole budget.
const MAX_FILE: u64 = 1_000_000;
/// A line longer than this is windowed around the match.
const MAX_LINE: usize = 240;

/// What to look for, and where not to.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    /// Match only where the neighbours are not identifier characters.
    pub whole_word: bool,
    /// Treat `text` as a regex rather than literal text.
    pub regex: bool,
    /// Gitignore-style globs, comma-separated, as search boxes write them:
    /// `*.rs, src/**`. Empty means everything.
    pub include: String,
    pub exclude: String,
}

pub fn search(root: &Path, query: &Query) -> SearchResults {
    if query.text.is_empty() {
        return SearchResults::default();
    }

    let matcher = match build_matcher(query) {
        Ok(matcher) => matcher,
        Err(error) => {
            return SearchResults {
                error: Some(error),
                ..SearchResults::default()
            };
        }
    };

    let overrides = match build_overrides(root, query) {
        Ok(overrides) => overrides,
        Err(error) => {
            return SearchResults {
                error: Some(error),
                ..SearchResults::default()
            };
        }
    };

    let hits: Mutex<Vec<SearchHit>> = Mutex::new(Vec::new());
    let count = AtomicUsize::new(0);

    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        .require_git(false)
        .overrides(overrides)
        // `.git` holds thousands of files nobody is text-searching; without
        // this it drowned every query the moment a project had history.
        .filter_entry(|entry| entry.file_name().to_string_lossy() != ".git")
        .build_parallel()
        .run(|| {
            // Per-thread searcher (stateful); the matcher clones cheaply.
            let matcher = matcher.clone();
            let mut searcher = SearcherBuilder::new()
                .binary_detection(BinaryDetection::quit(0))
                .line_number(true)
                .build();
            let hits = &hits;
            let count = &count;
            let root = root.to_path_buf();

            Box::new(move |entry| {
                if count.load(Ordering::Relaxed) >= MAX_HITS {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                if entry.metadata().map(|m| m.len() > MAX_FILE).unwrap_or(true) {
                    return WalkState::Continue;
                }
                let Ok(relative) = entry.path().strip_prefix(&root) else {
                    return WalkState::Continue;
                };
                let relative = relative.to_string_lossy().replace('\\', "/");

                let mut file_hits: Vec<SearchHit> = Vec::new();
                let _ = searcher.search_path(
                    &matcher,
                    entry.path(),
                    Lossy(|line_number, line| {
                        // The sink hands over whole matching lines; the
                        // matcher re-runs on the line for exact spans —
                        // several per line when the text repeats.
                        let line = line.strip_suffix('\n').unwrap_or(line);
                        let line = line.strip_suffix('\r').unwrap_or(line);
                        let _ = matcher.find_iter(line.as_bytes(), |found| {
                            file_hits.push(windowed(
                                &relative,
                                (line_number as u32).saturating_sub(1),
                                line,
                                found.start(),
                                found.end() - found.start(),
                            ));
                            true
                        });
                        Ok(true)
                    }),
                );

                if file_hits.is_empty() {
                    return WalkState::Continue;
                }
                let already = count.fetch_add(file_hits.len(), Ordering::Relaxed);
                if already < MAX_HITS
                    && let Ok(mut all) = hits.lock()
                {
                    all.extend(file_hits);
                }
                if count.load(Ordering::Relaxed) >= MAX_HITS {
                    return WalkState::Quit;
                }
                WalkState::Continue
            })
        });

    let mut hits = hits.into_inner().unwrap_or_default();
    let truncated = hits.len() > MAX_HITS || count.load(Ordering::Relaxed) > MAX_HITS;
    hits.truncate(MAX_HITS);
    // Parallel arrival order is racy; sorting keeps the panel identical
    // between two runs of the same query.
    hits.sort_by(|a, b| (&a.path, a.line, a.col).cmp(&(&b.path, b.line, b.col)));

    let mut files = 0u32;
    let mut last: Option<&str> = None;
    for hit in &hits {
        if last != Some(hit.path.as_str()) {
            files += 1;
            last = Some(hit.path.as_str());
        }
    }

    SearchResults {
        hits,
        files,
        truncated,
        error: None,
    }
}

/// The compiled pattern: literal unless asked otherwise, word-bounded and
/// case-folded by the engine rather than by hand.
/// Replace every match in the project, and say what was left alone.
///
/// **The same query, the same scope, the same walk as [`search`].** What is
/// rewritten is exactly what the panel listed; a second set of rules built a
/// slightly different way is how a replace touches a file nobody saw. The one
/// deliberate difference is that this has no hit cap — replacing the first 500
/// of 900 matches would be worse than refusing.
///
/// **A file with unsaved changes in the editor is never written.** Replacing
/// on disk under a draft is the "editor eats work" failure in its purest form:
/// the draft is still on screen, still looks authoritative, and the next save
/// puts the un-replaced text back. The caller passes those paths in `drafts`
/// and they come back in `skipped`, named.
///
/// Bytes in, bytes out, and a match never spans a line — so a CRLF file stays
/// CRLF and a file with no trailing newline keeps not having one. Not
/// incidental: a bulk edit turning line endings over is how this repository
/// twice made a diff unreviewable.
pub fn replace(root: &Path, query: &Query, replacement: &str, drafts: &[String]) -> ReplaceOutcome {
    if query.text.is_empty() {
        return ReplaceOutcome::default();
    }

    let engine = match build_replacer(query) {
        Ok(engine) => engine,
        Err(error) => {
            return ReplaceOutcome {
                error: Some(error),
                ..ReplaceOutcome::default()
            };
        }
    };
    let overrides = match build_overrides(root, query) {
        Ok(overrides) => overrides,
        Err(error) => {
            return ReplaceOutcome {
                error: Some(error),
                ..ReplaceOutcome::default()
            };
        }
    };

    let outcome: Mutex<ReplaceOutcome> = Mutex::new(ReplaceOutcome::default());

    WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        .require_git(false)
        .overrides(overrides)
        .filter_entry(|entry| entry.file_name().to_string_lossy() != ".git")
        .build_parallel()
        .run(|| {
            let engine = engine.clone();
            let outcome = &outcome;
            let root = root.to_path_buf();
            let replacement = replacement.to_string();
            // Only a *regex* search promises `$1`. In literal mode the
            // replacement is text somebody typed, and expanding it turned
            // `$1` — or `$100`, or `$HOME` — into nothing at all, deleting
            // the match instead of replacing it.
            let expand = query.regex;

            Box::new(move |entry| {
                let Ok(entry) = entry else {
                    return WalkState::Continue;
                };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                if entry.metadata().map(|m| m.len() > MAX_FILE).unwrap_or(true) {
                    return WalkState::Continue;
                }
                let Ok(relative) = entry.path().strip_prefix(&root) else {
                    return WalkState::Continue;
                };
                let relative = relative.to_string_lossy().replace('\\', "/");

                // Read as bytes and decode. A file that is not UTF-8 is passed
                // over in silence, exactly as the search's binary detection
                // passes over it — reporting files the panel never listed
                // would be noise about something nobody asked to change.
                let Ok(bytes) = std::fs::read(entry.path()) else {
                    return WalkState::Continue;
                };
                let Ok(text) = String::from_utf8(bytes) else {
                    return WalkState::Continue;
                };

                let count = engine.find_iter(&text).count();
                if count == 0 {
                    return WalkState::Continue;
                }
                if drafts.contains(&relative) {
                    outcome.lock().unwrap().skipped.push(Skipped {
                        path: relative,
                        reason: "unsaved".to_string(),
                    });
                    return WalkState::Continue;
                }

                let replaced = if expand {
                    engine.replace_all(&text, replacement.as_str())
                } else {
                    engine.replace_all(&text, regex::NoExpand(replacement.as_str()))
                };
                match std::fs::write(entry.path(), replaced.as_bytes()) {
                    Ok(()) => {
                        let mut outcome = outcome.lock().unwrap();
                        outcome.changed.push(relative);
                        outcome.replaced += count as u32;
                    }
                    Err(_) => outcome.lock().unwrap().skipped.push(Skipped {
                        path: relative,
                        reason: "write-failed".to_string(),
                    }),
                }
                WalkState::Continue
            })
        });

    let mut outcome = outcome.into_inner().unwrap_or_default();
    // The walk is parallel, so the order it finished in is not an order.
    outcome.changed.sort();
    outcome.skipped.sort_by(|a, b| a.path.cmp(&b.path));
    outcome
}

/// The substituting half of [`build_matcher`].
///
/// A second engine rather than the grep matcher, because only this one
/// substitutes: `$1` in the replacement has to mean the first capture, which
/// is what anybody who turned the regex toggle on is expecting. The flags are
/// the same three, so what it finds is what the panel found.
fn build_replacer(query: &Query) -> Result<regex::Regex, String> {
    let pattern = if query.regex {
        query.text.clone()
    } else {
        regex::escape(&query.text)
    };
    let pattern = if query.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!query.case_sensitive)
        // Line-anchored like the search is: `^` and `$` are a line's edges,
        // not the file's, or a pattern that worked in the panel finds nothing
        // here.
        .multi_line(true)
        .build()
        .map_err(|error| {
            let reason = error.to_string();
            let reason = reason
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("the pattern does not parse");
            format!("the pattern does not parse: {reason}")
        })
}

/// Which files are in scope, as override globs.
///
/// Shared with [`replace`] deliberately: search and replace disagreeing about
/// what `*.rs` covers would mean rewriting a file the panel never listed, and
/// two copies of this is exactly how that happens.
fn build_overrides(root: &Path, query: &Query) -> Result<ignore::overrides::Override, String> {
    // The same matcher gitignore uses, so `*.rs` and `src/**` mean what they
    // mean there.
    let mut overrides = OverrideBuilder::new(root);
    for pattern in split_globs(&query.include) {
        overrides
            .add(&pattern)
            .map_err(|_| format!("cannot parse include pattern `{pattern}`"))?;
    }
    for pattern in split_globs(&query.exclude) {
        overrides
            .add(&format!("!{pattern}"))
            .map_err(|_| format!("cannot parse exclude pattern `{pattern}`"))?;
    }
    overrides
        .build()
        .map_err(|_| "the include/exclude patterns do not combine".to_string())
}

fn build_matcher(query: &Query) -> Result<RegexMatcher, String> {
    let pattern = if query.regex {
        query.text.clone()
    } else {
        escape_literal(&query.text)
    };
    RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        .word(query.whole_word)
        .build(&pattern)
        .map_err(|error| {
            // The first line of the regex error names the problem; the rest
            // is a caret diagram that does not survive a status line.
            let reason = error.to_string();
            let reason = reason
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("the pattern does not parse");
            format!("the pattern does not parse: {reason}")
        })
}

/// Escape a literal for the regex engine, without pulling the full regex
/// crate into this crate's interface for one function.
fn escape_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() && !ch.is_ascii_alphanumeric() && ch != '_' && ch != ' ' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// `*.rs, src/**` → the patterns, trimmed, empties dropped.
fn split_globs(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// A hit, with the line cut down to a window around the match when it is too
/// long to show whole.
fn windowed(path: &str, line_index: u32, line: &str, at: usize, len: usize) -> SearchHit {
    let col = line[..at].chars().count() as u32;

    if line.len() <= MAX_LINE {
        return SearchHit {
            path: path.to_string(),
            line: line_index,
            col,
            text: line.to_string(),
            span_start: at as u32,
            span_end: (at + len) as u32,
        };
    }

    let mut start = at.saturating_sub(80);
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + len + 120).min(line.len());
    while end < line.len() && !line.is_char_boundary(end) {
        end += 1;
    }

    let mut text = String::new();
    if start > 0 {
        text.push('…');
    }
    let ellipsis = text.len();
    text.push_str(&line[start..end]);
    if end < line.len() {
        text.push('…');
    }

    SearchHit {
        path: path.to_string(),
        line: line_index,
        col,
        span_start: (ellipsis + (at - start)) as u32,
        span_end: (ellipsis + (at - start) + len) as u32,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("rusty-search")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("dirs");
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    let gain = 3; // GAIN knob\n}\n",
        )
        .expect("write");
        std::fs::write(dir.path().join(".gitignore"), "target/\n").expect("write");
        std::fs::create_dir_all(dir.path().join("target")).expect("dirs");
        std::fs::write(dir.path().join("target/out.rs"), "gain gain gain\n").expect("write");
        dir
    }

    fn plain(text: &str, case: bool) -> Query {
        Query {
            text: text.to_string(),
            case_sensitive: case,
            ..Query::default()
        }
    }

    #[test]
    fn finds_case_insensitively_and_reports_scalar_columns() {
        let dir = project();
        let results = search(dir.path(), &plain("gain", false));
        // Two on the same line — the identifier and the comment — and none
        // from target/, which the ignore rules hide without a git repo.
        assert_eq!(results.hits.len(), 2);
        assert_eq!(results.files, 1);
        assert_eq!(results.hits[0].path, "src/main.rs");
        assert_eq!(results.hits[0].line, 1);
        assert_eq!(results.hits[0].col, 8);
        assert_eq!(results.hits[1].col, 21);
        let hit = &results.hits[1];
        assert_eq!(
            &hit.text[hit.span_start as usize..hit.span_end as usize],
            "GAIN"
        );
    }

    #[test]
    fn case_sensitive_narrows_to_the_exact_spelling() {
        let dir = project();
        let results = search(dir.path(), &plain("GAIN", true));
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].line, 1);
    }

    #[test]
    fn whole_word_rejects_substrings() {
        let dir = project();
        let fragment = search(
            dir.path(),
            &Query {
                text: "gai".to_string(),
                whole_word: true,
                ..Query::default()
            },
        );
        assert_eq!(fragment.hits.len(), 0, "a fragment is not a word");
        let word = search(
            dir.path(),
            &Query {
                text: "gain".to_string(),
                whole_word: true,
                ..Query::default()
            },
        );
        assert_eq!(word.hits.len(), 2);
    }

    #[test]
    fn regex_mode_is_explicit_and_bad_patterns_are_named() {
        let dir = project();
        // As a literal, `g.in` matches nothing — the dot is a dot.
        let literal = search(dir.path(), &plain("g.in", false));
        assert_eq!(literal.hits.len(), 0);
        // As a regex, it matches both spellings of gain.
        let regex = search(
            dir.path(),
            &Query {
                text: "g.in".to_string(),
                regex: true,
                ..Query::default()
            },
        );
        assert_eq!(regex.hits.len(), 2);
        let bad = search(
            dir.path(),
            &Query {
                text: "(unclosed".to_string(),
                regex: true,
                ..Query::default()
            },
        );
        assert!(bad.error.is_some(), "a bad regex is named, not ignored");
    }

    #[test]
    fn include_and_exclude_globs_scope_the_walk() {
        let dir = project();
        std::fs::write(dir.path().join("notes.md"), "gain notes\n").expect("write");

        let all = search(dir.path(), &plain("gain", false));
        assert_eq!(all.files, 2);

        let only_rs = search(
            dir.path(),
            &Query {
                text: "gain".to_string(),
                include: "*.rs".to_string(),
                ..Query::default()
            },
        );
        assert_eq!(only_rs.files, 1);
        assert!(only_rs.hits.iter().all(|h| h.path.ends_with(".rs")));

        let no_rs = search(
            dir.path(),
            &Query {
                text: "gain".to_string(),
                exclude: "*.rs".to_string(),
                ..Query::default()
            },
        );
        assert_eq!(no_rs.files, 1);
        assert!(no_rs.hits.iter().all(|h| h.path.ends_with(".md")));

        let bad = search(
            dir.path(),
            &Query {
                text: "gain".to_string(),
                include: "[".to_string(),
                ..Query::default()
            },
        );
        assert!(bad.error.is_some(), "a bad glob is named, not ignored");
    }

    #[test]
    fn the_git_directory_is_never_searched() {
        let dir = project();
        std::fs::create_dir_all(dir.path().join(".git/info")).expect("dirs");
        std::fs::write(dir.path().join(".git/info/exclude"), "gain gain\n").expect("write");

        let results = search(dir.path(), &plain("gain", false));
        assert!(
            results.hits.iter().all(|h| !h.path.starts_with(".git")),
            "{:?}",
            results.hits,
        );
    }

    #[test]
    fn a_long_line_is_windowed_around_the_match() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-search")
            .tempdir()
            .expect("tempdir");
        let line = format!("{}needle{}", "x".repeat(400), "y".repeat(400));
        std::fs::write(dir.path().join("big.txt"), &line).expect("write");

        let results = search(dir.path(), &plain("needle", false));
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert!(hit.text.len() < 300, "windowed, not the whole line");
        assert!(hit.text.starts_with('…') && hit.text.ends_with('…'));
        assert_eq!(
            &hit.text[hit.span_start as usize..hit.span_end as usize],
            "needle"
        );
        assert_eq!(hit.col, 400);
    }

    #[test]
    fn cjk_before_the_match_does_not_break_the_span() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-search")
            .tempdir()
            .expect("tempdir");
        std::fs::write(dir.path().join("a.rs"), "// 中文注释 needle\n").expect("write");

        let results = search(dir.path(), &plain("NEEDLE", false));
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert_eq!(
            &hit.text[hit.span_start as usize..hit.span_end as usize],
            "needle"
        );
        assert_eq!(hit.col, 8);
    }

    #[test]
    fn results_are_sorted_despite_the_parallel_walk() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-search")
            .tempdir()
            .expect("tempdir");
        for name in ["zz.txt", "aa.txt", "mm.txt"] {
            std::fs::write(dir.path().join(name), "needle\nneedle\n").expect("write");
        }
        let results = search(dir.path(), &plain("needle", false));
        let order: Vec<_> = results.hits.iter().map(|h| (&h.path, h.line)).collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted, "stable output regardless of thread timing");
        assert_eq!(results.files, 3);
    }
    fn write(dir: &std::path::Path, name: &str, text: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    fn query(text: &str) -> Query {
        Query {
            text: text.to_string(),
            ..Query::default()
        }
    }

    #[test]
    fn replace_rewrites_every_match_and_counts_them() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "let gain = gain + 1;
",
        );
        write(
            dir.path(),
            "b.rs",
            "// no matches here
",
        );

        let outcome = replace(dir.path(), &query("gain"), "kp", &[]);
        assert_eq!(outcome.changed, vec!["a.rs"], "b.rs had nothing to change");
        assert_eq!(outcome.replaced, 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "let kp = kp + 1;
"
        );
    }

    /// The one that matters most: a file open with unsaved changes is named
    /// and left alone. Writing under a draft leaves the old text on screen
    /// looking authoritative, and the next save undoes the replace.
    #[test]
    fn a_file_with_an_unsaved_draft_is_skipped_and_named() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/main.rs",
            "let gain = 1;
",
        );

        let outcome = replace(
            dir.path(),
            &query("gain"),
            "kp",
            &["src/main.rs".to_string()],
        );
        assert!(
            outcome.changed.is_empty(),
            "nothing should have been written"
        );
        assert_eq!(outcome.replaced, 0);
        assert_eq!(outcome.skipped.len(), 1);
        assert_eq!(outcome.skipped[0].path, "src/main.rs");
        assert_eq!(outcome.skipped[0].reason, "unsaved");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
            "let gain = 1;
",
            "the file on disk is untouched"
        );
    }

    /// A bulk edit that turns line endings over makes the diff unreviewable,
    /// which is the failure this repository has already had twice.
    #[test]
    fn crlf_and_a_missing_final_newline_both_survive() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "let gain = 1;
let b = 2;
let c = gain;",
        );

        let outcome = replace(dir.path(), &query("gain"), "kp", &[]);
        assert_eq!(outcome.replaced, 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "let kp = 1;
let b = 2;
let c = kp;",
            "line endings and the absent trailing newline are not the replace's business"
        );
    }

    /// Turning the regex toggle on is a promise that `$1` means something.
    #[test]
    fn regex_mode_substitutes_captures() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "GPIO26 GPIO5
",
        );

        let spec = Query {
            text: r"GPIO(\d+)".to_string(),
            regex: true,
            ..Query::default()
        };
        let outcome = replace(dir.path(), &spec, "pin_$1", &[]);
        assert_eq!(outcome.replaced, 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "pin_26 pin_5
"
        );
    }

    /// Literal mode must not read `$1` as a capture, or replacing with a shell
    /// variable rewrites the file with nothing.
    #[test]
    fn literal_mode_takes_the_replacement_literally() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "let x = VALUE;
",
        );

        let outcome = replace(dir.path(), &query("VALUE"), "$1", &[]);
        assert_eq!(outcome.replaced, 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "let x = $1;
",
            "a literal search takes a literal replacement"
        );
    }

    /// The scope rules are the search's, so a replace cannot reach a file the
    /// panel would not have listed.
    #[test]
    fn exclude_globs_keep_a_file_out_of_a_replace() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "keep.rs",
            "gain
",
        );
        write(
            dir.path(),
            "skip.rs",
            "gain
",
        );

        let spec = Query {
            text: "gain".to_string(),
            exclude: "skip.rs".to_string(),
            ..Query::default()
        };
        let outcome = replace(dir.path(), &spec, "kp", &[]);
        assert_eq!(outcome.changed, vec!["keep.rs"]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("skip.rs")).unwrap(),
            "gain
"
        );
    }

    #[test]
    fn a_pattern_that_does_not_parse_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "a.rs",
            "gain
",
        );

        let spec = Query {
            text: "gain(".to_string(),
            regex: true,
            ..Query::default()
        };
        let outcome = replace(dir.path(), &spec, "kp", &[]);
        assert!(outcome.error.is_some(), "a broken pattern has to say so");
        assert!(outcome.changed.is_empty());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a.rs")).unwrap(),
            "gain
"
        );
    }
}
