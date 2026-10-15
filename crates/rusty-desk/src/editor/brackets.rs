//! The bracket beside the caret and its partner, outlined — VS Code's
//! `editor.matchBrackets`.
//!
//! Found on the painted lines, whose tokens already say what a string and a
//! comment are, so a `{` in `"{}"` or in `'{'` pairs with nothing and a
//! block comment's brackets are not counted however many lines it runs
//! over. A line typed on since its painting is read on its own by the lexer
//! the pair rules count with (`pairs::code_brackets`) until its repaint
//! lands, which is the one moment a string opened on the line above can
//! fool it.
//!
//! Pure, over a function answering a line's brackets, so the rules are
//! tests: the bracket after the caret first and then the one before, as VS
//! Code takes them, whichever has a partner; a partner found by nesting
//! over every kind, so `{ ( }` pairs its braces and leaves the parenthesis
//! with nothing rather than sending it to some later `)`; and a partner
//! looked for no further than [`REACH`] lines either way, so a caret beside
//! a brace in a long file does not walk the whole of it on every key.

use rusty_edit::{Line, Token};

/// How many lines either way a partner is looked for.
pub const REACH: usize = 3000;

/// Where a bracket is: its line, and its column in characters.
pub type At = (usize, usize);

/// The brackets on a line that are code, by column, in order: from its
/// painting when `painted` is the painting of `text`, and from the lexer
/// otherwise. A string, a character, a comment, and the placeholders the
/// grammar marks inside a format string, hold none.
pub fn on_line(text: &str, painted: Option<&Line>) -> Vec<(usize, char)> {
    let Some(painted) = painted else {
        let mut cols = text.char_indices().enumerate();
        return super::pairs::code_brackets(text, text.len())
            .into_iter()
            .filter_map(|(byte, b)| {
                let (col, _) = cols.find(|(_, (at, _))| *at == byte)?;
                Some((col, b as char))
            })
            .collect();
    };
    let mut out = Vec::new();
    let mut col = 0;
    for span in &painted.spans {
        let code = !matches!(span.token, Token::Str | Token::Comment | Token::Number);
        for ch in span.text.chars() {
            if code && is_bracket(ch) {
                out.push((col, ch));
            }
            col += 1;
        }
    }
    out
}

fn is_bracket(ch: char) -> bool {
    matches!(ch, '(' | ')' | '[' | ']' | '{' | '}')
}

fn opens(ch: char) -> bool {
    matches!(ch, '(' | '[' | '{')
}

/// Whether `open` is closed by `close`.
fn closes(open: char, close: char) -> bool {
    matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
}

/// The bracket beside the caret at `caret` and its partner, in that order,
/// in a text of `count` lines whose brackets `brackets` answers line by
/// line. `None` beside no bracket, and beside one with no partner in reach.
pub fn matching(
    caret: At,
    count: usize,
    mut brackets: impl FnMut(usize) -> Vec<(usize, char)>,
) -> Option<[At; 2]> {
    let (line, col) = caret;
    let here = brackets(line);
    let after = here.iter().find(|(c, _)| *c == col).copied();
    let before = col
        .checked_sub(1)
        .and_then(|left| here.iter().find(|(c, _)| *c == left).copied());
    for (at, ch) in after.into_iter().chain(before) {
        if let Some(partner) = partner((line, at), ch, count, &here, &mut brackets) {
            return Some([(line, at), partner]);
        }
    }
    None
}

/// Where the bracket `ch` at `from` is closed or opened. Brackets between
/// nest whatever their kind, and one that closes something the walk began
/// outside of ends it with nothing.
fn partner(
    from: At,
    ch: char,
    count: usize,
    here: &[(usize, char)],
    brackets: &mut impl FnMut(usize) -> Vec<(usize, char)>,
) -> Option<At> {
    let (line, col) = from;
    let forward = opens(ch);
    // The brackets the walk has passed and not seen answered.
    let mut open: Vec<char> = Vec::new();
    let mut step = |at: At, b: char| -> Option<Option<At>> {
        if opens(b) == forward {
            open.push(b);
            return None;
        }
        let (outer, inner) = if forward { (ch, b) } else { (b, ch) };
        let pairs = |o: char, c: char| if forward { closes(o, c) } else { closes(c, o) };
        if let Some(depth) = open.iter().rposition(|&o| pairs(o, b)) {
            // Whatever it passed over is left unanswered, as an editor
            // forgives a bracket typed and not yet closed.
            open.truncate(depth);
            return None;
        }
        Some(closes(outer, inner).then_some(at))
    };
    if forward {
        let last = count.min(line + REACH + 1);
        for n in line..last {
            let list = if n == line {
                here.to_vec()
            } else {
                brackets(n)
            };
            for (c, b) in list {
                if n == line && c <= col {
                    continue;
                }
                if let Some(found) = step((n, c), b) {
                    return found;
                }
            }
        }
    } else {
        for n in (line.saturating_sub(REACH)..=line).rev() {
            let list = if n == line {
                here.to_vec()
            } else {
                brackets(n)
            };
            for (c, b) in list.into_iter().rev() {
                if n == line && c >= col {
                    continue;
                }
                if let Some(found) = step((n, c), b) {
                    return found;
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pair beside the caret `‸` in `text`, written as Rust and painted
    /// as the editor paints it.
    fn pair(text: &str) -> Option<[At; 2]> {
        let caret = text
            .split('\n')
            .enumerate()
            .find_map(|(line, l)| l.chars().position(|c| c == '‸').map(|col| (line, col)))
            .expect("a caret in the text");
        let text = text.replace('‸', "");
        let lines: Vec<&str> = text.split('\n').collect();
        let painted = rusty_edit::Files::new().highlight_snippet("rust", &text);
        matching(caret, lines.len(), |n| {
            let line = lines[n];
            let painting = painted
                .get(n)
                .filter(|p| super::super::rows::painted_is(p, line));
            on_line(line, painting)
        })
    }

    #[test]
    fn nested_pairs_find_their_own_partner() {
        assert_eq!(pair("f‸(a(b), c)"), Some([(0, 1), (0, 9)]));
        assert_eq!(pair("f(a‸(b), c)"), Some([(0, 3), (0, 5)]));
        assert_eq!(pair("let v = [‸[1, 2], [3]];"), Some([(0, 9), (0, 14)]));
    }

    /// The bracket after the caret first, then the one before — VS Code's
    /// order — and from either end of the pair.
    #[test]
    fn the_caret_takes_the_bracket_after_it_then_the_one_before() {
        assert_eq!(pair("‸(x)"), Some([(0, 0), (0, 2)]));
        assert_eq!(pair("(x)‸"), Some([(0, 2), (0, 0)]));
        assert_eq!(pair("(a)‸(b)"), Some([(0, 3), (0, 5)]));
        assert_eq!(pair("(a‸)"), Some([(0, 2), (0, 0)]));
        assert_eq!(pair("a‸ (b)"), None, "beside no bracket");
    }

    /// A bracket in a string, a character literal, a format string or a
    /// comment pairs with nothing and is passed over; a lifetime is not a
    /// character literal.
    #[test]
    fn a_bracket_that_is_not_code_is_passed_over() {
        assert_eq!(pair("f‸(\"(\", x)"), Some([(0, 1), (0, 8)]));
        assert_eq!(pair("println!‸(\"{}\", a)"), Some([(0, 8), (0, 16)]));
        assert_eq!(pair("println!(\"‸{}\", a)"), None);
        assert_eq!(pair("let c = '‸(';"), None);
        assert_eq!(
            pair("fn f<'a>(s: &'a str) -> char ‸{\n    '}'\n}"),
            Some([(0, 29), (2, 0)])
        );
        assert_eq!(pair("fn f<'a>‸(s: &'a str)"), Some([(0, 8), (0, 19)]));
        assert_eq!(pair("f‸(x) // )"), Some([(0, 1), (0, 3)]));
        assert_eq!(pair("f‸(\n    /* ) */ x,\n)"), Some([(0, 1), (2, 0)]));
        assert_eq!(pair("// ‸(x)"), None);
    }

    /// No partner is no outline — not the next bracket of the kind that
    /// happens to come along.
    #[test]
    fn an_unmatched_bracket_gets_no_partner() {
        assert_eq!(pair("f‸(x"), None);
        assert_eq!(pair("{ ‸( }"), None, "its block closes first");
        assert_eq!(
            pair("‸{ ( }"),
            Some([(0, 0), (0, 4)]),
            "the block forgives it"
        );
        assert_eq!(pair("x)‸"), None);
    }

    /// Columns are characters: a `中` before a bracket is one column, not
    /// three bytes.
    #[test]
    fn columns_count_characters_past_cjk() {
        assert_eq!(pair("let 名字 = 计算‸(1);"), Some([(0, 11), (0, 13)]));
        assert_eq!(
            pair("// 中文注释 (\nlet a = 名‸(\"中\", [1]);"),
            Some([(1, 9), (1, 18)])
        );
    }

    #[test]
    fn a_pair_split_across_lines_is_found_from_either_end() {
        let text = "fn a() {\n    let x = [1,\n        2];\n}";
        let at = |line: usize, col: usize| {
            let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
            let byte = lines[line]
                .char_indices()
                .nth(col)
                .map_or(lines[line].len(), |(i, _)| i);
            lines[line].insert(byte, '‸');
            pair(&lines.join("\n"))
        };
        assert_eq!(at(0, 7), Some([(0, 7), (3, 0)]));
        assert_eq!(at(3, 1), Some([(3, 0), (0, 7)]));
        assert_eq!(at(1, 12), Some([(1, 12), (2, 9)]));
    }

    /// A line typed on and not yet painted again is read on its own: its
    /// strings and comments are still not code.
    #[test]
    fn an_unpainted_line_is_read_by_the_lexer() {
        assert_eq!(on_line("f(\"(\") // )", None), [(1, '('), (5, ')')]);
        assert_eq!(on_line("名(", None), [(1, '(')]);
    }

    /// A partner is looked for no further than the reach.
    #[test]
    fn a_partner_out_of_reach_is_not_looked_for() {
        let lines = |far: usize| {
            move |n: usize| match n {
                0 => vec![(0, '{')],
                n if n == far => vec![(0, '}')],
                _ => Vec::new(),
            }
        };
        assert_eq!(
            matching((0, 0), REACH + 1, lines(REACH)),
            Some([(0, 0), (REACH, 0)])
        );
        assert_eq!(matching((0, 0), REACH + 2, lines(REACH + 1)), None);
    }
}
