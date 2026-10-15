//! The compiler's colours over the painter's: a language server's semantic
//! tokens re-cut into a painted line, so its colours win where it has them
//! and the lexical base shows everywhere else. Pure, for both windows.

use crate::{Line, Span, Token};

/// A server's legend name, mapped into the palette — rust-analyzer's and
/// clangd's share most of them.
///
/// Unmapped kinds — operators, punctuation — return `None` and keep the
/// lexical base colour: overlaying everything would repaint half the
/// buffer in one hue and lose more than it adds.
pub fn token_of(kind: &str) -> Option<Token> {
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

/// `line` re-cut so each mark — a first column, the column after its last,
/// and its token, columns in characters — colours what it covers.
pub fn overlay(line: Line, marks: &[(u32, u32, Token)]) -> Line {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Line {
        Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        }
    }

    fn cut(line: Line) -> Vec<(String, Token)> {
        line.spans.into_iter().map(|s| (s.text, s.token)).collect()
    }

    #[test]
    fn a_mark_colours_what_it_covers_and_nothing_else() {
        let out = overlay(plain("let radio = 1;"), &[(4, 9, Token::Variable)]);
        assert_eq!(
            cut(out),
            [
                ("let ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
                (" = 1;".to_string(), Token::Plain),
            ]
        );
    }

    /// "中文 radio" — the field starts at character 3, not byte 7.
    #[test]
    fn columns_are_characters() {
        let out = overlay(plain("中文 radio"), &[(3, 8, Token::Variable)]);
        assert_eq!(
            cut(out),
            [
                ("中文 ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
            ]
        );
    }

    #[test]
    fn operators_keep_the_lexical_colour() {
        assert_eq!(token_of("operator"), None);
        assert_eq!(token_of("method"), Some(Token::Function));
    }
}
