//! Raw HTML in a Markdown file, read rather than run: a small tag reader for
//! what books put in their chapters — a `<figure>` round an `<img>` and its
//! `<figcaption>`, a `<span>` that only styles, a `<br>`, a link. The page
//! decides what each tag is allowed to be (`markdown.rs`); nothing here is
//! interpreted, and a tag it cannot read is text.

/// A piece of HTML: a tag opened (`void` when it closes itself), a tag
/// closed, text between tags, or a comment.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Open {
        name: String,
        attrs: Vec<(String, String)>,
        void: bool,
    },
    Close(String),
    Text(String),
    Comment,
}

/// The tags that never have a closing tag.
fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

/// HTML as tokens. Something that starts like a tag and is not one is text,
/// so `a < b` survives.
pub fn tokens(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        text.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(body) = tail.strip_prefix("<!--") {
            let end = body.find("-->").map_or(body.len(), |e| e + 3);
            flush(&mut text, &mut out);
            out.push(Token::Comment);
            rest = &body[end..];
            continue;
        }
        match tag(tail) {
            Some((token, used)) => {
                flush(&mut text, &mut out);
                out.push(token);
                rest = &tail[used..];
            }
            None => {
                text.push('<');
                rest = &tail[1..];
            }
        }
    }
    text.push_str(rest);
    flush(&mut text, &mut out);
    out
}

fn flush(text: &mut String, out: &mut Vec<Token>) {
    if !text.is_empty() {
        out.push(Token::Text(decode(text)));
        text.clear();
    }
}

/// One tag at the start of `s`, and how many bytes it took.
fn tag(s: &str) -> Option<(Token, usize)> {
    let end = s.find('>')?;
    let inner = &s[1..end];
    let used = end + 1;
    if let Some(name) = inner.strip_prefix('/') {
        let name = name.trim();
        if !is_name(name) {
            return None;
        }
        return Some((Token::Close(name.to_ascii_lowercase()), used));
    }
    let (inner, self_closed) = match inner.strip_suffix('/') {
        Some(inner) => (inner, true),
        None => (inner, false),
    };
    let name_end = inner
        .find(|c: char| c.is_whitespace())
        .unwrap_or(inner.len());
    let name = &inner[..name_end];
    if !is_name(name) {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let attrs = attributes(&inner[name_end..]);
    let void = self_closed || is_void(&name);
    Some((Token::Open { name, attrs, void }, used))
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// `name="value" name='value' name=value name`, in order.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = s.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        rest = rest[name_end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (raw, left) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &after[1..];
                    let close = body.find(q).unwrap_or(body.len());
                    (&body[..close], body.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let close = after
                        .find(|c: char| c.is_whitespace())
                        .unwrap_or(after.len());
                    (&after[..close], &after[close..])
                }
            };
            value = decode(raw);
            rest = left.trim_start();
        }
        if name.is_empty() {
            break;
        }
        out.push((name, value));
    }
    out
}

/// An attribute's value, by name.
pub fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// Character references made the characters they stand for; an unknown one
/// stays as written.
fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let entity = tail[1..]
            .find(';')
            .filter(|&e| e <= 10)
            .map(|e| &tail[1..e + 1]);
        let ch = entity.and_then(|name| match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => {
                let n = name.strip_prefix('#')?;
                let code = match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => n.parse().ok()?,
                };
                char::from_u32(code)
            }
        });
        match (ch, entity) {
            (Some(c), Some(name)) => {
                out.push(c);
                rest = &tail[name.len() + 2..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A book's figure: the picture, its words, and the caption's tags, in
    /// order — the caption's own text is Markdown between two HTML blocks.
    #[test]
    fn a_figure_is_read_as_its_tags() {
        let read = tokens(
            "<figure>\n<img src=\"figures/fig-04.svg\" alt=\"三次转动 &amp; more\">\n<figcaption>\n",
        );
        let tags: Vec<&Token> = read
            .iter()
            .filter(|t| !matches!(t, Token::Text(s) if s.trim().is_empty()))
            .collect();
        assert_eq!(tags.len(), 3);
        let Token::Open { name, attrs, void } = tags[1] else {
            panic!("the picture")
        };
        assert_eq!((name.as_str(), *void), ("img", true));
        assert_eq!(attr(attrs, "src"), Some("figures/fig-04.svg"));
        assert_eq!(attr(attrs, "alt"), Some("三次转动 & more"));
        assert_eq!(
            tags[2],
            &Token::Open {
                name: "figcaption".into(),
                attrs: vec![],
                void: false
            }
        );
        assert_eq!(
            tokens("</figcaption>\n</figure>")[0],
            Token::Close("figcaption".into())
        );
    }

    /// What only looks like a tag is text, and a comment is nothing.
    #[test]
    fn what_is_not_a_tag_is_text() {
        assert_eq!(
            tokens("a < b and c<3"),
            [Token::Text("a < b and c<3".into())]
        );
        assert_eq!(
            tokens("<!-- note -->x"),
            [Token::Comment, Token::Text("x".into())]
        );
        let span = tokens("<span style=\"white-space:nowrap\">姿态估计</span>");
        assert_eq!(span[1], Token::Text("姿态估计".into()));
        assert_eq!(span[2], Token::Close("span".into()));
        assert!(
            matches!(&tokens("<br/>")[0], Token::Open { name, void: true, .. } if name == "br")
        );
    }

    /// Single quotes, no quotes and bare names all read; references in
    /// text are decoded and an unknown one is left alone.
    #[test]
    fn attributes_and_references_read_every_way_they_are_written() {
        let Token::Open { attrs, .. } = &tokens("<img src='a b.png' width=40 hidden>")[0] else {
            panic!("a tag")
        };
        assert_eq!(attr(attrs, "src"), Some("a b.png"));
        assert_eq!(attr(attrs, "width"), Some("40"));
        assert_eq!(attr(attrs, "hidden"), Some(""));
        assert_eq!(decode("&lt;&#x4e2d;&#25991;&bogus; &"), "<中文&bogus; &");
    }
}
