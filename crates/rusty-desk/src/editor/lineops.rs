//! What VS Code does to whole lines, as arithmetic over the text and the
//! cursors: move them, copy them, delete them, open one above or below,
//! select one, join them, comment them.
//!
//! Every function takes the text and the cursors (document bytes, the
//! first one primary) and answers the text and the cursors after, so undo
//! and the view treat a line moved like any other edit.

use super::cursors::{Cursor, line_end, line_start};

/// The first and last line every cursor touches, merged where they meet,
/// as byte spans of whole lines without their final `\n`: (start, end).
fn blocks(text: &str, cursors: &[Cursor]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = cursors
        .iter()
        .map(|cursor| {
            let start = line_start(text, cursor.start());
            // A selection that ends at a line's first column does not take
            // that line, as VS Code reads it.
            let end_at = if !cursor.is_caret()
                && cursor.end() > start
                && at_line_start(text, cursor.end())
            {
                cursor.end() - 1
            } else {
                cursor.end()
            };
            (start, line_end(text, end_at))
        })
        .collect();
    spans.sort();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 + 1 => last.1 = last.1.max(span.1),
            _ => merged.push(span),
        }
    }
    merged
}

fn at_line_start(text: &str, at: usize) -> bool {
    at == 0 || text.as_bytes().get(at - 1) == Some(&b'\n')
}

fn shift(cursors: &[Cursor], from: usize, to: usize, by: isize) -> Vec<Cursor> {
    let moved = |at: usize| {
        if at >= from && at <= to {
            (at as isize + by) as usize
        } else {
            at
        }
    };
    cursors
        .iter()
        .map(|c| Cursor {
            anchor: moved(c.anchor),
            head: moved(c.head),
        })
        .collect()
}

/// Alt+Up and Alt+Down: the lines the cursors touch swap with the line
/// above or below, the cursors going with them. Nothing moves past the
/// first or last line.
pub fn moved(text: &str, cursors: &[Cursor], up: bool) -> Option<(String, Vec<Cursor>)> {
    let spans = blocks(text, cursors);
    let mut out = text.to_string();
    let mut after = cursors.to_vec();
    let ordered: Vec<(usize, usize)> = if up {
        spans
    } else {
        spans.into_iter().rev().collect()
    };
    let mut changed = false;
    for (start, end) in ordered {
        if up {
            if start == 0 {
                continue;
            }
            let above = line_start(&out, start - 1);
            let block = out[start..end].to_string();
            let line = out[above..start - 1].to_string();
            out.replace_range(above..end, &format!("{block}\n{line}"));
            after = shift(&after, start, end, -((start - above) as isize));
        } else {
            if end >= out.len() {
                continue;
            }
            let below_end = line_end(&out, end + 1);
            let block = out[start..end].to_string();
            let line = out[end + 1..below_end].to_string();
            out.replace_range(start..below_end, &format!("{line}\n{block}"));
            after = shift(&after, start, end, (below_end - end) as isize);
        }
        changed = true;
    }
    changed.then_some((out, after))
}

/// Shift+Alt+Up and Shift+Alt+Down: the lines the cursors touch, doubled;
/// the cursors stay on the copy above (Up) or move to the copy below.
pub fn copied(text: &str, cursors: &[Cursor], up: bool) -> (String, Vec<Cursor>) {
    let mut out = text.to_string();
    let mut after = cursors.to_vec();
    for (start, end) in blocks(text, cursors).into_iter().rev() {
        let block = text[start..end].to_string();
        out.insert_str(end, &format!("\n{block}"));
        let length = (end - start + 1) as isize;
        if up {
            after = shift(&after, end + 1, usize::MAX, length);
        } else {
            after = shift(&after, start, usize::MAX, length);
        }
    }
    (out, after)
}

/// Ctrl+Shift+K: the lines the cursors touch, gone, each cursor at the
/// start of the line that came up into their place.
pub fn deleted(text: &str, cursors: &[Cursor]) -> (String, Vec<Cursor>) {
    let mut out = text.to_string();
    let mut carets = Vec::new();
    for (start, end) in blocks(text, cursors).into_iter().rev() {
        let (from, to) = if end < out.len() {
            (start, end + 1)
        } else if start > 0 {
            (start - 1, end)
        } else {
            (start, end)
        };
        out.replace_range(from..to, "");
        let removed = to - from;
        for caret in &mut carets {
            *caret -= removed;
        }
        carets.push(line_start(&out, from.min(out.len())));
    }
    carets.sort_unstable();
    carets.dedup();
    let cursors = carets.into_iter().map(Cursor::caret).collect();
    (out, cursors)
}

/// Ctrl+Enter and Ctrl+Shift+Enter: a line opened below (or above) each
/// cursor's, indented as that line is, the cursor on it.
pub fn opened(text: &str, cursors: &[Cursor], above: bool) -> (String, Vec<Cursor>) {
    let (out, after) = super::cursors::edit(text, cursors, |text, cursor| {
        let start = line_start(text, cursor.head);
        let end = line_end(text, cursor.head);
        let indent: String = text[start..end]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if above {
            (start, start, format!("{indent}\n"))
        } else {
            (end, end, format!("\n{indent}"))
        }
    });
    if !above {
        return (out, after);
    }
    // Above, each cursor stands after what it wrote — the new line's break —
    // so one byte back is the end of the new line.
    let after = after
        .into_iter()
        .map(|c| Cursor::caret(c.head - 1))
        .collect();
    (out, after)
}

/// Ctrl+L: each cursor's lines selected with their break; pressed again on
/// a selection of whole lines, the next line joins it.
pub fn line_selected(text: &str, cursors: &[Cursor]) -> Vec<Cursor> {
    cursors
        .iter()
        .map(|cursor| {
            let start = line_start(text, cursor.start());
            let whole =
                !cursor.is_caret() && cursor.start() == start && at_line_start(text, cursor.end());
            // On whole lines already, the line after them; otherwise the
            // line the selection ends on.
            let end = line_end(text, cursor.end());
            let end = if whole && end == cursor.end() {
                line_end(text, end + 1)
            } else {
                end
            };
            let end = if end < text.len() { end + 1 } else { end };
            Cursor {
                anchor: start,
                head: end,
            }
        })
        .collect()
}

/// Toggle line comments across the lines the cursors touch: if every
/// non-blank one is already commented, uncomment; otherwise comment them
/// all, the marker at the first non-blank so indented code keeps its shape.
pub fn commented(text: &str, cursors: &[Cursor], marker: &str) -> Option<(String, Vec<Cursor>)> {
    let spans = blocks(text, cursors);
    let lines: Vec<(usize, usize)> = spans
        .iter()
        .flat_map(|&(start, end)| {
            let mut out = Vec::new();
            let mut at = start;
            loop {
                let e = line_end(text, at);
                out.push((at, e.min(end)));
                if e >= end {
                    break;
                }
                at = e + 1;
            }
            out
        })
        .collect();
    let body = |(a, b): (usize, usize)| &text[a..b];
    let all = lines
        .iter()
        .map(|span| body(*span))
        .filter(|line| !line.trim().is_empty())
        .all(|line| line.trim_start().starts_with(marker));
    let mut out = text.to_string();
    let mut after = cursors.to_vec();
    let mut changed = false;
    for &(a, b) in lines.iter().rev() {
        let line = body((a, b));
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let at = a + indent;
        if all {
            let rest = &line[indent + marker.len()..];
            let cut = marker.len() + usize::from(rest.starts_with(' '));
            out.replace_range(at..at + cut, "");
            after = after
                .iter()
                .map(|c| {
                    let back = |p: usize| {
                        if p > at {
                            p.saturating_sub(cut).max(at)
                        } else {
                            p
                        }
                    };
                    Cursor {
                        anchor: back(c.anchor),
                        head: back(c.head),
                    }
                })
                .collect();
        } else {
            let insert = format!("{marker} ");
            out.insert_str(at, &insert);
            after = shift(&after, at, usize::MAX, insert.len() as isize);
        }
        changed = true;
    }
    changed.then_some((out, after))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(at: usize) -> Vec<Cursor> {
        vec![Cursor::caret(at)]
    }

    #[test]
    fn a_line_moves_up_and_down_with_its_cursor() {
        let text = "a\nbb\nc";
        let (up, after) = moved(text, &caret(3), true).unwrap();
        assert_eq!(up, "bb\na\nc");
        assert_eq!(after, caret(1));
        let (down, after) = moved(text, &caret(3), false).unwrap();
        assert_eq!(down, "a\nc\nbb");
        assert_eq!(after, caret(5));
        assert!(
            moved(text, &caret(0), true).is_none(),
            "the first line stays"
        );
        assert!(
            moved(text, &caret(5), false).is_none(),
            "the last line stays"
        );
    }

    #[test]
    fn a_selection_over_two_lines_moves_both() {
        let text = "a\nb\nc\nd";
        let both = [Cursor { anchor: 2, head: 5 }];
        let (out, after) = moved(text, &both, false).unwrap();
        assert_eq!(out, "a\nd\nb\nc");
        assert_eq!(after, [Cursor { anchor: 4, head: 7 }]);
    }

    #[test]
    fn a_line_is_copied_below_with_the_cursor_on_the_copy() {
        let (out, after) = copied("ab\ncd", &caret(1), false);
        assert_eq!(out, "ab\nab\ncd");
        assert_eq!(after, caret(4));
        let (out, after) = copied("ab\ncd", &caret(1), true);
        assert_eq!(out, "ab\nab\ncd");
        assert_eq!(after, caret(1));
    }

    #[test]
    fn a_deleted_line_leaves_the_cursor_at_the_start_of_the_next() {
        let (out, after) = deleted("a\nbb\nc", &caret(3));
        assert_eq!((out.as_str(), after), ("a\nc", caret(2)));
        let (out, _) = deleted("a\nb", &caret(2));
        assert_eq!(out, "a", "the last line takes the break before it");
    }

    #[test]
    fn a_line_opens_below_and_above_with_the_indentation_of_this_one() {
        let (out, after) = opened("    x\ny", &caret(5), false);
        assert_eq!(out, "    x\n    \ny");
        assert_eq!(after, caret(10));
        let (out, after) = opened("    x", &caret(5), true);
        assert_eq!(out, "    \n    x");
        assert_eq!(after, caret(4));
    }

    #[test]
    fn comments_go_on_where_any_line_lacks_one_and_come_off_where_all_have_one() {
        let text = "    a\n    // b";
        let all = [Cursor {
            anchor: 0,
            head: text.len(),
        }];
        let (on, _) = commented(text, &all, "//").unwrap();
        assert_eq!(on, "    // a\n    // // b");
        let (off, _) = commented(
            "  // a\n  //b",
            &[Cursor {
                anchor: 0,
                head: 11,
            }],
            "//",
        )
        .unwrap();
        assert_eq!(off, "  a\n  b");
    }

    #[test]
    fn a_selection_ending_at_a_lines_start_does_not_take_that_line() {
        let text = "a\nb\nc";
        let (out, _) = deleted(text, &[Cursor { anchor: 0, head: 2 }]);
        assert_eq!(out, "b\nc");
    }

    #[test]
    fn ctrl_l_selects_the_line_and_its_break() {
        let first = line_selected("ab\ncd\nef", &caret(1));
        assert_eq!(first, [Cursor { anchor: 0, head: 3 }]);
        assert_eq!(
            line_selected("ab\ncd\nef", &first),
            [Cursor { anchor: 0, head: 6 }]
        );
    }
}
