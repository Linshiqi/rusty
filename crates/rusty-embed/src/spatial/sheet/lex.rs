//! A row's text as tokens.

use super::value::Problem;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Ident(String),
    Str(String),
    /// `°`, after a number or a bracket.
    Degree,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Open,
    Close,
    Comma,
    Dot,
    Equals,
}

/// A token and the character it starts at, for pointing at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned {
    pub token: Token,
    pub at: usize,
}

pub fn tokens(text: &str) -> Result<Vec<Spanned>, Problem> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let at = i;
        let token = match c {
            ' ' | '\t' => {
                i += 1;
                continue;
            }
            // The rest of the line is a remark.
            '#' => break,
            '0'..='9' | '.' if c != '.' || chars.get(i + 1).is_some_and(char::is_ascii_digit) => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // An exponent: `1e-3`, `2.5E6`.
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let value = text
                    .parse::<f64>()
                    .map_err(|_| Problem::BadNumber { text: text.clone() })?;
                out.push(Spanned {
                    token: Token::Number(value),
                    at,
                });
                continue;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                out.push(Spanned {
                    token: Token::Ident(word),
                    at,
                });
                continue;
            }
            '"' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != '"' {
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(Problem::Unclosed { at });
                }
                let text: String = chars[start..j].iter().collect();
                i = j + 1;
                out.push(Spanned {
                    token: Token::Str(text),
                    at,
                });
                continue;
            }
            '°' => Token::Degree,
            '+' => Token::Plus,
            // An ASCII hyphen, or the minus sign a copied formula brings.
            '-' | '\u{2212}' => Token::Minus,
            '*' | '×' | '⊗' | '·' => Token::Star,
            '/' => Token::Slash,
            '^' => Token::Caret,
            '(' | '（' => Token::Open,
            ')' | '）' => Token::Close,
            ',' | '，' => Token::Comma,
            '.' => Token::Dot,
            '=' => Token::Equals,
            other => {
                return Err(Problem::Unexpected {
                    found: other.to_string(),
                    at,
                });
            }
        };
        out.push(Spanned { token, at });
        i += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<Token> {
        tokens(text).unwrap().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn a_row_reads_as_names_numbers_and_signs() {
        assert_eq!(
            kinds("q = euler(30°, -1.5e-1, yaw)"),
            vec![
                Token::Ident("q".into()),
                Token::Equals,
                Token::Ident("euler".into()),
                Token::Open,
                Token::Number(30.0),
                Token::Degree,
                Token::Comma,
                Token::Minus,
                Token::Number(0.15),
                Token::Comma,
                Token::Ident("yaw".into()),
                Token::Close,
            ]
        );
    }

    /// What a Chinese keyboard and a copied formula bring: full-width
    /// brackets and commas, a real minus sign, `×` and `⊗`.
    #[test]
    fn full_width_punctuation_and_typeset_signs_read_as_their_ascii() {
        assert_eq!(
            kinds("a × b ⊗ c（1，2）− 3"),
            vec![
                Token::Ident("a".into()),
                Token::Star,
                Token::Ident("b".into()),
                Token::Star,
                Token::Ident("c".into()),
                Token::Open,
                Token::Number(1.0),
                Token::Comma,
                Token::Number(2.0),
                Token::Close,
                Token::Minus,
                Token::Number(3.0),
            ]
        );
    }

    #[test]
    fn a_remark_ends_the_row_and_a_string_is_quoted() {
        assert_eq!(
            kinds(r#"tel("gyro_x") # the firmware's own"#),
            vec![
                Token::Ident("tel".into()),
                Token::Open,
                Token::Str("gyro_x".into()),
                Token::Close,
            ]
        );
        assert!(kinds("# nothing but a remark").is_empty());
        assert_eq!(tokens(r#"tel("open"#), Err(Problem::Unclosed { at: 4 }));
    }

    #[test]
    fn a_number_is_a_number_or_it_is_named() {
        assert_eq!(kinds(".5"), vec![Token::Number(0.5)]);
        assert_eq!(
            kinds("q.w"),
            vec![
                Token::Ident("q".into()),
                Token::Dot,
                Token::Ident("w".into())
            ]
        );
        assert_eq!(
            tokens("1.2.3"),
            Err(Problem::BadNumber {
                text: "1.2.3".into()
            })
        );
        assert_eq!(
            tokens("a $ b"),
            Err(Problem::Unexpected {
                found: "$".into(),
                at: 2
            })
        );
    }
}
