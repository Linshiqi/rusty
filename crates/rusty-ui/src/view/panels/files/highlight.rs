//! Painting a line: syntax spans, semantic overlay, and squiggles.
//!
//! Tokens travel as *meanings* rather than colours, so the stylesheet decides
//! how each one looks and a light window is never painted in dark-theme
//! pastels.

use leptos::prelude::*;

use rusty_edit::{Line, Span, Token};
use rusty_lsp::{DiagSeverity, FileDiagnostic, SemanticSpan};

/// A line's spans with the diagnostics for that line woven in.
///
/// Splitting the highlight runs at the diagnostic's scalar columns keeps the
/// squiggle in the text flow — an absolutely-positioned overlay multiplied by
/// `ch` would drift on every CJK glyph, which is two columns wide.
pub(super) fn decorate(line: Line, index: u32, diags: &[FileDiagnostic]) -> AnyView {
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

    if segments.is_empty() {
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

    // One painted run: its text, its syntax class, and the squiggle over it.
    type Run = (String, Token, Option<(DiagSeverity, String)>);
    let mut out: Vec<Run> = Vec::new();
    let mut col = 0u32;
    for span in line.spans {
        for ch in span.text.chars() {
            let mark = mark_at(col).map(|(severity, message)| (severity, message.to_string()));
            match out.last_mut() {
                Some((text, token, last_mark)) if *token == span.token && *last_mark == mark => {
                    text.push(ch);
                }
                _ => out.push((ch.to_string(), span.token, mark)),
            }
            col += 1;
        }
    }

    out.into_iter()
        .map(|(text, token, mark)| {
            let base = class_of(token);
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
/// a light window is not painted with a dark theme's syntax colours.
fn class_of(token: Token) -> &'static str {
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
