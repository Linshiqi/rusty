//! Markdown, rendered.
//!
//! Two callers with the same needs and different sizes: hover text and
//! assistant answers, which are a few paragraphs, and a project's own
//! documentation — a README, a `book/` — which is everything Markdown has.
//!
//! It was a hand-rolled block splitter for a while, on the reasoning that six
//! constructs do not need a parser. That was true of hover text and false of a
//! README the moment anyone opened one: heading levels were flattened to one
//! size, ordered lists rendered as bullets, nested lists came out flat, and
//! tables were paragraphs of pipes. So the parsing is `pulldown-cmark`'s now,
//! and what stays here is the two decisions that are ours.
//!
//! **Links do not navigate.** A WebView navigation would replace the workbench
//! with docs.rs and there is no back button — this is not a browser tab.
//! Clicking copies the URL; the tooltip shows where it points.
//!
//! **Images are not fetched.** A local path cannot be loaded into this
//! WebView, and fetching a remote one tells its host that somebody opened this
//! file. Both render as their alt text with the source in the tooltip, which
//! is the honest version of a broken-image icon.
//!
//! The parse is a pure `&str -> Vec<Node>`, so the shapes are tested without a
//! browser and the rendering below has no logic worth hiding.

use leptos::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::view::components::copy_to_clipboard;

/// One block-level element.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Node {
    /// 1–6, as written. A README's structure is its heading levels, and a
    /// renderer that draws them all the same size has thrown it away.
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Node>),
    List {
        /// The first number for an ordered list, `None` for bullets.
        start: Option<u64>,
        items: Vec<Vec<Node>>,
    },
    Table {
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
}

/// One span inside a block.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Inline {
    Text(String),
    Code(String),
    Strong(Vec<Inline>),
    Em(Vec<Inline>),
    Strike(Vec<Inline>),
    Link {
        text: Vec<Inline>,
        url: String,
    },
    /// Alt text and where the image would have come from. See the header.
    Image {
        alt: String,
        url: String,
    },
    Break,
}

/// Parse Markdown into blocks. Pure.
pub fn parse(text: &str) -> Vec<Node> {
    // The extensions a project's own documentation actually uses. Footnotes
    // and maths are left off: `pulldown-cmark` would emit them and there is
    // nothing here that renders them better than the source line.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut parser = Parser::new_ext(text, options).peekable();
    blocks_until(&mut parser, None)
}

type Events<'a> = std::iter::Peekable<Parser<'a>>;

/// Whether an event opens a block rather than sitting inside one.
fn opens_a_block(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Rule
            | Event::Start(
                Tag::Heading { .. }
                    | Tag::Paragraph
                    | Tag::CodeBlock(_)
                    | Tag::BlockQuote(_)
                    | Tag::List(_)
                    | Tag::Table(_)
                    | Tag::HtmlBlock
            )
    )
}

/// Blocks until `end` closes, or until the events run out.
fn blocks_until(events: &mut Events<'_>, end: Option<TagEnd>) -> Vec<Node> {
    let mut out = Vec::new();
    while let Some(event) = events.peek() {
        if let Event::End(tag) = event
            && Some(*tag) == end
        {
            events.next();
            break;
        }
        // **A tight list item has no `Paragraph` around its text.** CommonMark
        // calls a list tight when no item is separated by a blank line, and
        // `pulldown-cmark` then emits `Item → Text → End(Item)` with nothing
        // between. Reading only block openings here dropped every one of them:
        // the bullets and the numbers rendered, with no words beside any of
        // them.
        if !opens_a_block(event) {
            let text = inlines_while_inside(events);
            if !text.is_empty() {
                out.push(Node::Para(text));
            } else {
                // Nothing was read, so nothing advanced: the event under the
                // cursor is a closer that is not ours — a block this reader
                // does not know, ending at the top level where no `end` is
                // awaited. Left there, this loop spins for ever, and a page
                // view that spins is a window that stops answering: a book
                // chapter with a `<figure>` block did exactly that. Eat it.
                events.next();
            }
            continue;
        }
        let Some(event) = events.next() else { break };
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let text = inlines_until(events, TagEnd::Heading(level));
                out.push(Node::Heading(level_of(level), text));
            }
            Event::Start(Tag::Paragraph) => {
                let text = inlines_until(events, TagEnd::Paragraph);
                // A paragraph holding one image and nothing else is a figure,
                // not a sentence; it still renders as its alt text, but an
                // empty paragraph from a stripped image would be a blank gap.
                if !text.is_empty() {
                    out.push(Node::Para(text));
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        let lang = info.split_whitespace().next().unwrap_or("").to_string();
                        (!lang.is_empty()).then_some(lang)
                    }
                    CodeBlockKind::Indented => None,
                };
                let mut text = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Text(chunk) => text.push_str(&chunk),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                // The fence's own trailing newline is the fence's, not the
                // code's, and it prints as a blank last line.
                let text = text.strip_suffix('\n').unwrap_or(&text).to_string();
                out.push(Node::Code { lang, text });
            }
            Event::Start(Tag::BlockQuote(_)) => {
                out.push(Node::Quote(blocks_until(
                    events,
                    Some(TagEnd::BlockQuote(None)),
                )));
            }
            // Raw HTML at block level — a `<figure>` in a book chapter — is
            // shown as the markup it is, like inline HTML is, rather than
            // injected into this window or dropped. Before this arm existed
            // the block's closer was never consumed and the reader looped.
            Event::Start(Tag::HtmlBlock) => {
                let mut text = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Html(chunk) | Event::Text(chunk) => text.push_str(&chunk),
                        Event::End(TagEnd::HtmlBlock) => break,
                        _ => {}
                    }
                }
                let text = text.trim_end().to_string();
                if !text.is_empty() {
                    out.push(Node::Code {
                        lang: Some("html".to_string()),
                        text,
                    });
                }
            }
            Event::Start(Tag::List(start)) => out.push(list(events, start)),
            Event::Start(Tag::Table(_)) => out.push(table(events)),
            Event::Rule => out.push(Node::Rule),
            // Anything left is either handled inside a block above or is a
            // construct no extension enabled here can produce.
            _ => {}
        }
    }
    out
}

fn list(events: &mut Events<'_>, start: Option<u64>) -> Node {
    let mut items = Vec::new();
    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::Item) => {
                items.push(blocks_until(events, Some(TagEnd::Item)));
            }
            Event::End(TagEnd::List(_)) => break,
            _ => {}
        }
    }
    Node::List { start, items }
}

fn table(events: &mut Events<'_>) -> Node {
    let mut head = Vec::new();
    let mut rows = Vec::new();
    let mut row: Vec<Vec<Inline>> = Vec::new();
    let mut in_head = false;

    while let Some(event) = events.next() {
        match event {
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => {
                head = std::mem::take(&mut row);
                in_head = false;
            }
            Event::Start(Tag::TableRow) => row = Vec::new(),
            Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
            Event::Start(Tag::TableCell) => {
                row.push(inlines_until(events, TagEnd::TableCell));
            }
            Event::End(TagEnd::Table) => break,
            _ => {}
        }
    }
    // A header row that never closed leaves its cells in `row`.
    if in_head && !row.is_empty() {
        head = row;
    }
    Node::Table { head, rows }
}

/// Inline spans that are sitting loose inside a block — the tight-list case.
///
/// Stops *before* whatever ends them, so the caller's loop still sees the
/// closing tag or the next block and decides what it is.
fn inlines_while_inside(events: &mut Events<'_>) -> Vec<Inline> {
    let mut out = Vec::new();
    while let Some(event) = events.peek() {
        if opens_a_block(event) {
            break;
        }
        // `one_inline` consumes the closer of anything it opens, so an End
        // arriving here belongs to a block further out and is not ours to eat.
        if matches!(event, Event::End(_)) {
            break;
        }
        let Some(event) = events.next() else { break };
        out.extend(one_inline(events, event));
    }
    out
}

/// Inline spans until `end` closes.
fn inlines_until(events: &mut Events<'_>, end: TagEnd) -> Vec<Inline> {
    let mut out = Vec::new();
    while let Some(event) = events.next() {
        if let Event::End(tag) = &event
            && *tag == end
        {
            break;
        }
        out.extend(one_inline(events, event));
    }
    out
}

/// One inline event, and whatever it opens.
///
/// Shared by both callers on purpose: a span that rendered inside a paragraph
/// and not inside a tight list item would be a difference nobody could see a
/// reason for.
fn one_inline(events: &mut Events<'_>, event: Event<'_>) -> Vec<Inline> {
    match event {
        Event::Text(text) => vec![Inline::Text(text.to_string())],
        Event::Code(text) => vec![Inline::Code(text.to_string())],
        Event::SoftBreak | Event::HardBreak => vec![Inline::Break],
        Event::Start(Tag::Strong) => vec![Inline::Strong(inlines_until(events, TagEnd::Strong))],
        Event::Start(Tag::Emphasis) => vec![Inline::Em(inlines_until(events, TagEnd::Emphasis))],
        Event::Start(Tag::Strikethrough) => {
            vec![Inline::Strike(inlines_until(events, TagEnd::Strikethrough))]
        }
        Event::Start(Tag::Link { dest_url, .. }) => vec![Inline::Link {
            text: inlines_until(events, TagEnd::Link),
            url: dest_url.to_string(),
        }],
        Event::Start(Tag::Image { dest_url, .. }) => vec![Inline::Image {
            alt: flatten(&inlines_until(events, TagEnd::Image)),
            url: dest_url.to_string(),
        }],
        // A task list marker is the checkbox `- [x]` writes.
        Event::TaskListMarker(done) => {
            vec![Inline::Text(if done { "☑ " } else { "☐ " }.to_string())]
        }
        // Raw HTML is shown as the text it is rather than injected: a document
        // is not markup this window should execute.
        Event::Html(text) | Event::InlineHtml(text) => {
            vec![Inline::Code(text.trim_end().to_string())]
        }
        _ => Vec::new(),
    }
}

/// The plain text of a run of spans — an image's alt, a link's tooltip.
fn flatten(spans: &[Inline]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Inline::Text(text) | Inline::Code(text) => out.push_str(text),
            Inline::Strong(inner) | Inline::Em(inner) | Inline::Strike(inner) => {
                out.push_str(&flatten(inner));
            }
            Inline::Link { text, .. } => out.push_str(&flatten(text)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Break => out.push(' '),
        }
    }
    out
}

fn level_of(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

// ─── rendering ───────────────────────────────────────────────────────────────

#[component]
pub fn Markdown(#[prop(into)] text: String) -> impl IntoView {
    view! { <div class="flex flex-col gap-2">{render(parse(&text))}</div> }
}

fn render(nodes: Vec<Node>) -> AnyView {
    nodes.into_iter().map(node).collect_view().into_any()
}

fn node(node: Node) -> AnyView {
    match node {
        Node::Heading(level, text) => {
            // Sized by level, and only h1/h2 get a rule — a README with six
            // ruled headings in a 300px panel is a page of lines.
            let class = match level {
                1 => "mt-3 border-b border-line pb-1 text-body font-semibold text-strong",
                2 => "mt-3 border-b border-line pb-1 text-callout font-semibold text-strong",
                3 => "mt-2 text-callout font-semibold text-label",
                _ => "mt-2 text-footnote font-semibold text-label-2",
            };
            view! { <div class=class>{spans(text)}</div> }.into_any()
        }
        Node::Para(text) => view! {
            <p class="text-footnote leading-relaxed text-label-2 select-text">{spans(text)}</p>
        }
        .into_any(),
        Node::Code { lang, text } => view! {
            <pre class="overflow-x-auto rounded-[6px] bg-sunken px-3 py-2 font-mono text-caption text-label select-text">
                {lang
                    .map(|lang| {
                        view! {
                            <div class="mb-1 text-caption text-label-4">{lang}</div>
                        }
                    })}
                {text}
            </pre>
        }
        .into_any(),
        Node::Quote(inner) => view! {
            <blockquote class="border-l-2 border-line-strong pl-3 text-label-3">
                {render(inner)}
            </blockquote>
        }
        .into_any(),
        Node::List { start, items } => {
            let ordered = start.is_some();
            let first = start.unwrap_or(1);
            let rows = items
                .into_iter()
                .enumerate()
                .map(|(index, item)| {
                    let marker = if ordered {
                        format!("{}.", first + index as u64)
                    } else {
                        "•".to_string()
                    };
                    view! {
                        <li class="flex gap-2">
                            <span class="shrink-0 text-label-4">{marker}</span>
                            <div class="min-w-0 flex-1 flex flex-col gap-1">{render(item)}</div>
                        </li>
                    }
                })
                .collect_view();
            view! { <ul class="flex flex-col gap-1 pl-1 text-footnote text-label-2">{rows}</ul> }
                .into_any()
        }
        Node::Table { head, rows } => view! {
            // Its own scroller: a wide table must not make the page scroll
            // sideways, which is the one thing a reading pane cannot do.
            <div class="overflow-x-auto">
                <table class="w-full border-collapse text-footnote">
                    <thead>
                        <tr>
                            {head
                                .into_iter()
                                .map(|cell| {
                                    view! {
                                        <th class="border-b border-line px-2 py-1 text-left font-semibold text-label">
                                            {spans(cell)}
                                        </th>
                                    }
                                })
                                .collect_view()}
                        </tr>
                    </thead>
                    <tbody>
                        {rows
                            .into_iter()
                            .map(|row| {
                                view! {
                                    <tr>
                                        {row
                                            .into_iter()
                                            .map(|cell| {
                                                view! {
                                                    <td class="border-b border-line px-2 py-1 align-top text-label-2 select-text">
                                                        {spans(cell)}
                                                    </td>
                                                }
                                            })
                                            .collect_view()}
                                    </tr>
                                }
                            })
                            .collect_view()}
                    </tbody>
                </table>
            </div>
        }
        .into_any(),
        Node::Rule => view! { <div class="my-1 h-px bg-line" /> }.into_any(),
    }
}

fn spans(spans: Vec<Inline>) -> AnyView {
    spans.into_iter().map(span).collect_view().into_any()
}

fn span(span: Inline) -> AnyView {
    match span {
        Inline::Text(text) => text.into_any(),
        Inline::Code(text) => view! {
            <code class="rounded-[4px] bg-sunken px-1 font-mono text-caption text-label">
                {text}
            </code>
        }
        .into_any(),
        Inline::Strong(inner) => {
            view! { <strong class="font-semibold text-label">{spans(inner)}</strong> }.into_any()
        }
        Inline::Em(inner) => view! { <em class="italic">{spans(inner)}</em> }.into_any(),
        Inline::Strike(inner) => {
            view! { <span class="line-through opacity-70">{spans(inner)}</span> }.into_any()
        }
        Inline::Link { text, url } => {
            let copy = url.clone();
            view! {
                // Copies rather than navigates — see the header.
                <button
                    type="button"
                    title=url
                    on:click=move |_| copy_to_clipboard(&copy)
                    class="cursor-pointer text-rust underline decoration-dotted underline-offset-2 hover:opacity-80"
                >
                    {spans(text)}
                </button>
            }
            .into_any()
        }
        Inline::Image { alt, url } => {
            let label = if alt.is_empty() { url.clone() } else { alt };
            view! {
                <span
                    title=url
                    class="rounded-[4px] bg-sunken px-1 text-caption text-label-3"
                >
                    "🖼 "{label}
                </span>
            }
            .into_any()
        }
        Inline::Break => " ".into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape that froze the window: a book chapter with a `<figure>`
    /// block between two paragraphs. Its closer was never consumed and the
    /// block reader looped for ever on it — and the page view runs the reader
    /// on every render, so the freeze was the whole window's.
    #[test]
    fn an_html_block_is_shown_as_markup_and_the_reader_moves_past_it() {
        let nodes = parse(
            "Text before.\n\n<figure>\n<img src=\"figures/fig.svg\" alt=\"wiring\">\n\
             <figcaption>Figure 1</figcaption>\n</figure>\n\nText after.\n",
        );
        assert_eq!(nodes.len(), 3, "{nodes:?}");
        assert_eq!(nodes[0], Node::Para(vec![text("Text before.")]));
        match &nodes[1] {
            Node::Code { lang, text } => {
                assert_eq!(lang.as_deref(), Some("html"));
                assert!(text.starts_with("<figure>"), "{text}");
                assert!(text.contains("<figcaption>Figure 1</figcaption>"));
            }
            other => panic!("the figure should be shown as markup, got {other:?}"),
        }
        assert_eq!(nodes[2], Node::Para(vec![text("Text after.")]));
    }

    /// A whole real book, when `RUSTY_MD_CORPUS` names its `src/`: every
    /// chapter parses in well under a second, since the page view runs this
    /// on the draft at every render. Skipped, and said so, without a corpus.
    #[test]
    fn a_book_corpus_parses_in_bounded_time() {
        let Ok(dir) = std::env::var("RUSTY_MD_CORPUS") else {
            eprintln!("skipping: RUSTY_MD_CORPUS is not set");
            return;
        };
        for entry in std::fs::read_dir(&dir).expect("the corpus directory") {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a readable chapter");
            let start = std::time::Instant::now();
            let nodes = parse(&source);
            let took = start.elapsed();
            eprintln!(
                "{}: {} blocks, {} bytes, {took:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                nodes.len(),
                source.len()
            );
            assert!(took.as_millis() < 500, "{} took {took:?}", path.display());
        }
    }

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    /// A README's structure *is* its heading levels. The renderer this
    /// replaced flattened all six to one size, so an outline read as a wall.
    #[test]
    fn heading_levels_survive() {
        let nodes = parse("# One\n\n### Three\n");
        assert_eq!(
            nodes,
            vec![
                Node::Heading(1, vec![text("One")]),
                Node::Heading(3, vec![text("Three")]),
            ]
        );
    }

    /// Ordered lists kept their numbers, and a list that starts at 3 starts
    /// at 3 — renumbering somebody's steps is worse than not numbering them.
    #[test]
    fn an_ordered_list_keeps_its_first_number() {
        let nodes = parse("3. third\n4. fourth\n");
        let Node::List { start, items } = &nodes[0] else {
            panic!("not a list: {nodes:?}");
        };
        assert_eq!(*start, Some(3));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_nested_list_stays_nested() {
        let nodes = parse("- outer\n  - inner\n");
        let Node::List { items, .. } = &nodes[0] else {
            panic!("not a list: {nodes:?}");
        };
        assert_eq!(items.len(), 1, "one outer item holding the inner list");
        assert!(
            items[0]
                .iter()
                .any(|node| matches!(node, Node::List { .. })),
            "the inner list was flattened away: {:?}",
            items[0]
        );
    }

    #[test]
    fn a_table_keeps_its_header_and_rows() {
        let nodes = parse("| Crate | Does |\n|---|---|\n| a | b |\n| c | d |\n");
        let Node::Table { head, rows } = &nodes[0] else {
            panic!("not a table: {nodes:?}");
        };
        assert_eq!(head, &vec![vec![text("Crate")], vec![text("Does")]]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], vec![vec![text("c")], vec![text("d")]]);
    }

    #[test]
    fn a_fenced_block_keeps_its_language_and_loses_the_fence_newline() {
        let nodes = parse("```bash\ncargo test\n```\n");
        assert_eq!(
            nodes,
            vec![Node::Code {
                lang: Some("bash".to_string()),
                text: "cargo test".to_string(),
            }]
        );
    }

    /// The one thing a document must not do is execute as markup.
    #[test]
    fn raw_html_is_shown_rather_than_injected() {
        let nodes = parse("a <b>bold</b> word\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert!(
            spans
                .iter()
                .any(|s| matches!(s, Inline::Code(t) if t.contains("<b>"))),
            "the tag should read as text: {spans:?}"
        );
    }

    #[test]
    fn an_image_becomes_its_alt_text_and_keeps_the_source() {
        let nodes = parse("![a badge](https://example.test/b.svg)\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert_eq!(
            spans,
            &vec![Inline::Image {
                alt: "a badge".to_string(),
                url: "https://example.test/b.svg".to_string(),
            }]
        );
    }

    #[test]
    fn inline_marks_nest() {
        let nodes = parse("**bold `code`** and *em*\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert_eq!(
            spans[0],
            Inline::Strong(vec![text("bold "), Inline::Code("code".to_string())])
        );
        assert!(spans.iter().any(|s| matches!(s, Inline::Em(_))));
    }

    /// A *tight* list — no blank line between items — is what almost every
    /// README writes, and `pulldown-cmark` gives its items no `Paragraph`.
    /// Reading only block openings dropped the words and left the bullets.
    #[test]
    fn a_tight_list_item_keeps_its_text() {
        let nodes = parse(
            "1. Install the toolchain
2. Open a project
",
        );
        let Node::List { items, .. } = &nodes[0] else {
            panic!("not a list: {nodes:?}");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            vec![Node::Para(vec![text("Install the toolchain")])],
            "the item rendered as a number with nothing beside it",
        );
    }

    /// And the nested case, which is the same bug one level down.
    #[test]
    fn a_tight_nested_item_keeps_its_text_too() {
        let nodes = parse(
            "- outer
  - inner
",
        );
        let Node::List { items, .. } = &nodes[0] else {
            panic!("not a list: {nodes:?}");
        };
        assert_eq!(items[0][0], Node::Para(vec![text("outer")]));
        let Node::List { items: inner, .. } = &items[0][1] else {
            panic!("no nested list: {:?}", items[0]);
        };
        assert_eq!(inner[0], vec![Node::Para(vec![text("inner")])]);
    }

    #[test]
    fn a_blockquote_holds_blocks() {
        let nodes = parse("> quoted\n>\n> - a bullet\n");
        let Node::Quote(inner) = &nodes[0] else {
            panic!("not a quote: {nodes:?}");
        };
        assert!(matches!(inner[0], Node::Para(_)));
        assert!(matches!(inner[1], Node::List { .. }));
    }
}
