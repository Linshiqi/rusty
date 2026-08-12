//! Project-wide text search.
//!
//! Literal text, not regex — the first question a search box answers is
//! "where is this string", and a query language that quietly treats `.` as
//! wildcard answers a different question than the one asked. The walker is
//! the tree's: same ignore rules, so search never surfaces a file the tree
//! would not show.

use std::path::Path;

use ignore::WalkBuilder;

use crate::model::{SearchHit, SearchResults};

/// Stop after this many hits. The panel says so when it happens.
const MAX_HITS: usize = 500;
/// Files bigger than this are skipped — nobody is text-searching an ELF, and
/// a generated 40MB file would eat the whole budget.
const MAX_FILE: u64 = 1_000_000;
/// A line longer than this is windowed around the match.
const MAX_LINE: usize = 240;

pub fn search(root: &Path, query: &str, case_sensitive: bool) -> SearchResults {
    if query.is_empty() {
        return SearchResults::default();
    }

    let mut hits = Vec::new();
    let mut files = 0u32;
    let mut truncated = false;

    let walk = WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .parents(false)
        .require_git(false)
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
            while let Some(at) = find_from(line, query, from, case_sensitive) {
                any = true;
                hits.push(windowed(&relative, index as u32, line, at, query.len()));
                from = at + query.len().max(1);
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
    }
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

    #[test]
    fn finds_case_insensitively_and_reports_scalar_columns() {
        let dir = project();
        let results = search(dir.path(), "gain", false);
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
        let results = search(dir.path(), "GAIN", true);
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

        let results = search(dir.path(), "needle", false);
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

        let results = search(dir.path(), "NEEDLE", false);
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert_eq!(
            &hit.text[hit.span_start as usize..hit.span_end as usize],
            "needle"
        );
        assert_eq!(hit.col, 8);
    }
}
