//! The filter over what a bottom-dock panel lists — Output's lines,
//! Problems' rows — as VS Code's filter box in a panel's title row: the
//! text typed narrows to the rows that hold it, in any case, and the three
//! level switches beside it narrow to errors, to warnings or to the rest.
//!
//! It decides what is shown and nothing else: the lines a run printed and
//! the problems the servers found are kept whole under it, so a filter
//! cleared shows everything again and a line arriving while one is in force
//! is judged as it arrives.

use std::ops::Range;

use crate::ProblemRow;

/// What a row is counted under, for the level switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Other,
}

impl Level {
    /// An Output line's level, by the kind it is drawn as.
    pub fn of_output(kind: i32) -> Level {
        match kind {
            crate::run::kind::ERROR => Level::Error,
            crate::run::kind::WARNING => Level::Warning,
            _ => Level::Other,
        }
    }

    /// A problem's level, by its row's kind: 1 an error, 2 a warning.
    pub fn of_problem(kind: i32) -> Level {
        match kind {
            1 => Level::Error,
            2 => Level::Warning,
            _ => Level::Other,
        }
    }
}

/// The box and its switches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// What was typed, as typed.
    text: String,
    /// The same folded for comparing; empty is every row.
    folded: String,
    /// The levels switched on. None on is every level, as a row of chips
    /// none of which is picked narrows nothing.
    errors: bool,
    warnings: bool,
    other: bool,
}

impl Filter {
    /// What the box holds now — committed text, never an input method's
    /// composition, which is a reading on its way and not a word.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.folded = fold(text.trim());
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// A level's switch flipped.
    pub fn toggle(&mut self, level: Level) {
        let on = match level {
            Level::Error => &mut self.errors,
            Level::Warning => &mut self.warnings,
            Level::Other => &mut self.other,
        };
        *on = !*on;
    }

    pub fn is_on(&self, level: Level) -> bool {
        match level {
            Level::Error => self.errors,
            Level::Warning => self.warnings,
            Level::Other => self.other,
        }
    }

    /// Whether it narrows anything: an empty box and no switch on is every
    /// row, and the count beside the box says nothing then.
    pub fn narrows(&self) -> bool {
        !self.folded.is_empty() || self.errors || self.warnings || self.other
    }

    /// Whether a row of `level` passes the switches.
    pub fn passes(&self, level: Level) -> bool {
        !(self.errors || self.warnings || self.other) || self.is_on(level)
    }

    /// Whether `text` holds what was typed, in any case.
    pub fn matches(&self, text: &str) -> bool {
        self.folded.is_empty() || fold(text).contains(&self.folded)
    }

    /// Whether a row saying `text` at `level` is shown.
    pub fn shows(&self, text: &str, level: Level) -> bool {
        self.passes(level) && self.matches(text)
    }

    /// Where in `text` what was typed is, as byte ranges of `text` itself —
    /// every place, none overlapping, for the panel to mark. Empty while
    /// the box is.
    pub fn hits(&self, text: &str) -> Vec<Range<usize>> {
        if self.folded.is_empty() {
            return Vec::new();
        }
        // Folding can change a character's length (`İ` is two characters
        // in lower case), so each folded byte remembers which of `text`'s
        // characters it came from.
        let mut folded = String::with_capacity(text.len());
        let mut from = Vec::with_capacity(text.len());
        for (at, ch) in text.char_indices() {
            for lower in ch.to_lowercase() {
                let start = folded.len();
                folded.push(lower);
                from.extend(std::iter::repeat_n(at, folded.len() - start));
            }
        }
        let original = |folded_at: usize| from.get(folded_at).copied().unwrap_or(text.len());
        folded
            .match_indices(&self.folded)
            .filter_map(|(at, found)| {
                let start = original(at);
                let mut end = original(at + found.len());
                // A match ending inside one character's folding takes the
                // whole character.
                if end <= start {
                    end = text[start..]
                        .chars()
                        .next()
                        .map_or(text.len(), |c| start + c.len_utf8());
                }
                (end > start).then_some(start..end)
            })
            .collect()
    }

    /// `text` cut where it holds what was typed: each piece and whether it
    /// is a match, in order, covering the whole of it. Empty — draw the
    /// text as it is — when nothing matches or nothing was typed.
    pub fn pieces<'a>(&self, text: &'a str) -> Vec<(&'a str, bool)> {
        let hits = self.hits(text);
        if hits.is_empty() {
            return Vec::new();
        }
        let mut pieces = Vec::with_capacity(hits.len() * 2 + 1);
        let mut at = 0;
        for hit in hits {
            if hit.start > at {
                pieces.push((&text[at..hit.start], false));
            }
            pieces.push((&text[hit.clone()], true));
            at = hit.end;
        }
        if at < text.len() {
            pieces.push((&text[at..], false));
        }
        pieces
    }
}

/// `text` as it is compared: lower case.
fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// Problems under a filter: each problem shown when its message or its
/// place holds the text — or its file's name or folder does — and its level
/// passes; a file's heading only while something under it is shown, with
/// the count of what is. A heading is a row of kind 0, the problems under
/// it until the next.
pub fn narrow_problems(rows: Vec<ProblemRow>, filter: &Filter) -> Vec<ProblemRow> {
    if !filter.narrows() {
        return rows;
    }
    let mut shown = Vec::with_capacity(rows.len());
    let mut heading: Option<(ProblemRow, bool)> = None;
    let mut under: Vec<ProblemRow> = Vec::new();
    let flush = |shown: &mut Vec<ProblemRow>,
                 heading: Option<(ProblemRow, bool)>,
                 under: &mut Vec<ProblemRow>| {
        if under.is_empty() {
            return;
        }
        if let Some((mut heading, _)) = heading {
            heading.count = under.len() as i32;
            shown.push(heading);
        }
        shown.append(under);
    };
    for row in rows {
        if row.kind == 0 {
            flush(&mut shown, heading.take(), &mut under);
            let named = filter.matches(&row.text) || filter.matches(&row.detail);
            heading = Some((row, named));
            continue;
        }
        let named = heading.as_ref().is_some_and(|(_, named)| *named);
        let said = filter.matches(&row.text) || filter.matches(&row.detail);
        if filter.passes(Level::of_problem(row.kind)) && (said || named) {
            under.push(row);
        }
    }
    flush(&mut shown, heading, &mut under);
    shown
}

/// How many problems `rows` lists, headings left out.
pub fn problem_count(rows: &[ProblemRow]) -> usize {
    rows.iter().filter(|row| row.kind != 0).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Filter {
        let mut filter = Filter::default();
        filter.set_text(text);
        filter
    }

    #[test]
    fn an_empty_box_shows_everything_and_narrows_nothing() {
        let filter = typed("");
        assert!(!filter.narrows());
        assert!(filter.shows("anything at all", Level::Other));
        assert!(filter.hits("anything").is_empty());
        assert!(!typed("   ").narrows(), "spaces alone are not a word");
    }

    #[test]
    fn the_text_is_found_in_any_case() {
        let filter = typed("Error");
        assert!(filter.matches("error[E0308]: mismatched types"));
        assert!(filter.matches("  ERROR: could not compile"));
        assert!(!filter.matches("warning: unused variable"));
        assert!(typed("e0308").matches("error[E0308]: mismatched types"));
    }

    /// What is marked is where the text is in the line as printed, every
    /// place it is, in bytes of the line itself — a Chinese line included.
    #[test]
    fn every_match_is_marked_where_it_is_in_the_line() {
        let filter = typed("ab");
        assert_eq!(filter.hits("xAbyaBzab"), vec![1..3, 4..6, 7..9]);
        let line = "编译错误: Vector 在 向量.rs";
        let filter = typed("vector");
        let hits = filter.hits(line);
        assert_eq!(hits.len(), 1);
        assert_eq!(&line[hits[0].clone()], "Vector");
        let filter = typed("向量");
        assert_eq!(&line[filter.hits(line)[0].clone()], "向量");
        // A character whose lower case is longer than itself.
        let line = "İstanbul is here";
        let hit = &typed("stanbul").hits(line)[0];
        assert_eq!(&line[hit.clone()], "stanbul");
    }

    #[test]
    fn the_pieces_cover_the_line_and_say_which_are_matches() {
        let filter = typed("ec");
        let pieces = filter.pieces("vector: Vec");
        assert_eq!(
            pieces,
            vec![("v", false), ("ec", true), ("tor: V", false), ("ec", true)]
        );
        assert_eq!(
            pieces.iter().map(|(text, _)| *text).collect::<String>(),
            "vector: Vec"
        );
        assert!(filter.pieces("nothing here").is_empty());
    }

    /// No switch on is every level; one on is that level alone; two are
    /// either.
    #[test]
    fn the_level_switches_narrow_to_what_is_on() {
        let mut filter = Filter::default();
        assert!(filter.passes(Level::Error) && filter.passes(Level::Other));
        filter.toggle(Level::Error);
        assert!(filter.narrows());
        assert!(filter.passes(Level::Error));
        assert!(!filter.passes(Level::Warning));
        assert!(!filter.passes(Level::Other));
        filter.toggle(Level::Warning);
        assert!(filter.passes(Level::Warning));
        assert!(!filter.passes(Level::Other));
        filter.toggle(Level::Error);
        filter.toggle(Level::Warning);
        assert!(!filter.narrows(), "both off again is everything");
        assert_eq!(Level::of_output(crate::run::kind::ERROR), Level::Error);
        assert_eq!(Level::of_output(crate::run::kind::COMMAND), Level::Other);
        assert_eq!(Level::of_problem(2), Level::Warning);
    }

    fn heading(name: &str, folder: &str, count: i32) -> ProblemRow {
        ProblemRow {
            kind: 0,
            text: name.into(),
            detail: folder.into(),
            count,
            ..Default::default()
        }
    }

    fn problem(kind: i32, message: &str, place: &str) -> ProblemRow {
        ProblemRow {
            kind,
            text: message.into(),
            detail: place.into(),
            ..Default::default()
        }
    }

    fn listed(rows: &[ProblemRow]) -> Vec<String> {
        rows.iter()
            .map(|row| format!("{}:{}:{}", row.kind, row.text, row.count))
            .collect()
    }

    fn problems() -> Vec<ProblemRow> {
        vec![
            heading("lib.rs", "src", 2),
            problem(1, "cannot find type `Vector3d`", "rustc(E0412) 7:12"),
            problem(2, "unused import: `std::fmt`", "rustc 1:5"),
            heading("main.rs", "examples", 1),
            problem(1, "mismatched types", "rustc(E0308) 3:9"),
        ]
    }

    /// A problem is found by its message or its place, and a file's
    /// heading stays only over what is still shown, counting it.
    #[test]
    fn problems_are_narrowed_under_their_files() {
        let rows = narrow_problems(problems(), &typed("vector3d"));
        assert_eq!(
            listed(&rows),
            ["0:lib.rs:1", "1:cannot find type `Vector3d`:0"]
        );
        let rows = narrow_problems(problems(), &typed("E0308"));
        assert_eq!(listed(&rows), ["0:main.rs:1", "1:mismatched types:0"]);
        assert!(narrow_problems(problems(), &typed("nowhere")).is_empty());
        assert_eq!(problem_count(&problems()), 3);
    }

    /// A file's name in the box shows every problem in that file.
    #[test]
    fn a_file_named_shows_what_is_in_it() {
        let rows = narrow_problems(problems(), &typed("lib.rs"));
        assert_eq!(problem_count(&rows), 2);
        assert_eq!(rows[0].count, 2);
    }

    #[test]
    fn problems_narrow_by_level_too() {
        let mut filter = Filter::default();
        filter.toggle(Level::Warning);
        let rows = narrow_problems(problems(), &filter);
        assert_eq!(
            listed(&rows),
            ["0:lib.rs:1", "2:unused import: `std::fmt`:0"]
        );
        // Nothing narrows: the rows come back as they went in.
        assert_eq!(narrow_problems(problems(), &Filter::default()), problems());
    }
}
