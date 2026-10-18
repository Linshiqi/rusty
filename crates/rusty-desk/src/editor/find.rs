//! Find and replace in the open document, as arithmetic: what a query
//! matches, where the next match is, what a replacement writes.
//!
//! VS Code's find widget is the reference: match case, whole word and
//! regular expressions as toggles, a scope that is the selection, `$1` and
//! `\n` in a regular expression's replacement, Enter for the next match and
//! Shift+Enter for the one before, wrapping at the ends. One engine for
//! every mode — a literal query is a pattern escaped by `regex::escape`,
//! never by a second escaper of our own (CLAUDE.md, `\<` and `\>`).

use regex::{Captures, Regex, RegexBuilder};

use super::lines::Lines;

/// More matches than this are not drawn or counted one by one: a one-letter
/// query in a long file would otherwise be a hundred thousand marks.
pub const CAP: usize = 20_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case: bool,
    pub word: bool,
    pub regex: bool,
}

/// What a query found: the matches in order, by byte, and whether there
/// were more than [`CAP`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub matches: Vec<(usize, usize)>,
    pub capped: bool,
}

impl Query {
    /// The pattern the query is, or the regular expression's own complaint.
    fn pattern(&self) -> Result<Regex, String> {
        let source = if self.regex {
            self.text.clone()
        } else {
            regex::escape(&self.text)
        };
        RegexBuilder::new(&source)
            .case_insensitive(!self.case)
            .multi_line(true)
            .build()
            .map_err(|error| match error {
                regex::Error::Syntax(text) => text.lines().last().unwrap_or("").trim().to_string(),
                other => other.to_string(),
            })
    }

    /// Every match within `scope` (the whole text when `None`). An empty
    /// query finds nothing; a regular expression that does not read is the
    /// error, for the bar to say.
    pub fn search(&self, text: &str, scope: Option<(usize, usize)>) -> Result<Found, String> {
        if self.text.is_empty() {
            return Ok(Found::default());
        }
        let pattern = self.pattern()?;
        let (from, to) = scope.unwrap_or((0, text.len()));
        let mut found = Found::default();
        // `find_at` rather than `find_iter` over a slice: a scope starting
        // part way into a word must still see what comes before it.
        let mut at = from;
        while at <= to {
            let Some(m) = pattern.find_at(&text[..to], at) else {
                break;
            };
            let (start, end) = (m.start(), m.end());
            if !self.word || whole_word(text, start, end) {
                if found.matches.len() == CAP {
                    found.capped = true;
                    break;
                }
                found.matches.push((start, end));
            }
            // An empty match (`^`) moves on by a character, not by nothing.
            at = if end > start {
                end
            } else {
                match text[end..to].chars().next() {
                    Some(ch) => end + ch.len_utf8(),
                    None => break,
                }
            };
        }
        Ok(found)
    }

    /// What replacing the match at `range` with `with` writes. A regular
    /// expression's replacement reads `$1`, `$0` and `$&`, `$<name>`, `$$`,
    /// `\n`, `\t` and `\\`, as VS Code's does; anything else is written as
    /// typed.
    pub fn replacement(&self, text: &str, range: (usize, usize), with: &str) -> String {
        if !self.regex {
            return with.to_string();
        }
        let captures = self
            .pattern()
            .ok()
            .and_then(|pattern| pattern.captures_at(&text[..range.1], range.0))
            .filter(|caps| caps.get(0).is_some_and(|m| (m.start(), m.end()) == range));
        expand(with, captures.as_ref(), &text[range.0..range.1])
    }

    /// Every match in `scope` replaced, in one text: the text after and
    /// where each replacement landed in it. `None` when nothing matched.
    pub fn replace_all(
        &self,
        text: &str,
        scope: Option<(usize, usize)>,
        with: &str,
    ) -> Option<(String, Vec<(usize, usize)>)> {
        let found = self.search(text, scope).ok()?;
        if found.matches.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut landed = Vec::with_capacity(found.matches.len());
        let mut copied = 0;
        for &range in &found.matches {
            out.push_str(&text[copied..range.0]);
            let replaced = self.replacement(text, range, with);
            landed.push((out.len(), out.len() + replaced.len()));
            out.push_str(&replaced);
            copied = range.1;
        }
        out.push_str(&text[copied..]);
        Some((out, landed))
    }
}

use rusty_lsp::complete::is_word;

/// VS Code's whole word: each end of the match is the text's end, a
/// character that is not a word's, or the match's own first or last
/// character not being one — so `->` is found whole beside a name.
fn whole_word(text: &str, start: usize, end: usize) -> bool {
    let matched = &text[start..end];
    let left = text[..start]
        .chars()
        .next_back()
        .is_none_or(|c| !is_word(c))
        || matched.chars().next().is_none_or(|c| !is_word(c));
    let right = text[end..].chars().next().is_none_or(|c| !is_word(c))
        || matched.chars().next_back().is_none_or(|c| !is_word(c));
    left && right
}

/// A replacement with its references filled in from `captures`; `whole` is
/// the matched text, for when the match has no captures to offer.
fn expand(with: &str, captures: Option<&Captures>, whole: &str) -> String {
    let group = |n: usize| -> &str {
        match captures {
            Some(caps) => caps.get(n).map_or("", |m| m.as_str()),
            None if n == 0 => whole,
            None => "",
        }
    };
    let groups = captures.map_or(1, |caps| caps.len());
    let mut out = String::with_capacity(with.len());
    let mut chars = with.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        match ch {
            '\\' => match chars.peek().map(|&(_, c)| c) {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push('\\'),
            },
            '$' => match chars.peek().map(|&(_, c)| c) {
                Some('$') => {
                    chars.next();
                    out.push('$');
                }
                Some('&') => {
                    chars.next();
                    out.push_str(group(0));
                }
                Some('<') => {
                    let rest = &with[at + 2..];
                    match (rest.find('>'), captures) {
                        (Some(close), Some(caps)) => {
                            let name = &rest[..close];
                            out.push_str(caps.name(name).map_or("", |m| m.as_str()));
                            for _ in 0..=close {
                                chars.next();
                            }
                            chars.next();
                        }
                        _ => out.push('$'),
                    }
                }
                Some(d) if d.is_ascii_digit() => {
                    // Two digits when the two name a group there is, as
                    // JavaScript reads `$12`; otherwise one, so `$1a` is
                    // group one and an `a`.
                    let first = d.to_digit(10).unwrap_or(0) as usize;
                    chars.next();
                    let second = chars
                        .peek()
                        .and_then(|&(_, c)| c.to_digit(10))
                        .map(|s| first * 10 + s as usize)
                        .filter(|&both| both < groups);
                    match second {
                        Some(both) => {
                            chars.next();
                            out.push_str(group(both));
                        }
                        None if first < groups => out.push_str(group(first)),
                        None => {
                            out.push('$');
                            out.push(d);
                        }
                    }
                }
                _ => out.push('$'),
            },
            other => out.push(other),
        }
    }
    out
}

/// What the rows draw for find: every match, the one the selection is, and
/// the scope.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub matches: Vec<(usize, usize)>,
    pub current: Option<usize>,
    pub scope: Option<(usize, usize)>,
}

impl Marks {
    /// The matches drawn on a line running from `start` to `end`, by index:
    /// those that touch it, less one that only ends where it starts.
    pub fn on(&self, start: usize, end: usize) -> std::ops::Range<usize> {
        let first = self
            .matches
            .partition_point(|m| m.1 < start || (m.1 == start && m.0 < start));
        let last = self.matches.partition_point(|m| m.0 <= end);
        first..last.max(first)
    }

    /// Every line something is drawn on, in order.
    pub fn lines(&self, lines: &Lines) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        let mut span = |from: usize, to: usize| {
            for line in lines.line_of(from)..=lines.line_of(to) {
                if out.last() != Some(&line) {
                    out.push(line);
                }
            }
        };
        for &(from, to) in &self.matches {
            span(from, to);
        }
        if let Some((from, to)) = self.scope {
            span(from, to);
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Where `at` lands after `matches` were replaced by what `landed` says: past
/// the replacements before it, and at the start of one it was inside.
pub fn carried(at: usize, matches: &[(usize, usize)], landed: &[(usize, usize)]) -> usize {
    let mut shift: isize = 0;
    for (&(from, to), &(new_from, new_to)) in matches.iter().zip(landed) {
        if to <= at {
            shift += (new_to - new_from) as isize - (to - from) as isize;
        } else if from < at {
            return new_from;
        } else {
            break;
        }
    }
    (at as isize + shift) as usize
}

/// The match after `at` — the first starting at or past it — wrapping to
/// the first; `None` when there are none.
pub fn after(matches: &[(usize, usize)], at: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let next = matches.partition_point(|m| m.0 < at);
    Some(if next == matches.len() { 0 } else { next })
}

/// The match before `at` — the last ending at or before it — wrapping to
/// the last.
pub fn before(matches: &[(usize, usize)], at: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let past = matches.partition_point(|m| m.0 < at);
    Some(if past == 0 {
        matches.len() - 1
    } else {
        past - 1
    })
}

/// Which match a selection is, when it is exactly one.
pub fn current(matches: &[(usize, usize)], selection: (usize, usize)) -> Option<usize> {
    let at = matches.partition_point(|m| m.0 < selection.0);
    (matches.get(at) == Some(&selection)).then_some(at)
}

/// What Ctrl+F puts in the field: a selection on one line, else the word
/// at the caret — VS Code's `seedSearchStringFromSelection`.
pub fn seed(text: &str, selection: (usize, usize)) -> Option<String> {
    let (start, end) = selection;
    if start < end {
        let picked = &text[start..end];
        return (!picked.contains('\n')).then(|| picked.to_string());
    }
    super::cursors::word_at(text, start).map(|(s, e)| text[s..e].to_string())
}

/// A range carried across an edit that rewrote `[from, to)` of the new text
/// and changed its length by `delta`: before the edit it stays, after it
/// moves by `delta`, and inside it is held to what the edit left.
pub fn follow(range: (usize, usize), from: usize, to: usize, delta: isize) -> (usize, usize) {
    let old_to = (to as isize - delta).max(from as isize) as usize;
    let carry = |at: usize| {
        if at <= from {
            at
        } else if at >= old_to {
            (at as isize + delta).max(from as isize) as usize
        } else {
            at.min(to)
        }
    };
    let (start, end) = (carry(range.0), carry(range.1));
    (start, end.max(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(text: &str) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }

    fn found(q: &Query, text: &str) -> Vec<(usize, usize)> {
        q.search(text, None).unwrap().matches
    }

    #[test]
    fn a_literal_query_is_literal_even_with_the_regex_alphabet_in_it() {
        let text = "let v: Vec<u8> = a -> b; Vec<u8>";
        assert_eq!(found(&query("Vec<u8>"), text), [(7, 14), (25, 32)]);
        assert_eq!(found(&query("->"), text), [(19, 21)]);
        assert_eq!(found(&query("a.b"), "axb a.b"), [(4, 7)]);
    }

    #[test]
    fn case_is_ignored_unless_asked_for_and_not_only_in_ascii() {
        let text = "Gain gain GAIN Äpfel äpfel";
        assert_eq!(found(&query("gain"), text).len(), 3);
        let exact = Query {
            case: true,
            ..query("gain")
        };
        assert_eq!(found(&exact, text), [(5, 9)]);
        assert_eq!(found(&query("äpfel"), text).len(), 2);
    }

    /// `gain` is not in `gain2`, and `->` beside a name is still whole.
    #[test]
    fn a_whole_word_is_bounded_by_what_is_not_a_word() {
        let word = |t: &str| Query {
            word: true,
            ..query(t)
        };
        assert_eq!(
            found(&word("gain"), "gain gain2 again gain"),
            [(0, 4), (17, 21)]
        );
        assert_eq!(found(&word("->"), "a->b"), [(1, 3)]);
        assert_eq!(found(&word("中文"), "中文 中文字"), [(0, 6)]);
    }

    #[test]
    fn a_regex_reads_lines_and_says_what_is_wrong_with_it() {
        let re = |t: &str| Query {
            regex: true,
            ..query(t)
        };
        assert_eq!(
            found(&re(r"^fn \w+"), "fn a() {}\n  fn b() {}\nfn c"),
            [(0, 4), (22, 26)]
        );
        // An empty match at every line start, for putting something there.
        assert_eq!(found(&re("^"), "a\nb\n"), [(0, 0), (2, 2), (4, 4)]);
        let broken = re("(unclosed").search("x", None).unwrap_err();
        assert!(broken.contains("unclosed"), "{broken}");
    }

    #[test]
    fn a_scope_holds_the_search_and_still_sees_the_word_before_it() {
        let text = "gain = gain2 + gain;";
        let word = Query {
            word: true,
            ..query("gain")
        };
        // The scope starts inside `gain2`: what is before it still counts.
        assert_eq!(
            word.search(text, Some((8, 20))).unwrap().matches,
            [(15, 19)]
        );
        assert_eq!(
            query("gain").search(text, Some((0, 6))).unwrap().matches,
            [(0, 4)]
        );
    }

    #[test]
    fn a_replacement_reads_groups_escapes_and_names() {
        let re = Query {
            regex: true,
            ..query(r"(\w+)\.(?<field>\w+)")
        };
        let text = "pid.kp";
        let range = (0, text.len());
        assert_eq!(re.replacement(text, range, "$2_$1"), "kp_pid");
        assert_eq!(re.replacement(text, range, "$<field>"), "kp");
        assert_eq!(re.replacement(text, range, "[$&] $0"), "[pid.kp] pid.kp");
        assert_eq!(re.replacement(text, range, r"$1\n\t$$\\"), "pid\n\t$\\");
        // `$1a` is group one and an `a`; a group that is not there is text.
        assert_eq!(re.replacement(text, range, "$1a $9"), "pida $9");
        // A literal query writes its replacement exactly as typed.
        assert_eq!(query("pid").replacement(text, (0, 3), "$1\\n"), "$1\\n");
    }

    #[test]
    fn replace_all_says_where_each_replacement_landed() {
        let (text, landed) = query("ab")
            .replace_all("ab-ab-ab", Some((2, 8)), "xyz")
            .unwrap();
        assert_eq!(text, "ab-xyz-xyz");
        assert_eq!(landed, [(3, 6), (7, 10)]);
        assert!(query("zz").replace_all("ab", None, "x").is_none());
        let lines = Query {
            regex: true,
            ..query("^")
        };
        assert_eq!(
            lines.replace_all("a\nb", None, "// ").unwrap().0,
            "// a\n// b"
        );
    }

    #[test]
    fn the_caret_rides_past_what_replace_all_wrote() {
        let matches = [(2, 4), (10, 12)];
        // `ab` → `xyz` twice: one byte more each time.
        let landed = [(2, 5), (11, 14)];
        assert_eq!(carried(1, &matches, &landed), 1);
        assert_eq!(carried(6, &matches, &landed), 7);
        assert_eq!(carried(3, &matches, &landed), 2);
        assert_eq!(carried(20, &matches, &landed), 22);
    }

    #[test]
    fn a_line_draws_what_touches_it_and_not_what_only_ends_there() {
        let text = "ab\nab\n";
        let lines = Lines::of(text);
        let marks = Marks {
            // The second runs over the first line's break.
            matches: vec![(0, 2), (1, 3), (3, 5)],
            current: None,
            scope: None,
        };
        assert_eq!(marks.on(lines.start(0), lines.end(0)), 0..2);
        assert_eq!(marks.on(lines.start(1), lines.end(1)), 2..3);
        assert_eq!(marks.lines(&lines), [0, 1]);
    }

    #[test]
    fn next_and_previous_wrap_at_the_ends() {
        let matches = [(2, 4), (10, 12), (20, 22)];
        assert_eq!(after(&matches, 0), Some(0));
        assert_eq!(after(&matches, 4), Some(1));
        assert_eq!(after(&matches, 21), Some(0));
        assert_eq!(before(&matches, 10), Some(0));
        assert_eq!(before(&matches, 2), Some(2));
        assert_eq!(current(&matches, (10, 12)), Some(1));
        assert_eq!(current(&matches, (10, 11)), None);
        assert_eq!(after(&[], 3), None);
    }

    #[test]
    fn the_field_is_seeded_from_a_selection_on_one_line_or_the_word() {
        let text = "let gain = 2;\nnext";
        assert_eq!(seed(text, (6, 6)).as_deref(), Some("gain"));
        assert_eq!(seed(text, (4, 10)).as_deref(), Some("gain ="));
        assert_eq!(seed(text, (4, 16)), None);
    }

    #[test]
    fn a_scope_follows_the_edits_around_it() {
        // Typed before the scope: it moves; after: it stays; inside: it grows.
        assert_eq!(follow((10, 20), 2, 5, 3), (13, 23));
        assert_eq!(follow((10, 20), 25, 27, 2), (10, 20));
        assert_eq!(follow((10, 20), 12, 16, 4), (10, 24));
        // Deleted across its start: it starts where the deletion did.
        assert_eq!(follow((10, 20), 5, 5, -10), (5, 10));
    }
}
