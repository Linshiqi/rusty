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

/// One level of the tree, and every level under it.
///
/// Returns `AnyView` rather than `impl IntoView` because it calls itself: an
/// opaque return type has no fixed point, and the compiler says so with
/// "recursive opaque type" pointing at the signature.
#[component]
fn Level(entries: Vec<Entry>, depth: usize, parent: String) -> AnyView {
    let state = AppState::expect();
    let Naming(naming) = expect_context::<Naming>();

    // A new entry is named where it will be, as VS Code does it: a row inside
    // the folder it is being created in, indented with its future siblings.
    // The box used to sit above the whole tree with the folder's path beside
    // it — which is a form, not a file being made, and it said `core/src/`
    // in eleven characters of grey where the tree was already showing that
    // folder open.
    let box_here = {
        let parent = parent.clone();
        Signal::derive(move || {
            naming
                .get()
                .filter(|(at, _)| at == &parent)
                .map(|(_, dir)| dir)
        })
    };
    let new_row = {
        let parent = parent.clone();
        move || {
            box_here.get().map(|dir| {
                view! { <NewBox parent=parent.clone() dir=dir depth=depth /> }
            })
        }
    };

    let rows = entries
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
                        // Into the group the user is in, not always the first.
                        controller::open_file(state.focused(), path.clone());
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

            // Drag and drop. A row is both a thing to carry and, for a
            // folder — or a file standing in for its folder — a place to
            // drop. Every row stops the event on its way to the sheet, so
            // an invalid target is refused rather than falling through to
            // "the root".
            let Renaming(renaming) = expect_context::<Renaming>();
            let Dragging(dragging) = expect_context::<Dragging>();
            let DropTarget(drop_target) = expect_context::<DropTarget>();
            let target = TreeTarget {
                path: path.clone(),
                is_dir,
            };
            let cut = Signal::derive({
                let path = path.clone();
                move || {
                    state
                        .editor
                        .clipboard
                        .with(|clip| clip.as_ref().is_some_and(|c| c.cut && c.path == path))
                }
            });
            let receiving = Signal::derive({
                let path = path.clone();
                move || is_dir && drop_target.get().as_deref() == Some(path.as_str())
            });
            let renaming_this = Signal::derive({
                let path = path.clone();
                move || renaming.get().as_deref() == Some(path.as_str())
            });
            // Outside the module tree, so rust-analyzer offers nothing in it:
            // no completion, no hover, no jump, for ever, while the squiggles
            // keep arriving. VS Code dims a file the project does not build,
            // and until this there was nothing on screen that said so — the
            // diagnostic is a hint the Problems panel filters out, and the
            // user's report was exactly "现在看不出来".
            //
            // Two sources, the authoritative one winning: rust-analyzer's own
            // `unlinked-file` for a file somebody has opened, and rusty's
            // reading of the `mod` declarations for the rest (see
            // `rusty_edit::modules`, which refuses wherever it cannot be
            // sure). A folder is never dimmed — a directory is not a module.
            let unlinked = Signal::derive({
                let path = path.clone();
                move || !is_dir && state.is_unlinked(&path)
            });
            // What is wrong in it, as VS Code's explorer says it: a file with
            // errors in red with how many, one with only warnings in amber,
            // and a folder in the colour of the worst thing inside it — so a
            // compile error in a file nobody has open is visible without
            // building. The check's results cover every file, not only the
            // open ones, which is what makes the folders worth colouring.
            let mark = Signal::derive({
                let path = path.clone();
                move || {
                    state
                        .lsp
                        .diagnostics
                        .with(|by_file| problem_mark(by_file, &path, is_dir))
                }
            });
            let on_dragstart = {
                let target = target.clone();
                move |event: ev::DragEvent| {
                    if let Some(transfer) = event.data_transfer() {
                        let _ = transfer.set_data("text/plain", &target.path);
                        transfer.set_effect_allowed("move");
                    }
                    dragging.set(Some(target.clone()));
                }
            };
            let on_dragend = move |_| {
                dragging.set(None);
                drop_target.set(None);
            };
            let on_dragover = {
                let target = target.clone();
                move |event: ev::DragEvent| {
                    event.stop_propagation();
                    let Some(carried) = dragging.get_untracked() else {
                        return;
                    };
                    match drop_target_for(&carried, Some(&target)) {
                        Some(into) => {
                            event.prevent_default();
                            if let Some(transfer) = event.data_transfer() {
                                transfer.set_drop_effect("move");
                            }
                            if drop_target.get_untracked().as_deref() != Some(into.as_str()) {
                                drop_target.set(Some(into));
                            }
                        }
                        None => {
                            if drop_target.get_untracked().is_some() {
                                drop_target.set(None);
                            }
                        }
                    }
                }
            };
            let on_drop = {
                let target = target.clone();
                move |event: ev::DragEvent| {
                    event.stop_propagation();
                    event.prevent_default();
                    let carried = dragging.get_untracked();
                    dragging.set(None);
                    drop_target.set(None);
                    if let Some(carried) = carried
                        && let Some(into) = drop_target_for(&carried, Some(&target))
                    {
                        controller::move_entry(state, carried.path, into, carried.is_dir);
                    }
                }
            };

            let name = entry.name.clone();
            let row_path = path.clone();
            view! {
                {move || {
                    if renaming_this.get() {
                        return view! {
                            <RenameBox
                                path=row_path.clone()
                                original=name.clone()
                                depth=depth
                                is_dir=is_dir
                            />
                        }
                            .into_any();
                    }
                    let name = name.clone();
                    view! {
                        <button
                            type="button"
                            draggable="true"
                            on:click=activate.clone()
                            on:contextmenu=menu.clone()
                            on:dragstart=on_dragstart.clone()
                            on:dragend=on_dragend
                            on:dragover=on_dragover.clone()
                            on:drop=on_drop.clone()
                            style=format!("padding-left: {}px", 10 + depth * 12)
                            class=move || {
                                let base = "flex w-full items-center gap-1.5 py-[3px] pr-2 text-left \
                                            text-callout transition-colors";
                                let tone = match (selected.get(), mark.get()) {
                                    (true, _) => "bg-selection text-rust",
                                    (false, Some(Mark { errors, .. })) if errors > 0 => {
                                        "text-crimson hover:bg-sunken"
                                    }
                                    (false, Some(_)) => "text-amber hover:bg-sunken",
                                    (false, None) => "text-label-2 hover:bg-sunken hover:text-label",
                                };
                                let drop = if receiving.get() {
                                    " bg-selection/60 ring-1 ring-rust/70 ring-inset"
                                } else {
                                    ""
                                };
                                let dim = if cut.get() {
                                    " opacity-50"
                                } else if unlinked.get() {
                                    // Weaker than a cut row, which is a state
                                    // the user just put it in and will undo in
                                    // a moment; this one is how the file sits.
                                    " opacity-60"
                                } else {
                                    ""
                                };
                                format!("{base} {tone}{drop}{dim}")
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
                            <span class="min-w-0 flex-1 truncate">{name}</span>
                            // The reason, on hover, where a shade of grey
                            // cannot say one. A dim row with no explanation
                            // is a rendering bug as far as anyone can tell.
                            {move || {
                                unlinked
                                    .get()
                                    .then(|| {
                                        view! {
                                            <span
                                                class="shrink-0 leading-none text-label-3"
                                                title=t!("tree.unlinked")
                                            >
                                                "◌"
                                            </span>
                                        }
                                    })
                            }}
                            {move || {
                                mark.get().map(|Mark { errors, warnings }| {
                                    let (count, tone) = if errors > 0 {
                                        (errors, "text-crimson")
                                    } else {
                                        (warnings, "text-amber")
                                    };
                                    // A file says how many; a folder only that
                                    // there is something, since a sum over a
                                    // whole subtree is a number nobody acts on.
                                    let text = if is_dir { "●".to_string() } else { count.to_string() };
                                    view! {
                                        <span class=format!("shrink-0 text-caption tnum {tone}")>{text}</span>
                                    }
                                })
                            }}
                        </button>
                    }
                        .into_any()
                }}

                <Show when=move || is_dir && open.get()>
                    <Level entries=children.clone() depth=depth + 1 parent=path.clone() />
                </Show>
            }
        })
        .collect_view();

    // First, where VS Code puts it, and where an alphabetical tree would put
    // a name nobody has typed yet.
    view! { {new_row} {rows} }.into_any()
}

/// A new entry being named, as a row of the folder it is being created in.
///
/// The sibling of [`RenameBox`], and the same shape on purpose: the same
/// indent, the same keys, the same commit-on-blur. VS Code makes a file by
/// growing a row where the file will be, and the difference from a form at
/// the top of the panel is that you can see what you are naming it *beside*.
#[component]
fn NewBox(parent: String, dir: bool, depth: usize) -> impl IntoView {
    let state = AppState::expect();
    let Naming(naming) = expect_context::<Naming>();
    let input: NodeRef<html::Input> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(input) = input.get() {
            let _ = input.focus();
        }
    });

    let commit = {
        let parent = parent.clone();
        move |value: String| {
            naming.set(None);
            // A name is a name: a separator in it would be a path into a
            // folder the tree is not showing, which is the rename rule.
            let name = value.trim().trim_matches('/');
            if name.is_empty() {
                return;
            }
            let path = if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            };
            controller::create_entry(state, path, dir);
        }
    };
    let on_key = {
        let commit = commit.clone();
        move |event: ev::KeyboardEvent| match event.key().as_str() {
            "Enter" => commit(event_target_value(&event)),
            "Escape" => naming.set(None),
            _ => {}
        }
    };
    // Clicking away is giving up, not creating `` — the empty name is what
    // "I changed my mind" looks like, and `commit` refuses it.
    let on_blur = move |event: ev::FocusEvent| {
        if naming.get_untracked().is_some() {
            commit(event_target_value(&event));
        }
    };
    let hint = if dir {
        t!("tree.folder-name")
    } else {
        t!("tree.file-name")
    };

    view! {
        <div
            class="flex w-full items-center gap-1.5 py-[2px] pr-2"
            style=format!("padding-left: {}px", 10 + depth * 12)
        >
            <span class="w-3 shrink-0 text-center text-footnote text-label-3">
                {if dir { "\u{25b8}" } else { "" }}
            </span>
            <input
                node_ref=input
                placeholder=hint
                on:keydown=on_key
                on:blur=on_blur
                class="min-w-0 flex-1 rounded-[4px] bg-sunken px-1 py-0 font-mono text-callout outline-none ring-1 ring-rust"
            />
        </div>
    }
}

/// A row's name as an input: Enter renames, Escape gives up, and leaving
/// the box commits what was typed, as VS Code does. The stem is selected
/// on open, so typing replaces the name and keeps the extension.
#[component]
fn RenameBox(path: String, original: String, depth: usize, is_dir: bool) -> impl IntoView {
    let state = AppState::expect();
    let Renaming(renaming) = expect_context::<Renaming>();
    let input: NodeRef<html::Input> = NodeRef::new();
    let stem = original
        .rfind('.')
        .filter(|at| *at > 0 && !is_dir)
        .unwrap_or(original.len());
    Effect::new(move |_| {
        if let Some(input) = input.get() {
            let _ = input.focus();
            let _ = input.set_selection_range(0, stem as u32);
        }
    });
    let commit = {
        let path = path.clone();
        let original = original.clone();
        move |value: String| {
            renaming.set(None);
            let value = value.trim().to_string();
            if !value.is_empty() && value != original {
                controller::rename_entry(state, path.clone(), value, is_dir);
            }
        }
    };
    let on_key = {
        let commit = commit.clone();
        move |event: ev::KeyboardEvent| match event.key().as_str() {
            "Enter" => commit(event_target_value(&event)),
            "Escape" => renaming.set(None),
            _ => {}
        }
    };
    let on_blur = move |event: ev::FocusEvent| {
        // Escape already closed the box; a blur that follows it must not
        // rename with the text that was being abandoned.
        if renaming.get_untracked().as_deref() == Some(path.as_str()) {
            commit(event_target_value(&event));
        }
    };
    view! {
        <div
            class="flex w-full items-center gap-1.5 py-[2px] pr-2"
            style=format!("padding-left: {}px", 10 + depth * 12)
        >
            <span class="w-3 shrink-0"></span>
            <input
                node_ref=input
                type="text"
                spellcheck="false"
                value=original
                class="min-w-0 flex-1 rounded-[4px] bg-sunken px-1 py-0 font-mono text-callout outline-none ring-1 ring-rust"
                on:keydown=on_key
                on:blur=on_blur
                on:click=move |event: ev::MouseEvent| event.stop_propagation()
            />
        </div>
    }
}

/// Where a drop would put `carried`: the folder under the pointer, a
/// file's folder, or the root when `over` is the empty sheet — and `None`
/// where nothing would move: the folder it is already in, or a folder
/// inside itself.
fn drop_target_for(carried: &TreeTarget, over: Option<&TreeTarget>) -> Option<String> {
    let into = match over {
        None => String::new(),
        Some(row) if row.is_dir => row.path.clone(),
        Some(row) => parent_of(&row.path),
    };
    if parent_of(&carried.path) == into {
        return None;
    }
    if carried.is_dir && (into == carried.path || into.starts_with(&format!("{}/", carried.path))) {
        return None;
    }
    Some(into)
}

fn parent_of(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str, is_dir: bool) -> TreeTarget {
        TreeTarget {
            path: path.to_string(),
            is_dir,
        }
    }

    #[test]
    fn a_drop_lands_in_the_folder_under_the_pointer_or_a_files_folder_or_the_root() {
        let file = at("src/main.rs", false);
        assert_eq!(
            drop_target_for(&file, Some(&at("firmware", true))).as_deref(),
            Some("firmware")
        );
        assert_eq!(
            drop_target_for(&file, Some(&at("firmware/Cargo.toml", false))).as_deref(),
            Some("firmware")
        );
        assert_eq!(drop_target_for(&file, None).as_deref(), Some(""));
        // Already there: nothing to do, so nothing to offer.
        assert_eq!(drop_target_for(&file, Some(&at("src", true))), None);
        assert_eq!(drop_target_for(&file, Some(&at("src/lib.rs", false))), None);
        assert_eq!(drop_target_for(&at("Cargo.toml", false), None), None);
    }

    #[test]
    fn a_folder_never_drops_into_itself_or_below() {
        let src = at("src", true);
        assert_eq!(drop_target_for(&src, Some(&src)), None);
        assert_eq!(drop_target_for(&src, Some(&at("src/驱动", true))), None);
        assert_eq!(drop_target_for(&src, Some(&at("src/main.rs", false))), None);
        assert_eq!(
            drop_target_for(&src, Some(&at("firmware", true))).as_deref(),
            Some("firmware")
        );
        // `src2` is beside `src`, not inside it.
        assert_eq!(
            drop_target_for(&src, Some(&at("src2", true))).as_deref(),
            Some("src2")
        );
    }
}

/// How much is wrong in one tree row: its errors and warnings, or those of
/// everything under it for a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Mark {
    pub errors: usize,
    pub warnings: usize,
}

/// The mark for `path`, or `None` when nothing there is an error or a
/// warning. Hints and information are not problems — the Problems panel's
/// rule — so a crate full of `#[cfg]`-inactive code is not painted amber.
/// A folder counts what is under it by path, with the separator: `src2` is
/// not under `src`.
pub(super) fn problem_mark(
    by_file: &std::collections::HashMap<String, Vec<rusty_lsp::FileDiagnostic>>,
    path: &str,
    is_dir: bool,
) -> Option<Mark> {
    use rusty_lsp::DiagSeverity;

    let prefix = format!("{}/", path.trim_end_matches('/'));
    let mut mark = Mark {
        errors: 0,
        warnings: 0,
    };
    for (file, items) in by_file {
        let inside = if is_dir {
            path.is_empty() || file.starts_with(&prefix)
        } else {
            file == path
        };
        if !inside {
            continue;
        }
        for item in items {
            match item.severity {
                DiagSeverity::Error => mark.errors += 1,
                DiagSeverity::Warning => mark.warnings += 1,
                _ => {}
            }
        }
    }
    (mark.errors + mark.warnings > 0).then_some(mark)
}

#[cfg(test)]
mod mark_tests {
    use std::collections::HashMap;

    use rusty_lsp::{DiagSeverity, FileDiagnostic};

    use super::*;

    fn items(severities: &[DiagSeverity]) -> Vec<FileDiagnostic> {
        severities
            .iter()
            .map(|severity| FileDiagnostic {
                severity: *severity,
                message: String::new(),
                source: Some("rustc".into()),
                code: None,
                start_line: 0,
                start_col: 0,
                end_line: 0,
                end_col: 0,
            })
            .collect()
    }

    /// The report: an error in `core/src/math/quaternion.rs` marks the file
    /// and each folder above it, and nothing beside it.
    #[test]
    fn an_error_marks_its_file_and_every_folder_above_it() {
        let by_file = HashMap::from([(
            "core/src/math/quaternion.rs".to_string(),
            items(&[
                DiagSeverity::Error,
                DiagSeverity::Warning,
                DiagSeverity::Hint,
            ]),
        )]);
        let file = problem_mark(&by_file, "core/src/math/quaternion.rs", false);
        assert_eq!(
            file,
            Some(Mark {
                errors: 1,
                warnings: 1
            }),
            "a hint is not a problem"
        );
        for folder in ["core", "core/src", "core/src/math"] {
            assert_eq!(
                problem_mark(&by_file, folder, true).map(|m| m.errors),
                Some(1),
                "{folder}"
            );
        }
        assert_eq!(
            problem_mark(&by_file, "core/src/math/vector.rs", false),
            None
        );
        assert_eq!(problem_mark(&by_file, "firmware", true), None);
    }

    /// A folder whose name another begins with is not its parent, and a file
    /// with only hints is unmarked.
    #[test]
    fn a_prefix_is_not_a_parent_and_hints_mark_nothing() {
        let by_file = HashMap::from([
            ("src2/lib.rs".to_string(), items(&[DiagSeverity::Error])),
            (
                "src/main.rs".to_string(),
                items(&[DiagSeverity::Hint, DiagSeverity::Info]),
            ),
        ]);
        assert_eq!(problem_mark(&by_file, "src", true), None);
        assert_eq!(problem_mark(&by_file, "src/main.rs", false), None);
        assert_eq!(
            problem_mark(&by_file, "src2", true).map(|m| m.errors),
            Some(1)
        );
    }
}
