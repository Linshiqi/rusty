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
//! and what stays here is the decisions that are ours.
//!
//! **Links do not navigate.** A WebView navigation would replace the workbench
//! with docs.rs and there is no back button — this is not a browser tab.
//! Clicking copies the URL; the tooltip shows where it points.
//!
//! **A picture in the project is fetched; a picture elsewhere is not.** A
//! figure beside the chapter that names it is read through the backend, as a
//! picture in a diff is, and resolved against the page's own path — so a
//! page with no path (hover text, an answer) shows its alt text. A remote
//! image tells its host that somebody opened this file, so it stays alt text
//! with the reason in the tooltip.
//!
//! **A formula is MathML.** `$…$` and `$$…$$` go through `pulldown-latex` and
//! the WebView draws the result itself; a formula the converter refuses is
//! shown as the source it is, with the refusal in the tooltip, because a
//! formula that quietly vanished is worse than one that quietly stayed text.
//!
//! **Raw HTML is read, not injected.** A book's `<figure>` and `<kbd>` render
//! as the elements they name through the allowlist in [`element`]; an element
//! the allowlist does not know shows its children; a script or a frame is
//! named and not run. See [`html`] for the reader and why it is not html5ever.
//!
//! The parse is a pure `&str -> Vec<Node>`, so the shapes are tested without a
//! browser and the rendering below has no logic worth hiding.

use leptos::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, ImageLoad},
    view::components::copy_to_clipboard,
};

mod html;

pub use html::HtmlNode;
use html::TagToken;

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
    /// A block of raw markup, read into a tree — a `<figure>`, a `<details>`.
    Html(Vec<HtmlNode>),
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
    /// Alt text and where the image comes from. Fetched or not by
    /// [`picture`]'s rules; see the header.
    Image {
        alt: String,
        url: String,
    },
    Break,
    /// A formula, as written: `$m$` inline, `$$F = ma$$` on its own.
    Math {
        tex: String,
        display: bool,
    },
    /// An inline element written as a tag pair — `<kbd>Ctrl</kbd>` — with
    /// the Markdown between the tags read as Markdown.
    Tag {
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<Inline>,
    },
    /// A raw tag that paired with nothing, or one this reader does not fold:
    /// shown as the text it is, never injected.
    Html(String),
}

/// Parse Markdown into blocks. Pure.
pub fn parse(text: &str) -> Vec<Node> {
    // The extensions a project's own documentation actually uses. Footnotes
    // are left off: `pulldown-cmark` would emit them and there is nothing
    // here that renders them better than the source line.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_MATH);

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
            // read into a tree the renderer walks with its allowlist. It was
            // shown as markup before, and before that its closer was never
            // consumed and the reader looped.
            Event::Start(Tag::HtmlBlock) => {
                let mut text = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Html(chunk) | Event::Text(chunk) => text.push_str(&chunk),
                        Event::End(TagEnd::HtmlBlock) => break,
                        _ => {}
                    }
                }
                let nodes = html::parse(&text);
                if !nodes.is_empty() {
                    out.push(Node::Html(nodes));
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
    fold_html(out)
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
    fold_html(out)
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
        Event::InlineMath(tex) => vec![Inline::Math {
            tex: tex.trim().to_string(),
            display: false,
        }],
        Event::DisplayMath(tex) => vec![Inline::Math {
            tex: tex.trim().to_string(),
            display: true,
        }],
        // Raw HTML arrives one tag at a time with Markdown between; the fold
        // at the end of the run pairs what it can. Until then, the tag as
        // written.
        Event::Html(text) | Event::InlineHtml(text) => {
            vec![Inline::Html(text.trim_end().to_string())]
        }
        _ => Vec::new(),
    }
}

/// The paired inline tags the fold turns into [`Inline::Tag`] — the ones
/// that mark a run of text. A `<div>` written inline is not one, and stays
/// the text it is.
const INLINE_TAGS: &[&str] = &[
    "a", "abbr", "b", "cite", "code", "del", "em", "i", "ins", "kbd", "mark", "q", "s", "samp",
    "small", "span", "strike", "strong", "sub", "sup", "tt", "u", "var",
];

/// Pair the raw tags in a run of spans into elements.
///
/// `<kbd>Ctrl</kbd>` reaches this as three spans — a tag, a word, a tag —
/// because `pulldown-cmark` reads the Markdown between them as Markdown, which
/// is what makes `<kbd>**Ctrl**</kbd>` work. A stack pairs each closer with
/// the nearest open tag of its name; whatever never closes, or closes out of
/// order, goes back to being the text it was, so a stray tag is a visible
/// mistake and never a swallowed one. `<br>` and `<img>` are the things they
/// write.
fn fold_html(spans: Vec<Inline>) -> Vec<Inline> {
    struct Frame {
        raw: String,
        tag: String,
        attrs: Vec<(String, String)>,
        children: Vec<Inline>,
    }
    fn put(stack: &mut [Frame], out: &mut Vec<Inline>, span: Inline) {
        match stack.last_mut() {
            Some(frame) => frame.children.push(span),
            None => out.push(span),
        }
    }

    let mut stack: Vec<Frame> = Vec::new();
    let mut out = Vec::new();
    for span in spans {
        let Inline::Html(raw) = span else {
            put(&mut stack, &mut out, span);
            continue;
        };
        match html::tag_of(&raw) {
            Some(TagToken::Open {
                tag,
                attrs,
                void: false,
            }) if INLINE_TAGS.contains(&tag.as_str()) => stack.push(Frame {
                raw,
                tag,
                attrs,
                children: Vec::new(),
            }),
            Some(TagToken::Open { tag, .. }) if tag == "br" => {
                put(&mut stack, &mut out, Inline::Break);
            }
            Some(TagToken::Open { tag, attrs, .. }) if tag == "img" => {
                let value = |name: &str| {
                    attrs
                        .iter()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| value.clone())
                        .unwrap_or_default()
                };
                put(
                    &mut stack,
                    &mut out,
                    Inline::Image {
                        alt: value("alt"),
                        url: value("src"),
                    },
                );
            }
            Some(TagToken::Close { tag }) => match stack.iter().rposition(|f| f.tag == tag) {
                Some(at) => {
                    // Anything still open inside goes back to being text,
                    // its children after it.
                    let unclosed = stack.split_off(at + 1);
                    let Frame {
                        tag,
                        attrs,
                        mut children,
                        ..
                    } = stack.pop().expect("the frame at `at`");
                    for frame in unclosed {
                        children.push(Inline::Html(frame.raw));
                        children.extend(frame.children);
                    }
                    put(
                        &mut stack,
                        &mut out,
                        Inline::Tag {
                            tag,
                            attrs,
                            children,
                        },
                    );
                }
                None => put(&mut stack, &mut out, Inline::Html(raw)),
            },
            _ => put(&mut stack, &mut out, Inline::Html(raw)),
        }
    }
    while let Some(frame) = stack.pop() {
        let mut spans = vec![Inline::Html(frame.raw)];
        spans.extend(frame.children);
        for span in spans {
            put(&mut stack, &mut out, span);
        }
    }
    out
}

/// The plain text of a run of spans — an image's alt, a link's tooltip.
fn flatten(spans: &[Inline]) -> String {
    let mut out = String::new();
    for span in spans {
        match span {
            Inline::Text(text) | Inline::Code(text) | Inline::Html(text) => out.push_str(text),
            Inline::Strong(inner) | Inline::Em(inner) | Inline::Strike(inner) => {
                out.push_str(&flatten(inner));
            }
            Inline::Link { text, .. } => out.push_str(&flatten(text)),
            Inline::Tag { children, .. } => out.push_str(&flatten(children)),
            Inline::Image { alt, .. } => out.push_str(alt),
            Inline::Math { tex, .. } => out.push_str(tex),
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

/// Where a picture named in a page lives, as a project-relative path.
///
/// `src` is taken relative to the directory of `base_file`, the page's own
/// project-relative path — `figures/a.svg` beside `book/src/ch1.md` is
/// `book/src/figures/a.svg`. `..` walks up and never past the project root:
/// a path that would is `None`, as is one written from the root or from
/// another machine, because the page cannot say what either means here.
pub fn resolve_relative(base_file: &str, src: &str) -> Option<String> {
    let src = src.split(['#', '?']).next().unwrap_or("");
    if src.is_empty() || src.starts_with(['/', '\\']) || is_remote(src) {
        return None;
    }
    let dir = base_file.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for piece in src.split(['/', '\\']) {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Whether a URL names something on another machine.
fn is_remote(url: &str) -> bool {
    url.contains("://") || url.starts_with("//") || url.starts_with("mailto:")
}

/// A formula as MathML, or the parser's reason for refusing it.
///
/// `pulldown-latex`, after a release on `latex2mathml`: that one read `v_i^2`
/// as a superscript on the subscript — a picture that is subtly wrong, the
/// worst kind — and knew no `aligned`, which a physics book uses on every
/// other page. The parse runs to completion before anything is written, so a
/// formula with a mistake in it is refused whole rather than drawn up to the
/// mistake and cut.
pub fn math_html(tex: &str, display: bool) -> Result<String, String> {
    use pulldown_latex::{Parser, ParserError, RenderConfig, Storage, config::DisplayMode};

    let storage = Storage::new();
    let events = Parser::new(tex, &storage)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let config = RenderConfig {
        display_mode: if display {
            DisplayMode::Block
        } else {
            DisplayMode::Inline
        },
        ..RenderConfig::default()
    };
    let mut out = String::new();
    pulldown_latex::push_mathml(
        &mut out,
        events.into_iter().map(Ok::<_, ParserError>),
        config,
    )
    .map_err(|error| error.to_string())?;
    Ok(out)
}

// ─── rendering ───────────────────────────────────────────────────────────────

/// The page's own project-relative path, when it has one: what its pictures
/// are relative to. Absent for hover text and answers, whose pictures stay
/// alt text.
#[derive(Clone)]
struct PageBase(Option<String>);

const CODE_BLOCK: &str = "overflow-x-auto rounded-[6px] bg-sunken px-3 py-2 font-mono text-caption \
                          text-label select-text";
const CODE_SPAN: &str = "rounded-[4px] bg-sunken px-1 font-mono text-caption text-label";
const PARA: &str = "text-footnote leading-relaxed text-label-2 select-text";
const LINK: &str = "cursor-pointer text-rust underline decoration-dotted underline-offset-2 \
                    hover:opacity-80";
const IMG: &str = "h-auto max-w-full rounded-[6px]";
const CELL: &str = "border-b border-line px-2 py-1 align-top text-label-2 select-text";
const HEAD_CELL: &str = "border-b border-line px-2 py-1 text-left font-semibold text-label";

#[component]
pub fn Markdown(
    #[prop(into)] text: String,
    /// The file this Markdown is, project-relative, for resolving its
    /// pictures. Omitted for text that is not a file.
    #[prop(optional_no_strip)]
    base: Option<String>,
) -> impl IntoView {
    provide_context(PageBase(base));
    view! { <div class="flex flex-col gap-2">{render(parse(&text))}</div> }
}

fn render(nodes: Vec<Node>) -> AnyView {
    nodes.into_iter().map(node).collect_view().into_any()
}

fn node(node: Node) -> AnyView {
    match node {
        Node::Heading(level, text) => heading(level, spans(text)),
        Node::Para(text) => view! { <p class=PARA>{spans(text)}</p> }.into_any(),
        Node::Code { lang, text } => view! {
            <pre class=CODE_BLOCK>
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
        Node::Quote(inner) => quote(render(inner)),
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
        Node::Table { head, rows } => {
            let head = view! {
                <tr>
                    {head
                        .into_iter()
                        .map(|cell| view! { <th class=HEAD_CELL>{spans(cell)}</th> })
                        .collect_view()}
                </tr>
            }
            .into_any();
            let body = rows
                .into_iter()
                .map(|row| {
                    view! {
                        <tr>
                            {row
                                .into_iter()
                                .map(|cell| view! { <td class=CELL>{spans(cell)}</td> })
                                .collect_view()}
                        </tr>
                    }
                })
                .collect_view()
                .into_any();
            table_frame(view! { <thead>{head}</thead><tbody>{body}</tbody> }.into_any())
        }
        Node::Rule => rule(),
        Node::Html(nodes) => view! {
            <div class="flex flex-col gap-2 text-footnote text-label-2 select-text">
                {html_nodes(nodes)}
            </div>
        }
        .into_any(),
    }
}

/// Sized by level, and only h1/h2 get a rule — a README with six ruled
/// headings in a 300px panel is a page of lines.
fn heading(level: u8, inner: AnyView) -> AnyView {
    let class = match level {
        1 => "mt-3 border-b border-line pb-1 text-body font-semibold text-strong",
        2 => "mt-3 border-b border-line pb-1 text-callout font-semibold text-strong",
        3 => "mt-2 text-callout font-semibold text-label",
        _ => "mt-2 text-footnote font-semibold text-label-2",
    };
    view! { <div class=class>{inner}</div> }.into_any()
}

fn quote(inner: AnyView) -> AnyView {
    view! { <blockquote class="border-l-2 border-line-strong pl-3 text-label-3">{inner}</blockquote> }
        .into_any()
}

fn rule() -> AnyView {
    view! { <div class="my-1 h-px bg-line" /> }.into_any()
}

/// A table in its own scroller: a wide one must not make the page scroll
/// sideways, which is the one thing a reading pane cannot do.
fn table_frame(inner: AnyView) -> AnyView {
    view! {
        <div class="overflow-x-auto">
            <table class="w-full border-collapse text-footnote">{inner}</table>
        </div>
    }
    .into_any()
}

/// Copies rather than navigates — see the header.
fn link(url: String, inner: AnyView) -> AnyView {
    let copy = url.clone();
    view! {
        <button
            type="button"
            title=url
            on:click=move |_| copy_to_clipboard(&copy)
            class=LINK
        >
            {inner}
        </button>
    }
    .into_any()
}

fn spans(spans: Vec<Inline>) -> AnyView {
    spans.into_iter().map(span).collect_view().into_any()
}

fn span(span: Inline) -> AnyView {
    match span {
        Inline::Text(text) => text.into_any(),
        Inline::Code(text) => view! { <code class=CODE_SPAN>{text}</code> }.into_any(),
        Inline::Strong(inner) => {
            view! { <strong class="font-semibold text-label">{spans(inner)}</strong> }.into_any()
        }
        Inline::Em(inner) => view! { <em class="italic">{spans(inner)}</em> }.into_any(),
        Inline::Strike(inner) => {
            view! { <span class="line-through opacity-70">{spans(inner)}</span> }.into_any()
        }
        Inline::Link { text, url } => link(url, spans(text)),
        Inline::Image { alt, url } => picture(url, alt),
        Inline::Break => " ".into_any(),
        Inline::Math { tex, display } => math(tex, display),
        Inline::Tag {
            tag,
            attrs,
            children,
        } => element(&tag, &attrs, spans(children)),
        // A tag that paired with nothing is shown as the text it is.
        Inline::Html(raw) => view! { <code class=CODE_SPAN>{raw}</code> }.into_any(),
    }
}

/// A formula, drawn by the WebView from MathML; one the converter refused is
/// its own source, with the refusal in the tooltip.
fn math(tex: String, display: bool) -> AnyView {
    match math_html(&tex, display) {
        Ok(markup) => {
            let class = if display {
                "md-math my-2 block overflow-x-auto text-center text-label"
            } else {
                "md-math text-label"
            };
            view! { <span class=class inner_html=markup /> }.into_any()
        }
        Err(error) => view! {
            <code
                title=t!("markdown.math-error", error = error)
                class="rounded-[4px] bg-sunken px-1 font-mono text-caption text-amber"
            >
                {tex}
            </code>
        }
        .into_any(),
    }
}

/// A tree of raw markup, element by element through [`element`].
fn html_nodes(nodes: Vec<HtmlNode>) -> AnyView {
    nodes
        .into_iter()
        .map(|node| match node {
            HtmlNode::Text(text) => text.into_any(),
            HtmlNode::Element {
                tag,
                attrs,
                children,
            } => {
                let inner = html_nodes(children);
                element(&tag, &attrs, inner)
            }
        })
        .collect_view()
        .into_any()
}

/// One element of a document's own markup, by allowlist.
///
/// Each entry is a shape this page already draws — a figure is a centred
/// column, a `<kbd>` is a key cap, an `<a>` copies like a Markdown link. An
/// element not listed draws its children and nothing of itself, so a `<div
/// class="warning">` still shows its text; the tags that would run or embed
/// something are named and left at that. A script in a document is not a
/// document.
fn element(tag: &str, attrs: &[(String, String)], inner: AnyView) -> AnyView {
    let attr = |name: &str| {
        attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_default()
    };
    match tag {
        "img" => picture(attr("src"), attr("alt")),
        "br" => view! { <br /> }.into_any(),
        "hr" => rule(),
        "figure" => {
            view! { <figure class="my-2 flex flex-col items-center gap-2">{inner}</figure> }
                .into_any()
        }
        "figcaption" => view! {
            <figcaption class="max-w-[64ch] text-center text-caption text-label-3">{inner}</figcaption>
        }
        .into_any(),
        "p" => view! { <p class=PARA>{inner}</p> }.into_any(),
        "center" => {
            view! { <div class="flex flex-col items-center gap-2 text-center">{inner}</div> }
                .into_any()
        }
        "div" | "section" | "article" | "aside" | "main" | "header" | "footer" | "nav" => {
            view! { <div class="flex flex-col gap-2">{inner}</div> }.into_any()
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => heading(tag[1..].parse().unwrap_or(6), inner),
        "blockquote" => quote(inner),
        "pre" => view! { <pre class=CODE_BLOCK>{inner}</pre> }.into_any(),
        "code" | "tt" | "samp" | "var" => view! { <code class=CODE_SPAN>{inner}</code> }.into_any(),
        "kbd" => view! {
            <kbd class="rounded-[4px] border border-line bg-sunken px-1 font-mono text-caption text-label">
                {inner}
            </kbd>
        }
        .into_any(),
        "b" | "strong" => {
            view! { <strong class="font-semibold text-label">{inner}</strong> }.into_any()
        }
        "i" | "em" | "cite" | "q" => view! { <em class="italic">{inner}</em> }.into_any(),
        "u" | "ins" => view! { <span class="underline">{inner}</span> }.into_any(),
        "s" | "del" | "strike" => {
            view! { <span class="line-through opacity-70">{inner}</span> }.into_any()
        }
        "sub" => view! { <sub>{inner}</sub> }.into_any(),
        "sup" => view! { <sup>{inner}</sup> }.into_any(),
        "small" => view! { <span class="text-caption">{inner}</span> }.into_any(),
        "mark" => {
            view! { <mark class="rounded-[3px] bg-amber/30 px-0.5 text-label">{inner}</mark> }
                .into_any()
        }
        "a" => link(attr("href"), inner),
        "ul" => view! { <ul class="flex list-disc flex-col gap-1 pl-5">{inner}</ul> }.into_any(),
        "ol" => view! { <ol class="flex list-decimal flex-col gap-1 pl-5">{inner}</ol> }.into_any(),
        "li" => view! { <li>{inner}</li> }.into_any(),
        "table" => table_frame(inner),
        "thead" => view! { <thead>{inner}</thead> }.into_any(),
        "tbody" => view! { <tbody>{inner}</tbody> }.into_any(),
        "tr" => view! { <tr>{inner}</tr> }.into_any(),
        "th" => view! { <th class=HEAD_CELL>{inner}</th> }.into_any(),
        "td" => view! { <td class=CELL>{inner}</td> }.into_any(),
        "details" => {
            view! { <details class="rounded-[6px] border border-line px-3 py-2">{inner}</details> }
                .into_any()
        }
        "summary" => {
            view! { <summary class="cursor-pointer font-semibold text-label">{inner}</summary> }
                .into_any()
        }
        "script" | "style" | "iframe" | "object" | "embed" | "video" | "audio" | "canvas"
        | "form" | "input" | "button" | "link" | "meta" | "template" | "noscript" => view! {
            <span class="rounded-[4px] bg-sunken px-1 text-caption text-label-4">
                {t!("markdown.html-skipped", tag = tag.to_string())}
            </span>
        }
        .into_any(),
        _ => view! { <span>{inner}</span> }.into_any(),
    }
}

/// A picture named in the page, by the header's rules: fetched when it is a
/// file in the project the page can point at, alt text otherwise.
fn picture(url: String, alt: String) -> AnyView {
    if url.starts_with("data:") {
        return view! { <img src=url alt=alt class=IMG /> }.into_any();
    }
    if is_remote(&url) {
        let title = format!("{}\n{url}", t!("markdown.image-remote"));
        return chip(label_of(&alt, &url), title);
    }
    let base = use_context::<PageBase>().and_then(|base| base.0);
    match base.and_then(|base| resolve_relative(&base, &url)) {
        Some(path) if rusty_git::image_mime(&path).is_some() => {
            view! { <LocalPicture path=path alt=alt /> }.into_any()
        }
        _ => chip(label_of(&alt, &url), url),
    }
}

fn label_of(alt: &str, url: &str) -> String {
    if alt.is_empty() {
        url.to_string()
    } else {
        alt.to_string()
    }
}

/// What stands in for a picture that is not drawn: its alt text, and in the
/// tooltip why — the honest version of a broken-image icon.
fn chip(label: String, title: String) -> AnyView {
    view! {
        <span title=title class="rounded-[4px] bg-sunken px-1 text-caption text-label-3">
            "🖼 "{label}
        </span>
    }
    .into_any()
}

/// A picture read from the project, through the controller's one fetch and
/// the shared cache — so a page redrawn on every keystroke asks for each
/// figure once, and two pages showing one figure read it once.
#[component]
fn LocalPicture(path: String, alt: String) -> impl IntoView {
    let state = AppState::expect();
    controller::load_image(state, path.clone());
    let key = path.clone();
    let label = label_of(&alt, &path);
    move || match state.editor.images.with(|images| images.get(&key).cloned()) {
        Some(ImageLoad::Ready(url)) => {
            view! { <img src=url alt=alt.clone() title=key.clone() class=IMG /> }.into_any()
        }
        Some(ImageLoad::Failed(error)) => chip(
            label.clone(),
            t!("markdown.image-failed", path = key.clone(), error = error),
        ),
        _ => chip(
            label.clone(),
            t!("markdown.image-loading", path = key.clone()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_string())
    }

    /// The shape that froze the window once and was shown as markup after: a
    /// book chapter with a `<figure>` block between two paragraphs. It is
    /// read into a tree now, and the reader still moves past it.
    #[test]
    fn an_html_block_is_read_into_a_tree_and_the_reader_moves_past_it() {
        let nodes = parse(
            "Text before.\n\n<figure>\n<img src=\"figures/fig.svg\" alt=\"wiring\">\n\
             <figcaption>Figure 1</figcaption>\n</figure>\n\nText after.\n",
        );
        assert_eq!(nodes.len(), 3, "{nodes:?}");
        assert_eq!(nodes[0], Node::Para(vec![text("Text before.")]));
        let Node::Html(tree) = &nodes[1] else {
            panic!("the figure should be read as markup, got {:?}", nodes[1]);
        };
        let HtmlNode::Element { tag, children, .. } = &tree[0] else {
            panic!("not an element: {tree:?}");
        };
        assert_eq!(tag, "figure");
        assert!(
            matches!(&children[0], HtmlNode::Element { tag, attrs, .. }
                if tag == "img" && attrs.contains(&("src".to_string(), "figures/fig.svg".to_string()))),
            "{children:?}"
        );
        assert_eq!(
            children[1],
            HtmlNode::Element {
                tag: "figcaption".into(),
                attrs: vec![],
                children: vec![HtmlNode::Text("Figure 1".into())],
            }
        );
        assert_eq!(nodes[2], Node::Para(vec![text("Text after.")]));
    }

    /// `$m$` in a sentence and `$$…$$` on its own lines, as a physics book
    /// writes them; the display form keeps its own paragraph.
    #[test]
    fn math_is_read_inline_and_on_its_own() {
        let nodes = parse("Mass $m$ moves.\n\n$$\nF = ma\n$$\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert_eq!(
            spans[1],
            Inline::Math {
                tex: "m".into(),
                display: false,
            }
        );
        assert_eq!(
            nodes[1],
            Node::Para(vec![Inline::Math {
                tex: "F = ma".into(),
                display: true,
            }]),
            "{nodes:?}"
        );
    }

    /// The converter answers MathML for a formula and a reason for a broken
    /// one — which the page shows as the source, never as nothing.
    #[test]
    fn a_formula_becomes_mathml_and_a_broken_one_is_refused_by_name() {
        let markup = math_html(r"T = 2\rho a v_i^2", false).unwrap();
        assert!(markup.starts_with("<math"), "{markup}");
        assert!(
            markup.contains("<msubsup>"),
            "a subscript and a superscript on one base, not one on the other: {markup}"
        );
        assert!(
            math_html(r"\begin{aligned} a &= b \\ c &= d \end{aligned}", true).is_ok(),
            "the environment a physics book writes its systems in"
        );
        assert!(math_html(r"\frac{1", false).is_err());
    }

    /// `<kbd>Ctrl</kbd>` arrives as three spans and leaves as one element;
    /// a tag this reader does not fold stays the text it is, so a stray one
    /// is a visible mistake and never a swallowed one.
    #[test]
    fn inline_tags_fold_into_elements_and_unknown_ones_stay_as_text() {
        let nodes = parse("press <kbd>Ctrl</kbd> and <blink>x</blink>\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert_eq!(
            spans[1],
            Inline::Tag {
                tag: "kbd".into(),
                attrs: vec![],
                children: vec![text("Ctrl")],
            }
        );
        assert!(spans.contains(&Inline::Html("<blink>".into())), "{spans:?}");
        assert!(spans.contains(&Inline::Html("</blink>".into())));
    }

    /// A closer with no opener, and an opener with no closer, both come out
    /// as the text they were rather than as an element or as nothing.
    #[test]
    fn an_unpaired_inline_tag_is_text_again() {
        let nodes = parse("a</b> and <i>b\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert!(spans.contains(&Inline::Html("</b>".into())), "{spans:?}");
        assert!(spans.contains(&Inline::Html("<i>".into())), "{spans:?}");
        assert!(
            !spans.iter().any(|s| matches!(s, Inline::Tag { .. })),
            "{spans:?}"
        );
    }

    /// `<br>` and `<img>` written as tags are the things they write.
    #[test]
    fn a_break_and_an_image_written_as_tags_are_read_as_such() {
        let nodes = parse("one<br>two <img src=\"a.png\" alt=\"A\">\n");
        let Node::Para(spans) = &nodes[0] else {
            panic!("not a paragraph: {nodes:?}");
        };
        assert!(spans.contains(&Inline::Break), "{spans:?}");
        assert!(
            spans.contains(&Inline::Image {
                alt: "A".into(),
                url: "a.png".into(),
            }),
            "{spans:?}"
        );
    }

    /// Where a page's picture is: beside the page, up from it, and never
    /// above the project or on another machine.
    #[test]
    fn a_figure_is_found_beside_its_page_and_never_above_the_root() {
        assert_eq!(
            resolve_relative("book/src/ch1.md", "figures/a.svg").as_deref(),
            Some("book/src/figures/a.svg")
        );
        assert_eq!(
            resolve_relative("book/src/ch1.md", "../img/a.png#x").as_deref(),
            Some("book/img/a.png")
        );
        assert_eq!(
            resolve_relative("README.md", "./docs/a.png").as_deref(),
            Some("docs/a.png")
        );
        assert_eq!(resolve_relative("README.md", "../a.png"), None);
        assert_eq!(resolve_relative("book/src/ch1.md", "/a.png"), None);
        assert_eq!(
            resolve_relative("book/src/ch1.md", "https://example.test/a.png"),
            None
        );
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

    #[test]
    fn an_image_keeps_its_alt_text_and_its_source() {
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
