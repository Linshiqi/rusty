//! The open editors, one tab each.

use leptos::{ev, prelude::*};

use rusty_edit::Document;

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, MenuItem, MenuSeparator},
};

/// The open editors, one tab each. Clicking fronts a tab with its draft and
/// caret exactly as left; the cross closes it, asking first when unsaved
/// work would go with it.
#[component]
pub(super) fn TabStrip() -> impl IntoView {
    let state = AppState::expect();
    let menu = RwSignal::new(None::<(f64, f64, String)>);

    if state.app.detached.with_untracked(Option::is_some) {
        return ().into_any();
    }
    view! {
        <div class="flex flex-none items-stretch overflow-x-auto border-b border-line bg-sidebar">
            {move || {
                let active = state.active_path()
                    .unwrap_or_default();
                state
                    .editor.tabs
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
                        // One derivation, in `AppState`: the replace has to
                        // ask the same question of the same files, and two
                        // copies of "is this dirty" is how a replace writes
                        // over a draft the dot said was there.
                        let dirty = {
                            let path = path.clone();
                            Signal::derive(move || state.is_dirty(&path))
                        };
                        // The disk moved under an unsaved draft. Distinct
                        // from dirty, and shown as well as it rather than
                        // instead: the tab has two problems at once and
                        // saving it would overwrite somebody else's change.
                        let stale = {
                            let path = path.clone();
                            Signal::derive(move || {
                                state.editor.stale.with(|list| list.contains(&path))
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
                                                    title=t!("files.unsaved")
                                                />
                                            }
                                        })
                                }}
                                {move || {
                                    stale
                                        .get()
                                        .then(|| {
                                            view! {
                                                <span
                                                    class="shrink-0 leading-none text-amber"
                                                    title=t!("files.stale")
                                                >
                                                    "⚠"
                                                </span>
                                            }
                                        })
                                }}
                                <button
                                    type="button"
                                    title=t!("files.close")
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
                let (this, others, copy, float) =
                    (path.clone(), path.clone(), path.clone(), path.clone());
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label=t!("context.tab-close")
                                shortcut="Ctrl+W"
                                on_select=Callback::new(move |_| {
                                    controller::close_tab(state, this.clone());
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tab-new-window")
                                on_select=Callback::new(move |_| {
                                    controller::detach_file(state, float.clone());
                                    // The dirty guard inside close_tab still
                                    // applies: unsaved work keeps its tab here.
                                    controller::close_tab(state, float.clone());
                                    menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tab-close-others")
                                on_select=Callback::new(move |_| {
                                    for open in state.editor.tabs.get_untracked() {
                                        if open != others {
                                            controller::close_tab(state, open);
                                        }
                                    }
                                    menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.tab-copy-path")
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
    .into_any()
}

#[component]
pub(super) fn Header(document: Document) -> impl IntoView {
    let state = AppState::expect();
    let saved = document.text.clone();
    let path = document.path.clone();
    let dirty = Signal::derive(move || state.editor.draft.with(|draft| draft != &saved));

    view! {
        <div class="flex flex-none items-center gap-2 border-b border-line px-3 py-1.5">
            <span class="truncate font-mono text-footnote">{document.path}</span>
            {move || {
                dirty
                    .get()
                    .then(|| {
                        view! {
                            <span class="size-1.5 shrink-0 rounded-full bg-rust" title=t!("misc.unsaved") />
                        }
                    })
            }}
            <span class="flex-1" />
            // Markdown only: every other file has one way to read it, and a
            // toggle that does nothing on 95% of tabs is chrome.
            {super::editor::is_markdown(&path)
                .then(|| {
                    let path = path.clone();
                    let showing_source = {
                        let path = path.clone();
                        Signal::derive(move || {
                            state.editor.source_view.with(|v| v.contains(&path))
                        })
                    };
                    view! {
                        <button
                            type="button"
                            title=move || {
                                if showing_source.get() {
                                    t!("markdown.show-preview")
                                } else {
                                    t!("markdown.show-source")
                                }
                            }
                            on:click=move |_| {
                                let path = path.clone();
                                state
                                    .editor
                                    .source_view
                                    .update(|open| match open.iter().position(|p| *p == path) {
                                        Some(at) => {
                                            open.remove(at);
                                        }
                                        None => open.push(path),
                                    })
                            }
                            class="shrink-0 rounded-[5px] px-2 py-0.5 text-footnote text-label-2 hover:bg-sunken hover:text-label"
                        >
                            {move || {
                                if showing_source.get() {
                                    t!("markdown.preview")
                                } else {
                                    t!("markdown.source")
                                }
                            }}
                        </button>
                    }
                })}
            {document
                .read_only
                .then(|| {
                    view! {
                        <span
                            class="rounded-full bg-sunken px-2 text-footnote text-label-2"
                            title=t!("tabs.read-only-hint")
                        >
                            {t!("tabs.read-only")}
                        </span>
                    }
                })}
            {document
                .truncated
                .then(|| {
                    view! {
                        <span class="text-footnote text-amber">
                            {t!("misc.tab-cap")}
                        </span>
                    }
                })}
        </div>
    }
}
