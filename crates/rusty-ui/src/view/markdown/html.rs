//! A small reading of the HTML a Markdown document carries.
//!
//! A book chapter puts its figures in `<figure>` and its keyboard keys in
//! `<kbd>`, and `pulldown-cmark` hands both over as raw markup. The page can
//! show that markup, inject it, or read it. Injecting is out — a document is
//! not markup this window should execute, and an `<a href>` in it would
//! navigate the workbench away with no way back — so this reads it: a
//! tolerant tag reader that turns the markup into a tree the renderer walks
//! with an allowlist. What the renderer does not know it draws as the
//! element's children; a script or a frame is named and not run.
//!
//! Not an HTML5 parser, on purpose. html5ever is a megabyte of wasm to read
//! `<figure><img><figcaption>`, and a parser that accepts everything is the
//! first half of a renderer that renders everything. This one knows tags,
//! attributes, comments, entities and void elements, closes what was left
//! open the way a browser would, and nothing else.

/// One node of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlNode {
    Element {
        /// Lower-cased.
        tag: String,
        /// Lower-cased names; values with their entities decoded.
        attrs: Vec<(String, String)>,
        children: Vec<HtmlNode>,
    },
    Text(String),
}

/// One tag as written, for the inline reader that meets tags one at a time
/// with Markdown between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagToken {
    Open {
        tag: String,
        attrs: Vec<(String, String)>,
        /// `<br>`, `<img>`, or anything written `<x />`: nothing to close.
        void: bool,
    },
    Close {
        tag: String,
    },
}

/// Tags that never hold content, so no closer is awaited for them.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Tags whose content is not markup: read to the closer and dropped.
const RAW: &[&str] = &["script", "style"];

/// Read a block of markup into a tree.
pub fn parse(html: &str) -> Vec<HtmlNode> {
    let mut root = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut rest = html;

    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            push_text(&mut stack, &mut root, rest);
            break;
        };
        if lt > 0 {
            push_text(&mut stack, &mut root, &rest[..lt]);
        }
        rest = &rest[lt..];

        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |at| &after[at + 3..]);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            rest = rest.find('>').map_or("", |at| &rest[at + 1..]);
            continue;
        }
        let Some((token, consumed)) = read_tag(rest) else {
            // A `<` that opens no tag — `a < b` in prose — is text.
            push_text(&mut stack, &mut root, "<");
            rest = &rest[1..];
            continue;
        };
        rest = &rest[consumed..];
        match token {
            TagToken::Close { tag } => close(&mut stack, &mut root, &tag),
            TagToken::Open { tag, attrs, .. } if RAW.contains(&tag.as_str()) => {
                // To the closer, keeping nothing: the element reaches the
                // renderer with no children, which is what "not run" shows as.
                let closer = format!("</{tag}");
                rest = match rest.to_ascii_lowercase().find(&closer) {
                    Some(at) => rest[at..].find('>').map_or("", |gt| &rest[at + gt + 1..]),
                    None => "",
                };
                attach(&mut stack, &mut root, element(tag, attrs, Vec::new()));
            }
            TagToken::Open { tag, attrs, void } => {
                if void {
                    attach(&mut stack, &mut root, element(tag, attrs, Vec::new()));
                } else {
                    stack.push(Frame {
                        tag,
                        attrs,
                        children: Vec::new(),
                    });
                }
            }
        }
    }
    // Whatever is still open closes at the end, as a browser closes it.
    while let Some(frame) = stack.pop() {
        attach(&mut stack, &mut root, frame.into_element());
    }
    root
}

/// One tag on its own — `<kbd class="x">`, `</kbd>`, `<br/>` — as the inline
/// reader receives them. `None` for anything that is not a tag: a comment,
/// a bare `<`.
pub fn tag_of(raw: &str) -> Option<TagToken> {
    let raw = raw.trim();
    read_tag(raw).map(|(token, _)| token)
}

/// An open element being read.
struct Frame {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<HtmlNode>,
}

impl Frame {
    fn into_element(self) -> HtmlNode {
        element(self.tag, self.attrs, self.children)
    }
}

fn element(tag: String, attrs: Vec<(String, String)>, children: Vec<HtmlNode>) -> HtmlNode {
    HtmlNode::Element {
        tag,
        attrs,
        children,
    }
}

/// Put a finished node under the innermost open element, or at the top.
fn attach(stack: &mut [Frame], root: &mut Vec<HtmlNode>, node: HtmlNode) {
    match stack.last_mut() {
        Some(frame) => frame.children.push(node),
        None => root.push(node),
    }
}

/// Text between tags, entities decoded, runs merged.
///
/// Whitespace alone between tags is layout, not content: with a newline in
/// it, it is the indentation of a block and is dropped; without one it is the
/// space between two inline elements and is kept as one space. Dropping both
/// glued `<b>a</b> <i>b</i>` into `ab`; keeping both put blank lines around
/// every figure.
fn push_text(stack: &mut [Frame], root: &mut Vec<HtmlNode>, raw: &str) {
    let text = if raw.trim().is_empty() {
        if raw.contains('\n') {
            return;
        }
        " ".to_string()
    } else {
        decode_entities(raw)
    };
    let siblings = match stack.last_mut() {
        Some(frame) => &mut frame.children,
        None => root,
    };
    match siblings.last_mut() {
        Some(HtmlNode::Text(existing)) => existing.push_str(&text),
        _ => siblings.push(HtmlNode::Text(text)),
    }
}

/// A closer: pops to the matching element, closing anything left open inside
/// it on the way, as a browser does. One that matches nothing is ignored.
fn close(stack: &mut Vec<Frame>, root: &mut Vec<HtmlNode>, tag: &str) {
    let Some(at) = stack.iter().rposition(|frame| frame.tag == tag) else {
        return;
    };
    while stack.len() > at {
        let frame = stack.pop().expect("a frame at or above `at`");
        attach(stack, root, frame.into_element());
    }
}

/// Read one tag from the front of `s`, which starts with `<`. Answers the
/// token and how many bytes it took; `None` when the `<` opens no tag.
fn read_tag(s: &str) -> Option<(TagToken, usize)> {
    let body = s.strip_prefix('<')?;
    let (closing, body) = match body.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, body),
    };
    let name_len = body
        .char_indices()
        .take_while(|(index, ch)| {
            if *index == 0 {
                ch.is_ascii_alphabetic()
            } else {
                ch.is_ascii_alphanumeric() || *ch == '-' || *ch == ':'
            }
        })
        .count();
    if name_len == 0 {
        return None;
    }
    let tag = body[..name_len].to_ascii_lowercase();
    let mut cursor = &body[name_len..];

    if closing {
        let end = cursor.find('>').map_or(cursor.len(), |at| at + 1);
        let consumed = s.len() - cursor.len() + end;
        return Some((TagToken::Close { tag }, consumed));
    }

    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        cursor = cursor.trim_start();
        if cursor.is_empty() {
            break;
        }
        if let Some(rest) = cursor.strip_prefix("/>") {
            self_closing = true;
            cursor = rest;
            break;
        }
        if let Some(rest) = cursor.strip_prefix('>') {
            cursor = rest;
            break;
        }
        if let Some(rest) = cursor.strip_prefix('/') {
            cursor = rest;
            continue;
        }
        // An attribute name runs to whitespace, `=`, `>` or `/`.
        let name_end = cursor
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '=' | '>' | '/'))
            .unwrap_or(cursor.len());
        let name = cursor[..name_end].to_ascii_lowercase();
        cursor = cursor[name_end..].trim_start();
        let value = match cursor.strip_prefix('=') {
            Some(rest) => {
                let rest = rest.trim_start();
                let (value, after) = match rest.chars().next() {
                    Some(quote @ ('"' | '\'')) => {
                        let inner = &rest[1..];
                        // An unclosed quote runs to the end of the tag.
                        let end = inner
                            .find(quote)
                            .unwrap_or(inner.find('>').unwrap_or(inner.len()));
                        let skip = if inner[end..].starts_with(quote) {
                            1
                        } else {
                            0
                        };
                        (&inner[..end], &inner[end + skip..])
                    }
                    _ => {
                        let end = rest
                            .find(|ch: char| ch.is_whitespace() || ch == '>')
                            .unwrap_or(rest.len());
                        (&rest[..end], &rest[end..])
                    }
                };
                cursor = after;
                decode_entities(value)
            }
            None => String::new(),
        };
        if !name.is_empty() {
            attrs.push((name, value));
        }
    }
    let void = self_closing || VOID.contains(&tag.as_str());
    let consumed = s.len() - cursor.len();
    Some((TagToken::Open { tag, attrs, void }, consumed))
}

/// The entities a document actually writes, plus numeric ones. Anything else
/// stays as typed — `&foo;` shown as `&foo;` is a visible mistake, where a
/// guessed character would be an invisible one.
fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[1..].find(';').filter(|at| *at <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..1 + semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            "ensp" => Some('\u{2002}'),
            "emsp" => Some('\u{2003}'),
            "thinsp" => Some('\u{2009}'),
            "ndash" => Some('–'),
            "mdash" => Some('—'),
            "hellip" => Some('…'),
            "times" => Some('×'),
            "middot" => Some('·'),
            "deg" => Some('°'),
            "copy" => Some('©'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            _ => name
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &rest[2 + semi..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> HtmlNode {
        HtmlNode::Text(s.to_string())
    }

    /// The shape every book chapter has: a figure holding an image and a
    /// caption, indented over three lines. The indentation is not content.
    #[test]
    fn a_figure_reads_as_a_tree_without_its_indentation() {
        let nodes = parse(
            "<figure>\n<img src=\"figures/fig.svg\" alt=\"Plan view\">\n\
             <figcaption>Figure 1-1&emsp;Plan view.</figcaption>\n</figure>\n",
        );
        assert_eq!(
            nodes,
            vec![HtmlNode::Element {
                tag: "figure".into(),
                attrs: vec![],
                children: vec![
                    HtmlNode::Element {
                        tag: "img".into(),
                        attrs: vec![
                            ("src".into(), "figures/fig.svg".into()),
                            ("alt".into(), "Plan view".into()),
                        ],
                        children: vec![],
                    },
                    HtmlNode::Element {
                        tag: "figcaption".into(),
                        attrs: vec![],
                        children: vec![text("Figure 1-1\u{2003}Plan view.")],
                    },
                ],
            }]
        );
    }

    /// Attributes as people write them: quoted either way, bare, boolean,
    /// mixed case, with an entity inside; and `<X/>` is void whatever X is.
    #[test]
    fn attributes_are_read_however_they_are_quoted() {
        let nodes = parse("<IMG SRC='a&amp;b.png' width=120 hidden alt=\"x > y\"/>after");
        let HtmlNode::Element {
            tag,
            attrs,
            children,
        } = &nodes[0]
        else {
            panic!("not an element: {nodes:?}");
        };
        assert_eq!(tag, "img");
        assert_eq!(
            attrs,
            &vec![
                ("src".to_string(), "a&b.png".to_string()),
                ("width".to_string(), "120".to_string()),
                ("hidden".to_string(), String::new()),
                ("alt".to_string(), "x > y".to_string()),
            ]
        );
        assert!(children.is_empty());
        assert_eq!(nodes[1], text("after"));
    }

    /// What is left open closes at the end; a closer for an element inside
    /// closes the inner one only; a closer that matches nothing is ignored.
    #[test]
    fn unclosed_and_mismatched_tags_close_the_way_a_browser_closes_them() {
        assert_eq!(
            parse("<b>bold"),
            vec![HtmlNode::Element {
                tag: "b".into(),
                attrs: vec![],
                children: vec![text("bold")],
            }]
        );
        assert_eq!(
            parse("<i>x</b>y</i>"),
            vec![HtmlNode::Element {
                tag: "i".into(),
                attrs: vec![],
                children: vec![text("xy")],
            }]
        );
        // `</div>` closes the `<p>` still open inside it, as a browser would.
        let nodes = parse("<div><p>one</div>two");
        assert_eq!(nodes.len(), 2, "{nodes:?}");
        assert_eq!(nodes[1], text("two"));
    }

    /// A script is named and empty, a comment is nothing, and a `<` that
    /// opens no tag is the character it is.
    #[test]
    fn scripts_are_kept_empty_comments_vanish_and_a_bare_lt_is_text() {
        let nodes = parse("a <!-- note --> < b<script>alert(1)</script>c");
        assert_eq!(
            nodes,
            vec![
                text("a  < b"),
                HtmlNode::Element {
                    tag: "script".into(),
                    attrs: vec![],
                    children: vec![],
                },
                text("c"),
            ]
        );
    }

    /// Whitespace between two inline elements is a space; whitespace with a
    /// newline in it is a block's indentation and is nothing.
    #[test]
    fn whitespace_between_tags_is_a_space_inline_and_nothing_across_lines() {
        assert_eq!(
            parse("<b>a</b> <i>b</i>"),
            vec![
                HtmlNode::Element {
                    tag: "b".into(),
                    attrs: vec![],
                    children: vec![text("a")],
                },
                text(" "),
                HtmlNode::Element {
                    tag: "i".into(),
                    attrs: vec![],
                    children: vec![text("b")],
                },
            ]
        );
        assert_eq!(parse("<br>\n  <br>").len(), 2);
    }

    #[test]
    fn entities_decode_and_unknown_ones_stay_as_typed() {
        assert_eq!(
            decode_entities("1&emsp;&amp; 2 &#x41;&#66; &foo; & bare"),
            "1\u{2003}& 2 AB &foo; & bare"
        );
    }

    /// The inline reader's view of a tag on its own.
    #[test]
    fn a_lone_tag_is_read_as_the_token_it_is() {
        assert_eq!(
            tag_of("<kbd class=\"k\">"),
            Some(TagToken::Open {
                tag: "kbd".into(),
                attrs: vec![("class".into(), "k".into())],
                void: false,
            })
        );
        assert_eq!(
            tag_of("</kbd>"),
            Some(TagToken::Close { tag: "kbd".into() })
        );
        assert!(matches!(
            tag_of("<br>"),
            Some(TagToken::Open { void: true, .. })
        ));
        assert!(matches!(
            tag_of("<br />"),
            Some(TagToken::Open { void: true, .. })
        ));
        assert_eq!(tag_of("<!-- x -->"), None);
    }
}
