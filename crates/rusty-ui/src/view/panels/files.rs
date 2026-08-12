//! The project's files, and an editor for them.
//!
//! The editor is a highlighted `<pre>` with a transparent `<textarea>` laid
//! exactly over it. That is how a text editor is built on the web without
//! pulling in Monaco or CodeMirror — both of which are npm, which this
//! repository does not have. The two layers share a font, a size and a line
//! height, so the caret sits where the glyph under it is.
//!
//! Completion, diagnostics and navigation come from rust-analyzer and are not
//! here yet. What is here is enough to read a generated project and change a
//! pin number without leaving the window.

use leptos::{ev, html, prelude::*};

use rusty_edit::{Document, Entry, Line, Span, Token};
use rusty_lsp::{DiagSeverity, FileDiagnostic};

use crate::{controller, state::AppState, view::components::Empty};

/// Shared by both layers. They must agree exactly or the caret drifts from the
/// character it is over, a column at a time, all the way across the line.
const FONT_SIZE: f64 = 12.5;
const LINE_HEIGHT: f64 = 19.0;

#[component]
pub fn FilesPanel() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.file_tree.with(Vec::is_empty) {
            controller::refresh_tree(state);
        }
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title="No project open"
                    detail="Open a folder to browse and edit what is in it."
                />
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 flex-1">
                <Tree />
                <Editor />
            </div>
        }
        .into_any()
    }
}

#[component]
fn Tree() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="flex w-[240px] flex-none flex-col border-r border-line bg-sidebar">
            <div class="flex items-center gap-2 px-3 py-2">
                <span class="flex-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    "Files"
                </span>
                <button
                    type="button"
                    title="Re-read the project"
                    class="rounded-[5px] px-1.5 py-0.5 text-footnote text-label-3 hover:text-label"
                    on:click=move |_| controller::refresh_tree(state)
                >
                    "Refresh"
                </button>
            </div>
            <div class="min-h-0 flex-1 overflow-auto pb-2">
                {move || {
                    let tree = state.file_tree.get();
                    if tree.is_empty() {
                        return view! {
                            <p class="px-3 text-footnote text-label-3">"Nothing to show."</p>
                        }
                            .into_any();
                    }
                    view! { <Level entries=tree depth=0 /> }.into_any()
                }}
            </div>
        </div>
    }
}

/// One level of the tree, and every level under it.
///
/// Returns `AnyView` rather than `impl IntoView` because it calls itself: an
/// opaque return type has no fixed point, and the compiler says so with
/// "recursive opaque type" pointing at the signature.
#[component]
fn Level(entries: Vec<Entry>, depth: usize) -> AnyView {
    let state = AppState::expect();

    entries
        .into_iter()
        .map(|entry| {
            let path = entry.path.clone();
            let is_dir = entry.is_dir;
            let children = entry.children.clone();

            let open = Signal::derive({
                let path = path.clone();
                move || state.expanded.with(|open| open.iter().any(|p| p == &path))
            });
            let selected = Signal::derive({
                let path = path.clone();
                move || {
                    state
                        .document
                        .with(|d| d.as_ref().is_some_and(|d| d.path == path))
                }
            });

            let activate = {
                let path = path.clone();
                move |_| {
                    if is_dir {
                        state.expanded.update(|open| {
                            match open.iter().position(|p| p == &path) {
                                Some(at) => {
                                    open.remove(at);
                                }
                                None => open.push(path.clone()),
                            }
                        });
                    } else {
                        controller::open_file(state, path.clone());
                    }
                }
            };

            view! {
                <button
                    type="button"
                    on:click=activate
                    style=format!("padding-left: {}px", 10 + depth * 12)
                    class=move || {
                        let base = "flex w-full items-center gap-1.5 py-[3px] pr-2 text-left \
                                    text-callout transition-colors";
                        if selected.get() {
                            format!("{base} bg-selection text-rust")
                        } else {
                            format!("{base} text-label-2 hover:bg-sunken hover:text-label")
                        }
                    }
                >
                    <span class="w-3 shrink-0 text-center text-footnote text-label-3">
                        {move || {
                            if !is_dir {
                                ""
                            } else if open.get() {
                                "▾"
                            } else {
                                "▸"
                            }
                        }}
                    </span>
                    <span class="truncate">{entry.name}</span>
                </button>

                <Show when=move || is_dir && open.get()>
                    <Level entries=children.clone() depth=depth + 1 />
                </Show>
            }
        })
        .collect_view()
        .into_any()
}

#[component]
fn Editor() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let Some(document) = state.document.get() else {
            return view! {
                <div class="flex min-w-0 flex-1 items-center justify-center">
                    <p class="text-callout text-label-3">"Choose a file."</p>
                </div>
            }
            .into_any();
        };

        if document.binary {
            return view! {
                <div class="flex min-w-0 flex-1 items-center justify-center px-6 text-center">
                    <p class="max-w-[44ch] text-callout leading-relaxed text-label-2">
                        "This is not a text file. rusty will not render a firmware image as \
                         characters — the result is noise, and for a large one it would take \
                         the window down with it."
                    </p>
                </div>
            }
            .into_any();
        }

        view! {
            <div class="flex min-w-0 flex-1 flex-col">
                <Header document=document.clone() />
                <Surface document=document />
            </div>
        }
        .into_any()
    }
}

#[component]
fn Header(document: Document) -> impl IntoView {
    let state = AppState::expect();
    let saved = document.text.clone();
    let dirty = Signal::derive(move || state.draft.with(|draft| draft != &saved));
    let language = document.language.clone();

    view! {
        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
            <span class="truncate font-mono text-footnote">{document.path}</span>
            {move || {
                dirty
                    .get()
                    .then(|| {
                        view! {
                            <span class="size-1.5 shrink-0 rounded-full bg-rust" title="Unsaved" />
                        }
                    })
            }}
            <span class="flex-1" />
            {document
                .truncated
                .then(|| {
                    view! {
                        <span class="text-footnote text-amber">
                            "shown to 5,000 lines"
                        </span>
                    }
                })}
            {language.map(|name| view! { <span class="text-footnote text-label-3">{name}</span> })}
            <button
                type="button"
                disabled=move || !dirty.get()
                title="Ctrl S"
                class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 transition-colors hover:text-label disabled:pointer-events-none disabled:opacity-35"
                on:click=move |_| controller::save_file(state)
            >
                "Save"
            </button>
        </div>
    }
}

/// The two stacked layers: highlighted text underneath, a transparent text
/// area on top taking every keystroke.
///
/// The painted layer follows `state.highlighted`, not the document: on each
/// keystroke the edited lines are patched in plainly so the text under the
/// caret is never stale, and a debounced re-highlight restores the colours.
/// Without the immediate patch, typed characters are invisible for a quarter
/// of a second — the textarea's own glyphs are transparent by design.
#[component]
fn Surface(document: Document) -> impl IntoView {
    let state = AppState::expect();
    let area: NodeRef<html::Textarea> = NodeRef::new();
    let path = document.path.clone();
    let read_only = document.truncated;

    // Both layers carry this verbatim. Any difference in font, size or line
    // height and the caret walks away from its glyph.
    let metrics = format!(
        "font-size: {FONT_SIZE}px; line-height: {LINE_HEIGHT}px; \
         font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
         tab-size: 4"
    );

    let on_input = move |event: ev::Event| {
        let new = event_target_value(&event);
        echo_edit(state, &new);
        state.draft.set(new);
        controller::schedule_pulse(state);
    };

    view! {
        <div class="relative min-h-0 flex-1 overflow-auto">
            <div class="flex min-h-full">
                // Line numbers scroll with the text rather than floating, so a
                // long file's numbers stay beside their lines.
                {
                    let metrics = metrics.clone();
                    move || {
                        let count = state.highlighted.with(Vec::len).max(1);
                        // Tailwind's border-box made a bare `width: 5ch` mean
                        // "5ch including 20px of padding", which left 4-digit
                        // numbers 14px of room — they clipped against the code
                        // column. The width now names the digits and adds the
                        // padding explicitly.
                        let digits = count.to_string().len().max(3);
                        view! {
                            <div
                                class="flex-none py-2 pr-2 pl-3 text-right text-label-4 select-none"
                                style=format!("{metrics}; width: calc({digits}ch + 20px)")
                            >
                                {(1..=count)
                                    .map(|n| view! { <div>{n.to_string()}</div> })
                                    .collect_view()}
                            </div>
                        }
                    }
                }

                <div class="relative min-w-0 flex-1">
                    <pre
                        class="pointer-events-none m-0 overflow-visible py-2 pr-4 pl-2 whitespace-pre"
                        style=metrics.clone()
                        aria-hidden="true"
                    >
                        {
                            let path = path.clone();
                            move || {
                                let diags = state
                                    .diagnostics
                                    .with(|by_file| by_file.get(&path).cloned())
                                    .unwrap_or_default();
                                state
                                    .highlighted
                                    .get()
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, line)| {
                                        view! {
                                            <div>
                                                {decorate(line, index as u32, &diags)}
                                                // An empty line still occupies
                                                // one, or the caret above sits a
                                                // row too high for the rest of
                                                // the file.
                                                {"\u{200b}"}
                                            </div>
                                        }
                                    })
                                    .collect_view()
                            }
                        }
                    </pre>

                    <textarea
                        node_ref=area
                        spellcheck="false"
                        autocapitalize="off"
                        autocomplete="off"
                        disabled=read_only
                        class="absolute inset-0 m-0 resize-none overflow-hidden border-0 bg-transparent py-2 pr-4 pl-2 whitespace-pre text-transparent caret-rust outline-none"
                        style=metrics
                        prop:value=move || state.draft.get()
                        on:input=on_input
                        on:keydown=move |event: ev::KeyboardEvent| {
                            if (event.ctrl_key() || event.meta_key())
                                && event.key().eq_ignore_ascii_case("s")
                            {
                                event.prevent_default();
                                controller::save_file(state);
                                return;
                            }
                            // A text area would move focus on Tab. In an editor
                            // that is never what was meant.
                            if event.key() == "Tab" {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    insert_tab(&element, state);
                                }
                            }
                        }
                    />
                </div>
            </div>
        </div>
    }
}

/// Patch the painted lines for an edit, without waiting for the re-highlight.
///
/// A line diff against what the paint currently shows: unchanged lines keep
/// their colours, edited ones are swapped for plain text immediately. The
/// debounced pulse recolours them a beat later — the same catch-up every
/// editor's highlighting does, built from a splice instead of a parser.
fn echo_edit(state: AppState, new: &str) {
    let old = state.echo_text.get_untracked();
    if old == new {
        return;
    }

    let old_lines: Vec<&str> = old.split('\n').collect();
    let new_lines: Vec<&str> = new.split('\n').collect();

    let prefix = old_lines
        .iter()
        .zip(&new_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = old_lines[prefix..]
        .iter()
        .rev()
        .zip(new_lines[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let replacement: Vec<Line> = new_lines[prefix..new_lines.len() - suffix]
        .iter()
        .map(|text| Line {
            spans: vec![Span {
                text: (*text).to_string(),
                token: Token::Plain,
            }],
        })
        .collect();

    state.highlighted.update(|lines| {
        // The paint can be shorter than the text (a truncated open); clamp so
        // a splice out of range cannot panic the whole window.
        let end = (old_lines.len() - suffix).min(lines.len());
        let start = prefix.min(end);
        lines.splice(start..end, replacement);
    });
    state.echo_text.set(new.to_string());
}

/// A line's spans with the diagnostics for that line woven in.
///
/// Splitting the highlight runs at the diagnostic's scalar columns keeps the
/// squiggle in the text flow — an absolutely-positioned overlay multiplied by
/// `ch` would drift on every CJK glyph, which is two columns wide.
fn decorate(line: Line, index: u32, diags: &[FileDiagnostic]) -> AnyView {
    let mut segments: Vec<(u32, u32, DiagSeverity, String)> = Vec::new();
    let length = line.spans.iter().map(|s| s.text.chars().count() as u32).sum::<u32>();
    for d in diags {
        if index < d.start_line || index > d.end_line {
            continue;
        }
        let from = if index == d.start_line { d.start_col } else { 0 };
        let to = if index == d.end_line { d.end_col } else { length };
        // A zero-width diagnostic still deserves a visible squiggle.
        let to = to.max(from + 1).min(length.max(from + 1));
        segments.push((from, to, d.severity, d.message.clone()));
    }

    if segments.is_empty() {
        return line
            .spans
            .into_iter()
            .map(|span| view! { <span class=class_of(span.token)>{span.text}</span> })
            .collect_view()
            .into_any();
    }

    // Worst severity wins where ranges overlap; DiagSeverity orders worst-first.
    let mark_at = |col: u32| -> Option<(DiagSeverity, &str)> {
        segments
            .iter()
            .filter(|(from, to, ..)| (*from..*to).contains(&col))
            .min_by_key(|(_, _, severity, _)| *severity)
            .map(|(_, _, severity, message)| (*severity, message.as_str()))
    };

    // One painted run: its text, its syntax class, and the squiggle over it.
    type Run = (String, Token, Option<(DiagSeverity, String)>);
    let mut out: Vec<Run> = Vec::new();
    let mut col = 0u32;
    for span in line.spans {
        for ch in span.text.chars() {
            let mark = mark_at(col).map(|(severity, message)| (severity, message.to_string()));
            match out.last_mut() {
                Some((text, token, last_mark))
                    if *token == span.token && *last_mark == mark =>
                {
                    text.push(ch);
                }
                _ => out.push((ch.to_string(), span.token, mark)),
            }
            col += 1;
        }
    }

    out.into_iter()
        .map(|(text, token, mark)| {
            let base = class_of(token);
            match mark {
                None => view! { <span class=base>{text}</span> }.into_any(),
                Some((severity, message)) => {
                    let squiggle = match severity {
                        DiagSeverity::Error => "diag-error",
                        DiagSeverity::Warning => "diag-warning",
                        _ => "diag-hint",
                    };
                    view! {
                        <span class=format!("{base} {squiggle}") title=message>{text}</span>
                    }
                        .into_any()
                }
            }
        })
        .collect_view()
        .into_any()
}

/// Put four spaces at the caret and leave it after them.
fn insert_tab(area: &web_sys::HtmlTextAreaElement, state: AppState) {
    // selectionStart counts UTF-16 units — it is a JS string index. Feeding it
    // to `replace_range` as bytes panics on the first CJK comment, so convert.
    let byte_at = |text: &str, utf16: usize| -> usize {
        let mut units = 0usize;
        for (offset, ch) in text.char_indices() {
            if units >= utf16 {
                return offset;
            }
            units += ch.len_utf16();
        }
        text.len()
    };

    let start_units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    let end_units = area.selection_end().ok().flatten().unwrap_or(0) as usize;
    let mut text = state.draft.get_untracked();
    let start = byte_at(&text, start_units);
    let end = byte_at(&text, end_units).max(start);

    text.replace_range(start..end, "    ");
    echo_edit(state, &text);
    state.draft.set(text.clone());
    area.set_value(&text);
    let at = (start_units + 4) as u32;
    let _ = area.set_selection_start(Some(at));
    let _ = area.set_selection_end(Some(at));
    controller::schedule_pulse(state);
}

/// Token to a class the stylesheet owns.
///
/// Classes rather than inline colours, so the palette lives with the theme and
/// a light window is not painted with a dark theme's syntax colours.
fn class_of(token: Token) -> &'static str {
    match token {
        Token::Plain => "text-label",
        Token::Keyword => "tok-keyword",
        Token::Str => "tok-string",
        Token::Number => "tok-number",
        Token::Comment => "tok-comment",
        Token::Type => "tok-type",
        Token::Function => "tok-function",
        Token::Macro => "tok-macro",
        Token::Punctuation => "tok-punctuation",
        Token::Variable => "text-label",
    }
}
