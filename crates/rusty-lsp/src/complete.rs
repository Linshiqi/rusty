//! Completion as an editor shows it: when to ask the server, which of
//! its items to show for the word typed and in what order, and what a
//! snippet it offers inserts.
//!
//! **When to ask is VS Code's rule** ([`ask_for`]): the first letter of a
//! word asks, `.` and `::` ask about what follows them, and the word is
//! asked about again as it grows while the server says its answer is
//! incomplete — which rust-analyzer always does, because its imports are
//! searched by the word typed. Asking once per word meant an ask that came
//! back empty while the index warmed up was no completion for that word.
//!
//! **What is shown is ranked here** ([`ranked`]): names that start with the
//! word first, in the server's order, then names the word only fuzzily
//! matches — `itr` finds `iter`, `hm` finds `HashMap`.
//!
//! Here, beside [`crate::squiggle`], because every editor that shows a
//! server's completions needs the same answers, and two copies of a
//! ranking are two rankings.

use crate::CompletionItem;

/// The word typed since the popup opened — what the list narrows against.
pub fn typed_word(text: &str, line: u32, word_start: u32) -> String {
    text.split('\n')
        .nth(line as usize)
        .map(|line_text| {
            line_text
                .chars()
                .skip(word_start as usize)
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect()
        })
        .unwrap_or_default()
}

// ─── when to ask ─────────────────────────────────────────────────────────────

/// What a change to the text asks of the completion popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// Leave it as it is.
    Keep,
    /// Close it, and drop any answer on its way.
    Close,
    /// Ask the server about the word that starts at `word_start`: `now` for
    /// the first ask about a word, after a pause in the typing for a word
    /// that is already showing.
    Complete { word_start: u32, now: bool },
}

/// The popup — or the ask still waiting for its first answer — as the
/// decision needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Showing {
    pub line: u32,
    pub word_start: u32,
    /// The server said its answer was incomplete, or has not answered yet.
    pub incomplete: bool,
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// What typing — or deleting, when `typing` is false — asks of the popup,
/// judged by the text before the caret on its `line`.
///
/// The first letter of a word asks, unless the word is a number; `.` and
/// `::` ask about what follows them. A word already showing is asked about
/// again as it changes while its answer is incomplete, and only narrowed
/// when it is complete. Deleting never opens a popup, but widens one that
/// is open. The popup closes when the caret leaves its word: onto another
/// line, back past where the word starts, or past a character that ends it.
pub fn ask_for(before: &[char], line: u32, showing: Option<Showing>, typing: bool) -> Ask {
    let col = before.len() as u32;
    let word = before.iter().rev().take_while(|ch| is_word(**ch)).count() as u32;
    let start = col - word;
    if let Some(showing) = showing {
        if showing.line != line || col < showing.word_start {
            return Ask::Close;
        }
        if start == showing.word_start {
            return if showing.incomplete {
                Ask::Complete {
                    word_start: start,
                    now: false,
                }
            } else {
                Ask::Keep
            };
        }
    }
    let last = before.last().copied();
    let prev = before.len().checked_sub(2).map(|at| before[at]);
    let opens = match last {
        Some('.') => typing,
        Some(':') => typing && prev == Some(':'),
        Some(ch) if is_word(ch) => typing && !before[start as usize].is_ascii_digit(),
        _ => false,
    };
    if opens {
        let word_start = if word == 0 { col } else { start };
        Ask::Complete {
            word_start,
            now: true,
        }
    } else if showing.is_some() {
        Ask::Close
    } else {
        Ask::Keep
    }
}

// ─── what to show ────────────────────────────────────────────────────────────

/// Where a word starts inside a name: the first character, one after `_`
/// or `:`, or an upper-case letter after a lower-case one — `HashMap`'s `M`.
fn starts_word(chars: &[char], at: usize) -> bool {
    at == 0
        || matches!(chars[at - 1], '_' | ':' | '.' | ' ' | '(' | '<' | '!')
        || (chars[at].is_uppercase() && chars[at - 1].is_lowercase())
}

/// How well `typed` — lower-case — matches `name`, or `None` when it does
/// not. Every typed character must appear in order, the first where a word
/// starts: `itr` finds `iter`, `hm` finds `HashMap`, `iter` finds
/// `into_iter`, and `ln` does not find `println`. A character that follows
/// the one before it scores three, one that starts a word two, one found
/// across a gap nothing, and the best alignment counts.
pub fn fuzzy_score(typed: &[char], name: &str) -> Option<u32> {
    let Some(first) = typed.first() else {
        return Some(0);
    };
    let chars: Vec<char> = name.chars().collect();
    let lower: Vec<char> = chars
        .iter()
        .map(|ch| ch.to_lowercase().next().unwrap_or(*ch))
        .collect();
    (0..lower.len())
        .filter(|&at| lower[at] == *first && starts_word(&chars, at))
        .filter_map(|at| follow(&typed[1..], &chars, &lower, at).map(|rest| 2 + rest))
        .max()
}

/// The rest of a fuzzy match, after its first character matched at `at`.
fn follow(rest: &[char], chars: &[char], lower: &[char], at: usize) -> Option<u32> {
    let mut score = 0;
    let mut last = at;
    for want in rest {
        last = if lower.get(last + 1) == Some(want) {
            score += 3;
            last + 1
        } else if let Some(next) =
            (last + 1..lower.len()).find(|&next| lower[next] == *want && starts_word(chars, next))
        {
            score += 2;
            next
        } else {
            (last + 1..lower.len()).find(|&next| lower[next] == *want)?
        };
    }
    Some(score)
}

/// The indices of the items `typed` keeps, best first. Names that start
/// with it come first, in the server's order — rust-analyzer's ranking is
/// good, and a prefix is what most typing is — an exact-case start ahead of
/// another, so `Str` puts `String` before `str`; then the names it only
/// fuzzily matches, the best match first. An item's `filter` stands in for
/// its label when the server gave one.
pub fn ranked(items: &[CompletionItem], typed: &str) -> Vec<usize> {
    if typed.is_empty() {
        return (0..items.len()).collect();
    }
    let lower = typed.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut kept: Vec<((u8, u8, u32), usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(order, item)| {
            let name = item.filter.as_deref().unwrap_or(&item.label);
            if name.to_lowercase().starts_with(&lower) {
                Some(((0, u8::from(!name.starts_with(typed)), 0), order))
            } else {
                fuzzy_score(&chars, name).map(|score| ((1, 0, u32::MAX - score), order))
            }
        })
        .collect();
    kept.sort_unstable();
    kept.into_iter().map(|(_, order)| order).collect()
}

// ─── snippets ────────────────────────────────────────────────────────────────

/// A snippet as the text it inserts, and what to select afterwards: the
/// first placeholder — `${1:x}` — so typing replaces it, else the caret at
/// `$0`, else the end. Offsets are chars into `text`.
#[derive(Debug, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    pub select: (usize, usize),
}

/// Expand an LSP snippet: `$1`, `${1}`, `${1:default}` (nested), `${1|a,b|}`,
/// `$NAME` and `${NAME:default}`, with `\` escaping `$`, `}` and itself.
///
/// Only the first tabstop is honoured — the editor does not walk the rest
/// with Tab — which is why functions are asked for as `name($0)` rather than
/// with their arguments filled in (the client's `callable.snippets`).
/// Continuation lines take `indent`, the indentation of the line the snippet
/// lands on, and a tab is four spaces, as everywhere else in this editor.
pub fn expand_snippet(snippet: &str, indent: &str) -> Expanded {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = Out {
        text: String::new(),
        len: 0,
        indent,
        stops: Vec::new(),
    };
    let mut at = 0;
    parse(&chars, &mut at, &mut out, false);
    let first = out
        .stops
        .iter()
        .filter(|stop| stop.0 > 0)
        .min_by_key(|stop| (stop.0, stop.1))
        .copied();
    let last = out.stops.iter().find(|stop| stop.0 == 0).copied();
    let select = first
        .or(last)
        .map_or((out.len, out.len), |(_, start, end)| (start, end));
    Expanded {
        text: out.text,
        select,
    }
}

/// The text being built, its length in chars, and every tabstop met:
/// its number, where it starts and where it ends.
struct Out<'a> {
    text: String,
    len: usize,
    indent: &'a str,
    stops: Vec<(u32, usize, usize)>,
}

impl Out<'_> {
    fn push(&mut self, ch: char) {
        match ch {
            '\n' => {
                self.text.push('\n');
                self.len += 1;
                for ch in self.indent.chars() {
                    self.text.push(ch);
                    self.len += 1;
                }
            }
            '\t' => {
                self.text.push_str("    ");
                self.len += 4;
            }
            ch => {
                self.text.push(ch);
                self.len += 1;
            }
        }
    }
}

/// Read to the end — or, inside a placeholder, to the `}` that closes it.
fn parse(chars: &[char], at: &mut usize, out: &mut Out, nested: bool) {
    while *at < chars.len() {
        match chars[*at] {
            '\\' if chars
                .get(*at + 1)
                .is_some_and(|next| matches!(next, '$' | '}' | '\\')) =>
            {
                out.push(chars[*at + 1]);
                *at += 2;
            }
            '}' if nested => {
                *at += 1;
                return;
            }
            '$' => {
                *at += 1;
                dollar(chars, at, out);
            }
            ch => {
                out.push(ch);
                *at += 1;
            }
        }
    }
}

/// After a `$`: a tabstop, a placeholder, a choice, a variable — or a dollar
/// that starts none of them, which is a dollar.
fn dollar(chars: &[char], at: &mut usize, out: &mut Out) {
    match chars.get(*at) {
        Some(ch) if ch.is_ascii_digit() => {
            let number = digits(chars, at).unwrap_or(0);
            out.stops.push((number, out.len, out.len));
        }
        Some('{') => {
            *at += 1;
            if let Some(number) = digits(chars, at) {
                let start = out.len;
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        parse(chars, at, out, true);
                    }
                    Some('|') => {
                        *at += 1;
                        choice(chars, at, out);
                    }
                    Some('}') => *at += 1,
                    _ => {}
                }
                out.stops.push((number, start, out.len));
            } else if variable(chars, at) {
                // rusty knows no variables; one with a default is its default.
                match chars.get(*at) {
                    Some(':') => {
                        *at += 1;
                        parse(chars, at, out, true);
                    }
                    Some('}') => *at += 1,
                    _ => {}
                }
            } else {
                out.push('$');
                out.push('{');
            }
        }
        Some(ch) if ch.is_ascii_alphabetic() || *ch == '_' => {
            variable(chars, at);
        }
        _ => out.push('$'),
    }
}

/// `${1|one,two|}`: the first choice, as text.
fn choice(chars: &[char], at: &mut usize, out: &mut Out) {
    let mut first = true;
    while *at < chars.len() {
        match chars[*at] {
            '\\' if *at + 1 < chars.len() => {
                if first {
                    out.push(chars[*at + 1]);
                }
                *at += 2;
            }
            ',' => {
                first = false;
                *at += 1;
            }
            '|' if chars.get(*at + 1) == Some(&'}') => {
                *at += 2;
                return;
            }
            ch => {
                if first {
                    out.push(ch);
                }
                *at += 1;
            }
        }
    }
}

fn digits(chars: &[char], at: &mut usize) -> Option<u32> {
    let from = *at;
    while *at < chars.len() && chars[*at].is_ascii_digit() {
        *at += 1;
    }
    if *at == from {
        return None;
    }
    chars[from..*at].iter().collect::<String>().parse().ok()
}

fn variable(chars: &[char], at: &mut usize) -> bool {
    let from = *at;
    while *at < chars.len() && (chars[*at].is_ascii_alphanumeric() || chars[*at] == '_') {
        *at += 1;
    }
    *at > from
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    fn item(label: &str) -> CompletionItem {
        CompletionItem {
            label: label.into(),
            kind: None,
            detail: None,
            insert: label.into(),
            edit: None,
            index: 0,
            label_detail: None,
            filter: None,
            snippet: false,
            description: None,
        }
    }

    fn names(items: &[CompletionItem], typed: &str) -> Vec<String> {
        ranked(items, typed)
            .into_iter()
            .map(|at| items[at].label.clone())
            .collect()
    }

    #[test]
    fn the_first_letter_asks_and_the_word_keeps_asking_while_incomplete() {
        let now = |word_start| Ask::Complete {
            word_start,
            now: true,
        };
        let later = |word_start| Ask::Complete {
            word_start,
            now: false,
        };
        // Nothing showing: a letter opens it, `.` and `::` open it after them.
        assert_eq!(ask_for(&chars("    l"), 3, None, true), now(4));
        assert_eq!(ask_for(&chars("v."), 3, None, true), now(2));
        assert_eq!(ask_for(&chars("std::"), 3, None, true), now(5));
        assert_eq!(ask_for(&chars("std:"), 3, None, true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("x = 1"), 3, None, true),
            Ask::Keep,
            "a number"
        );
        assert_eq!(ask_for(&chars("let "), 3, None, true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("pri"), 3, None, false),
            Ask::Keep,
            "deleting never opens one"
        );

        // Showing: the same word asks again after a pause while incomplete,
        // and is only narrowed once complete.
        let open = Showing {
            line: 3,
            word_start: 4,
            incomplete: true,
        };
        assert_eq!(ask_for(&chars("    pr"), 3, Some(open), true), later(4));
        let done = Showing {
            incomplete: false,
            ..open
        };
        assert_eq!(ask_for(&chars("    pr"), 3, Some(done), true), Ask::Keep);
        assert_eq!(
            ask_for(&chars("    p"), 3, Some(open), false),
            later(4),
            "deleting inside the word widens it"
        );

        // The caret leaves the word: back past its start, onto another line,
        // or past a character that ends it.
        assert_eq!(ask_for(&chars("   "), 3, Some(open), false), Ask::Close);
        assert_eq!(ask_for(&chars("    pr"), 4, Some(open), true), Ask::Close);
        assert_eq!(ask_for(&chars("    pr "), 3, Some(open), true), Ask::Close);
        assert_eq!(ask_for(&chars("    pr("), 3, Some(open), true), Ask::Close);
        // A `.` after the word asks about what follows it instead.
        assert_eq!(ask_for(&chars("    pr."), 3, Some(open), true), now(7));
        // Right after the `.`, the popup's word is empty and still its own.
        let dot = Showing {
            line: 0,
            word_start: 2,
            incomplete: false,
        };
        assert_eq!(ask_for(&chars("v."), 0, Some(dot), false), Ask::Keep);
    }

    #[test]
    fn a_prefix_keeps_the_servers_order_and_fuzzy_matches_follow_it() {
        let items: Vec<CompletionItem> = [
            "print",
            "println",
            "eprintln",
            "iter",
            "into_iter",
            "HashMap",
            "hash",
            "String",
            "str",
        ]
        .into_iter()
        .map(item)
        .collect();
        assert_eq!(
            names(&items, "pr"),
            ["print", "println"],
            "`p` inside a word is not where a match may start"
        );
        assert_eq!(names(&items, "iter"), ["iter", "into_iter"]);
        assert_eq!(names(&items, "itr"), ["iter", "into_iter"]);
        assert_eq!(names(&items, "hm"), ["HashMap"]);
        assert_eq!(
            names(&items, "Str"),
            ["String", "str"],
            "an exact-case start first"
        );
        assert_eq!(names(&items, "str"), ["str", "String"]);
        assert!(names(&items, "ln").is_empty());
        assert_eq!(
            names(&items, "").len(),
            items.len(),
            "no word keeps everything"
        );
    }

    #[test]
    fn an_item_is_matched_by_its_filter_text_when_it_has_one() {
        let mut postfix = item("if expr {}");
        postfix.filter = Some("if".into());
        let items = vec![item("iter"), postfix];
        assert_eq!(names(&items, "if"), ["if expr {}"]);
    }

    #[test]
    fn a_snippet_expands_to_its_text_with_the_first_placeholder_selected() {
        let e = expand_snippet("push($0)", "");
        assert_eq!((e.text.as_str(), e.select), ("push()", (5, 5)));
        let e = expand_snippet("if ${1:cond} {\n\t$0\n}", "    ");
        assert_eq!(e.text, "if cond {\n        \n    }");
        assert_eq!(e.select, (3, 7), "selected, so typing replaces it");
        let e = expand_snippet("${1:foo(${2:x})}$0", "");
        assert_eq!((e.text.as_str(), e.select), ("foo(x)", (0, 6)));
        let e = expand_snippet("${2:b}${1:a}", "");
        assert_eq!(
            (e.text.as_str(), e.select),
            ("ba", (1, 2)),
            "the lowest number first, wherever it is"
        );
        let e = expand_snippet("${1|one,two|}", "");
        assert_eq!((e.text.as_str(), e.select), ("one", (0, 3)));
        let e = expand_snippet(r"cost \$5 \} done", "");
        assert_eq!(
            (e.text.as_str(), e.select),
            ("cost $5 } done", (14, 14)),
            "escapes are text, and no stop is the end"
        );
        let e = expand_snippet("${TM_SELECTED_TEXT:it}$0", "");
        assert_eq!((e.text.as_str(), e.select), ("it", (2, 2)));
        assert_eq!(expand_snippet("a $ b", "").text, "a $ b");
    }
}
