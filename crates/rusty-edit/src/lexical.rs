//! A lexical refinement pass over plain spans.
//!
//! syntect's Rust grammar colours declarations well and expressions barely:
//! `TimerGroup::new(peripherals.TIMG0)` came back one long plain run, which
//! on screen is a wall of white. This pass splits plain text by the naming
//! conventions the language enforces anyway — `UpperCamel` is a type, an
//! identifier feeding `(` is a call, an identifier feeding `!` is a macro,
//! digits are numbers, and a keyword is a keyword wherever it appears.
//!
//! Model-side and IO-free deliberately: the frontend runs the same pass over
//! fenced code in hover markdown, so the tooltip and the editor cannot
//! disagree about what a type looks like.

use crate::model::{Span, Token};

/// Rust's keywords, for contexts where no grammar has run (hover snippets).
const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];

/// Split every plain span by lexical convention; leave styled spans alone.
pub fn refine(spans: Vec<Span>) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        if span.token != Token::Plain {
            out.push(span);
            continue;
        }
        split_plain(&span.text, &mut out);
    }
    out
}

fn split_plain(text: &str, out: &mut Vec<Span>) {
    let bytes = text.as_bytes();
    let mut plain_start = 0;
    let mut at = 0;

    let flush = |from: usize, to: usize, out: &mut Vec<Span>| {
        if from < to {
            out.push(Span {
                text: text[from..to].to_string(),
                token: Token::Plain,
            });
        }
    };

    while at < text.len() {
        let ch = text[at..].chars().next().expect("in bounds");

        if ch.is_alphabetic() || ch == '_' {
            let start = at;
            let mut end = at;
            for c in text[at..].chars() {
                if c.is_alphanumeric() || c == '_' {
                    end += c.len_utf8();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            let next = bytes.get(end).copied();
            let token = if KEYWORDS.contains(&word) {
                Some(Token::Keyword)
            } else if next == Some(b'!') && word.chars().next().is_some_and(char::is_lowercase) {
                Some(Token::Macro)
            } else if word.chars().next().is_some_and(char::is_uppercase)
                && word.chars().any(char::is_lowercase)
            {
                // UpperCamel with at least one lowercase letter: `TimerGroup`
                // yes, `TIMG0` no — screaming-case is a constant, and painting
                // every register name as a type made the code louder, not
                // clearer.
                Some(Token::Type)
            } else if next == Some(b'(') {
                Some(Token::Function)
            } else {
                None
            };
            if let Some(token) = token {
                flush(plain_start, start, out);
                out.push(Span {
                    text: word.to_string(),
                    token,
                });
                plain_start = end;
            }
            at = end;
            continue;
        }

        if ch.is_ascii_digit() {
            let start = at;
            let mut end = at;
            for c in text[at..].chars() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                    end += c.len_utf8();
                } else {
                    break;
                }
            }
            // `0xFF`, `1_000`, `19.0`, `98768usize` — one run, all number.
            flush(plain_start, start, out);
            out.push(Span {
                text: text[start..end].to_string(),
                token: Token::Number,
            });
            plain_start = end;
            at = end;
            continue;
        }

        at += ch.len_utf8();
    }
    flush(plain_start, text.len(), out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans_of(text: &str) -> Vec<(String, Token)> {
        refine(vec![Span {
            text: text.to_string(),
            token: Token::Plain,
        }])
        .into_iter()
        .map(|s| (s.text, s.token))
        .collect()
    }

    #[test]
    fn a_path_expression_gets_types_and_calls() {
        // Unclassified words merge into their neighbouring plain runs — the
        // exact shape is part of the contract, because every span is a DOM
        // node in the editor.
        let spans = spans_of("esp_hal::interrupt::SoftwareInterruptControl::new(x)");
        assert_eq!(
            spans,
            vec![
                ("esp_hal::interrupt::".to_string(), Token::Plain),
                ("SoftwareInterruptControl".to_string(), Token::Type),
                ("::".to_string(), Token::Plain),
                ("new".to_string(), Token::Function),
                ("(x)".to_string(), Token::Plain),
            ],
        );
    }

    #[test]
    fn screaming_case_is_not_a_type_and_numbers_are_numbers() {
        let spans = spans_of("peripherals.TIMG0, size: 98768");
        assert!(
            !spans.iter().any(|(text, token)| text == "TIMG0" && *token == Token::Type),
            "{spans:?}"
        );
        assert!(spans.contains(&("98768".to_string(), Token::Number)), "{spans:?}");
    }

    #[test]
    fn keywords_and_macros_survive_without_a_grammar() {
        let spans = spans_of("let x = vec![1]; fn main()");
        assert!(spans.contains(&("let".to_string(), Token::Keyword)), "{spans:?}");
        assert!(spans.contains(&("vec".to_string(), Token::Macro)), "{spans:?}");
        assert!(spans.contains(&("fn".to_string(), Token::Keyword)), "{spans:?}");
        assert!(spans.contains(&("main".to_string(), Token::Function)), "{spans:?}");
    }

    #[test]
    fn styled_spans_pass_through_untouched() {
        let spans = refine(vec![Span {
            text: "\"TimerGroup inside a string\"".to_string(),
            token: Token::Str,
        }]);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].token, Token::Str);
    }

    #[test]
    fn cjk_text_stays_plain_and_unsplit_boundaries_hold() {
        let spans = spans_of("// 中文注释 CpuClock 之后");
        assert!(spans.contains(&("CpuClock".to_string(), Token::Type)), "{spans:?}");
    }
}
