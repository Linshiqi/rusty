//! The project tree, and the panel that frames it.

use leptos::{ev, html, prelude::*};

use rusty_edit::Entry;

use super::*;
use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, Empty, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

#[component]
pub fn FilesPanel() -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() && state.editor.tree.with(Vec::is_empty) {
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

        // A detached window is the editor alone — VSCode's shape: the tree
        // and the strip belong to the shell that spawned it.
        if state.app.detached.with_untracked(Option::is_some) {
            return view! {
                <div class="flex min-h-0 flex-1">
                    <Editor />
                </div>
            }
            .into_any();
        }

        view! {
            <div class="flex min-h-0 flex-1">
                <Tree />
                <crate::view::split::Handle divider=crate::state::Divider::Tree />
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
    // A pending "New file" / "New folder": (directory it lands in, is_dir).
    // The name is typed into a strip under the header; Enter creates.
    let naming = RwSignal::new(None::<(String, bool)>);
    let name_box: NodeRef<html::Input> = NodeRef::new();
    Effect::new(move |_| {
        if naming.get().is_some()
            && let Some(input) = name_box.get()
        {
            let _ = input.focus();
        }
    });

    view! {
        <div
            class="flex flex-none flex-col border-r border-line bg-sidebar"
            style=move || format!("width: {}px", state.layout.tree_width.get())
        >
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
            {move || {
                let (parent, dir) = naming.get()?;
                let hint = if dir { "folder name" } else { "file name" };
                let shown_parent = if parent.is_empty() {
                    "./".to_string()
                } else {
                    format!("{parent}/")
                };
                let commit_parent = parent.clone();
                Some(
                    view! {
                        <div class="flex items-center gap-1.5 border-b border-line px-3 pb-2">
                            <span class="max-w-[9ch] truncate font-mono text-caption text-label-3">
                                {shown_parent}
                            </span>
                            <input
                                node_ref=name_box
                                placeholder=hint
                                class="min-w-0 flex-1 rounded-[5px] bg-sunken px-1.5 py-0.5 font-mono text-footnote outline-none ring-1 ring-line focus:ring-rust"
                                on:keydown=move |event: ev::KeyboardEvent| {
                                    match event.key().as_str() {
                                        "Enter" => {
                                            let name = event_target_value(&event);
                                            let name = name.trim().trim_matches('/');
                                            if name.is_empty() {
                                                return;
                                            }
                                            let path = if commit_parent.is_empty() {
                                                name.to_string()
                                            } else {
                                                format!("{commit_parent}/{name}")
                                            };
                                            controller::create_entry(state, path, dir);
                                            naming.set(None);
                                        }
                                        "Escape" => naming.set(None),
                                        _ => {}
                                    }
                                }
                            />
                        </div>
                    },
                )
            }}
            // Right-clicking the empty space targets the project root: rows
            // stop propagation, so only the sheet itself reaches this.
            <div
                class="min-h-0 flex-1 overflow-auto pb-2"
                on:contextmenu=move |event: ev::MouseEvent| {
                    event.prevent_default();
                    tree_menu.set(Some((
                        f64::from(event.client_x()),
                        f64::from(event.client_y()),
                        TreeTarget {
                            path: String::new(),
                            is_dir: true,
                        },
                    )));
                }
            >
                {move || {
                    let tree = state.editor.tree.get();
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
                let is_dir = target.is_dir;
                // Where a "New …" from this row lands: the directory itself,
                // or a file's parent.
                let into = if is_dir {
                    path.clone()
                } else {
                    path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
                };
                let (file_into, folder_into) = (into.clone(), into);

                // Right-clicking the empty sheet targets the project root:
                // only creation makes sense there.
                if path.is_empty() {
                    return Some(
                        view! {
                            <ContextMenu x=x y=y on_close=close>
                                <MenuItem
                                    label="New file…"
                                    on_select=Callback::new(move |_| {
                                        naming.set(Some((String::new(), false)));
                                        tree_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label="New folder…"
                                    on_select=Callback::new(move |_| {
                                        naming.set(Some((String::new(), true)));
                                        tree_menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label="Refresh"
                                    on_select=Callback::new(move |_| {
                                        controller::refresh_tree(state);
                                        tree_menu.set(None);
                                    })
                                />
                            </ContextMenu>
                        }
                        .into_any(),
                    );
                }

                let (open_path, copy_path, search_path) =
                    (path.clone(), path.clone(), path.clone());
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            <MenuItem
                                label=if is_dir { "Expand or collapse" } else { "Open" }
                                on_select=Callback::new(move |_| {
                                    if is_dir {
                                        state
                                            .editor.expanded
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
                            {(!is_dir)
                                .then(|| {
                                    let float = path.clone();
                                    view! {
                                        <MenuItem
                                            label="Open in new window"
                                            on_select=Callback::new(move |_| {
                                                controller::detach_file(
                                                    state,
                                                    float.clone(),
                                                );
                                                tree_menu.set(None);
                                            })
                                        />
                                    }
                                })}
                            <MenuSeparator />
                            <MenuItem
                                label="New file…"
                                on_select=Callback::new(move |_| {
                                    naming.set(Some((file_into.clone(), false)));
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label="New folder…"
                                on_select=Callback::new(move |_| {
                                    naming.set(Some((folder_into.clone(), true)));
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
                    }
                    .into_any(),
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

/// Scope the project search to one path and go there.
fn search_within(state: AppState, path: &str, is_dir: bool) {
    state.search.include.set(if is_dir {
        format!("{path}/**")
    } else {
        path.to_string()
    });
    state.layout.panel.set("search".to_string());
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
                move || {
                    state
                        .editor
                        .expanded
                        .with(|open| open.iter().any(|p| p == &path))
                }
            });
            let selected = Signal::derive({
                let path = path.clone();
                move || {
                    state
                        .editor
                        .document
                        .with(|d| d.as_ref().is_some_and(|d| d.path == path))
                }
            });

            let activate = {
                let path = path.clone();
                move |_| {
                    if is_dir {
                        state.editor.expanded.update(|open| {
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
