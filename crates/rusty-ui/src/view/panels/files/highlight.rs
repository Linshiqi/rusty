//! Painting a line: syntax spans, semantic overlay, and squiggles.
//!
//! Tokens travel as *meanings* rather than colours, so the stylesheet decides
//! how each one looks and a light window is never painted in dark-theme
//! pastels.

use leptos::prelude::*;

use rusty_edit::{Line, Span, Token};
use rusty_lsp::{DiagSeverity, FileDiagnostic, SemanticSpan};

use super::Placed;

/// A line's spans with the diagnostics for that line and its inlay hints
/// woven in.
///
/// Splitting the highlight runs at the diagnostic's scalar columns keeps the
/// squiggle in the text flow — an absolutely-positioned overlay multiplied by
/// `ch` would drift on every CJK glyph, which is two columns wide. The hints
/// are in the flow for the same reason, and for one more: everything after a
/// hint on its line has to move over, and only the flow moves it
/// (`hints.rs`). `link` is the name Ctrl over it would go to the definition
/// of, drawn as a link while Ctrl is held.
pub(super) fn decorate(
    line: Line,
    index: u32,
    diags: &[FileDiagnostic],
    hints: &[Placed],
    link: Option<(u32, u32)>,
) -> AnyView {
    let mut segments: Vec<(u32, u32, DiagSeverity, String)> = Vec::new();
    let length = line
        .spans
        .iter()
        .map(|s| s.text.chars().count() as u32)
        .sum::<u32>();
    for d in diags {
        if index < d.start_line || index > d.end_line {
            continue;
        }
        let from = if index == d.start_line {
            d.start_col
        } else {
            0
        };
        let to = if index == d.end_line {
            d.end_col
        } else {
            length
        };
        // A zero-width diagnostic still deserves a visible squiggle.
        let to = to.max(from + 1).min(length.max(from + 1));
        segments.push((from, to, d.severity, d.message.clone()));
    }

    if segments.is_empty() && hints.is_empty() && link.is_none() {
        return line
            .spans
            .into_iter()
            .map(|span| view! { <span class=class_of(span.token)>{span.text}</span> })
            .collect_view()
            .into_any();
    }

    // Worst severity wins where ranges overlap; DiagSeverity orders worst-first.
    let mark_at = |col: u32| -> Option<(DiagSeverity, &str)> {
        segments
            .iter()
            .filter(|(from, to, ..)| (*from..*to).contains(&col))
            .min_by_key(|(_, _, severity, _)| *severity)
            .map(|(_, _, severity, message)| (*severity, message.as_str()))
    };

    // One painted run — its text, its syntax class, and the squiggle over
    // it — or a hint, between two of them.
    enum Piece<'h> {
        Run(String, Token, Option<(DiagSeverity, String)>, bool),
        Hint(&'h Placed),
    }
    let mut out: Vec<Piece> = Vec::new();
    let mut col = 0u32;
    let mut next = 0;
    for span in line.spans {
        for ch in span.text.chars() {
            while let Some(hint) = hints.get(next).filter(|hint| hint.col.min(length) == col) {
                out.push(Piece::Hint(hint));
                next += 1;
            }
            let mark = mark_at(col).map(|(severity, message)| (severity, message.to_string()));
            let linked = link.is_some_and(|(from, to)| (from..to).contains(&col));
            match out.last_mut() {
                Some(Piece::Run(text, token, last_mark, last_linked))
                    if *token == span.token && *last_mark == mark && *last_linked == linked =>
                {
                    text.push(ch);
                }
                _ => out.push(Piece::Run(ch.to_string(), span.token, mark, linked)),
            }
            col += 1;
        }
    }
    // At the end of the line, and past it: a line shortened under a hint
    // before the next answer draws the hint at its end.
    out.extend(hints[next.min(hints.len())..].iter().map(Piece::Hint));

    out.into_iter()
        .map(|piece| {
            let (text, token, mark, linked) = match piece {
                Piece::Run(text, token, mark, linked) => (text, token, mark, linked),
                Piece::Hint(hint) => return hint_view(hint),
            };
            let base = if linked {
                format!("{} editor-link", class_of(token))
            } else {
                class_of(token).to_string()
            };
            match mark {
                None => view! { <span class=base>{text}</span> }.into_any(),
                Some((severity, message)) => {
                    let squiggle = match severity {
                        DiagSeverity::Error => "diag-error",
                        DiagSeverity::Warning => "diag-warning",
                        _ => "diag-hint",
                    };
                    view! {
                        <span class=format!("{base} {squiggle}") title=message>{text}</span>
                    }
                    .into_any()
                }
            }
        })
        .collect_view()
        .into_any()
}

/// A hint as the echo draws it: its label on a shade of its own, and the
/// spaces it asked for either side of it outside the shade, as VS Code
/// draws them. No padding and no margin — the hint is exactly as wide as
/// `hints.rs` measures it, or everything after it would sit off its glyph.
fn hint_view(hint: &Placed) -> AnyView {
    view! {
        {hint.pad_left.then_some(" ")}
        <span class="inlay-hint">{hint.label.clone()}</span>
        {hint.pad_right.then_some(" ")}
    }
    .into_any()
}

/// Hover markdown, minimally: fenced blocks become highlighted code, `---`
/// becomes a divider, everything else is prose. The code gets the same
/// lexical colours the editor uses, so the tooltip does not describe Rust
/// in monochrome an inch above a highlighted buffer.
pub(super) fn hover_parts(text: &str) -> AnyView {
    enum Part {
        Code(Vec<String>),
        Prose(String),
        Rule,
    }

    let mut parts: Vec<Part> = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
                parts.push(Part::Code(Vec::new()));
            }
            continue;
        }
        if in_code {
            if let Some(Part::Code(lines)) = parts.last_mut() {
                lines.push(line.to_string());
            }
            continue;
        }
        if line.trim() == "---" {
            parts.push(Part::Rule);
            continue;
        }
        match parts.last_mut() {
            Some(Part::Prose(prose)) => {
                prose.push('\n');
                prose.push_str(line);
            }
            _ => parts.push(Part::Prose(line.to_string())),
        }
    }

    parts
        .into_iter()
        .filter(|part| !matches!(part, Part::Prose(text) if text.trim().is_empty()))
        .map(|part| match part {
            Part::Rule => view! { <div class="my-1.5 h-px bg-line" /> }.into_any(),
            Part::Prose(prose) => view! {
                <div class="font-sans">
                    <crate::view::markdown::Markdown text=prose.trim().to_string() />
                </div>
            }
            .into_any(),
            Part::Code(lines) => view! {
                <pre class="my-1 overflow-x-auto whitespace-pre">
                    {lines
                        .into_iter()
                        .map(|line| {
                            let spans = rusty_edit::lexical::refine(vec![Span {
                                text: line,
                                token: Token::Plain,
                            }]);
                            view! {
                                <div>
                                    {spans
                                        .into_iter()
                                        .map(|span| {
                                            view! {
                                                <span class=class_of(
                                                    span.token,
                                                )>{span.text}</span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                        })
                        .collect_view()}
                </pre>
            }
            .into_any(),
        })
        .collect_view()
        .into_any()
}

/// rust-analyzer's legend names, mapped into the palette.
///
/// Unmapped kinds — operators, namespaces, punctuation — return `None` and
/// keep the lexical base colour: overlaying everything would repaint half
/// the buffer in one hue and lose more than it adds.
fn semantic_token(kind: &str) -> Option<Token> {
    Some(match kind {
        "keyword" | "lifetime" | "boolean" | "selfKeyword" | "selfTypeKeyword" => Token::Keyword,
        "comment" => Token::Comment,
        "string" | "character" => Token::Str,
        "number" => Token::Number,
        "macro" | "macroBang" | "derive" | "deriveHelper" | "attribute" | "attributeBracket" => {
            Token::Macro
        }
        "function" | "method" | "procMacro" => Token::Function,
        "struct" | "enum" | "union" | "trait" | "typeAlias" | "type" | "builtinType" | "class"
        | "interface" | "enumMember" | "typeParameter" => Token::Type,
        "variable" | "parameter" | "property" | "field" | "const" | "static" => Token::Variable,
        "namespace" => Token::Namespace,
        _ => return None,
    })
}

/// Re-cut a line's spans so the compiler's colours win where they exist and
/// the lexical base shows everywhere else.
/// The semantic spans on one line, out of a list sorted by line: found by
/// halving the list rather than by filtering it, which the overlay did once
/// for every line it drew — a scan of every token in the file, per line.
pub(super) fn semantic_on(spans: &[SemanticSpan], line: u32) -> &[SemanticSpan] {
    let start = spans.partition_point(|span| span.line < line);
    let end = start + spans[start..].partition_point(|span| span.line <= line);
    &spans[start..end]
}

/// Everything a row of the echo draws, as one number: its runs, the
/// squiggles over it, its hints, its fold and its indent guides. A row whose
/// number did not change draws what it drew, so the window keeps its markup.
pub(super) fn row_hash(
    line: &Line,
    diags: &[FileDiagnostic],
    hints: &[Placed],
    link: Option<(u32, u32)>,
    folded: Option<u32>,
    guides: u8,
) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for span in &line.spans {
        span.text.hash(&mut hasher);
        (span.token as u8).hash(&mut hasher);
    }
    for d in diags {
        let place = (d.start_line, d.start_col, d.end_line, d.end_col);
        (place, d.severity as u8).hash(&mut hasher);
        d.message.hash(&mut hasher);
    }
    hints.hash(&mut hasher);
    link.hash(&mut hasher);
    folded.hash(&mut hasher);
    guides.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn overlay_semantic(line: Line, index: u32, semantic: &[SemanticSpan]) -> Line {
    let marks: Vec<(u32, u32, Token)> = semantic
        .iter()
        .filter(|span| span.line == index)
        .filter_map(|span| {
            semantic_token(&span.kind)
                .map(|token| (span.start_col, span.start_col + span.length, token))
        })
        .collect();
    if marks.is_empty() {
        return line;
    }

    let mut out: Vec<Span> = Vec::with_capacity(line.spans.len());
    let mut col = 0u32;
    for span in line.spans {
        let mut text = String::new();
        let mut current = span.token;
        for ch in span.text.chars() {
            let token = marks
                .iter()
                .find(|(from, to, _)| (*from..*to).contains(&col))
                .map(|(_, _, token)| *token)
                .unwrap_or(span.token);
            if token != current && !text.is_empty() {
                out.push(Span {
                    text: std::mem::take(&mut text),
                    token: current,
                });
            }
            current = token;
            text.push(ch);
            col += 1;
        }
        if !text.is_empty() {
            out.push(Span {
                text,
                token: current,
            });
        }
    }
    Line { spans: out }
}

/// Token to a class the stylesheet owns.
///
/// Classes rather than inline colours, so the palette lives with the theme and
/// a light window is not painted with a dark theme's syntax colours. The
/// Markdown page's code blocks read the same map, so a fence and the file it
/// was copied from are the same colours.
pub(crate) fn class_of(token: Token) -> &'static str {
    match token {
        Token::Plain => "text-label",
        Token::Keyword => "tok-keyword",
        Token::Str => "tok-string",
        Token::Number => "tok-number",
        Token::Comment => "tok-comment",
        Token::Type => "tok-type",
        Token::Function => "tok-function",
        Token::Macro => "tok-macro",
        Token::Punctuation => "tok-punctuation",
        Token::Variable => "tok-variable",
        Token::Namespace => "tok-namespace",
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    fn span_at(line: u32, start_col: u32) -> SemanticSpan {
        SemanticSpan {
            line,
            start_col,
            length: 1,
            kind: "variable".to_string(),
        }
    }

    /// A line's spans out of a sorted list are exactly the ones a filter
    /// finds — on a line with several, a line with none, and past the end.
    #[test]
    fn a_lines_semantic_spans_are_found_by_halving_the_sorted_list() {
        let spans = [
            span_at(0, 1),
            span_at(2, 0),
            span_at(2, 4),
            span_at(2, 9),
            span_at(5, 3),
        ];
        for line in 0..8 {
            let halved: Vec<_> = semantic_on(&spans, line).to_vec();
            let filtered: Vec<_> = spans.iter().filter(|s| s.line == line).cloned().collect();
            assert_eq!(halved, filtered, "line {line}");
        }
    }

    /// A row's number moves with anything it draws — a token, a squiggle, a
    /// hint, a fold, its indent guides — and stays put otherwise, which is
    /// what keeps a row's markup.
    #[test]
    fn a_rows_hash_changes_with_what_it_draws_and_only_then() {
        let line = line_of("let x = 1;");
        let base = row_hash(&line, &[], &[], None, None, 0);
        assert_eq!(
            row_hash(&line_of("let x = 1;"), &[], &[], None, None, 0),
            base
        );
        assert_ne!(
            row_hash(&line_of("let x = 2;"), &[], &[], None, None, 0),
            base
        );
        let mut keyword = line.clone();
        keyword.spans[0].token = Token::Keyword;
        assert_ne!(row_hash(&keyword, &[], &[], None, None, 0), base);
        let squiggle = FileDiagnostic {
            severity: DiagSeverity::Error,
            message: "no".to_string(),
            source: None,
            code: None,
            start_line: 3,
            start_col: 4,
            end_line: 3,
            end_col: 5,
        };
        assert_ne!(
            row_hash(&line, std::slice::from_ref(&squiggle), &[], None, None, 0),
            base
        );
        assert_ne!(row_hash(&line, &[], &[], None, Some(12), 0), base);
        assert_ne!(row_hash(&line, &[], &[], None, None, 2), base);
        let hint = Placed {
            col: 5,
            label: ": i32".to_string(),
            pad_left: false,
            pad_right: false,
            parameter: false,
        };
        assert_ne!(
            row_hash(&line, &[], std::slice::from_ref(&hint), None, None, 0),
            base
        );
        assert_ne!(row_hash(&line, &[], &[], Some((4, 5)), None, 0), base);
    }

    fn line_of(text: &str) -> Line {
        Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        }
    }

    fn span(line: u32, start_col: u32, length: u32, kind: &str) -> SemanticSpan {
        SemanticSpan {
            line,
            start_col,
            length,
            kind: kind.to_string(),
        }
    }

    #[test]
    fn the_compilers_colour_wins_inside_its_range_only() {
        let out = overlay_semantic(line_of("let radio = 1;"), 0, &[span(0, 4, 5, "variable")]);
        let texts: Vec<(String, Token)> =
            out.spans.into_iter().map(|s| (s.text, s.token)).collect();
        assert_eq!(
            texts,
            vec![
                ("let ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
                (" = 1;".to_string(), Token::Plain),
            ],
        );
    }

    #[test]
    fn other_lines_and_unknown_kinds_change_nothing() {
        let untouched = overlay_semantic(
            line_of("let radio = 1;"),
            0,
            &[span(3, 0, 5, "variable"), span(0, 0, 3, "operator")],
        );
        assert_eq!(untouched.spans.len(), 1);
        assert_eq!(untouched.spans[0].token, Token::Plain);
    }

    #[test]
    fn cjk_columns_are_scalar_not_bytes() {
        // "中文 radio" — the variable starts at scalar column 3.
        let out = overlay_semantic(line_of("中文 radio"), 0, &[span(0, 3, 5, "field")]);
        let texts: Vec<(String, Token)> =
            out.spans.into_iter().map(|s| (s.text, s.token)).collect();
        assert_eq!(
            texts,
            vec![
                ("中文 ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
            ],
        );
    }
}
