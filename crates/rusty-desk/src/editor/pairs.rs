//! Bracket pairs, and the indentation that follows them.
//!
//! A textarea types what it is given and nothing more. Every editor since the
//! nineties has done four things on top: an opener brings its closer and puts
//! the caret between them; a closer typed against the one already there steps
//! over it instead of doubling it; Enter between the two puts the caret on an
//! indented line of its own with the closer below; and a closer typed on a
//! blank line takes its opener's indentation. Their absence is the first thing
//! anyone used to a real editor notices — `{}` was the complaint, in as many
//! characters.
//!
//! Pure functions over the document text and byte offsets, so the rules are
//! tests rather than something discovered by typing. The component applies
//! the [`Edit`] each returns through the same pipeline as every other write,
//! which is what keeps undo, the echo and the folds honest.

/// One replacement in the document, with where the caret goes afterwards.
///
/// Offsets are bytes into the text the function was given (`range`) and into
/// the text after the replacement (`caret`, `select`). `select` is the range
/// left highlighted when the edit wraps a selection in brackets, so typing
/// `(` around `foo` leaves `foo` selected as every editor does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Edit {
    pub range: (usize, usize),
    pub text: String,
    pub caret: usize,
    pub select: Option<(usize, usize)>,
}

impl Edit {
    fn insert(at: usize, text: &str) -> Self {
        Edit {
            range: (at, at),
            text: text.to_string(),
            caret: at + text.len(),
            select: None,
        }
    }
}

/// The closer an opener brings with it. Single quotes are absent on purpose:
/// in Rust `'a` is a lifetime far more often than a character literal, and an
/// auto-closed `''` after every lifetime is the kind of help that gets a
/// feature switched off.
pub(super) fn closer(open: char) -> Option<char> {
    match open {
        '{' => Some('}'),
        '(' => Some(')'),
        '[' => Some(']'),
        '"' => Some('"'),
        _ => None,
    }
}

fn opener(close: char) -> Option<char> {
    match close {
        '}' => Some('{'),
        ')' => Some('('),
        ']' => Some('['),
        _ => None,
    }
}

/// What typing `key` should do with the caret at `from` (and a selection to
/// `to` when they differ), or `None` to let the browser type it.
pub(super) fn on_type(text: &str, from: usize, to: usize, key: char) -> Option<Edit> {
    let (from, to) = (from.min(to).min(text.len()), from.max(to).min(text.len()));
    if !text.is_char_boundary(from) || !text.is_char_boundary(to) {
        return None;
    }

    // A selection typed over with an opener is wrapped, not replaced — the
    // one case where the browser's own behaviour loses text the user meant to
    // keep.
    if from < to {
        let close = closer(key)?;
        let inner = &text[from..to];
        let mut wrapped = String::new();
        wrapped.push(key);
        wrapped.push_str(inner);
        wrapped.push(close);
        let start = from + key.len_utf8();
        return Some(Edit {
            range: (from, to),
            text: wrapped,
            caret: start + inner.len(),
            select: Some((start, start + inner.len())),
        });
    }

    let caret = from;
    let next = text[caret..].chars().next();
    let previous = text[..caret].chars().next_back();

    // A closer typed where that closer already stands: step over it. Typing
    // `)` at `foo(|)` is finishing the call, not starting a second bracket.
    //
    // Only when what stands there closes something opened earlier on this
    // line, which is what an auto-closed pair always is — VS Code steps
    // over only the closers it inserted, and this is that rule without a
    // record of them. A `}` at the start of a line closes a block opened
    // above, and one typed in front of it is a block being closed that was
    // missing its brace; stepped over, the typed brace vanished and the file
    // ended one short, rustfmt's `found <eof>`.
    if key == '"' || opener(key).is_some() {
        let ours = match opener(key) {
            Some(open) => open_on_line(text, caret, open as u8, key as u8),
            None => true,
        };
        if next == Some(key) && ours {
            return Some(Edit {
                range: (caret, caret),
                text: String::new(),
                caret: caret + key.len_utf8(),
                select: None,
            });
        }
        // A closer on a line of nothing but indentation takes its opener's
        // indentation, which is how `}` lands under the `fn` it closes after
        // Enter deepened the line above it.
        if let Some(open) = opener(key) {
            let line_start = text[..caret].rfind('\n').map(|at| at + 1).unwrap_or(0);
            let blank = text[line_start..caret]
                .chars()
                .all(|c| c == ' ' || c == '\t');
            if blank
                && let Some(indent) = opener_indent(text, caret, open, key)
                && indent != text[line_start..caret]
            {
                let replacement = format!("{indent}{key}");
                return Some(Edit {
                    range: (line_start, caret),
                    caret: line_start + replacement.len(),
                    text: replacement,
                    select: None,
                });
            }
        }
    }

    let close = closer(key)?;
    if key == '"' {
        // Inside a string literal — an odd number of quotes so far on the
        // line — the quote being typed is the one that closes it.
        let line_start = text[..caret].rfind('\n').map(|at| at + 1).unwrap_or(0);
        if unescaped_quotes(&text[line_start..caret]) % 2 == 1 {
            return None;
        }
        // Against a word the quote is not opening a string: a `"` typed after
        // an identifier is nearly always a typo being corrected. `r` and `b`
        // are the prefixes of raw and byte strings and the exception.
        if let Some(p) = previous
            && (p == '"' || p == '\\' || (p.is_alphanumeric() && p != 'r' && p != 'b'))
        {
            return None;
        }
    }
    // Only in front of nothing, whitespace or punctuation that ends a term:
    // `(` typed in front of `foo` is being wrapped around it by hand, and a
    // `)` inserted in the middle of that would have to be deleted again.
    let before_term = next.is_none_or(|c| " \t\n;:.,=}])>".contains(c));
    if !before_term {
        return None;
    }
    // Not when the file already holds a closer with no opener: that closer
    // is the one this opener is for — a `{` retyped after deleting one, or
    // added to a line whose block and `}` are already below. Paired, it
    // left a second `}` behind, the wrong brace reported. Counted in code
    // only, so a `}` in a string, a character or a comment is not one.
    if key != '"' && balance(text, key as u8, close as u8) < 0 {
        return None;
    }
    let mut pair = String::new();
    pair.push(key);
    pair.push(close);
    Some(Edit {
        range: (caret, caret),
        text: pair,
        caret: caret + key.len_utf8(),
        select: None,
    })
}

/// Quotes on a line that are not escaped by a backslash.
fn unescaped_quotes(line: &str) -> usize {
    let mut count = 0;
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => count += 1,
            _ => {}
        }
    }
    count
}

/// The indentation of the line holding the unmatched `open` before `caret` —
/// the line a closer typed at `caret` should line up with. Brackets in a
/// string or a comment are not counted: a `"{"` above would otherwise send
/// the closer under the wrong block.
fn opener_indent(text: &str, caret: usize, open: char, close: char) -> Option<String> {
    let mut depth = 0usize;
    let mut found = None;
    for (index, b) in code_brackets(text, caret).into_iter().rev() {
        if b == close as u8 {
            depth += 1;
        } else if b == open as u8 {
            if depth == 0 {
                found = Some(index);
                break;
            }
            depth -= 1;
        }
    }
    let at = found?;
    let line_start = text[..at].rfind('\n').map(|at| at + 1).unwrap_or(0);
    Some(
        text[line_start..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect(),
    )
}

/// Openers less closers of one kind in the whole text, counting code only.
/// Below zero, a closer somewhere has no opener.
fn balance(text: &str, open: u8, close: u8) -> i64 {
    code_brackets(text, text.len())
        .into_iter()
        .map(|(_, b)| match b {
            b if b == open => 1,
            b if b == close => -1,
            _ => 0,
        })
        .sum()
}

/// Whether the line holding `caret` opens an `open` before it that nothing
/// before the caret has closed yet — whether a `close` just after the caret
/// belongs to this line.
fn open_on_line(text: &str, caret: usize, open: u8, close: u8) -> bool {
    let line_start = text[..caret].rfind('\n').map(|at| at + 1).unwrap_or(0);
    let mut depth = 0u32;
    for (at, b) in code_brackets(text, caret) {
        if at < line_start {
            continue;
        }
        if b == open {
            depth += 1;
        } else if b == close {
            depth = depth.saturating_sub(1);
        }
    }
    depth > 0
}

/// Every bracket in `text[..end]` that is code, with its byte offset: not in
/// a string, a character literal or a comment.
///
/// As much of Rust's lexer as brackets need — line comments, nested block
/// comments, strings with their escapes, raw strings with any number of
/// `#`, and a `'` that is a character literal only when a quote closes it
/// one character (or an escape) later, which a lifetime never does. On
/// bytes: every delimiter is ASCII, and no byte of a multi-byte character
/// is one.
fn code_brackets(text: &str, end: usize) -> Vec<(usize, u8)> {
    let bytes = text.as_bytes();
    let end = end.min(bytes.len());
    let at = |i: usize| bytes.get(i).copied();
    let word = |i: Option<usize>| {
        i.and_then(at)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < end {
        match bytes[i] {
            b'/' if at(i + 1) == Some(b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if at(i + 1) == Some(b'*') => {
                let mut depth = 1;
                i += 2;
                while i < bytes.len() && depth > 0 {
                    match (bytes[i], at(i + 1)) {
                        (b'/', Some(b'*')) => {
                            depth += 1;
                            i += 2;
                        }
                        (b'*', Some(b'/')) => {
                            depth -= 1;
                            i += 2;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => i += 2,
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            // `r"…"`, `r#"…"#`, `br"…"`: raw, so no escapes, and closed only
            // by a quote with as many `#` after it as opened it. Not the `r`
            // at the end of a word, nor a raw identifier's `r#type`.
            b'r' if !word(i.checked_sub(1))
                || (at(i.wrapping_sub(1)) == Some(b'b') && !word(i.checked_sub(2))) =>
            {
                let mut j = i + 1;
                while at(j) == Some(b'#') {
                    j += 1;
                }
                let hashes = j - i - 1;
                if at(j) != Some(b'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                while j < bytes.len() {
                    if bytes[j] == b'"' && (1..=hashes).all(|k| at(j + k) == Some(b'#')) {
                        j += 1 + hashes;
                        break;
                    }
                    j += 1;
                }
                i = j;
            }
            b'\'' => {
                if at(i + 1) == Some(b'\\') {
                    // An escape: `'\n'`, `'\''`, `'\u{7b}'` — to the quote
                    // after the escaped character, never past the line.
                    let mut j = i + 3;
                    while let Some(b) = at(j) {
                        j += 1;
                        if b == b'\'' || b == b'\n' {
                            break;
                        }
                    }
                    i = j;
                } else {
                    let len = match at(i + 1) {
                        Some(b) if b >= 0xF0 => 4,
                        Some(b) if b >= 0xE0 => 3,
                        Some(b) if b >= 0xC0 => 2,
                        _ => 1,
                    };
                    i += if at(i + 1 + len) == Some(b'\'') {
                        2 + len
                    } else {
                        1
                    };
                }
            }
            b @ (b'{' | b'}' | b'(' | b')' | b'[' | b']') => {
                out.push((i, b));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// What Enter at `caret` should insert.
///
/// The current line's indentation, one level deeper when the caret sits right
/// after an opening bracket — and when its closer is right after the caret
/// too, the closer goes onto a line of its own at the outer level, with the
/// caret on the indented line between: `{|}` becomes the three lines every
/// block in the file is shaped like. Anything cleverer is the language
/// server's.
pub(super) fn on_enter(text: &str, caret: usize) -> Edit {
    let caret = caret.min(text.len());
    let before = &text[..caret];
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let line = &before[line_start..];
    let indent: String = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();

    let last = line.trim_end().chars().last();
    let deeper = matches!(last, Some('{' | '(' | '['));
    let next = text[caret..].chars().next();
    let between = deeper
        && line.chars().last() == last
        && last
            .and_then(closer)
            .is_some_and(|close| next == Some(close));

    if between {
        let insert = format!("\n{indent}    \n{indent}");
        Edit {
            range: (caret, caret),
            caret: caret + 1 + indent.len() + 4,
            text: insert,
            select: None,
        }
    } else if deeper {
        Edit::insert(caret, &format!("\n{indent}    "))
    } else {
        Edit::insert(caret, &format!("\n{indent}"))
    }
}

/// Backspace inside an empty pair removes both halves — the undo of the
/// auto-close, so a `(` typed by mistake costs one key to take back rather
/// than two.
pub(super) fn on_backspace(text: &str, caret: usize) -> Option<Edit> {
    let caret = caret.min(text.len());
    if !text.is_char_boundary(caret) {
        return None;
    }
    let previous = text[..caret].chars().next_back()?;
    let close = closer(previous)?;
    let next = text[caret..].chars().next()?;
    if next != close {
        return None;
    }
    Some(Edit {
        range: (caret - previous.len_utf8(), caret + close.len_utf8()),
        text: String::new(),
        caret: caret - previous.len_utf8(),
        select: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(text: &str, edit: &Edit) -> (String, usize) {
        let mut out = text.to_string();
        out.replace_range(edit.range.0..edit.range.1, &edit.text);
        (out, edit.caret)
    }

    fn typed(text: &str, caret: usize, key: char) -> Option<(String, usize)> {
        on_type(text, caret, caret, key).map(|edit| applied(text, &edit))
    }

    #[test]
    fn an_opener_before_nothing_or_whitespace_brings_its_closer() {
        assert_eq!(typed("let x = ", 8, '('), Some(("let x = ()".into(), 9)));
        assert_eq!(
            typed("fn main() ", 10, '{'),
            Some(("fn main() {}".into(), 11))
        );
        assert_eq!(typed("let v = ", 8, '['), Some(("let v = []".into(), 9)));
        assert_eq!(typed("foo(", 4, '"'), Some(("foo(\"\"".into(), 5)));
        // Before a closer or a separator too: `foo(|)` typing `[` is fine.
        assert_eq!(typed("foo()", 4, '['), Some(("foo([])".into(), 5)));
        assert_eq!(typed("a, b", 1, '('), Some(("a(), b".into(), 2)));
    }

    #[test]
    fn an_opener_in_front_of_a_word_is_left_to_the_browser() {
        assert_eq!(typed("foo bar", 4, '('), None);
        assert_eq!(typed("xbar", 0, '{'), None);
    }

    #[test]
    fn a_closer_against_its_twin_steps_over_it() {
        assert_eq!(typed("foo()", 4, ')'), Some(("foo()".into(), 5)));
        assert_eq!(typed("a[]", 2, ']'), Some(("a[]".into(), 3)));
        assert_eq!(typed("\"\"", 1, '"'), Some(("\"\"".into(), 2)));
        // A closer with nothing to step over is typed as it is.
        assert_eq!(typed("foo(a", 5, ')'), None);
    }

    /// The wrong brace: a `{` added to a line whose block and `}` are
    /// already below — or retyped after deleting one — was paired, and the
    /// file ended with a `}` too many.
    #[test]
    fn an_opener_whose_closer_is_already_waiting_is_not_paired() {
        let text = "impl Rate \n    fn a() {}\n}\n";
        assert_eq!(typed(text, 10, '{'), None);
        // A balanced file still pairs, as ever.
        assert_eq!(typed("fn a() \n", 7, '{'), Some(("fn a() {}\n".into(), 8)));
        // And a file one closer short pairs too: that is a file being
        // written, and the new block needs its own.
        let short = "fn a() {\n    if x \n";
        assert!(typed(short, short.len() - 1, '{').is_some());
    }

    /// Only code counts: a brace in a string, a character, a comment or a
    /// raw string is text, a lifetime is not a character literal, and a
    /// character literal can be any character.
    #[test]
    fn brackets_in_strings_characters_and_comments_are_not_counted() {
        let text = "let s = \"}\\\"}\";\nlet c = '}';\nlet e = '\\u{7d}';\n// }\n\
                    /* } /* } */ } */\nlet r = r#\"}\"#;\nlet z = '中';\n\
                    fn f<'a>(x: &'a str) {}\n";
        assert_eq!(balance(text, b'{', b'}'), 0);
        assert_eq!(balance(text, b'(', b')'), 0);
        // So an opener after all of it still pairs.
        let then = format!("{text}let v = ");
        assert!(typed(&then, then.len(), '{').is_some());
        // And a closer on a blank line finds its opener past a `"{"`.
        let quoted = "fn a() {\n    let s = \"{\";\n    ";
        let (out, _) = typed(quoted, quoted.len(), '}').expect("outdents");
        assert_eq!(out, "fn a() {\n    let s = \"{\";\n}");
    }

    /// A closer steps over only what its own line opened, the pair an
    /// auto-close makes. In front of a `}` that closes a block opened
    /// above, the one typed is a block's missing brace, and stepping over
    /// it left the file one short.
    #[test]
    fn a_closer_steps_over_only_what_its_line_opened() {
        assert_eq!(typed("Self { x }", 9, '}'), Some(("Self { x }".into(), 10)));
        let short = "impl A {\n    fn b() {\n        x\n}";
        let caret = short.len() - 1;
        let (out, _) = typed(short, caret, '}').expect("typed, not stepped over");
        assert_eq!(out, "impl A {\n    fn b() {\n        x\n    }}");
        // A call closed on a line of its own: typed as it is, not skipped.
        let call = "foo(\n    a\n)";
        assert_eq!(typed(call, call.len() - 1, ')'), None);
    }

    #[test]
    fn a_closer_on_a_blank_line_takes_its_openers_indent() {
        let text = "    if x {\n        \n        ";
        let (out, caret) = typed(text, text.len(), '}').expect("outdents");
        assert_eq!(out, "    if x {\n        \n    }");
        assert_eq!(caret, out.len());
        // Nested: the matching opener, not the nearest.
        let nested = "fn f() {\n    if x {\n    }\n    ";
        let (out, _) = typed(nested, nested.len(), '}').expect("outdents");
        assert_eq!(out, "fn f() {\n    if x {\n    }\n}");
        // Already at the right indent: nothing to do, the browser types it.
        let aligned = "    if x {\n    ";
        assert_eq!(typed(aligned, aligned.len(), '}'), None);
        // A blank line with no opener to match is left alone.
        assert_eq!(typed("    ", 4, ')'), None);
    }

    #[test]
    fn quotes_know_whether_they_open_or_close() {
        // Inside an open string the quote closes it, so no pair.
        assert_eq!(typed("let s = \"abc", 12, '"'), None);
        // After a word: not a string starting.
        assert_eq!(typed("let s = abc", 11, '"'), None);
        // After an escape.
        assert_eq!(typed("\"a\\", 3, '"'), None);
        // Raw and byte strings are the exception to the word rule.
        assert_eq!(
            typed("let s = r", 9, '"'),
            Some(("let s = r\"\"".into(), 10))
        );
        assert_eq!(
            typed("let s = b", 9, '"'),
            Some(("let s = b\"\"".into(), 10))
        );
    }

    #[test]
    fn a_selection_is_wrapped_and_stays_selected() {
        let edit = on_type("say hello now", 4, 9, '(').expect("wraps");
        let (out, _) = applied("say hello now", &edit);
        assert_eq!(out, "say (hello) now");
        assert_eq!(edit.select, Some((5, 10)));
        assert_eq!(&out[5..10], "hello");
        // A plain letter typed over a selection replaces it as ever.
        assert_eq!(on_type("say hello now", 4, 9, 'x'), None);
    }

    #[test]
    fn enter_copies_the_indent_and_deepens_after_an_opener() {
        let text = "fn main() {\n    let x = 1;\n";
        assert_eq!(on_enter(text, 11).text, "\n    ");
        assert_eq!(on_enter(text, text.len()).text, "\n");
        let nested = "    if x {\n";
        assert_eq!(on_enter(nested, nested.len() - 1).text, "\n        ");
    }

    #[test]
    fn enter_between_a_pair_puts_the_closer_on_its_own_line() {
        let text = "    fn f() {}";
        let edit = on_enter(text, text.len() - 1);
        let (out, caret) = applied(text, &edit);
        assert_eq!(out, "    fn f() {\n        \n    }");
        assert_eq!(&out[..caret], "    fn f() {\n        ");
        // Not between: the closer is not directly after the caret.
        let apart = "fn f() { x }";
        assert_eq!(on_enter(apart, 8).text, "\n    ");
    }

    #[test]
    fn backspace_inside_an_empty_pair_removes_both() {
        let edit = on_backspace("foo()", 4).expect("removes the pair");
        assert_eq!(applied("foo()", &edit), ("foo".into(), 3));
        let quotes = on_backspace("x = \"\"", 5).expect("quotes too");
        assert_eq!(applied("x = \"\"", &quotes), ("x = ".into(), 4));
        // Anything else is an ordinary Backspace.
        assert_eq!(on_backspace("foo(a)", 4), None);
        assert_eq!(on_backspace("foo", 3), None);
        assert_eq!(on_backspace("()", 0), None);
    }

    #[test]
    fn offsets_inside_a_multibyte_character_are_refused_not_panicked() {
        let text = "中文";
        assert_eq!(on_type(text, 1, 1, '('), None);
        assert_eq!(on_backspace(text, 1), None);
        // Between the two characters the opener faces a word and is left to
        // the browser; at the end it pairs, and the offsets are bytes.
        assert_eq!(typed(text, 3, '('), None);
        assert_eq!(typed(text, 6, '('), Some(("中文()".into(), 7)));
    }
}
