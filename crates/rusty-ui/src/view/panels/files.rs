//! The project's files, and an editor for them.
//!
//! The editor is a highlighted `<pre>` with a transparent `<textarea>` laid
//! exactly over it. That is how a text editor is built on the web without
//! pulling in Monaco or CodeMirror — both of which are npm, which this
//! repository does not have. The two layers share a font, a size and a line
//! height, so the caret sits where the glyph under it is.
//!
//! Completion, diagnostics, navigation and the signature card come from
//! rust-analyzer over `rusty-lsp`; saving runs the buffer through rustfmt
//! first. The editor half of the IDE lives in this file.

use leptos::{ev, html, prelude::*};
use wasm_bindgen::JsCast;

use rusty_edit::{Document, Entry, Line, Span, Token};
use rusty_lsp::{CompletionItem, DiagSeverity, FileDiagnostic, SemanticSpan};

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, Empty, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

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
    let tree_menu = RwSignal::new(None::<(f64, f64, TreeTarget)>);
    provide_context(TreeMenu(tree_menu));

    view! {
        <div class="flex w-[240px] flex-none flex-col border-r border-line bg-sidebar">
            <div class="flex items-center gap-2 px-3 py-2">
                <span class="flex-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    "Files"
                </span>
                <button
                    type="button"
                    title="Re-read the project"
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| controller::refresh_tree(state)
                >
                    <IconView icon=Icon::Refresh size=13 />
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

            {move || {
                let (x, y, target) = tree_menu.get()?;
                let close = Callback::new(move |_| tree_menu.set(None));
                let path = target.path.clone();
                let (open_path, copy_path, search_path) =
                    (path.clone(), path.clone(), path.clone());
                let is_dir = target.is_dir;
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label=if is_dir { "Expand or collapse" } else { "Open" }
                                on_select=Callback::new(move |_| {
                                    if is_dir {
                                        state
                                            .expanded
                                            .update(|open| {
                                                match open.iter().position(|p| p == &open_path) {
                                                    Some(at) => {
                                                        open.remove(at);
                                                    }
                                                    None => open.push(open_path.clone()),
                                                }
                                            });
                                    } else {
                                        controller::open_file(state, open_path.clone());
                                    }
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label="Search in this scope"
                                on_select=Callback::new(move |_| {
                                    search_within(state, &search_path, is_dir);
                                    tree_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label="Copy path"
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&copy_path);
                                    tree_menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
    }
}

/// Where the file tree's right-click menu is, and what it is about.
///
/// Context rather than a prop: the tree renders itself recursively, and
/// threading a signal through every level would be a parameter that exists
/// only because of how the rows are drawn.
#[derive(Clone, Copy)]
struct TreeMenu(RwSignal<Option<(f64, f64, TreeTarget)>>);

#[derive(Clone)]
struct TreeTarget {
    path: String,
    is_dir: bool,
}

use crate::view::components::copy_to_clipboard;

/// Scope the project search to one path and go there.
fn search_within(state: AppState, path: &str, is_dir: bool) {
    state.search_include.set(if is_dir {
        format!("{path}/**")
    } else {
        path.to_string()
    });
    state.active_panel.set("search".to_string());
}

/// Format with rustfmt and save, landing the caret where the eye already is.
/// Shared by Ctrl+S and the editor's own menu, so the two cannot drift.
fn format_and_save(state: AppState, area: NodeRef<html::Textarea>) {
    let caret = area
        .get_untracked()
        .and_then(|element| caret_line_col(&element, &state.draft.get_untracked()));
    controller::format_then_save(state, caret, move |text, caret| {
        record_edit(state);
        echo_edit(state, text);
        let Some(element) = area.get_untracked() else {
            return;
        };
        element.set_value(text);
        // The old caret's line and column, clamped into the reformatted
        // text. rustfmt moves lines, not the one being typed on, so this
        // lands where the eye already is.
        if let Some((line, col)) = caret {
            let last = text.split('\n').count().saturating_sub(1);
            let line = (line as usize).min(last) as u32;
            let width = text
                .split('\n')
                .nth(line as usize)
                .map(|l| l.chars().count() as u32)
                .unwrap_or(0);
            let unit = utf16_offset_of(text, line, col.min(width));
            let _ = element.set_selection_start(Some(unit));
            let _ = element.set_selection_end(Some(unit));
        }
    });
}

/// UTF-16 code units in `text` — what a textarea counts selections in.
fn utf16_len(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// Read the clipboard and drop it in at the caret. Async because that is the
/// only way a browser hands over the clipboard; a refusal lands nowhere,
/// which is what a blocked paste should do.
fn paste_at_caret(state: AppState, area: NodeRef<html::Textarea>) {
    use wasm_bindgen_futures::JsFuture;

    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = window.navigator().clipboard().read_text();
    leptos::task::spawn_local(async move {
        let Ok(value) = JsFuture::from(promise).await else {
            return;
        };
        let Some(text) = value.as_string() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        if let Some(element) = area.get_untracked() {
            insert_at_caret(&element, state, &text);
        }
    });
}

/// The editor's selection as (byte range, text), when there is one.
fn selection_of(area: &web_sys::HtmlTextAreaElement, text: &str) -> Option<(usize, usize, String)> {
    let from = area.selection_start().ok().flatten()? as usize;
    let to = area.selection_end().ok().flatten()? as usize;
    if to <= from {
        return None;
    }
    let (from, to) = (byte_of_utf16(text, from), byte_of_utf16(text, to));
    Some((from, to, text[from..to].to_string()))
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

            let menu = {
                let path = path.clone();
                move |event: ev::MouseEvent| {
                    event.prevent_default();
                    event.stop_propagation();
                    let TreeMenu(menu) = expect_context::<TreeMenu>();
                    menu.set(Some((
                        f64::from(event.client_x()),
                        f64::from(event.client_y()),
                        TreeTarget {
                            path: path.clone(),
                            is_dir,
                        },
                    )));
                }
            };

            view! {
                <button
                    type="button"
                    on:click=activate
                    on:contextmenu=menu
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
pub(super) fn Editor() -> impl IntoView {
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
                <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                    <TabStrip />
                    <div class="flex flex-1 items-center justify-center px-6 text-center">
                        <p class="max-w-[44ch] text-callout leading-relaxed text-label-2">
                            "This is not a text file. rusty will not render a firmware image as \
                             characters — the result is noise, and for a large one it would take \
                             the window down with it."
                        </p>
                    </div>
                </div>
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 min-w-0 flex-1 flex-col">
                <TabStrip />
                <Header document=document.clone() />
                <Surface document=document />
            </div>
        }
        .into_any()
    }
}

/// The open editors, one tab each. Clicking fronts a tab with its draft and
/// caret exactly as left; the cross closes it, asking first when unsaved
/// work would go with it.
#[component]
fn TabStrip() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<(f64, f64, String)>);

    view! {
        <div class="flex flex-none items-stretch overflow-x-auto border-b border-line bg-sidebar">
            {move || {
                let active = state
                    .document
                    .with(|d| d.as_ref().map(|d| d.path.clone()))
                    .unwrap_or_default();
                state
                    .tabs
                    .get()
                    .into_iter()
                    .map(|path| {
                        let name = path
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(path.as_str())
                            .to_string();
                        let is_active = path == active;
                        // Dirty is per-tab: the active one compares live
                        // draft to document, a parked one compares its
                        // stashed pair.
                        let dirty = {
                            let path = path.clone();
                            Signal::derive(move || {
                                let on_screen = state
                                    .document
                                    .with(|d| d.as_ref().map(|d| d.path.clone()))
                                    .as_deref()
                                    == Some(path.as_str());
                                if on_screen {
                                    state.document.with(|d| {
                                        d.as_ref().is_some_and(|d| {
                                            !d.read_only
                                                && state
                                                    .draft
                                                    .with(|draft| draft != &d.text)
                                        })
                                    })
                                } else {
                                    state.parked.with(|parked| {
                                        parked
                                            .iter()
                                            .find(|e| e.document.path == path)
                                            .is_some_and(|e| {
                                                !e.document.read_only
                                                    && e.draft != e.document.text
                                            })
                                    })
                                }
                            })
                        };
                        let activate = {
                            let path = path.clone();
                            move |_| controller::activate_tab(state, path.clone())
                        };
                        let close = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                event.stop_propagation();
                                controller::close_tab(state, path.clone());
                            }
                        };
                        let middle_close = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                if event.button() == 1 {
                                    event.prevent_default();
                                    controller::close_tab(state, path.clone());
                                }
                            }
                        };
                        let tab_class = if is_active {
                            "group flex cursor-pointer items-center gap-1.5 border-r border-line \
                             bg-canvas px-2.5 py-1.5 font-mono text-footnote text-label"
                        } else {
                            "group flex cursor-pointer items-center gap-1.5 border-r border-line \
                             px-2.5 py-1.5 font-mono text-footnote text-label-3 hover:bg-sunken \
                             hover:text-label-2"
                        };
                        let open_menu = {
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                event.prevent_default();
                                event.stop_propagation();
                                menu.set(Some((
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                    path.clone(),
                                )));
                            }
                        };
                        view! {
                            <div
                                title=path.clone()
                                on:click=activate
                                on:auxclick=middle_close
                                on:contextmenu=open_menu
                                class=tab_class
                            >
                                <span class="max-w-[18ch] truncate">{name}</span>
                                {move || {
                                    dirty
                                        .get()
                                        .then(|| {
                                            view! {
                                                <span
                                                    class="size-1.5 shrink-0 rounded-full bg-rust"
                                                    title="Unsaved"
                                                />
                                            }
                                        })
                                }}
                                <button
                                    type="button"
                                    title="Close"
                                    on:click=close
                                    class="rounded-[4px] px-0.5 leading-none text-label-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-selection hover:text-label"
                                >
                                    "×"
                                </button>
                            </div>
                        }
                    })
                    .collect_view()
            }}

            {move || {
                let (x, y, path) = menu.get()?;
                let close = Callback::new(move |_| menu.set(None));
                let (this, others, copy) = (path.clone(), path.clone(), path.clone());
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label="Close"
                                shortcut="Ctrl+W"
                                on_select=Callback::new(move |_| {
                                    controller::close_tab(state, this.clone());
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label="Close others"
                                on_select=Callback::new(move |_| {
                                    for open in state.tabs.get_untracked() {
                                        if open != others {
                                            controller::close_tab(state, open);
                                        }
                                    }
                                    menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label="Copy path"
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&copy);
                                    menu.set(None);
                                })
                            />
                        </ContextMenu>
                    },
                )
            }}
        </div>
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
                .read_only
                .then(|| {
                    view! {
                        <span
                            class="rounded-full bg-sunken px-2 text-footnote text-label-2"
                            title="A dependency's source. rusty will not edit the shared \
                                   registry cache — a fix made here would bleed into every \
                                   project on the machine and vanish on the next update."
                        >
                            "read-only"
                        </span>
                    }
                })}
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
    let scroller: NodeRef<html::Div> = NodeRef::new();
    let path = document.path.clone();
    let read_only = document.truncated || document.read_only;
    // Hover only means something where a language server is listening.
    let is_rust = path.ends_with(".rs");

    // The cell the mouse was last over, and a generation so only the newest
    // 400ms-old position asks the server. Hover is ambient: it must cost
    // nothing while the mouse is moving and only speak once it has settled.
    let hover_cell = RwSignal::new(None::<(u32, u32)>);
    let hover_gen = RwSignal::new(0u64);
    // True while the pointer is over the card itself. Reading the card —
    // scrolling it, selecting from it — must not count as leaving.
    let on_card = RwSignal::new(false);
    let editor_menu = RwSignal::new(None::<(f64, f64)>);

    // The coding toolbar: what a person editing firmware reaches for. Save
    // rides the same format-then-save path as Ctrl+S; Build shares the one
    // session slot; the last two are the places this work goes next.
    let toolbar = Callback::new(move |_| {
        let running = state.session_running;
        view! {
            <button
                type="button"
                title="Format and save (Ctrl+S)"
                disabled=read_only
                on:click=move |_| format_and_save(state, area)
                class="grid size-7 place-items-center rounded-[6px] text-rust hover:bg-sunken disabled:pointer-events-none disabled:opacity-40"
            >
                <IconView icon=Icon::Save size=15 />
            </button>
            <button
                type="button"
                title="Build — cargo build --release, output in the dock"
                disabled=move || running.get()
                on:click=move |_| controller::build_project(state)
                class="grid size-7 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label disabled:pointer-events-none disabled:opacity-40"
            >
                <IconView icon=Icon::Hammer size=15 />
            </button>
            <span class="mx-1 h-5 w-px bg-line" />
            <button
                type="button"
                title="Flash the board"
                on:click=move |_| state.active_panel.set("flash".to_string())
                class="grid size-7 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Flash size=15 />
            </button>
            <button
                type="button"
                title="Run in the simulator"
                on:click=move |_| state.active_panel.set("simulate".to_string())
                class="grid size-7 place-items-center rounded-[6px] text-label-2 hover:bg-sunken hover:text-label"
            >
                <IconView icon=Icon::Play size=15 />
            </button>
            {move || {
                running
                    .get()
                    .then(|| {
                        view! {
                            <span class="text-footnote text-label-3">
                                "running — output in the dock"
                            </span>
                        }
                    })
            }}
        }
        .into_any()
    });
    Effect::new(move |_| {
        state.toolbar.set(Some(toolbar));
    });
    on_cleanup(move || state.toolbar.set(None));
    // Which completion row the keyboard is on. Reset when a new popup arrives.
    let picked = RwSignal::new(0usize);
    Effect::new(move |_| {
        let _ = state.completion.get();
        picked.set(0);
    });
    let picked_action = RwSignal::new(0usize);
    Effect::new(move |_| {
        let _ = state.actions.get();
        picked_action.set(0);
    });
    // The strip is remembered whenever it changes, so a crash loses nothing.
    Effect::new(move |_| {
        let _ = state.tabs.get();
        let _ = state.document.get();
        controller::remember_tabs(state);
    });

    // Apply a pending goto once this document is the one on screen.
    {
        let path = path.clone();
        Effect::new(move |_| {
            let Some(target) = state.reveal.get() else {
                return;
            };
            if target.path != path || state.highlighted.with(Vec::is_empty) {
                return;
            }
            state.reveal.set(None);
            // Deferred one tick: on a freshly mounted editor the textarea's
            // value lands after this effect runs, and a selection set before
            // the value is snapped to the end when the text arrives — the
            // caret ended at EOF instead of the target line, every time a
            // goto opened a file that was not already on screen.
            set_timeout(
                move || {
                    let Some(element) = area.get_untracked() else {
                        return;
                    };
                    let offset =
                        utf16_offset_of(&state.draft.get_untracked(), target.line, target.col);
                    // preventScroll, because the browser's own focus scroll
                    // lands asynchronously and overwrote the deliberate one
                    // below — the jump ended wherever Chrome felt like.
                    let options = web_sys::FocusOptions::new();
                    options.set_prevent_scroll(true);
                    let _ = element.focus_with_options(&options);
                    let _ = element.set_selection_start(Some(offset));
                    let _ = element.set_selection_end(Some(offset));
                    if let Some(scroller) = scroller.get_untracked() {
                        // A third of the viewport above the target line, so
                        // the jump lands in context rather than at the top
                        // edge.
                        let top = f64::from(target.line) * LINE_HEIGHT - 120.0;
                        scroller.set_scroll_top(top.max(0.0) as i32);
                    }
                },
                std::time::Duration::ZERO,
            );
        });
    }

    // Both layers carry this verbatim. Any difference in font, size or line
    // height and the caret walks away from its glyph.
    let metrics = format!(
        "font-size: {FONT_SIZE}px; line-height: {LINE_HEIGHT}px; \
         font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
         tab-size: 4"
    );

    let on_input = {
        let path = path.clone();
        move |event: ev::Event| {
            let new = event_target_value(&event);
            record_edit(state);
            echo_edit(state, &new);
            state.draft.set(new.clone());
            controller::schedule_pulse(state);
            if let Some(element) = area.get_untracked() {
                keep_caret_in_view(&element, state, scroller);
            }

            // Completion triggers, judged by the character behind the caret.
            if !is_rust {
                return;
            }
            let Some(element) = area.get_untracked() else { return };
            let Some((line, col)) = caret_line_col(&element, &new) else {
                return;
            };
            let line_text = new.split('\n').nth(line as usize).unwrap_or_default();
            let before: Vec<char> = line_text.chars().take(col as usize).collect();
            let last = before.last().copied();

            let popup_open = state.completion.with_untracked(Option::is_some);
            match last {
                // `foo.` and `foo::` are the moments completion answers a
                // question the typist actually has.
                Some('.') => {
                    controller::request_completion(state, path.clone(), line, col, col);
                }
                Some(':') if before.len() >= 2 && before[before.len() - 2] == ':' => {
                    controller::request_completion(state, path.clone(), line, col, col);
                }
                // Inside a word, the open popup narrows reactively — the
                // filter derives from the draft, so nothing to do here.
                Some(c) if c.is_alphanumeric() || c == '_' => {}
                // Anything else ends the word the popup was about.
                _ => {
                    if popup_open {
                        state.completion.set(None);
                    }
                }
            }

            // The signature card follows the parentheses.
            match last {
                Some('(') | Some(',') => {
                    controller::request_signature(state, path.clone(), line, col);
                }
                Some(')') => state.signature.set(None),
                _ => {}
            }
        }
    };

    view! {
        <div class="relative flex min-h-0 flex-1 flex-col">
            <FindBar area=area scroller=scroller />

            // The editor's own menu. It keeps the clipboard three every text
            // box has, and adds what only this editor knows: where a name is
            // defined, what rust-analyzer would fix, how the file formats.
            {
                let path = path.clone();
                move || {
                    let (x, y) = editor_menu.get()?;
                    let close = Callback::new(move |_| editor_menu.set(None));
                    let path = path.clone();
                    let has_selection = area
                        .get_untracked()
                        .zip(Some(state.draft.get_untracked()))
                        .and_then(|(element, text)| selection_of(&element, &text))
                        .is_some();
                    let (goto_path, fix_path) = (path.clone(), path.clone());
                    Some(
                        view! {
                            <ContextMenu x=x y=y on_close=close>
                                <MenuItem
                                    label="Cut"
                                    shortcut="Ctrl+X"
                                    disabled=!has_selection || read_only
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked() {
                                            let text = state.draft.get_untracked();
                                            if let Some((from, to, picked)) =
                                                selection_of(&element, &text)
                                            {
                                                copy_to_clipboard(&picked);
                                                record_edit(state);
                                                let mut next = text.clone();
                                                next.replace_range(from..to, "");
                                                echo_edit(state, &next);
                                                state.draft.set(next.clone());
                                                element.set_value(&next);
                                                let caret = utf16_len(&next[..from]);
                                                let _ = element.set_selection_start(Some(caret));
                                                let _ = element.set_selection_end(Some(caret));
                                                controller::schedule_pulse(state);
                                            }
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label="Copy"
                                    shortcut="Ctrl+C"
                                    disabled=!has_selection
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked() {
                                            let text = state.draft.get_untracked();
                                            if let Some((_, _, picked)) =
                                                selection_of(&element, &text)
                                            {
                                                copy_to_clipboard(&picked);
                                            }
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label="Paste"
                                    shortcut="Ctrl+V"
                                    disabled=read_only
                                    on_select=Callback::new(move |_| {
                                        paste_at_caret(state, area);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label="Go to definition"
                                    shortcut="Ctrl+Click"
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked()
                                            && let Some((line, col)) = caret_line_col(
                                                &element,
                                                &state.draft.get_untracked(),
                                            )
                                        {
                                            controller::goto_definition(
                                                state,
                                                goto_path.clone(),
                                                line,
                                                col,
                                            );
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label="Quick fix"
                                    shortcut="Ctrl+."
                                    disabled=!is_rust
                                    on_select=Callback::new(move |_| {
                                        if let Some(element) = area.get_untracked()
                                            && let Some((line, col)) = caret_line_col(
                                                &element,
                                                &state.draft.get_untracked(),
                                            )
                                        {
                                            controller::request_actions(
                                                state,
                                                fix_path.clone(),
                                                line,
                                                col,
                                            );
                                        }
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label="Format and save"
                                    shortcut="Ctrl+S"
                                    disabled=read_only
                                    on_select=Callback::new(move |_| {
                                        format_and_save(state, area);
                                        editor_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label="Find in file"
                                    shortcut="Ctrl+F"
                                    on_select=Callback::new(move |_| {
                                        state.find_open.set(true);
                                        editor_menu.set(None);
                                    })
                                />
                            </ContextMenu>
                        },
                    )
                }
            }
        <div node_ref=scroller class="relative min-h-0 flex-1 overflow-auto">
            // w-max: the row is as wide as the longest line, so the textarea
            // overlay (inset-0 in the column beside the gutter) covers every
            // glyph. At viewport width, a long line overflowed the column and
            // the caret inside it lived in the textarea's own hidden scroll —
            // drifting away from the echoed text.
            <div class="flex min-h-full w-max min-w-full">
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
                    // Find matches, washed under the text. Rectangles rather
                    // than woven spans: the wash must not disturb the span
                    // structure the caret math and diagnostics rely on.
                    {move || {
                        if !state.find_open.get() {
                            return ().into_any();
                        }
                        let text = state.draft.get();
                        let query = state.find_query.get();
                        let case = state.find_case.get();
                        let matches = find_matches(&text, &query, case);
                        if matches.is_empty() {
                            return ().into_any();
                        }
                        let current = state.find_index.get().min(matches.len() - 1);
                        matches
                            .iter()
                            .take(500)
                            .enumerate()
                            .map(|(index, (from, to))| {
                                let (line, col) = line_col_of_byte(&text, *from);
                                let (_, end_col) = line_col_of_byte(&text, *to);
                                let x = 8.0 + column_px(&text, line, col);
                                let width =
                                    (column_px(&text, line, end_col) - column_px(&text, line, col))
                                        .max(2.0);
                                let y = 8.0 + f64::from(line) * LINE_HEIGHT;
                                let wash = if index == current {
                                    "pointer-events-none absolute rounded-[3px] bg-amber-fill"
                                } else {
                                    "pointer-events-none absolute rounded-[3px] bg-selection"
                                };
                                view! {
                                    <div
                                        class=wash
                                        style=format!(
                                            "left: {x}px; top: {y}px; width: {width}px; height: {LINE_HEIGHT}px",
                                        )
                                    />
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
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
                                // The compiler's colours, when they have
                                // arrived for this document.
                                let semantic = state
                                    .semantic
                                    .with(|s| {
                                        s.as_ref()
                                            .filter(|(for_path, _)| for_path == &path)
                                            .map(|(_, spans)| spans.clone())
                                    })
                                    .unwrap_or_default();
                                state
                                    .highlighted
                                    .get()
                                    .into_iter()
                                    .enumerate()
                                    .map(|(index, line)| {
                                        let line = overlay_semantic(
                                            line,
                                            index as u32,
                                            &semantic,
                                        );
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
                        id="editor-area"
                        spellcheck="false"
                        autocapitalize="off"
                        autocomplete="off"
                        disabled=read_only
                        class="absolute inset-0 m-0 resize-none overflow-hidden border-0 bg-transparent py-2 pr-4 pl-2 whitespace-pre text-transparent caret-rust outline-none"
                        style=metrics
                        prop:value=move || state.draft.get()
                        on:contextmenu=move |event: ev::MouseEvent| {
                            event.prevent_default();
                            editor_menu
                                .set(Some((
                                    f64::from(event.client_x()),
                                    f64::from(event.client_y()),
                                )));
                        }
                        on:scroll=move |_| {
                            let Some(element) = area.get_untracked() else {
                                return;
                            };
                            let (top, left) = (element.scroll_top(), element.scroll_left());
                            if top != 0 || left != 0 {
                                if let Some(outer) = scroller.get_untracked() {
                                    outer.set_scroll_top(outer.scroll_top() + top);
                                    outer.set_scroll_left(outer.scroll_left() + left);
                                }
                                element.set_scroll_top(0);
                                element.set_scroll_left(0);
                            }
                        }
                        on:input=on_input
                        on:mousedown={
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                // Ctrl+Click asks where this is defined — the
                                // gesture every editor has taught.
                                if !(event.ctrl_key() || event.meta_key()) || !is_rust {
                                    state.completion.set(None);
                                    state.signature.set(None);
                                    state.actions.set(None);
                                    return;
                                }
                                event.prevent_default();
                                if let Some((line, col)) = cell_under(
                                    &state.draft.get_untracked(),
                                    event.offset_x() as f64,
                                    event.offset_y() as f64,
                                ) {
                                    controller::goto_definition(
                                        state,
                                        path.clone(),
                                        line,
                                        col,
                                    );
                                }
                            }
                        }
                        on:mousemove={
                            let path = path.clone();
                            move |event: ev::MouseEvent| {
                                if !is_rust {
                                    return;
                                }
                                let cell = cell_under(
                                    &state.draft.get_untracked(),
                                    event.offset_x() as f64,
                                    event.offset_y() as f64,
                                );
                                if hover_cell.get_untracked() == cell {
                                    return;
                                }
                                hover_cell.set(cell);

                                // Inside the shown token, there is nothing to
                                // dismiss and nothing to re-request.
                                let inside = state.hover.with_untracked(|h| {
                                    h.as_ref().is_some_and(|(_, range, _)| {
                                        cell.is_some_and(|(l, c)| within(range, l, c))
                                    })
                                });
                                if inside {
                                    return;
                                }

                                // Outside it: a short grace before the card
                                // goes, so the pointer can cross the gap onto
                                // the card without killing it en route.
                                let generation = hover_gen.get_untracked() + 1;
                                hover_gen.set(generation);
                                if state.hover.with_untracked(Option::is_some) {
                                    set_timeout(
                                        move || {
                                            if hover_gen.get_untracked() == generation
                                                && !on_card.get_untracked()
                                            {
                                                state.hover.set(None);
                                            }
                                        },
                                        std::time::Duration::from_millis(300),
                                    );
                                }

                                let Some((line, col)) = cell else { return };
                                let path = path.clone();
                                set_timeout(
                                    move || {
                                        if hover_gen.get_untracked() == generation
                                            && hover_cell.get_untracked() == Some((line, col))
                                        {
                                            controller::request_hover(state, path, line, col);
                                        }
                                    },
                                    std::time::Duration::from_millis(400),
                                );
                            }
                        }
                        on:mouseleave=move |_| {
                            hover_cell.set(None);
                            let generation = hover_gen.get_untracked() + 1;
                            hover_gen.set(generation);
                            set_timeout(
                                move || {
                                    if hover_gen.get_untracked() == generation
                                        && !on_card.get_untracked()
                                    {
                                        state.hover.set(None);
                                    }
                                },
                                std::time::Duration::from_millis(300),
                            );
                        }
                        on:keydown={
                            let path = path.clone();
                            move |event: ev::KeyboardEvent| {
                            // While an IME is composing, Enter confirms the
                            // candidate and Tab moves through them. Stealing
                            // either would break Chinese input entirely.
                            if event.is_composing() {
                                return;
                            }
                            // The actions popup owns its keys while it is up.
                            if state.actions.with_untracked(Option::is_some) {
                                match event.key().as_str() {
                                    "ArrowDown" => {
                                        event.prevent_default();
                                        picked_action.update(|i| *i += 1);
                                        return;
                                    }
                                    "ArrowUp" => {
                                        event.prevent_default();
                                        picked_action.update(|i| *i = i.saturating_sub(1));
                                        return;
                                    }
                                    "Enter" => {
                                        event.prevent_default();
                                        if let Some(element) = area.get_untracked() {
                                            apply_action(
                                                state,
                                                &element,
                                                picked_action.get_untracked(),
                                            );
                                        }
                                        return;
                                    }
                                    "Escape" => {
                                        event.prevent_default();
                                        event.stop_propagation();
                                        state.actions.set(None);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            // Ctrl+. asks what the server can fix here.
                            if (event.ctrl_key() || event.meta_key())
                                && event.key() == "."
                                && is_rust
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let text = state.draft.get_untracked();
                                    if let Some((line, col)) = caret_line_col(&element, &text) {
                                        state.completion.set(None);
                                        controller::request_actions(
                                            state,
                                            path.clone(),
                                            line,
                                            col,
                                        );
                                    }
                                }
                                return;
                            }
                            // The popup owns its keys while it is up.
                            if state.completion.with_untracked(Option::is_some) {
                                match event.key().as_str() {
                                    "ArrowDown" => {
                                        event.prevent_default();
                                        picked.update(|i| *i += 1);
                                        return;
                                    }
                                    "ArrowUp" => {
                                        event.prevent_default();
                                        picked.update(|i| *i = i.saturating_sub(1));
                                        return;
                                    }
                                    "Enter" | "Tab" => {
                                        event.prevent_default();
                                        if let Some(element) = area.get_untracked() {
                                            accept_completion(
                                                state,
                                                &element,
                                                picked.get_untracked(),
                                            );
                                        }
                                        return;
                                    }
                                    "Escape" => {
                                        event.prevent_default();
                                        // Swallowed here so the window's own
                                        // Escape handling does not also close
                                        // an overlay behind the editor.
                                        event.stop_propagation();
                                        state.completion.set(None);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            if event.key() == "Escape"
                                && state.signature.with_untracked(Option::is_some)
                            {
                                event.prevent_default();
                                event.stop_propagation();
                                state.signature.set(None);
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && (event.key().eq_ignore_ascii_case("f")
                                    || event.key().eq_ignore_ascii_case("h"))
                            {
                                event.prevent_default();
                                // Prefill from the selection, as every editor
                                // does — finding the thing under the cursor is
                                // the whole gesture.
                                if let Some(element) = area.get_untracked() {
                                    let text = state.draft.get_untracked();
                                    let from = element
                                        .selection_start()
                                        .ok()
                                        .flatten()
                                        .unwrap_or(0) as usize;
                                    let to = element
                                        .selection_end()
                                        .ok()
                                        .flatten()
                                        .unwrap_or(0) as usize;
                                    if to > from {
                                        let picked = text
                                            [byte_of_utf16(&text, from)..byte_of_utf16(&text, to)]
                                            .to_string();
                                        if !picked.contains('\n') && !picked.is_empty() {
                                            state.find_query.set(picked);
                                            state.find_index.set(0);
                                        }
                                    }
                                }
                                state.find_open.set(true);
                                if event.key().eq_ignore_ascii_case("h") {
                                    state.find_replace_open.set(true);
                                }
                                return;
                            }
                            if event.key() == "F3" && state.find_open.get_untracked() {
                                event.prevent_default();
                                find_jump(state, scroller, if event.shift_key() { -1 } else { 1 });
                                return;
                            }
                            if event.key() == "Escape"
                                && state.find_open.get_untracked()
                                && state
                                    .completion
                                    .with_untracked(Option::is_none)
                                && state.signature.with_untracked(Option::is_none)
                            {
                                event.prevent_default();
                                state.find_open.set(false);
                                state.find_replace_open.set(false);
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && event.key().eq_ignore_ascii_case("z")
                                && !event.shift_key()
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    apply_history(&element, state, scroller, true);
                                }
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && (event.key().eq_ignore_ascii_case("y")
                                    || (event.key().eq_ignore_ascii_case("z")
                                        && event.shift_key()))
                            {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    apply_history(&element, state, scroller, false);
                                }
                                return;
                            }
                            // Ctrl+Space asks without a trigger character.
                            if event.ctrl_key() && event.key() == " " {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let text = state.draft.get_untracked();
                                    if let Some((line, col)) = caret_line_col(&element, &text) {
                                        let start = word_start_before(&text, line, col);
                                        controller::request_completion(
                                            state,
                                            path.clone(),
                                            line,
                                            col,
                                            start,
                                        );
                                    }
                                }
                                return;
                            }
                            if (event.ctrl_key() || event.meta_key())
                                && event.key().eq_ignore_ascii_case("s")
                            {
                                event.prevent_default();
                                format_and_save(state, area);
                                return;
                            }
                            if event.key() == "Enter" {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    let caret = caret_byte(&element, state);
                                    let insert =
                                        newline_indent(&state.draft.get_untracked(), caret);
                                    insert_at_caret(&element, state, &insert);
                                }
                                return;
                            }
                            // A text area would move focus on Tab. In an editor
                            // that is never what was meant.
                            if event.key() == "Tab" {
                                event.prevent_default();
                                if let Some(element) = area.get_untracked() {
                                    insert_at_caret(&element, state, "    ");
                                }
                            }
                        }}
                    />

                    // What the server said about the token the mouse settled
                    // on. Interactive: long documentation scrolls inside it,
                    // and reading is not leaving.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, range, text)) = state.hover.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
                            let x = 8.0
                                + column_px(
                                    &state.draft.get_untracked(),
                                    range.start_line,
                                    range.start_col,
                                );
                            let y = 8.0 + f64::from(range.end_line + 1) * LINE_HEIGHT + 2.0;
                            view! {
                                <div
                                    class="absolute z-20 max-w-[70ch] overflow-y-auto rounded-[8px] bg-raised px-3 py-2 font-mono text-footnote leading-relaxed whitespace-pre-wrap shadow-2xl ring-1 ring-line-strong select-text"
                                    style=format!("left: {x}px; top: {y}px; max-height: 40vh")
                                    on:mouseenter=move |_| on_card.set(true)
                                    on:mouseleave=move |_| {
                                        on_card.set(false);
                                        state.hover.set(None);
                                    }
                                >
                                    {hover_parts(&text)}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The quick-fix popup, anchored under its line.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, line, fixes)) = state.actions.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
                            let chosen = picked_action.get().min(fixes.len().saturating_sub(1));
                            let y = 8.0 + f64::from(line + 1) * LINE_HEIGHT + 2.0;
                            view! {
                                <div
                                    class="absolute z-20 min-w-[280px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                                    style=format!("left: 48px; top: {y}px")
                                >
                                    {fixes
                                        .into_iter()
                                        .enumerate()
                                        .map(|(index, fix)| {
                                            let selected = index == chosen;
                                            let kind = fix.kind.clone().unwrap_or_default();
                                            view! {
                                                <button
                                                    type="button"
                                                    on:mousedown=move |event: ev::MouseEvent| {
                                                        event.prevent_default();
                                                        event.stop_propagation();
                                                        if let Some(element) =
                                                            area.get_untracked()
                                                        {
                                                            apply_action(
                                                                state, &element, index,
                                                            );
                                                        }
                                                    }
                                                    class=if selected {
                                                        "flex w-full items-baseline gap-2 bg-selection px-2.5 py-0.5 text-left text-rust"
                                                    } else {
                                                        "flex w-full items-baseline gap-2 px-2.5 py-0.5 text-left text-label-2"
                                                    }
                                                >
                                                    <span class="shrink-0">{fix.title.clone()}</span>
                                                    <span class="min-w-0 flex-1 truncate text-right text-label-3">
                                                        {kind}
                                                    </span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The signature card, floated above the line whose call
                    // it describes, with the active parameter lit.
                    {
                        let path = path.clone();
                        move || {
                            let Some((for_path, line, info)) = state.signature.get() else {
                                return ().into_any();
                            };
                            if for_path != path {
                                return ().into_any();
                            }
                            let top = 8.0 + f64::from(line) * LINE_HEIGHT - 4.0;
                            let label = info.label;
                            let split = match (info.param_start, info.param_end) {
                                (Some(start), Some(end)) => {
                                    let start = start as usize;
                                    let end = (end as usize).min(label.len());
                                    if start <= end
                                        && label.is_char_boundary(start)
                                        && label.is_char_boundary(end)
                                    {
                                        Some((start, end))
                                    } else {
                                        None
                                    }
                                }
                                _ => None,
                            };
                            let (before, active, after) = match split {
                                Some((start, end)) => (
                                    label[..start].to_string(),
                                    label[start..end].to_string(),
                                    label[end..].to_string(),
                                ),
                                None => (label, String::new(), String::new()),
                            };
                            // One line of docs, not the essay — hover exists.
                            let doc = info
                                .doc
                                .as_deref()
                                .and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
                                .map(str::to_string);
                            view! {
                                <div
                                    class="absolute z-10 max-w-[76ch] rounded-[8px] bg-raised px-3 py-1.5 font-mono text-footnote shadow-xl ring-1 ring-line-strong"
                                    style=format!(
                                        "left: 8px; top: {top}px; transform: translateY(-100%)",
                                    )
                                >
                                    <div class="whitespace-pre-wrap select-text">
                                        <span class="text-label-2">{before}</span>
                                        <span class="font-semibold text-rust">{active}</span>
                                        <span class="text-label-2">{after}</span>
                                    </div>
                                    {doc
                                        .map(|text| {
                                            view! {
                                                <div class="mt-0.5 max-w-[70ch] truncate font-sans text-caption text-label-3">
                                                    {text}
                                                </div>
                                            }
                                        })}
                                </div>
                            }
                            .into_any()
                        }
                    }

                    // The completion popup, anchored under the word it is
                    // completing.
                    {
                        let path = path.clone();
                        move || {
                            let Some(popup) = state.completion.get() else {
                                return ().into_any();
                            };
                            if popup.path != path {
                                return ().into_any();
                            }
                            let draft = state.draft.get();
                            let word = typed_word(&draft, popup.line, popup.word_start);
                            let shown: Vec<(usize, CompletionItem)> = popup
                                .items
                                .iter()
                                .filter(|item| {
                                    word.is_empty()
                                        || item
                                            .label
                                            .to_lowercase()
                                            .starts_with(&word.to_lowercase())
                                })
                                .take(50)
                                .cloned()
                                .enumerate()
                                .collect();
                            if shown.is_empty() {
                                return ().into_any();
                            }
                            let chosen = picked.get().min(shown.len() - 1);
                            let x = 8.0 + column_px(&draft, popup.line, popup.word_start);
                            let y = 8.0 + f64::from(popup.line + 1) * LINE_HEIGHT + 2.0;
                            // A window around the selection rather than a
                            // scrollbar: nine rows is what the eye takes in,
                            // and the arrows walk the rest into view.
                            let from = chosen.saturating_sub(4).min(shown.len().saturating_sub(9));
                            view! {
                                <div
                                    class="absolute z-20 min-w-[260px] rounded-[8px] bg-raised py-1 font-mono text-footnote shadow-2xl ring-1 ring-line-strong"
                                    style=format!("left: {x}px; top: {y}px")
                                >
                                    {shown
                                        .into_iter()
                                        .skip(from)
                                        .take(9)
                                        .map(|(index, item)| {
                                            let selected = index == chosen;
                                            let kind = item.kind.clone().unwrap_or_default();
                                            let detail = item.detail.clone().unwrap_or_default();
                                            view! {
                                                <button
                                                    type="button"
                                                    on:mousedown=move |event: ev::MouseEvent| {
                                                        // Before the textarea's
                                                        // own mousedown closes
                                                        // the popup.
                                                        event.prevent_default();
                                                        event.stop_propagation();
                                                        if let Some(element) =
                                                            area.get_untracked()
                                                        {
                                                            accept_completion(
                                                                state, &element, index,
                                                            );
                                                        }
                                                    }
                                                    class=if selected {
                                                        "flex w-full items-baseline gap-2 bg-selection px-2.5 py-0.5 text-left text-rust"
                                                    } else {
                                                        "flex w-full items-baseline gap-2 px-2.5 py-0.5 text-left text-label-2"
                                                    }
                                                >
                                                    <span class="w-[7ch] shrink-0 truncate text-label-3">
                                                        {kind}
                                                    </span>
                                                    <span class="shrink-0">{item.label.clone()}</span>
                                                    <span class="min-w-0 flex-1 truncate text-label-3">
                                                        {detail}
                                                    </span>
                                                </button>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                            .into_any()
                        }
                    }
                </div>
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

/// Snapshot the draft before an edit replaces it.
///
/// Bursts coalesce: pushes within 600ms collapse into one undo step, so
/// Ctrl+Z after typing a word removes the word, not one letter.
fn record_edit(state: AppState) {
    const CAP: usize = 200;
    const BURST_MS: f64 = 600.0;

    let now = js_sys::Date::now();
    let text = state.draft.get_untracked();
    state.history.update(|history| {
        history.redo.clear();
        let burst = now - history.last_push < BURST_MS && !history.undo.is_empty();
        if !burst && history.undo.last() != Some(&text) {
            history.undo.push(text);
            if history.undo.len() > CAP {
                history.undo.remove(0);
            }
        }
        history.last_push = now;
    });
}

/// Undo or redo one step.
fn apply_history(
    area: &web_sys::HtmlTextAreaElement,
    state: AppState,
    scroller: NodeRef<html::Div>,
    undo: bool,
) {
    let current = state.draft.get_untracked();
    let mut target = None;
    state.history.update(|history| {
        let (from, to) = if undo {
            (&mut history.undo, &mut history.redo)
        } else {
            (&mut history.redo, &mut history.undo)
        };
        while let Some(text) = from.pop() {
            if text != current {
                to.push(current.clone());
                target = Some(text);
                break;
            }
        }
        // The restore itself must not merge into a typing burst.
        history.last_push = 0.0;
    });
    let Some(text) = target else {
        return;
    };

    let caret = caret_after_restore(&text, &current);
    echo_edit(state, &text);
    state.draft.set(text.clone());
    area.set_value(&text);
    let _ = area.set_selection_start(Some(caret));
    let _ = area.set_selection_end(Some(caret));
    state.completion.set(None);
    state.signature.set(None);
    keep_caret_in_view(area, state, scroller);
    controller::schedule_pulse(state);
}

/// Where the caret lands after `target` replaces `other` on screen: the end
/// of the region where the two texts differ, in UTF-16 units — which is where
/// the eye is already looking.
fn caret_after_restore(target: &str, other: &str) -> u32 {
    let target_bytes = target.as_bytes();
    let other_bytes = other.as_bytes();

    let mut prefix = 0;
    while prefix < target_bytes.len().min(other_bytes.len())
        && target_bytes[prefix] == other_bytes[prefix]
    {
        prefix += 1;
    }
    while prefix > 0 && !target.is_char_boundary(prefix) {
        prefix -= 1;
    }

    let mut suffix = 0;
    while suffix < (target_bytes.len() - prefix).min(other_bytes.len() - prefix)
        && target_bytes[target_bytes.len() - 1 - suffix] == other_bytes[other_bytes.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let mut end = target_bytes.len() - suffix;
    while end < target_bytes.len() && !target.is_char_boundary(end) {
        end += 1;
    }
    let end = end.max(prefix);

    target[..end].encode_utf16().count() as u32
}

/// Scroll the shared scroller so the caret's line is on screen. The textarea
/// cannot do this itself any more — its own scrolling is pinned to zero.
fn keep_caret_in_view(
    area: &web_sys::HtmlTextAreaElement,
    state: AppState,
    scroller: NodeRef<html::Div>,
) {
    let Some(outer) = scroller.get_untracked() else {
        return;
    };
    let text = state.draft.get_untracked();
    let Some((line, _)) = caret_line_col(area, &text) else {
        return;
    };
    let y = 8.0 + f64::from(line) * LINE_HEIGHT;
    let view_top = f64::from(outer.scroll_top());
    let view_height = f64::from(outer.client_height());
    if y < view_top + LINE_HEIGHT {
        outer.set_scroll_top((y - LINE_HEIGHT * 3.0).max(0.0) as i32);
    } else if y + LINE_HEIGHT * 2.0 > view_top + view_height {
        outer.set_scroll_top((y + LINE_HEIGHT * 4.0 - view_height) as i32);
    }
}

/// Every occurrence of `query` in `text`, as byte ranges.
///
/// ASCII-case-folded like project search's literal mode, capped so a
/// one-letter query in a big file cannot melt the renderer.
fn find_matches(text: &str, query: &str, case: bool) -> Vec<(usize, usize)> {
    const CAP: usize = 2000;
    if query.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let hay = text.as_bytes();
    let needle = query.as_bytes();
    let mut at = 0;
    while at + needle.len() <= hay.len() && out.len() < CAP {
        let here = &hay[at..at + needle.len()];
        let matched = if case {
            here == needle
        } else {
            here.eq_ignore_ascii_case(needle)
        };
        if matched && text.is_char_boundary(at) {
            out.push((at, at + needle.len()));
            at += needle.len().max(1);
        } else {
            at += 1;
        }
    }
    out
}

/// (line, scalar column) of a byte offset.
fn line_col_of_byte(text: &str, at: usize) -> (u32, u32) {
    let before = &text[..at.min(text.len())];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map(|found| found + 1).unwrap_or(0);
    (line, before[start..].chars().count() as u32)
}

/// Step the current find match by `direction`, wrapping, and show it.
fn find_jump(state: AppState, scroller: NodeRef<html::Div>, direction: i32) {
    let text = state.draft.get_untracked();
    let query = state.find_query.get_untracked();
    let matches = find_matches(&text, &query, state.find_case.get_untracked());
    if matches.is_empty() {
        return;
    }
    let current = state.find_index.get_untracked().min(matches.len() - 1);
    let next = if direction >= 0 {
        (current + 1) % matches.len()
    } else {
        (current + matches.len() - 1) % matches.len()
    };
    state.find_index.set(next);
    let (line, _) = line_col_of_byte(&text, matches[next].0);
    if let Some(outer) = scroller.get_untracked() {
        let y = 8.0 + f64::from(line) * LINE_HEIGHT;
        let top = f64::from(outer.scroll_top());
        let height = f64::from(outer.client_height());
        if y < top + LINE_HEIGHT || y + LINE_HEIGHT * 2.0 > top + height {
            outer.set_scroll_top((y - height / 3.0).max(0.0) as i32);
        }
    }
}

/// Replace the current match, or every match, through the undo pipeline.
fn find_replace(state: AppState, area: NodeRef<html::Textarea>, all: bool) {
    let text = state.draft.get_untracked();
    let query = state.find_query.get_untracked();
    let matches = find_matches(&text, &query, state.find_case.get_untracked());
    if matches.is_empty() {
        return;
    }
    let replacement = state.find_replace.get_untracked();

    record_edit(state);
    let mut new = text.clone();
    if all {
        for (from, to) in matches.iter().rev() {
            new.replace_range(*from..*to, &replacement);
        }
    } else {
        let current = state.find_index.get_untracked().min(matches.len() - 1);
        let (from, to) = matches[current];
        new.replace_range(from..to, &replacement);
    }

    echo_edit(state, &new);
    state.draft.set(new.clone());
    if let Some(element) = area.get_untracked() {
        element.set_value(&new);
    }
    controller::schedule_pulse(state);
}

/// The floating find/replace bar, top right of the editor.
#[component]
fn FindBar(area: NodeRef<html::Textarea>, scroller: NodeRef<html::Div>) -> impl IntoView {
    let state = AppState::expect();
    let input: NodeRef<html::Input> = NodeRef::new();

    // Opening focuses the query box with its text selected, ready to retype.
    Effect::new(move |_| {
        if state.find_open.get()
            && let Some(element) = input.get_untracked()
        {
            let _ = element.focus();
            element.select();
        }
    });

    let counter = Signal::derive(move || {
        let text = state.draft.get();
        let query = state.find_query.get();
        let matches = find_matches(&text, &query, state.find_case.get());
        if query.is_empty() {
            String::new()
        } else if matches.is_empty() {
            "no results".to_string()
        } else {
            let current = state.find_index.get().min(matches.len() - 1);
            format!("{}/{}", current + 1, matches.len())
        }
    });

    let small = "grid size-6 place-items-center rounded-[5px] text-footnote                  text-label-3 hover:bg-sunken hover:text-label";

    view! {
        <Show when=move || state.find_open.get()>
            <div class="absolute top-2 right-6 z-30 flex flex-col gap-1 rounded-[8px] bg-raised p-1.5 shadow-2xl ring-1 ring-line-strong">
                <div class="flex items-center gap-1">
                    <input
                        node_ref=input
                        type="text"
                        placeholder="Find"
                        autocomplete="off"
                        spellcheck="false"
                        prop:value=move || state.find_query.get()
                        on:input=move |event: ev::Event| {
                            state.find_query.set(event_target_value(&event));
                            state.find_index.set(0);
                        }
                        on:keydown=move |event: ev::KeyboardEvent| {
                            match event.key().as_str() {
                                "Enter" => {
                                    event.prevent_default();
                                    find_jump(
                                        state,
                                        scroller,
                                        if event.shift_key() { -1 } else { 1 },
                                    );
                                }
                                "Escape" => {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    state.find_open.set(false);
                                    state.find_replace_open.set(false);
                                    if let Some(element) = area.get_untracked() {
                                        let _ = element.focus();
                                    }
                                }
                                _ => {}
                            }
                        }
                        class="w-[200px] rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote text-label placeholder:text-label-3"
                    />
                    <button
                        type="button"
                        title="Match case"
                        on:click=move |_| {
                            state.find_case.update(|case| *case = !*case);
                            state.find_index.set(0);
                        }
                        class=move || {
                            let base = "rounded-[5px] px-1.5 py-0.5 font-mono text-footnote";
                            if state.find_case.get() {
                                format!("{base} bg-selection text-rust")
                            } else {
                                format!("{base} text-label-3 hover:text-label")
                            }
                        }
                    >
                        "Aa"
                    </button>
                    <span class="min-w-[6ch] px-1 text-center font-mono text-caption text-label-3">
                        {counter}
                    </span>
                    <button
                        type="button"
                        title="Previous (Shift+Enter)"
                        on:click=move |_| find_jump(state, scroller, -1)
                        class=small
                    >
                        "↑"
                    </button>
                    <button
                        type="button"
                        title="Next (Enter)"
                        on:click=move |_| find_jump(state, scroller, 1)
                        class=small
                    >
                        "↓"
                    </button>
                    <button
                        type="button"
                        title="Replace…"
                        on:click=move |_| {
                            state.find_replace_open.update(|open| *open = !*open)
                        }
                        class=small
                    >
                        "⇄"
                    </button>
                    <button
                        type="button"
                        title="Close (Esc)"
                        on:click=move |_| {
                            state.find_open.set(false);
                            state.find_replace_open.set(false);
                        }
                        class=small
                    >
                        "×"
                    </button>
                </div>
                <Show when=move || state.find_replace_open.get()>
                    <div class="flex items-center gap-1">
                        <input
                            type="text"
                            placeholder="Replace with"
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || state.find_replace.get()
                            on:input=move |event: ev::Event| {
                                state.find_replace.set(event_target_value(&event))
                            }
                            class="w-[200px] rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote text-label placeholder:text-label-3"
                        />
                        <button
                            type="button"
                            on:click=move |_| find_replace(state, area, false)
                            class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            "Replace"
                        </button>
                        <button
                            type="button"
                            on:click=move |_| find_replace(state, area, true)
                            class="rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            "All"
                        </button>
                    </div>
                </Show>
            </div>
        </Show>
    }
}

/// Apply the chosen quick fix: splice its edits bottom-up so earlier ranges
/// stay valid, through the undo pipeline.
fn apply_action(state: AppState, area: &web_sys::HtmlTextAreaElement, index: usize) {
    let Some((_, _, fixes)) = state.actions.get_untracked() else {
        return;
    };
    let Some(fix) = fixes.get(index.min(fixes.len().saturating_sub(1))) else {
        return;
    };

    let text = state.draft.get_untracked();
    let mut edits: Vec<(usize, usize, &str)> = fix
        .edits
        .iter()
        .map(|edit| {
            let from = byte_of_utf16(
                &text,
                utf16_offset_of(&text, edit.range.start_line, edit.range.start_col) as usize,
            );
            let to = byte_of_utf16(
                &text,
                utf16_offset_of(&text, edit.range.end_line, edit.range.end_col) as usize,
            );
            (from, to.max(from), edit.new_text.as_str())
        })
        .collect();
    edits.sort_by_key(|(from, ..)| std::cmp::Reverse(*from));

    record_edit(state);
    let mut new = text.clone();
    for (from, to, replacement) in edits {
        new.replace_range(from..to, replacement);
    }

    echo_edit(state, &new);
    state.draft.set(new.clone());
    area.set_value(&new);
    state.actions.set(None);
    controller::schedule_pulse(state);
}

/// The caret as (line, scalar column) in `text`.
fn caret_line_col(area: &web_sys::HtmlTextAreaElement, text: &str) -> Option<(u32, u32)> {
    let units = area.selection_start().ok().flatten()? as usize;
    let byte = byte_of_utf16(text, units);
    let before = &text[..byte];
    let line = before.matches('\n').count() as u32;
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let col = before[line_start..].chars().count() as u32;
    Some((line, col))
}

/// Where the identifier under the caret begins, for Ctrl+Space.
fn word_start_before(text: &str, line: u32, col: u32) -> u32 {
    let Some(line_text) = text.split('\n').nth(line as usize) else {
        return col;
    };
    let chars: Vec<char> = line_text.chars().take(col as usize).collect();
    let mut start = chars.len();
    while start > 0 && (chars[start - 1].is_alphanumeric() || chars[start - 1] == '_') {
        start -= 1;
    }
    start as u32
}

/// The word typed since the popup opened — what the list narrows against.
fn typed_word(text: &str, line: u32, word_start: u32) -> String {
    text.split('\n')
        .nth(line as usize)
        .map(|line_text| {
            line_text
                .chars()
                .skip(word_start as usize)
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
                .collect()
        })
        .unwrap_or_default()
}

/// Apply the chosen completion to the draft.
fn accept_completion(state: AppState, area: &web_sys::HtmlTextAreaElement, index: usize) {
    let Some(popup) = state.completion.get_untracked() else {
        return;
    };
    let draft = state.draft.get_untracked();
    let word = typed_word(&draft, popup.line, popup.word_start);
    let shown: Vec<&CompletionItem> = popup
        .items
        .iter()
        .filter(|item| {
            word.is_empty() || item.label.to_lowercase().starts_with(&word.to_lowercase())
        })
        .collect();
    let Some(item) = shown.get(index.min(shown.len().saturating_sub(1))).copied() else {
        return;
    };

    // The server's own edit range wins; without one, the typed word is what
    // the insertion replaces.
    let (start_line, start_col, end_line, end_col) = match &item.edit {
        Some(edit) => (edit.start_line, edit.start_col, edit.end_line, edit.end_col),
        None => (
            popup.line,
            popup.word_start,
            popup.line,
            popup.word_start + word.chars().count() as u32,
        ),
    };

    record_edit(state);
    let start = byte_of_utf16(&draft, utf16_offset_of(&draft, start_line, start_col) as usize);
    let end = byte_of_utf16(&draft, utf16_offset_of(&draft, end_line, end_col) as usize);
    let mut text = draft;
    text.replace_range(start.min(end)..end.max(start), &item.insert);

    echo_edit(state, &text);
    state.draft.set(text.clone());
    area.set_value(&text);
    let caret = utf16_offset_of(&text, start_line, start_col)
        + item.insert.encode_utf16().count() as u32;
    let _ = area.set_selection_start(Some(caret));
    let _ = area.set_selection_end(Some(caret));
    state.completion.set(None);
    controller::schedule_pulse(state);
}

/// Whether a cell sits inside a hover range.
fn within(range: &rusty_lsp::EditRange, line: u32, col: u32) -> bool {
    if line < range.start_line || line > range.end_line {
        return false;
    }
    if line == range.start_line && col < range.start_col {
        return false;
    }
    if line == range.end_line && col >= range.end_col.max(range.start_col + 1) {
        return false;
    }
    true
}

/// The caret's byte offset into the draft.
fn caret_byte(area: &web_sys::HtmlTextAreaElement, state: AppState) -> usize {
    let units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    byte_of_utf16(&state.draft.get_untracked(), units)
}

/// selectionStart counts UTF-16 units — it is a JS string index. Treating it
/// as bytes panics on the first CJK comment.
fn byte_of_utf16(text: &str, units: usize) -> usize {
    let mut seen = 0usize;
    for (offset, ch) in text.char_indices() {
        if seen >= units {
            return offset;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// A (line, scalar column) as a UTF-16 offset, for placing the caret.
fn utf16_offset_of(text: &str, line: u32, col: u32) -> u32 {
    let mut offset = 0u32;
    for (index, content) in text.split('\n').enumerate() {
        if index as u32 == line {
            offset += content
                .chars()
                .take(col as usize)
                .map(|ch| ch.len_utf16() as u32)
                .sum::<u32>();
            return offset;
        }
        offset += content.encode_utf16().count() as u32 + 1;
    }
    offset
}

/// Put `insert` at the caret and leave the caret after it.
fn insert_at_caret(area: &web_sys::HtmlTextAreaElement, state: AppState, insert: &str) {
    record_edit(state);
    let start_units = area.selection_start().ok().flatten().unwrap_or(0) as usize;
    let end_units = area.selection_end().ok().flatten().unwrap_or(0) as usize;
    let mut text = state.draft.get_untracked();
    let start = byte_of_utf16(&text, start_units);
    let end = byte_of_utf16(&text, end_units).max(start);

    text.replace_range(start..end, insert);
    echo_edit(state, &text);
    state.draft.set(text.clone());
    area.set_value(&text);
    let at = (start_units + insert.encode_utf16().count()) as u32;
    let _ = area.set_selection_start(Some(at));
    let _ = area.set_selection_end(Some(at));
    controller::schedule_pulse(state);
}

/// What pressing Enter at `caret` should insert: a newline and the indentation
/// the next line starts with.
///
/// The current line's leading whitespace, plus one level when the caret sits
/// right after an opening bracket — the two rules that cover nearly every
/// Enter press in Rust. Anything cleverer belongs to the language server.
fn newline_indent(text: &str, caret: usize) -> String {
    let before = &text[..caret.min(text.len())];
    let line_start = before.rfind('\n').map(|at| at + 1).unwrap_or(0);
    let line = &before[line_start..];
    let indent: String = line
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();

    let deeper = matches!(line.trim_end().chars().last(), Some('{' | '(' | '['));
    if deeper {
        format!("\n{indent}    ")
    } else {
        format!("\n{indent}")
    }
}

/// Which (line, scalar column) sits under a point in the text column.
///
/// The column is found by measuring, not dividing: a CJK glyph is two cells
/// wide in a monospace font, so `x / ch` drifts one column per ideograph and
/// hover would describe the wrong token on any line with a Chinese comment.
fn cell_under(text: &str, offset_x: f64, offset_y: f64) -> Option<(u32, u32)> {
    // The 8s are the text column's pl-2 / py-2.
    let line = ((offset_y - 8.0) / LINE_HEIGHT).floor();
    if line < 0.0 {
        return None;
    }
    let line = line as u32;
    let content = text.split('\n').nth(line as usize)?;

    let x = offset_x - 8.0;
    if x < 0.0 {
        return Some((line, 0));
    }
    let mut reached = 0.0;
    for (index, ch) in content.chars().enumerate() {
        let advance = advance_of(ch);
        if reached + advance > x {
            return Some((line, index as u32));
        }
        reached += advance;
    }
    // Past the end of the line: the last column, where "what is this?" still
    // usually means the token the line ends with.
    Some((line, content.chars().count() as u32))
}

/// Pixels from the line start to a scalar column, for anchoring the tooltip.
fn column_px(text: &str, line: u32, col: u32) -> f64 {
    text.split('\n')
        .nth(line as usize)
        .map(|content| {
            content
                .chars()
                .take(col as usize)
                .map(advance_of)
                .sum()
        })
        .unwrap_or(0.0)
}

/// One glyph's advance in the editor's font, measured once per character via
/// canvas and cached — measuring is what makes CJK correct, caching is what
/// makes it affordable on every mouse move.
fn advance_of(ch: char) -> f64 {
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static CACHE: RefCell<HashMap<char, f64>> = RefCell::new(HashMap::new());
        static CONTEXT: RefCell<Option<web_sys::CanvasRenderingContext2d>> =
            const { RefCell::new(None) };
    }

    CACHE.with(|cache| {
        if let Some(width) = cache.borrow().get(&ch) {
            return *width;
        }
        let width = CONTEXT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.create_element("canvas").ok())
                    .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok())
                    .and_then(|c| c.get_context("2d").ok().flatten())
                    .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
                    .inspect(|context| {
                        context.set_font(
                            "12.5px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
                        );
                    });
            }
            slot.as_ref()
                .and_then(|context| context.measure_text(&ch.to_string()).ok())
                .map(|m| m.width())
                // The monospace advance at 12.5px, if the canvas is somehow
                // unavailable; wrong for CJK but never absurd.
                .unwrap_or(7.5)
        });
        cache.borrow_mut().insert(ch, width);
        width
    })
}

/// Hover text arrives as markdown with code fences; the tooltip is plain.
/// Hover markdown, minimally: fenced blocks become highlighted code, `---`
/// becomes a divider, everything else is prose. The code gets the same
/// lexical colours the editor uses, so the tooltip does not describe Rust
/// in monochrome an inch above a highlighted buffer.
fn hover_parts(text: &str) -> AnyView {
    enum Part {
        Code(Vec<String>),
        Prose(String),
        Rule,
    }

    let mut parts: Vec<Part> = Vec::new();
    let mut in_code = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if in_code {
                parts.push(Part::Code(Vec::new()));
            }
            continue;
        }
        if in_code {
            if let Some(Part::Code(lines)) = parts.last_mut() {
                lines.push(line.to_string());
            }
            continue;
        }
        if line.trim() == "---" {
            parts.push(Part::Rule);
            continue;
        }
        match parts.last_mut() {
            Some(Part::Prose(prose)) => {
                prose.push('\n');
                prose.push_str(line);
            }
            _ => parts.push(Part::Prose(line.to_string())),
        }
    }

    parts
        .into_iter()
        .filter(|part| !matches!(part, Part::Prose(text) if text.trim().is_empty()))
        .map(|part| match part {
            Part::Rule => view! { <div class="my-1.5 h-px bg-line" /> }.into_any(),
            Part::Prose(prose) => view! {
                <div class="font-sans whitespace-pre-wrap">{prose.trim().to_string()}</div>
            }
            .into_any(),
            Part::Code(lines) => view! {
                <pre class="my-1 overflow-x-auto whitespace-pre">
                    {lines
                        .into_iter()
                        .map(|line| {
                            let spans = rusty_edit::lexical::refine(vec![Span {
                                text: line,
                                token: Token::Plain,
                            }]);
                            view! {
                                <div>
                                    {spans
                                        .into_iter()
                                        .map(|span| {
                                            view! {
                                                <span class=class_of(
                                                    span.token,
                                                )>{span.text}</span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                        })
                        .collect_view()}
                </pre>
            }
            .into_any(),
        })
        .collect_view()
        .into_any()
}

/// rust-analyzer's legend names, mapped into the palette.
///
/// Unmapped kinds — operators, namespaces, punctuation — return `None` and
/// keep the lexical base colour: overlaying everything would repaint half
/// the buffer in one hue and lose more than it adds.
fn semantic_token(kind: &str) -> Option<Token> {
    Some(match kind {
        "keyword" | "lifetime" | "boolean" | "selfKeyword" | "selfTypeKeyword" => Token::Keyword,
        "comment" => Token::Comment,
        "string" | "character" => Token::Str,
        "number" => Token::Number,
        "macro" | "macroBang" | "derive" | "deriveHelper" | "attribute" | "attributeBracket" => {
            Token::Macro
        }
        "function" | "method" | "procMacro" => Token::Function,
        "struct" | "enum" | "union" | "trait" | "typeAlias" | "type" | "builtinType"
        | "class" | "interface" | "enumMember" | "typeParameter" => Token::Type,
        "variable" | "parameter" | "property" | "field" | "const" | "static" => Token::Variable,
        "namespace" => Token::Namespace,
        _ => return None,
    })
}

/// Re-cut a line's spans so the compiler's colours win where they exist and
/// the lexical base shows everywhere else.
fn overlay_semantic(line: Line, index: u32, semantic: &[SemanticSpan]) -> Line {
    let marks: Vec<(u32, u32, Token)> = semantic
        .iter()
        .filter(|span| span.line == index)
        .filter_map(|span| {
            semantic_token(&span.kind)
                .map(|token| (span.start_col, span.start_col + span.length, token))
        })
        .collect();
    if marks.is_empty() {
        return line;
    }

    let mut out: Vec<Span> = Vec::with_capacity(line.spans.len());
    let mut col = 0u32;
    for span in line.spans {
        let mut text = String::new();
        let mut current = span.token;
        for ch in span.text.chars() {
            let token = marks
                .iter()
                .find(|(from, to, _)| (*from..*to).contains(&col))
                .map(|(_, _, token)| *token)
                .unwrap_or(span.token);
            if token != current && !text.is_empty() {
                out.push(Span {
                    text: std::mem::take(&mut text),
                    token: current,
                });
            }
            current = token;
            text.push(ch);
            col += 1;
        }
        if !text.is_empty() {
            out.push(Span {
                text,
                token: current,
            });
        }
    }
    Line { spans: out }
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
        Token::Variable => "tok-variable",
        Token::Namespace => "tok-namespace",
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    #[test]
    fn matches_fold_ascii_case_and_respect_the_toggle() {
        let text = "Gain gain GAIN";
        assert_eq!(
            find_matches(text, "gain", false),
            vec![(0, 4), (5, 9), (10, 14)],
        );
        assert_eq!(find_matches(text, "gain", true), vec![(5, 9)]);
        assert!(find_matches(text, "", false).is_empty());
    }

    #[test]
    fn byte_offsets_convert_to_scalar_columns_past_cjk() {
        let text = "// 中文
let gain = 1;";
        let matches = find_matches(text, "gain", false);
        assert_eq!(matches.len(), 1);
        let (line, col) = line_col_of_byte(text, matches[0].0);
        assert_eq!((line, col), (1, 4));
    }

    #[test]
    fn overlapping_repeats_advance_past_each_match() {
        assert_eq!(find_matches("aaaa", "aa", false), vec![(0, 2), (2, 4)]);
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;

    fn line_of(text: &str) -> Line {
        Line {
            spans: vec![Span {
                text: text.to_string(),
                token: Token::Plain,
            }],
        }
    }

    fn span(line: u32, start_col: u32, length: u32, kind: &str) -> SemanticSpan {
        SemanticSpan {
            line,
            start_col,
            length,
            kind: kind.to_string(),
        }
    }

    #[test]
    fn the_compilers_colour_wins_inside_its_range_only() {
        let out = overlay_semantic(
            line_of("let radio = 1;"),
            0,
            &[span(0, 4, 5, "variable")],
        );
        let texts: Vec<(String, Token)> =
            out.spans.into_iter().map(|s| (s.text, s.token)).collect();
        assert_eq!(
            texts,
            vec![
                ("let ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
                (" = 1;".to_string(), Token::Plain),
            ],
        );
    }

    #[test]
    fn other_lines_and_unknown_kinds_change_nothing() {
        let untouched = overlay_semantic(
            line_of("let radio = 1;"),
            0,
            &[span(3, 0, 5, "variable"), span(0, 0, 3, "operator")],
        );
        assert_eq!(untouched.spans.len(), 1);
        assert_eq!(untouched.spans[0].token, Token::Plain);
    }

    #[test]
    fn cjk_columns_are_scalar_not_bytes() {
        // "中文 radio" — the variable starts at scalar column 3.
        let out = overlay_semantic(line_of("中文 radio"), 0, &[span(0, 3, 5, "field")]);
        let texts: Vec<(String, Token)> =
            out.spans.into_iter().map(|s| (s.text, s.token)).collect();
        assert_eq!(
            texts,
            vec![
                ("中文 ".to_string(), Token::Plain),
                ("radio".to_string(), Token::Variable),
            ],
        );
    }
}

#[cfg(test)]
mod history_tests {
    use super::caret_after_restore;

    #[test]
    fn undoing_an_insertion_lands_at_the_insertion_point() {
        // other = after typing "abXc", target = restore "abc"
        assert_eq!(caret_after_restore("abc", "abXc"), 2);
    }

    #[test]
    fn undoing_a_deletion_lands_after_the_restored_text() {
        // other = after deleting X, target restores it
        assert_eq!(caret_after_restore("abXc", "abc"), 3);
    }

    #[test]
    fn cjk_before_the_change_counts_utf16_units() {
        // "中" is one scalar, one UTF-16 unit; the change is after it.
        assert_eq!(caret_after_restore("中aZb", "中ab"), 3);
        // Beyond the BMP: "𝄞" is two UTF-16 units.
        assert_eq!(caret_after_restore("𝄞aZ", "𝄞a"), 4);
    }

    #[test]
    fn identical_texts_land_at_the_end() {
        assert_eq!(caret_after_restore("same", "same"), 4);
    }
}

#[cfg(test)]
mod tests {
    use super::{newline_indent, utf16_offset_of};

    #[test]
    fn enter_copies_the_indent_and_deepens_after_an_opener() {
        let text = "fn main() {\n    let x = 1;\n";
        // After the opening brace: one level deeper.
        assert_eq!(newline_indent(text, 11), "\n    ");
        // After the statement: same level.
        assert_eq!(newline_indent(text, text.len()), "\n");
        let nested = "    if x {\n";
        assert_eq!(newline_indent(nested, nested.len() - 1), "\n        ");
    }

    #[test]
    fn caret_offsets_survive_cjk() {
        let text = "// 中文\nfn main() {}\n";
        // Line 1 col 3: past "fn " — offset counts the CJK line as utf16.
        // "// 中文" = 5 utf16 units, +1 newline.
        assert_eq!(utf16_offset_of(text, 1, 3), 6 + 3);
    }
}
