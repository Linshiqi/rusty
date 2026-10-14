//! Markdown read into blocks the window draws: headings, paragraphs and
//! list items as Slint's styled text (emphasis, code, links, strikethrough
//! — what `StyledText` takes), code blocks as lines for the editor's own
//! painting, quotes, rules and tables.
//!
//! The assistant's answers and a `.md` file opened as a page both go
//! through it. `StyledText` takes a CommonMark subset and refuses headings,
//! tables and quotes, so the block structure is read here and only each
//! block's inline text is handed to it — written back as Markdown, with
//! every character it would read as markup escaped.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// One block of a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// A paragraph, or the paragraph of a list item: inline Markdown for
    /// `StyledText`, how deep in quotes it is, and its list marker and
    /// nesting when it is an item.
    Text {
        inline: String,
        quote: u32,
        item: Option<Item>,
    },
    Heading {
        level: u32,
        inline: String,
    },
    /// A fenced or indented code block: its language and its lines.
    Code {
        lang: String,
        lines: Vec<String>,
    },
    Rule,
    /// A table: each row's cells as inline Markdown, the first row the header.
    Table {
        rows: Vec<Vec<String>>,
    },
    /// A picture: where it is, and the words that stand in for it.
    Image {
        src: String,
        alt: String,
    },
}

/// A list item's marker and depth (0 for a top-level list).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub marker: String,
    pub depth: u32,
}

/// The blocks of a document.
pub fn blocks(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;
    let mut reader = Reader::default();
    for event in Parser::new_ext(text, options) {
        reader.take(event);
    }
    reader.flush();
    reader.out
}

/// Where a list is: its next number, or `None` for bullets.
struct List {
    next: Option<u64>,
}

#[derive(Default)]
struct Reader {
    out: Vec<Block>,
    inline: String,
    quote: u32,
    lists: Vec<List>,
    /// The marker of the item whose first paragraph has not been written.
    item: Option<Item>,
    heading: Option<u32>,
    code: Option<(String, String)>,
    /// Inside a link: where it goes, written when it closes.
    links: Vec<String>,
    table: Option<Vec<Vec<String>>>,
    /// Inside an image: its address and the alt text gathered so far.
    image: Option<(String, String)>,
}

impl Reader {
    fn take(&mut self, event: Event) {
        if let Some((_, code)) = self.code.as_mut() {
            match event {
                Event::Text(text) => code.push_str(&text),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, code) = self.code.take().unwrap_or_default();
                    let mut lines: Vec<String> = code.split('\n').map(str::to_string).collect();
                    if lines.last().is_some_and(String::is_empty) {
                        lines.pop();
                    }
                    self.out.push(Block::Code { lang, lines });
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match self.image.as_mut() {
                Some((_, alt)) => alt.push_str(&text),
                None => self.inline.push_str(&escape(&text)),
            },
            Event::Code(code) => self.inline.push_str(&code_span(&code)),
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                self.inline.push_str(&code_span(&math))
            }
            Event::SoftBreak => self.inline.push(' '),
            Event::HardBreak => self.inline.push('\n'),
            Event::Rule => {
                self.flush();
                self.out.push(Block::Rule);
            }
            Event::TaskListMarker(done) => self.inline.push_str(if done { "☑ " } else { "☐ " }),
            Event::Html(html) | Event::InlineHtml(html) => {
                // Raw HTML is shown as text, never interpreted: what a page
                // is allowed to do is decided here, not by the document.
                let text = html.trim();
                if !text.is_empty() && !text.starts_with("<!--") {
                    self.inline.push_str(&escape(text));
                }
            }
            Event::FootnoteReference(name) => self.inline.push_str(&format!("[{}]", escape(&name))),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split([' ', ','])
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(List { next: start });
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1) as u32;
                let marker = match self.lists.last_mut() {
                    Some(List { next: Some(n) }) => {
                        let marker = format!("{n}.");
                        *n += 1;
                        marker
                    }
                    _ => if depth.is_multiple_of(2) {
                        "•"
                    } else {
                        "◦"
                    }
                    .to_string(),
                };
                self.item = Some(Item { marker, depth });
            }
            Tag::Emphasis => self.inline.push('*'),
            Tag::Strong => self.inline.push_str("**"),
            Tag::Strikethrough => self.inline.push_str("~~"),
            Tag::Link { dest_url, .. } => {
                self.inline.push('[');
                self.links.push(dest_url.to_string());
            }
            Tag::Image { dest_url, .. } => {
                // A picture is a block of its own: the text before it is a
                // paragraph, and the text after it another.
                self.flush();
                self.image = Some((dest_url.to_string(), String::new()));
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                let inline = std::mem::take(&mut self.inline);
                if let Some(level) = self.heading.take() {
                    self.out.push(Block::Heading {
                        level,
                        inline: inline.trim().to_string(),
                    });
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis => self.inline.push('*'),
            TagEnd::Strong => self.inline.push_str("**"),
            TagEnd::Strikethrough => self.inline.push_str("~~"),
            TagEnd::Link => {
                let url = self.links.pop().unwrap_or_default();
                self.inline.push_str(&format!(
                    "]({})",
                    url.replace(' ', "%20").replace(')', "%29")
                ));
            }
            TagEnd::Image => {
                if let Some((src, alt)) = self.image.take() {
                    self.out.push(Block::Image { src, alt });
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(row) = self.table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(cell.trim().to_string());
                }
            }
            TagEnd::Table => {
                if let Some(rows) = self.table.take() {
                    self.out.push(Block::Table { rows });
                }
            }
            _ => {}
        }
    }

    /// The text gathered so far, as a block — a paragraph, or the first
    /// paragraph of the item that opened it. An item's later paragraphs are
    /// indented under it with no marker of their own.
    fn flush(&mut self) {
        if self.table.is_some() {
            return;
        }
        let inline = std::mem::take(&mut self.inline);
        let inline = inline.trim();
        if inline.is_empty() {
            return;
        }
        let item = self.item.take().or_else(|| {
            (!self.lists.is_empty()).then(|| Item {
                marker: String::new(),
                depth: self.lists.len().saturating_sub(1) as u32,
            })
        });
        self.out.push(Block::Text {
            inline: inline.to_string(),
            quote: self.quote,
            item,
        });
    }
}

/// Text with every character `StyledText`'s reader would take for markup
/// escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\`*_[]<>~#|!".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A code span, its fence longer than any run of backticks inside it.
fn code_span(code: &str) -> String {
    let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

/// Inline Markdown as Slint's styled text — or the same words plain when
/// its reader refuses them, rather than nothing.
pub fn styled(inline: &str) -> slint::StyledText {
    slint::StyledText::from_markdown(inline)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(&unescape(inline)))
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structure `StyledText` cannot read is read here, and only each
    /// block's inline part is left to it.
    #[test]
    fn a_document_is_read_into_blocks() {
        let blocks = blocks(
            "# Title\n\nSome *words* and `code`.\n\n- one\n- two\n  1. deep\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                inline: "Title".into()
            }
        );
        assert_eq!(
            blocks[1],
            Block::Text {
                inline: "Some *words* and `code`.".into(),
                quote: 0,
                item: None
            }
        );
        let items: Vec<(String, u32)> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text {
                    item: Some(item), ..
                } => Some((item.marker.clone(), item.depth)),
                _ => None,
            })
            .collect();
        assert_eq!(items, [("•".into(), 0), ("•".into(), 0), ("1.".into(), 1)]);
        assert!(
            blocks
                .iter()
                .any(|b| matches!(b, Block::Text { quote: 1, .. }))
        );
        assert!(blocks.contains(&Block::Code {
            lang: "rust".into(),
            lines: vec!["fn main() {}".into()]
        }));
        assert!(blocks.contains(&Block::Rule));
        let pictured = super::blocks("before ![a board](img/board.png) after\n");
        assert_eq!(
            pictured[1],
            Block::Image {
                src: "img/board.png".into(),
                alt: "a board".into()
            }
        );
        assert!(blocks.contains(&Block::Table {
            rows: vec![vec!["a".into(), "b".into()], vec!["1".into(), "2".into()]]
        }));
    }

    /// A character that reads as markup in the text stays a character.
    #[test]
    fn markup_in_the_text_is_escaped() {
        let blocks = blocks("a_b * c [d] <e>\n");
        let Block::Text { inline, .. } = &blocks[0] else {
            panic!("a paragraph")
        };
        assert_eq!(inline, "a\\_b \\* c \\[d\\] \\<e\\>");
        assert_eq!(unescape(inline), "a_b * c [d] <e>");
    }

    /// A code span holding a backtick is fenced by more of them.
    #[test]
    fn a_code_span_outlasts_its_backticks() {
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }
}
