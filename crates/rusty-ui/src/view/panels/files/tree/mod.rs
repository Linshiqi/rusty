//! The project tree, and the panel that frames it.

use leptos::{ev, html, prelude::*};

use rusty_edit::Entry;

use rusty_i18n::t;

use super::*;
use crate::{
    controller,
    state::{AppState, TreeClip},
    view::components::{ContextMenu, Empty, MenuItem, MenuSeparator, copy_to_clipboard},
    view::icon::{Icon, IconView},
};

mod level;
mod marks;

pub use level::*;
use marks::*;

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
                    title=t!("files.no-project-title")
                    detail=t!("files.no-project-detail")
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

        // The editor area holds one group, or two side by side with a grip
        // between them; the tree folds away on the switcher's second click.
        // The board can stand to the right of it all — Wokwi's shape, code
        // on the left and the board running it on the right — which is how
        // a playground opens and anybody may ask for.
        let area: NodeRef<html::Div> = NodeRef::new();
        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                <PlaygroundBar />
                <div class="flex min-h-0 flex-1">
                    {move || {
                        (!state.layout.tree_hidden.get()).then(|| {
                            view! {
                                <Tree />
                                <crate::view::split::Handle divider=crate::state::Divider::Tree />
                            }
                        })
                    }}
                    <div class="flex min-h-0 min-w-0 flex-1" node_ref=area>
                        <EditorGroup which=crate::state::Group::First />
                        {move || {
                            state.layout.split.get().then(|| {
                                view! {
                                    <SplitGrip area=area />
                                    <EditorGroup which=crate::state::Group::Second />
                                }
                            })
                        }}
                    </div>
                    {move || {
                        state.layout.board_beside.get().then(|| {
                            view! {
                                <crate::view::split::Handle divider=crate::state::Divider::Board />
                                // Never more than half the row: a window a
                                // laptop's width left the code a sliver
                                // beside a board dragged wide on a desk.
                                <div
                                    class="flex min-h-0 max-w-[50%] flex-none flex-col"
                                    style=move || {
                                        format!("width: {}px", state.layout.board_width.get())
                                    }
                                >
                                    {crate::view::panels::board_view()}
                                </div>
                            }
                        })
                    }}
                </div>
            </div>
        }
        .into_any()
    }
}

/// What a playground has that a project does not, in one row across the
/// top of the workspace and only in a playground: that it is one, the chip
/// it is for and the other chip's, its example back, and keeping it as a
/// project of its own. Without the row a playground would look exactly like
/// a project somebody had opened, which it is not — nothing in it is
/// anywhere they chose.
#[component]
fn PlaygroundBar() -> impl IntoView {
    let state = AppState::expect();
    const ACTION: &str = "rounded-[6px] px-2 py-0.5 text-footnote text-label-2 \
                          transition-colors hover:bg-sunken hover:text-label";

    move || {
        let open = state.playground()?;
        let chips = rusty_embed::PLAYGROUND_CHIPS
            .into_iter()
            .map(|chip| {
                let here = chip == open;
                let name = crate::command::chip_name(state, chip);
                view! {
                    <button
                        type="button"
                        disabled=here
                        on:click=move |_| controller::open_playground(state, chip)
                        class=if here {
                            "rounded-[5px] bg-raised px-2 py-px text-footnote font-medium text-label shadow-sm"
                        } else {
                            "rounded-[5px] px-2 py-px text-footnote text-label-3 hover:text-label"
                        }
                    >
                        {name}
                    </button>
                }
            })
            .collect_view();
        Some(view! {
            <div class="flex h-8 flex-none items-center gap-2 border-b border-line bg-sidebar px-3">
                <span class="text-rust">
                    <IconView icon=Icon::Simulate size=13 />
                </span>
                <span class="text-footnote font-semibold">{t!("playground.title")}</span>
                <div class="flex items-center gap-px rounded-[6px] bg-sunken p-0.5">{chips}</div>
                <span class="min-w-0 truncate text-caption text-label-4">
                    {t!("playground.hint")}
                </span>
                <span class="flex-1" />
                <button
                    type="button"
                    on:click=move |_| controller::reset_playground(state)
                    class=ACTION
                >
                    {t!("playground.reset")}
                </button>
                <button
                    type="button"
                    on:click=move |_| controller::keep_playground(state)
                    class=ACTION
                >
                    {t!("playground.keep")}
                </button>
            </div>
        })
    }
}

#[component]
fn Tree() -> impl IntoView {
    let state = AppState::expect();
    let tree_menu = RwSignal::new(None::<(f64, f64, TreeTarget)>);
    provide_context(TreeMenu(tree_menu));
    // Inline rename: the row whose name is an input right now.
    let renaming = RwSignal::new(None::<String>);
    provide_context(Renaming(renaming));
    // Drag and drop within the tree: what is being carried, and the folder
    // it would land in if released — highlighted, so the target is never a
    // guess. `""` is the root.
    let dragging = RwSignal::new(None::<TreeTarget>);
    let drop_target = RwSignal::new(None::<String>);
    provide_context(Dragging(dragging));
    provide_context(DropTarget(drop_target));
    // A pending "New file" / "New folder": (directory it lands in, is_dir).
    // The name is typed into a strip under the header; Enter creates.
    // Where a new entry is being named, and whether it is a folder. Context
    // like [`Renaming`], because the box is drawn by the level it belongs to
    // — see [`NewBox`].
    let naming = RwSignal::new(None::<(String, bool)>);
    provide_context(Naming(naming));

    view! {
        <div
            class="flex flex-none flex-col border-r border-line bg-sidebar"
            style=move || format!("width: {}px", state.layout.tree_width.get())
        >
            <div class="flex items-center gap-2 px-3 py-2">
                <span class="flex-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    {t!("tree.files")}
                </span>
                // New file and new folder at the root, where VS Code's
                // Explorer header offers them; the context menu still offers
                // them on any folder.
                <button
                    type="button"
                    title=t!("context.tree-new-file")
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| naming.set(Some((String::new(), false)))
                >
                    <IconView icon=Icon::FilePlus size=13 />
                </button>
                <button
                    type="button"
                    title=t!("context.tree-new-folder")
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| naming.set(Some((String::new(), true)))
                >
                    <IconView icon=Icon::FolderPlus size=13 />
                </button>
                <button
                    type="button"
                    title=t!("tree.refresh")
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                    on:click=move |_| controller::refresh_tree(state)
                >
                    <IconView icon=Icon::Refresh size=13 />
                </button>
            </div>
            // Right-clicking the empty space targets the project root: rows
            // stop propagation, so only the sheet itself reaches this.
            <div
                class=move || {
                    let base = "min-h-0 flex-1 overflow-auto pb-2";
                    if drop_target.get().as_deref() == Some("") {
                        format!("{base} ring-1 ring-rust/60 ring-inset")
                    } else {
                        base.to_string()
                    }
                }
                // The empty space is the root: a drop here moves the entry
                // out of whatever folder it was in. Rows stop propagation,
                // so only the sheet itself reaches these.
                on:dragover=move |event: ev::DragEvent| {
                    let Some(carried) = dragging.get_untracked() else {
                        return;
                    };
                    if drop_target_for(&carried, None).is_some() {
                        event.prevent_default();
                        if let Some(transfer) = event.data_transfer() {
                            transfer.set_drop_effect("move");
                        }
                        if drop_target.get_untracked().as_deref() != Some("") {
                            drop_target.set(Some(String::new()));
                        }
                    }
                }
                on:dragleave=move |_| {
                    if drop_target.get_untracked().as_deref() == Some("") {
                        drop_target.set(None);
                    }
                }
                on:drop=move |event: ev::DragEvent| {
                    event.prevent_default();
                    let carried = dragging.get_untracked();
                    dragging.set(None);
                    drop_target.set(None);
                    if let Some(carried) = carried
                        && let Some(into) = drop_target_for(&carried, None)
                    {
                        controller::move_entry(state, carried.path, into, carried.is_dir);
                    }
                }
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
                            <p class="px-3 text-footnote text-label-3">{t!("tree.empty")}</p>
                        }
                            .into_any();
                    }
                    view! { <Level entries=tree depth=0 parent=String::new() /> }.into_any()
                }}
            </div>

            {move || {
                let (x, y, target) = tree_menu.get()?;
                let close = Callback::new(move |_| tree_menu.set(None));
                let path = target.path.clone();
                let is_dir = target.is_dir;
                // Where a "New …" or a Paste from this row lands: the
                // directory itself, or a file's parent.
                let into = if is_dir {
                    path.clone()
                } else {
                    path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
                };
                let (file_into, folder_into) = (into.clone(), into.clone());

                // Paste is offered when something was cut or copied, and
                // is greyed where it could only be refused — a cut folder
                // over itself, an entry over the folder it is already in.
                let clip = state.editor.clipboard.get();
                let paste_ok = clip.as_ref().is_some_and(|clip| {
                    !clip.cut
                        || drop_target_for(
                            &TreeTarget {
                                path: clip.path.clone(),
                                is_dir: clip.is_dir,
                            },
                            Some(&TreeTarget {
                                path: into.clone(),
                                is_dir: true,
                            }),
                        )
                        .is_some()
                });
                let paste = {
                    let into = into.clone();
                    move || {
                        let Some(clip) = state.editor.clipboard.get_untracked() else {
                            return;
                        };
                        if clip.cut {
                            controller::move_entry(state, clip.path, into.clone(), clip.is_dir);
                            state.editor.clipboard.set(None);
                        } else {
                            controller::copy_entry(state, clip.path, into.clone());
                        }
                        tree_menu.set(None);
                    }
                };
                let paste_row = clip.is_some().then(|| {
                    let paste = paste.clone();
                    view! {
                        <MenuItem
                            label=t!("context.tree-paste")
                            disabled=!paste_ok
                            on_select=Callback::new(move |_| paste())
                        />
                    }
                });
                let reveal_label = if controller::host_is_windows() {
                    t!("context.tree-reveal-explorer")
                } else if controller::host_is_mac() {
                    t!("context.tree-reveal-finder")
                } else {
                    t!("context.tree-reveal")
                };

                // Right-clicking the empty sheet targets the project root:
                // creation, a paste, the root in the file manager, refresh.
                if path.is_empty() {
                    return Some(
                        view! {
                            <ContextMenu x=x y=y on_close=close>
                                <MenuItem
                                    label=t!("context.tree-new-file")
                                    on_select=Callback::new(move |_| {
                                        naming.set(Some((String::new(), false)));
                                        tree_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.tree-new-folder")
                                    on_select=Callback::new(move |_| {
                                        naming.set(Some((String::new(), true)));
                                        tree_menu.set(None);
                                    })
                                />
                                {paste_row}
                                <MenuSeparator />
                                <MenuItem
                                    label=reveal_label
                                    on_select=Callback::new(move |_| {
                                        controller::reveal_entry(state, String::new());
                                        tree_menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("context.tree-refresh")
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

                let (open_path, search_path, beside_path, reveal_path) =
                    (path.clone(), path.clone(), path.clone(), path.clone());
                let (cut_path, copy_path, rename_path, delete_path) =
                    (path.clone(), path.clone(), path.clone(), path.clone());
                let relative_path = path.clone();
                let absolute = {
                    let root = state
                        .project
                        .detected
                        .with_untracked(|p| p.as_ref().map(|p| p.root.clone()))
                        .unwrap_or_default();
                    absolute_path(&root, &path)
                };
                let float = path.clone();
                let Renaming(renaming) = expect_context::<Renaming>();
                // VS Code's order: what opens it, where it is, the clipboard,
                // the paths, then the two that change it.
                Some(
                    view! {
                        <ContextMenu x=x y=y on_close=close>
                            {if is_dir {
                                view! {
                                    <MenuItem
                                        label=t!("context.tree-new-file")
                                        on_select=Callback::new(move |_| {
                                            begin_naming(state, naming, &file_into, false);
                                            tree_menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("context.tree-new-folder")
                                        on_select=Callback::new(move |_| {
                                            begin_naming(state, naming, &folder_into, true);
                                            tree_menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("context.tree-toggle")
                                        on_select=Callback::new(move |_| {
                                            state
                                                .editor
                                                .expanded
                                                .update(|open| {
                                                    match open.iter().position(|p| p == &open_path) {
                                                        Some(at) => {
                                                            open.remove(at);
                                                        }
                                                        None => open.push(open_path.clone()),
                                                    }
                                                });
                                            tree_menu.set(None);
                                        })
                                    />
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <MenuItem
                                        label=t!("context.tree-open")
                                        on_select=Callback::new(move |_| {
                                            controller::open_file(state.focused(), open_path.clone());
                                            tree_menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("context.tree-open-beside")
                                        on_select=Callback::new(move |_| {
                                            controller::open_beside(state.focused(), beside_path.clone());
                                            tree_menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("context.tree-open-window")
                                        on_select=Callback::new(move |_| {
                                            controller::detach_file(state, float.clone());
                                            tree_menu.set(None);
                                        })
                                    />
                                }
                                    .into_any()
                            }}
                            <MenuItem
                                label=reveal_label
                                on_select=Callback::new(move |_| {
                                    controller::reveal_entry(state, reveal_path.clone());
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tree-search-scope")
                                on_select=Callback::new(move |_| {
                                    search_within(state, &search_path, is_dir);
                                    tree_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.tree-cut")
                                on_select=Callback::new(move |_| {
                                    state
                                        .editor
                                        .clipboard
                                        .set(Some(TreeClip {
                                            path: cut_path.clone(),
                                            is_dir,
                                            cut: true,
                                        }));
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tree-copy")
                                on_select=Callback::new(move |_| {
                                    state
                                        .editor
                                        .clipboard
                                        .set(Some(TreeClip {
                                            path: copy_path.clone(),
                                            is_dir,
                                            cut: false,
                                        }));
                                    tree_menu.set(None);
                                })
                            />
                            {paste_row}
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.tree-copy-path")
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&absolute);
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tree-copy-relative-path")
                                on_select=Callback::new(move |_| {
                                    copy_to_clipboard(&relative_path);
                                    tree_menu.set(None);
                                })
                            />
                            <MenuSeparator />
                            <MenuItem
                                label=t!("context.tree-rename")
                                on_select=Callback::new(move |_| {
                                    renaming.set(Some(rename_path.clone()));
                                    tree_menu.set(None);
                                })
                            />
                            <MenuItem
                                label=t!("context.tree-delete")
                                danger=true
                                on_select=Callback::new(move |_| {
                                    controller::delete_entry(state, delete_path.clone(), is_dir);
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeTarget {
    path: String,
    is_dir: bool,
}

/// The row being renamed inline, if any.
#[derive(Clone, Copy)]
struct Renaming(RwSignal<Option<String>>);

/// Where a new entry is being named — the folder's project-relative path,
/// and whether it is a folder. `""` is the project root.
#[derive(Clone, Copy)]
struct Naming(RwSignal<Option<(String, bool)>>);

/// Start naming a new entry inside `parent`, and open that folder.
///
/// The box is a row of the folder's own level, so a folder nobody has
/// expanded would put the caret somewhere nothing is drawn — a new file
/// named into a void, which is how the top-of-panel form got written in the
/// first place.
fn begin_naming(
    state: AppState,
    naming: RwSignal<Option<(String, bool)>>,
    parent: &str,
    dir: bool,
) {
    if !parent.is_empty() {
        let parent = parent.to_string();
        state.editor.expanded.update(|open| {
            if !open.iter().any(|p| p == &parent) {
                open.push(parent);
            }
        });
    }
    naming.set(Some((parent.to_string(), dir)));
}

/// The entry being dragged, and the folder a drop would land it in.
#[derive(Clone, Copy)]
struct Dragging(RwSignal<Option<TreeTarget>>);

#[derive(Clone, Copy)]
struct DropTarget(RwSignal<Option<String>>);

/// Scope the project search to one path and go there.
fn search_within(state: AppState, path: &str, is_dir: bool) {
    state.search.include.set(if is_dir {
        format!("{path}/**")
    } else {
        path.to_string()
    });
    state.layout.panel.set("search".to_string());
}
