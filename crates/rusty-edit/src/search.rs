//! Project-wide text search.
//!
//! Literal text, not regex — the first question a search box answers is
//! "where is this string", and a query language that quietly treats `.` as
//! wildcard answers a different question than the one asked. The walker is
//! the tree's: same ignore rules, so search never surfaces a file the tree
//! would not show.

use std::path::Path;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;

use crate::model::{SearchHit, SearchResults};

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
    /// Gitignore-style globs, comma-separated, as search boxes write them:
    /// `*.rs, src/**`. Empty means everything.
    pub include: String,
    pub exclude: String,
}

pub fn search(root: &Path, query: &Query) -> SearchResults {
    if query.text.is_empty() {
        return SearchResults::default();
    }

    let mut hits = Vec::new();
    let mut files = 0u32;
    let mut truncated = false;

    // Include/exclude are compiled into override globs — the same matcher
    // gitignore uses, so `*.rs` and `src/**` mean what they mean there.
    let mut overrides = OverrideBuilder::new(root);
    for pattern in split_globs(&query.include) {
        if overrides.add(&pattern).is_err() {
            return SearchResults {
                error: Some(format!("cannot parse include pattern `{pattern}`")),
                ..SearchResults::default()
            };
        }
    }
    for pattern in split_globs(&query.exclude) {
        if overrides.add(&format!("!{pattern}")).is_err() {
            return SearchResults {
                error: Some(format!("cannot parse exclude pattern `{pattern}`")),
                ..SearchResults::default()
            };
        }
    }
    let Ok(overrides) = overrides.build() else {
        return SearchResults {
            error: Some("the include/exclude patterns do not combine".to_string()),
            ..SearchResults::default()
        };
    };

    let walk = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        .require_git(false)
        .overrides(overrides)
        // `.git` holds thousands of files nobody is text-searching; without
        // this it drowned every query the moment a project had history.
        .filter_entry(|entry| entry.file_name().to_string_lossy() != ".git")
        .build();

    'files: for found in walk.flatten() {
        let path = found.path();
        if !found.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if found.metadata().map(|m| m.len() > MAX_FILE).unwrap_or(true) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        // A null byte early on is how binaries announce themselves.
        if bytes.iter().take(8192).any(|&b| b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");

        let mut any = false;
        for (index, line) in text.lines().enumerate() {
            let mut from = 0;
            while let Some(at) = find_from(line, &query.text, from, query.case_sensitive) {
                from = at + query.text.len().max(1);
                if query.whole_word && !word_bounded(line, at, query.text.len()) {
                    continue;
                }
                any = true;
                hits.push(windowed(&relative, index as u32, line, at, query.text.len()));
                if hits.len() >= MAX_HITS {
                    truncated = true;
                    files += 1;
                    break 'files;
                }
            }
        }
        if any {
            files += 1;
        }
    }

    SearchResults {
        hits,
        files,
        truncated,
        error: None,
    }
}

/// `*.rs, src/**` → the patterns, trimmed, empties dropped.
fn split_globs(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether the match at `at` has non-identifier characters (or edges) on both
/// sides.
fn word_bounded(line: &str, at: usize, len: usize) -> bool {
    let ident = |ch: char| ch.is_alphanumeric() || ch == '_';
    let before_ok = line[..at].chars().next_back().is_none_or(|ch| !ident(ch));
    let after_ok = line[at + len..].chars().next().is_none_or(|ch| !ident(ch));
    before_ok && after_ok
}

/// The next match at or after byte `from`, ASCII-case-insensitively unless
/// asked otherwise. Only offsets that fall on character boundaries count —
/// a byte-window comparison can otherwise land mid-UTF-8 and make slicing
/// panic later.
fn find_from(line: &str, query: &str, from: usize, case_sensitive: bool) -> Option<usize> {
    if from >= line.len() {
        return None;
    }
    if case_sensitive {
        return line[from..].find(query).map(|at| from + at);
    }
    let hay = line.as_bytes();
    let needle = query.as_bytes();
    let mut at = from;
    while at + needle.len() <= hay.len() {
        if line.is_char_boundary(at) && hay[at..at + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(at);
        }
        at += 1;
    }
    None
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
    while !line.is_char_boundary(end) {
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
    fn whole_word_rejects_substrings() {
        let dir = project();
        // "gain" appears as the whole identifier and inside nothing else in
        // the fixture; "gai" only as a fragment.
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
}
