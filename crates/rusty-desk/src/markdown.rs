//! Markdown read into blocks the window draws: headings, paragraphs and
//! list items as Slint's styled text (emphasis, code, links, strikethrough
//! — what `StyledText` takes), code blocks as lines for the editor's own
//! painting, quotes, rules, tables, pictures and formulas.
//!
//! The assistant's answers and a `.md` file opened as a page both go
//! through it. `StyledText` takes a CommonMark subset and refuses headings,
//! tables and quotes, so the block structure is read here and only each
//! block's inline text is handed to it — written back as Markdown, with
//! every character it would read as markup escaped.
//!
//! A paragraph holding a formula is kept as its runs as well (`flow`): it
//! cannot be one `StyledText`, since a formula is a picture, so the page
//! breaks it into lines itself (`flow.rs`) and draws each formula with
//! RaTeX (`math.rs`). Raw HTML is read by a small tag reader (`html.rs`) and
//! never run: a `<figure>` is a figure, a `<span>` its text, a `<script>` is
//! named and not run, and a tag the page does not know stays the text it is.

pub mod flow;
pub mod html;
pub mod math;
/// Selecting on the page: places in its laid-out blocks, and what a drag
/// and a copy mean against them.
pub mod select;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use self::flow::{Run, Style};
use self::html::Token;

/// One block of a page.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// A paragraph, or the paragraph of a list item: inline Markdown for
    /// `StyledText`, how deep in quotes it is, its list marker and nesting
    /// when it is an item, whether it is a figure's caption, and its runs
    /// when it holds a formula.
    Text {
        inline: String,
        quote: u32,
        item: Option<Item>,
        caption: bool,
        flow: Vec<Run>,
    },
    Heading {
        level: u32,
        inline: String,
        flow: Vec<Run>,
    },
    /// A fenced or indented code block: its language and its lines.
    Code {
        lang: String,
        lines: Vec<String>,
    },
    Rule,
    /// A table: each row's cells, the first row the header.
    Table {
        rows: Vec<Vec<Cell>>,
    },
    /// A picture: where it is, the words that stand in for it, and whether
    /// it is a figure's.
    Image {
        src: String,
        alt: String,
        figure: bool,
    },
    /// A formula set apart from the text: `$$…$$`.
    Math {
        tex: String,
        quote: u32,
    },
    /// Markup the page will not run, by its tag.
    Withheld {
        tag: String,
    },
}

/// A table's cell: inline Markdown, and its runs when it holds a formula.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub inline: String,
    pub flow: Vec<Run>,
}

/// A list item's marker and depth (0 for a top-level list).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub marker: String,
    pub depth: u32,
}

const OPTIONS: Options = Options::ENABLE_TABLES
    .union(Options::ENABLE_STRIKETHROUGH)
    .union(Options::ENABLE_TASKLISTS)
    .union(Options::ENABLE_MATH);

/// The blocks of a document.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut reader = Reader::default();
    for event in Parser::new_ext(text, OPTIONS) {
        reader.take(event);
    }
    reader.flush();
    reader.out
}

/// The blocks of a doc comment as a language server's hover carries it,
/// read the way rustdoc reads one: a link is a link only when it goes to
/// a web address — rust-analyzer resolves an intra-doc link to one where it
/// can — and an intra-doc link left as a name, `` [`Vec`] `` or
/// `[crate::Gain]`, is its text, with neither brackets nor a target that
/// would go nowhere. A heading is bold, which is how the card sets it off.
pub fn doc_blocks(text: &str) -> Vec<Block> {
    let mut reader = Reader {
        doc_comment: true,
        ..Reader::default()
    };
    for event in Parser::new_with_broken_link_callback(text, OPTIONS, Some(doc_name)) {
        reader.take(event);
    }
    reader.flush();
    reader.out
}

/// An intra-doc link the server left as a name: a link to nowhere, which
/// `doc_blocks` reads as its text.
fn doc_name<'a>(
    link: pulldown_cmark::BrokenLink<'a>,
) -> Option<(pulldown_cmark::CowStr<'a>, pulldown_cmark::CowStr<'a>)> {
    is_doc_name(&link.reference).then(|| ("".into(), "".into()))
}

/// Whether the words in `[…]` with no target name an item, as an intra-doc
/// link does: code, or a path — not `[0]` after an array, nor a note in
/// brackets.
fn is_doc_name(reference: &str) -> bool {
    let reference = reference.trim();
    let code = reference.len() > 2 && reference.starts_with('`') && reference.ends_with('`');
    let path = reference.contains("::")
        && !reference.contains(char::is_whitespace)
        && reference
            .chars()
            .all(|c| c.is_alphanumeric() || "_:<>!@()".contains(c));
    code || path
}

/// A link a click may open: an address on the web.
fn is_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// Where a list is: its next number, or `None` for bullets.
struct List {
    next: Option<u64>,
}

#[derive(Default)]
struct Reader {
    out: Vec<Block>,
    /// The paragraph so far.
    runs: Vec<Run>,
    quote: u32,
    lists: Vec<List>,
    /// The marker of the item whose first paragraph has not been written.
    item: Option<Item>,
    heading: Option<u32>,
    code: Option<(String, String)>,
    /// The styles open: how deep in each, and the links, innermost last —
    /// `None` for one read as its text (`doc_comment`).
    strong: u32,
    emphasis: u32,
    strike: u32,
    links: Vec<Option<String>>,
    /// Read as a hover card's doc comment (`doc_blocks`): a link is drawn
    /// only when it goes to the web, and a heading is bold.
    doc_comment: bool,
    /// Inline tags styling as Markdown would: `<code>` and `<kbd>` open.
    html_code: u32,
    table: Option<Vec<Vec<Cell>>>,
    /// Inside an image: its address and the alt text gathered so far.
    image: Option<(String, String)>,
    /// Inside an HTML block: its text so far.
    html: Option<String>,
    /// Inside a `<figure>`, and inside its `<figcaption>`.
    figure: bool,
    caption: bool,
}

impl Reader {
    fn style(&self) -> Style {
        Style {
            strong: self.strong > 0,
            emphasis: self.emphasis > 0,
            strike: self.strike > 0,
            code: self.html_code > 0,
            link: self.links.last().cloned().flatten(),
        }
    }

    fn text(&mut self, text: &str) {
        let style = self.style();
        self.styled(text, style);
    }

    /// A link opened, to `url` — or, read as rustdoc reads one, its text
    /// when `url` is no address on the web.
    fn link(&mut self, url: &str) {
        let shown = !self.doc_comment || is_web(url);
        self.links.push(shown.then(|| url.to_string()));
    }

    fn styled(&mut self, text: &str, style: Style) {
        if let Some(Run::Text {
            text: last,
            style: before,
        }) = self.runs.last_mut()
            && *before == style
        {
            last.push_str(text);
            return;
        }
        self.runs.push(Run::Text {
            text: text.to_string(),
            style,
        });
    }

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
        if let Some(html) = self.html.as_mut() {
            match event {
                Event::Html(text) | Event::Text(text) => html.push_str(&text),
                Event::End(TagEnd::HtmlBlock) => {
                    let html = self.html.take().unwrap_or_default();
                    self.html_block(&html);
                }
                // Nothing else arrives inside an HTML block; were it to, the
                // block ends there rather than swallow the rest.
                other => {
                    let html = self.html.take().unwrap_or_default();
                    self.html_block(&html);
                    self.take(other);
                }
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match self.image.as_mut() {
                Some((_, alt)) => alt.push_str(&text),
                None => self.text(&text),
            },
            Event::Code(code) => {
                let style = Style {
                    code: true,
                    ..self.style()
                };
                self.styled(&code, style);
            }
            Event::InlineMath(math) => self.runs.push(Run::Math(math.to_string())),
            Event::DisplayMath(math) => {
                // A formula set apart is a block of its own: the text before
                // it is a paragraph, and the text after it another.
                self.flush();
                self.out.push(Block::Math {
                    tex: math.to_string(),
                    quote: self.quote,
                });
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.runs.push(Run::Break),
            Event::Rule => {
                self.flush();
                self.out.push(Block::Rule);
            }
            Event::TaskListMarker(done) => self.text(if done { "☑ " } else { "☐ " }),
            Event::Html(html) => self.html_block(&html),
            Event::InlineHtml(html) => self.inline_html(&html),
            Event::FootnoteReference(name) => self.text(&format!("[{name}]")),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush();
                // A card's heading is set off by its weight, VS Code's hover,
                // not by a size a narrow card has no room for.
                if self.doc_comment {
                    self.strong += 1;
                }
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
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
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
            Tag::Emphasis => self.emphasis += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.link(&dest_url),
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
            Tag::TableCell => self.runs.clear(),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush(),
            TagEnd::Heading(_) => {
                if self.doc_comment {
                    self.strong = self.strong.saturating_sub(1);
                }
                let runs = std::mem::take(&mut self.runs);
                if let Some(level) = self.heading.take() {
                    let (inline, flow) = written(runs);
                    self.out.push(Block::Heading {
                        level,
                        inline: inline.trim().to_string(),
                        flow,
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
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image => {
                if let Some((src, alt)) = self.image.take() {
                    self.out.push(Block::Image {
                        src,
                        alt,
                        figure: self.figure,
                    });
                }
            }
            TagEnd::TableCell => {
                let runs = std::mem::take(&mut self.runs);
                let (inline, flow) = written(runs);
                if let Some(row) = self.table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(Cell { inline, flow });
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

    /// An HTML block, read: a figure and its picture and caption are what
    /// they say; what only arranges or styles shows its text; a script, a
    /// style or a frame is named and not run.
    fn html_block(&mut self, html: &str) {
        let mut withheld: Option<String> = None;
        for token in html::tokens(html) {
            if let Some(tag) = &withheld {
                if token == Token::Close(tag.clone()) {
                    withheld = None;
                }
                continue;
            }
            match token {
                Token::Open { name, attrs, void } => match name.as_str() {
                    "img" => {
                        self.flush();
                        self.out.push(Block::Image {
                            src: html::attr(&attrs, "src").unwrap_or_default().to_string(),
                            alt: html::attr(&attrs, "alt").unwrap_or_default().to_string(),
                            figure: self.figure,
                        });
                    }
                    "figure" => {
                        self.flush();
                        self.figure = true;
                    }
                    "figcaption" => {
                        self.flush();
                        self.caption = true;
                    }
                    "br" => self.runs.push(Run::Break),
                    name if withholds(name) => {
                        self.flush();
                        self.out.push(Block::Withheld {
                            tag: name.to_string(),
                        });
                        if !void {
                            withheld = Some(name.to_string());
                        }
                    }
                    "p" | "div" | "section" | "details" | "summary" | "center" | "hr" => {
                        self.flush()
                    }
                    _ => {}
                },
                Token::Close(name) => match name.as_str() {
                    "figcaption" => {
                        self.flush();
                        self.caption = false;
                    }
                    "figure" => {
                        self.flush();
                        self.figure = false;
                        self.caption = false;
                    }
                    "p" | "div" | "section" | "details" | "summary" | "center" => self.flush(),
                    _ => {}
                },
                Token::Text(text) => {
                    if !text.trim().is_empty() {
                        self.text(text.trim());
                    }
                }
                Token::Comment => {}
            }
        }
        self.flush();
    }

    /// A tag inside a line. The ones Markdown has a word for style as that
    /// word does; one that only styles shows its text; a picture is a block;
    /// anything else stays the text it is.
    fn inline_html(&mut self, html: &str) {
        let tokens = html::tokens(html);
        let [token] = tokens.as_slice() else {
            self.text(html);
            return;
        };
        match token {
            Token::Open { name, attrs, .. } => match name.as_str() {
                "br" => self.runs.push(Run::Break),
                "b" | "strong" => self.strong += 1,
                "i" | "em" => self.emphasis += 1,
                "s" | "del" | "strike" => self.strike += 1,
                "code" | "kbd" | "samp" => self.html_code += 1,
                "a" => self.link(html::attr(attrs, "href").unwrap_or_default()),
                "img" => {
                    self.flush();
                    self.out.push(Block::Image {
                        src: html::attr(attrs, "src").unwrap_or_default().to_string(),
                        alt: html::attr(attrs, "alt").unwrap_or_default().to_string(),
                        figure: self.figure,
                    });
                }
                "span" | "sub" | "sup" | "small" | "mark" | "u" | "abbr" | "font" | "wbr" => {}
                _ => self.text(html),
            },
            Token::Close(name) => match name.as_str() {
                "b" | "strong" => self.strong = self.strong.saturating_sub(1),
                "i" | "em" => self.emphasis = self.emphasis.saturating_sub(1),
                "s" | "del" | "strike" => self.strike = self.strike.saturating_sub(1),
                "code" | "kbd" | "samp" => self.html_code = self.html_code.saturating_sub(1),
                "a" => {
                    self.links.pop();
                }
                "span" | "sub" | "sup" | "small" | "mark" | "u" | "abbr" | "font" => {}
                _ => self.text(html),
            },
            Token::Comment => {}
            Token::Text(text) => self.text(text),
        }
    }

    /// The text gathered so far, as a block — a paragraph, or the first
    /// paragraph of the item that opened it. An item's later paragraphs are
    /// indented under it with no marker of their own.
    fn flush(&mut self) {
        if self.table.is_some() || self.heading.is_some() {
            return;
        }
        let runs = std::mem::take(&mut self.runs);
        let (inline, flow) = written(runs);
        if inline.trim().is_empty() {
            return;
        }
        let item = self.item.take().or_else(|| {
            (!self.lists.is_empty()).then(|| Item {
                marker: String::new(),
                depth: self.lists.len().saturating_sub(1) as u32,
            })
        });
        self.out.push(Block::Text {
            inline: inline.trim().to_string(),
            quote: self.quote,
            item,
            caption: self.caption,
            flow,
        });
    }
}

/// Tags whose content is to run or to load something, never to be read.
fn withholds(name: &str) -> bool {
    matches!(
        name,
        "script" | "style" | "iframe" | "frame" | "frameset" | "object" | "embed" | "applet"
    )
}

/// Runs as the Markdown `StyledText` reads — a formula as its source in a
/// code span, which is what a reader without the formula's picture sees —
/// and the runs themselves, which the page lays out into lines itself so
/// that it knows where every character stands (`flow`): a selection is
/// measured against that, and a formula drawn at it.
fn written(runs: Vec<Run>) -> (String, Vec<Run>) {
    let mut inline = String::new();
    for run in &runs {
        match run {
            Run::Text { text, style } => inline.push_str(&flow::marked(text, style)),
            Run::Math(tex) => inline.push_str(&code_span(tex.trim())),
            Run::Break => inline.push('\n'),
        }
    }
    (inline, runs)
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

/// A link's target as Markdown takes it inside `(…)`.
fn link_target(url: &str) -> String {
    url.replace(' ', "%20").replace(')', "%29")
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

    fn text_of(block: &Block) -> &str {
        match block {
            Block::Text { inline, .. } | Block::Heading { inline, .. } => inline,
            other => panic!("not text: {other:?}"),
        }
    }

    /// The structure `StyledText` cannot read is read here, and only each
    /// block's inline part is left to it.
    #[test]
    fn a_document_is_read_into_blocks() {
        let blocks = blocks(
            "# Title\n\nSome *words* and `code`.\n\n- one\n- two\n  1. deep\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
        );
        let plain = |text: &str| Run::Text {
            text: text.into(),
            style: Style::default(),
        };
        assert_eq!(
            blocks[0],
            Block::Heading {
                level: 1,
                inline: "Title".into(),
                flow: vec![plain("Title")]
            }
        );
        let Block::Text {
            inline,
            quote: 0,
            item: None,
            caption: false,
            flow,
        } = &blocks[1]
        else {
            panic!("a paragraph: {:?}", blocks[1]);
        };
        assert_eq!(inline, "Some *words* and `code`.");
        assert_eq!(flow.len(), 5, "every run kept: {flow:?}");
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
                alt: "a board".into(),
                figure: false,
            }
        );
        let cell = |s: &str| Cell {
            inline: s.into(),
            flow: vec![plain(s)],
        };
        assert!(blocks.contains(&Block::Table {
            rows: vec![vec![cell("a"), cell("b")], vec![cell("1"), cell("2")]]
        }));
    }

    /// A character that reads as markup in the text stays a character.
    #[test]
    fn markup_in_the_text_is_escaped() {
        let blocks = blocks("a_b * c [d] <e>\n");
        let inline = text_of(&blocks[0]);
        assert_eq!(inline, "a\\_b \\* c \\[d\\] \\<e\\>");
        assert_eq!(unescape(inline), "a_b * c [d] <e>");
    }

    /// A code span holding a backtick is fenced by more of them.
    #[test]
    fn a_code_span_outlasts_its_backticks() {
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x`"), "`` `x` ``");
    }

    /// A paragraph with a formula keeps its runs for the page to set; its
    /// Markdown has the formula's source, which is what a view that cannot
    /// draw formulas shows.
    #[test]
    fn a_paragraph_with_a_formula_keeps_its_runs() {
        let read = blocks("角 $\\psi$ 是**偏航**。\n\nno formula here\n");
        let Block::Text { inline, flow, .. } = &read[0] else {
            panic!("a paragraph")
        };
        assert_eq!(inline, "角 `\\psi` 是**偏航**。");
        assert_eq!(flow.len(), 5);
        assert_eq!(flow[1], Run::Math("\\psi".into()));
        assert!(matches!(&flow[2], Run::Text { text, style } if text == " 是" && !style.strong));
        assert!(matches!(&flow[3], Run::Text { text, style } if text == "偏航" && style.strong));
        assert!(matches!(&flow[4], Run::Text { text, .. } if text == "。"));
        let Block::Text { flow, .. } = &read[1] else {
            panic!("a paragraph")
        };
        assert_eq!(
            flow,
            &[Run::Text {
                text: "no formula here".into(),
                style: Style::default()
            }],
            "every paragraph keeps its runs: the page lays them all out"
        );
    }

    /// `$$…$$` is a block of its own between the paragraphs round it.
    #[test]
    fn display_math_is_a_block() {
        let read = blocks("before\n$$\nF = ma\n$$\nafter\n");
        assert_eq!(text_of(&read[0]), "before");
        assert!(matches!(&read[1], Block::Math { tex, quote: 0 } if tex.trim() == "F = ma"));
        assert_eq!(text_of(&read[2]), "after");
    }

    /// A figure as the book writes it: an HTML block with the picture, a
    /// Markdown caption, and the HTML that closes both.
    #[test]
    fn a_figure_is_its_picture_and_its_caption() {
        let read = blocks(
            "<figure>\n<img src=\"figures/fig-04.svg\" alt=\"三次转动\">\n<figcaption>\n\n图 4-1　三次转动 $\\psi$。\n\n</figcaption>\n</figure>\n\nafter\n",
        );
        assert_eq!(
            read[0],
            Block::Image {
                src: "figures/fig-04.svg".into(),
                alt: "三次转动".into(),
                figure: true
            }
        );
        let Block::Text { caption, flow, .. } = &read[1] else {
            panic!("the caption")
        };
        assert!(*caption && !flow.is_empty());
        let Block::Text { caption, .. } = &read[2] else {
            panic!("the text after")
        };
        assert!(!caption, "the caption ends with the figure");
        assert_eq!(read.len(), 3, "no tag is left as text: {read:?}");
    }

    /// Inline HTML that only styles shows its text; a script is named and
    /// its body never shown; a tag the page does not know stays text.
    #[test]
    fn inline_html_is_read_not_run() {
        let read =
            blocks("| <span style=\"white-space:nowrap\">姿态估计</span> | x |\n|---|---|\n");
        let Block::Table { rows } = &read[0] else {
            panic!("a table")
        };
        assert_eq!(rows[0][0].inline, "姿态估计");
        let read = blocks("<script>\nalert(1)\n</script>\n\nok <b>bold</b> <blink>x</blink>\n");
        assert_eq!(
            read[0],
            Block::Withheld {
                tag: "script".into()
            }
        );
        assert_eq!(text_of(&read[1]), "ok **bold** \\<blink\\>x\\</blink\\>");
        assert_eq!(read.len(), 2, "{read:?}");
    }

    /// rust-analyzer's hover on `true`, the shape of every std doc: a link
    /// resolved to the web, a heading, a list with links in its items —
    /// read as blocks, where `StyledText` alone refused the whole of it and
    /// the card showed the source.
    #[test]
    fn a_doc_comment_is_its_blocks_and_its_web_links() {
        let doc = "The boolean type, [`bool`](https://doc.rust-lang.org/nightly/std/primitive.bool.html).\n\n\
                   ## Control structures\n\n\
                   **true** is the condition of:\n\n\
                   * The condition in an [`if`](https://doc.rust-lang.org/nightly/std/keyword.if.html) expression\n\
                   * The condition of a [`while`] loop\n";
        let read = doc_blocks(doc);
        assert_eq!(
            text_of(&read[0]),
            "The boolean type, [`bool`](https://doc.rust-lang.org/nightly/std/primitive.bool.html)."
        );
        assert!(
            matches!(&read[1], Block::Heading { level: 2, inline, .. } if inline == "**Control structures**")
        );
        assert_eq!(text_of(&read[2]), "**true** is the condition of:");
        let items: Vec<&str> = read[3..].iter().map(text_of).collect();
        assert_eq!(
            items,
            [
                "The condition in an [`if`](https://doc.rust-lang.org/nightly/std/keyword.if.html) expression",
                // Left as a name by the server: its text, no brackets.
                "The condition of a `while` loop",
            ]
        );
        assert!(
            read[3..]
                .iter()
                .all(|b| matches!(b, Block::Text { item: Some(_), .. }))
        );
    }

    /// A link to no web address is its text in a doc comment — a path, an
    /// anchor — and stays a link on a page; brackets that name nothing stay
    /// brackets.
    #[test]
    fn a_doc_comment_links_only_to_the_web() {
        let doc = "See [the gain](crate::Gain), [above](#tuning), [`Vec::new`] and v[0] [sic].\n";
        assert_eq!(
            text_of(&doc_blocks(doc)[0]),
            "See the gain, above, `Vec::new` and v\\[0\\] \\[sic\\]."
        );
        assert_eq!(
            text_of(&blocks(doc)[0]),
            "See [the gain](crate::Gain), [above](#tuning), \\[`Vec::new`\\] and v\\[0\\] \\[sic\\]."
        );
    }

    /// A picture inside a link is still a picture.
    #[test]
    fn a_linked_picture_is_a_picture() {
        let read =
            blocks("<a href=\"figures/e1.png\"><img src=\"figures/e1.png\" alt=\"原理图\"></a>\n");
        assert!(
            read.iter()
                .any(|b| matches!(b, Block::Image { src, .. } if src == "figures/e1.png"))
        );
    }
}
