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
mod read;

pub use html::HtmlNode;
use html::TagToken;
use read::*;

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
    /// Still being written — an answer as it streams. Code blocks stay as
    /// written until it settles: a block re-rendered on every delta would
    /// ask the backend once per delta for a text about to change.
    #[prop(optional)]
    live: bool,
) -> impl IntoView {
    provide_context(PageBase(base));
    provide_context(PageLive(live));
    view! { <div class="flex flex-col gap-2">{render(parse(&text))}</div> }
}

/// Whether the page is still arriving — see [`Markdown`]'s `live`.
#[derive(Clone, Copy)]
struct PageLive(bool);

/// A fenced code block: in the editor's colours once the backend has read
/// it, and as written until then — or for good, when the fence names no
/// language, or one no grammar answers to, or there is no backend.
///
/// The runs come from `editor.snippets`, asked for on first sight and kept
/// by content, so the block costs one request however many times the page
/// re-renders and wherever else the same block appears.
#[component]
fn Snippet(lang: Option<String>, text: String) -> impl IntoView {
    let state = crate::state::AppState::expect();
    let live = use_context::<PageLive>().is_some_and(|live| live.0);
    let key = lang
        .as_deref()
        .filter(|_| !live)
        .map(|lang| crate::state::snippet_key(lang, &text));
    if let (Some(lang), Some(_)) = (&lang, key) {
        crate::controller::highlight_snippet(state, lang.clone(), text.clone());
    }
    let written = text.clone();
    let runs = move || {
        key.and_then(|key| {
            state.editor.snippets.with(|snippets| {
                snippets
                    .get(&key)
                    .filter(|lines| !lines.is_empty())
                    .cloned()
            })
        })
    };
    view! {
        <pre class=CODE_BLOCK>
            {lang
                .map(|lang| {
                    view! { <div class="mb-1 text-caption text-label-4">{lang}</div> }
                })}
            {move || match runs() {
                Some(lines) => highlighted(&lines).into_any(),
                None => written.clone().into_any(),
            }}
        </pre>
    }
}

/// Highlighted runs as the editor's echo draws them: one span per run, in
/// the class the stylesheet owns for its token, lines joined by newlines.
fn highlighted(lines: &[rusty_edit::Line]) -> AnyView {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let spans = line
                .spans
                .iter()
                .map(|span| {
                    let class = crate::view::panels::files::highlight::class_of(span.token);
                    view! { <span class=class>{span.text.clone()}</span> }
                })
                .collect_view();
            let newline = (index > 0).then_some("\n");
            view! {
                {newline}
                {spans}
            }
        })
        .collect_view()
        .into_any()
}

fn render(nodes: Vec<Node>) -> AnyView {
    nodes.into_iter().map(node).collect_view().into_any()
}

fn node(node: Node) -> AnyView {
    match node {
        Node::Heading(level, text) => heading(level, spans(text)),
        Node::Para(text) => view! { <p class=PARA>{spans(text)}</p> }.into_any(),
        Node::Code { lang, text } => view! { <Snippet lang=lang text=text /> }.into_any(),
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
