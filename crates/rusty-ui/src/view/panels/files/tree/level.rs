//! One level of the tree: its rows, what a drag may drop where, and the
//! boxes that name a new entry or rename one in place, drawn as rows of
//! the level they belong to.

use super::*;

/// One level of the tree, and every level under it.
///
/// Returns `AnyView` rather than `impl IntoView` because it calls itself: an
/// opaque return type has no fixed point, and the compiler says so with
/// "recursive opaque type" pointing at the signature.
#[component]
pub(super) fn Level(entries: Vec<Entry>, depth: usize, parent: String) -> AnyView {
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
pub(super) fn drop_target_for(carried: &TreeTarget, over: Option<&TreeTarget>) -> Option<String> {
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
