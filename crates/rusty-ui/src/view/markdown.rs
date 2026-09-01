//! A small Markdown renderer.
//!
//! Hover text and assistant answers arrive as Markdown; showing the raw
//! sigils made both look broken. This renders the subset those two sources
//! actually produce — headings, bullet lists, fenced code, rules, inline
//! `code`, **bold** and [links](url) — and nothing more. A full parser is a
//! dependency this crate does not need for six constructs.
//!
//! Links do not navigate: a WebView navigation would replace the workbench
//! with docs.rs. Clicking one copies the URL, and the tooltip shows where it
//! would have gone.

use leptos::prelude::*;

use crate::view::components::copy_to_clipboard;

/// One block-level element.
#[derive(Debug, PartialEq, Eq)]
pub enum Block {
    Heading(String),
    Bullets(Vec<String>),
    Code(String),
    Rule,
    Para(String),
}

/// Split text into block-level elements. Pure, so the shapes are testable
/// without a browser.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    let mut code: Option<Vec<String>> = None;

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match code.take() {
                Some(lines) => out.push(Block::Code(lines.join("\n"))),
                None => code = Some(Vec::new()),
            }
            continue;
        }
        if let Some(lines) = code.as_mut() {
            lines.push(line.to_string());
            continue;
        }

        let trimmed = line.trim();
        if trimmed == "---" {
            out.push(Block::Rule);
        } else if let Some(rest) = trimmed
            .strip_prefix("#### ")
            .or_else(|| trimmed.strip_prefix("### "))
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("# "))
        {
            out.push(Block::Heading(rest.to_string()));
        } else if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            match out.last_mut() {
                Some(Block::Bullets(items)) => items.push(rest.to_string()),
                _ => out.push(Block::Bullets(vec![rest.to_string()])),
            }
        } else if trimmed.is_empty() {
            // A blank line closes the current paragraph or list.
            if matches!(out.last(), Some(Block::Para(p)) if p.is_empty()) {
                continue;
            }
            if !matches!(out.last(), Some(Block::Para(_))) {
                continue;
            }
            out.push(Block::Para(String::new()));
        } else {
            match out.last_mut() {
                Some(Block::Para(prose)) if !prose.is_empty() => {
                    prose.push('\n');
                    prose.push_str(line);
                }
                Some(Block::Para(prose)) => *prose = line.to_string(),
                _ => out.push(Block::Para(line.to_string())),
            }
        }
    }
    if let Some(lines) = code.take() {
        out.push(Block::Code(lines.join("\n")));
    }
    out.retain(|block| !matches!(block, Block::Para(p) if p.trim().is_empty()));
    out
}

/// One inline run.
#[derive(Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Code(String),
    Bold(String),
    Link { label: String, url: String },
}

/// Split one run of prose into inline elements. Unclosed markers stay text —
/// a stray backtick must not eat the rest of the message.
pub fn inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;

    let flush = |plain: &mut String, out: &mut Vec<Inline>| {
        if !plain.is_empty() {
            out.push(Inline::Text(std::mem::take(plain)));
        }
    };

    while at < chars.len() {
        let rest: String = chars[at..].iter().collect();
        if chars[at] == '`'
            && let Some(end) = rest[1..].find('`')
        {
            flush(&mut plain, &mut out);
            out.push(Inline::Code(rest[1..1 + end].to_string()));
            at += end + 2;
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut plain, &mut out);
            out.push(Inline::Bold(after[..end].to_string()));
            at += end + 4;
            continue;
        }
        if chars[at] == '[' {
            // [label](url) — the label may itself carry `code`.
            if let Some(close) = rest.find("](")
                && let Some(end) = rest[close + 2..].find(')')
            {
                flush(&mut plain, &mut out);
                out.push(Inline::Link {
                    label: rest[1..close].to_string(),
                    url: rest[close + 2..close + 2 + end].to_string(),
                });
                at += close + 2 + end + 1;
                continue;
            }
        }
        plain.push(chars[at]);
        at += 1;
    }
    flush(&mut plain, &mut out);
    out
}

fn inline_view(text: &str) -> AnyView {
    inline(text)
        .into_iter()
        .map(|piece| match piece {
            Inline::Text(text) => text.into_any(),
            Inline::Code(code) => view! {
                <code class="rounded-[4px] bg-sunken px-1 py-px font-mono text-[0.92em]">
                    {code}
                </code>
            }
            .into_any(),
            Inline::Bold(text) => {
                view! { <strong class="font-semibold text-label">{text}</strong> }.into_any()
            }
            Inline::Link { label, url } => {
                let title = format!("{url} — click to copy the link");
                let copied = url.clone();
                view! {
                    <button
                        type="button"
                        title=title
                        on:click=move |_| copy_to_clipboard(&copied)
                        class="cursor-pointer text-rust underline decoration-dotted underline-offset-2"
                    >
                        {inline_view(&label)}
                    </button>
                }
                .into_any()
            }
        })
        .collect_view()
        .into_any()
}

/// Rendered Markdown, in the reading style of the surface it sits on.
#[component]
pub fn Markdown(#[prop(into)] text: String) -> impl IntoView {
    blocks(&text)
        .into_iter()
        .map(|block| match block {
            Block::Heading(text) => view! {
                <div class="mt-2 mb-1 text-callout font-semibold text-label first:mt-0">
                    {inline_view(&text)}
                </div>
            }
            .into_any(),
            Block::Bullets(items) => view! {
                <ul class="my-1 ml-4 list-disc space-y-0.5">
                    {items
                        .into_iter()
                        .map(|item| view! { <li>{inline_view(&item)}</li> })
                        .collect_view()}
                </ul>
            }
            .into_any(),
            Block::Code(code) => view! {
                <pre class="my-1.5 overflow-x-auto rounded-[6px] bg-sunken px-2.5 py-1.5 font-mono text-footnote leading-relaxed">
                    {code}
                </pre>
            }
            .into_any(),
            Block::Rule => view! { <div class="my-2 h-px bg-line" /> }.into_any(),
            Block::Para(prose) => view! {
                <p class="my-1 leading-relaxed whitespace-pre-wrap first:mt-0 last:mb-0">
                    {inline_view(&prose)}
                </p>
            }
            .into_any(),
        })
        .collect_view()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_hover_text_actually_has() {
        // Verbatim constructs from an esp-hal hover card.
        let got = blocks("`Dm` = `Blocking`\n\n## Errors\n\nSee below.");
        assert_eq!(
            got,
            vec![
                Block::Para("`Dm` = `Blocking`".to_string()),
                Block::Heading("Errors".to_string()),
                Block::Para("See below.".to_string()),
            ]
        );
    }

    #[test]
    fn fences_bullets_and_rules_split_cleanly() {
        let got = blocks("```rust\nfn x() {}\n```\n- one\n- two\n---\ntail");
        assert_eq!(
            got,
            vec![
                Block::Code("fn x() {}".to_string()),
                Block::Bullets(vec!["one".to_string(), "two".to_string()]),
                Block::Rule,
                Block::Para("tail".to_string()),
            ]
        );
    }

    #[test]
    fn inline_finds_code_bold_and_links() {
        let got = inline("**esp** toolchain, [`RxError`](https://docs.rs/x) since `read`.");
        assert_eq!(
            got,
            vec![
                Inline::Bold("esp".to_string()),
                Inline::Text(" toolchain, ".to_string()),
                Inline::Link {
                    label: "`RxError`".to_string(),
                    url: "https://docs.rs/x".to_string(),
                },
                Inline::Text(" since ".to_string()),
                Inline::Code("read".to_string()),
                Inline::Text(".".to_string()),
            ]
        );
    }

    #[test]
    fn unclosed_markers_stay_text() {
        // A stray backtick must not eat the rest of the message.
        assert_eq!(
            inline("a ` b ** c ["),
            vec![Inline::Text("a ` b ** c [".to_string())]
        );
    }
}
