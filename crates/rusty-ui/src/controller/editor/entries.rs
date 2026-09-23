//! The tree's own verbs — create, move, copy, rename, delete, reveal — and
//! the state that follows an entry when it moves.

use super::*;

/// Create a file or directory, then show it — the tree refreshes, and a new
/// file opens in the editor, because "New file" means "I want to type in it".
pub fn create_entry(state: AppState, path: String, dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        dir: bool,
    }

    if !state.has_project_now() {
        return;
    }
    let opened = path.clone();
    let args = Args { path, dir };
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::CREATE, &args).await },
        move |()| {
            refresh_tree(state);
            if !dir {
                open_file(state, opened.clone());
            }
        },
    );
}

/// Where a moved or renamed path went, for everything that holds paths.
///
/// A file is the path itself; a directory is the path and everything under
/// it, so `src` → `firmware/src` takes `src/main.rs` along. `None` for a
/// path the move did not touch — `src2/a.rs` is not under `src`, which is
/// why the separator is checked and not just the prefix.
pub(crate) fn retarget(path: &str, from: &str, to: &str, is_dir: bool) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    if is_dir
        && let Some(rest) = path.strip_prefix(from)
        && rest.starts_with('/')
    {
        return Some(format!("{to}{rest}"));
    }
    None
}

/// Move the tabs, the parked editors, the document on screen, the expanded
/// folders and the source-view choices along with a path that moved on disk.
///
/// Before the watcher hears about it, on purpose: its batch would find the
/// old paths gone and mark every open file under them as vanished, when
/// they are all still there under the new name. rust-analyzer learns the
/// new name from the buffer the editor already holds.
fn follow_move(state: AppState, from: &str, to: &str, is_dir: bool) {
    let moved = |path: &str| retarget(path, from, to, is_dir);
    let mut announced = std::collections::HashSet::new();
    for group in state.open_groups() {
        group.editor.tabs.update(|tabs| {
            for tab in tabs.iter_mut() {
                if let Some(new) = moved(tab) {
                    *tab = new;
                }
            }
        });
        let mut reopened = Vec::new();
        group.editor.parked.update(|parked| {
            for entry in parked.iter_mut() {
                if let Some(new) = moved(&entry.document.path) {
                    let old = std::mem::replace(&mut entry.document.path, new.clone());
                    reopened.push((old, new, entry.draft.clone()));
                }
            }
        });
        let active = group.editor.document.with_untracked(|d| {
            d.as_ref()
                .and_then(|d| moved(&d.path).map(|new| (d.path.clone(), new)))
        });
        if let Some((old, new)) = active {
            group.editor.document.update(|d| {
                if let Some(d) = d {
                    d.path = new.clone();
                }
            });
            reopened.push((old, new, group.editor.draft.get_untracked()));
        }
        // The old name closed and the new one opened: left open, the server
        // kept a document at a path that no longer exists — a second copy
        // of the module, under its old name, for as long as the session ran.
        // Once per file: a file open on both sides moved once.
        for (old, new, text) in reopened {
            if announced.insert(old.clone()) {
                lsp_closed_doc(old);
                lsp_open_doc(new, text);
            }
        }
    }
    // The undo history is the file's, under whatever it is called now.
    state.editor.histories.update_value(|all| {
        let moved: Vec<(String, String)> = all
            .keys()
            .filter_map(|path| moved(path).map(|new| (path.clone(), new)))
            .collect();
        for (old, new) in moved {
            if let Some(history) = all.remove(&old) {
                all.insert(new, history);
            }
        }
    });
    let rename_all = |list: &mut Vec<String>| {
        for path in list.iter_mut() {
            if let Some(new) = moved(path) {
                *path = new;
            }
        }
    };
    state.editor.expanded.update(rename_all);
    state.editor.source_view.update(rename_all);
    state.editor.stale.update(rename_all);
    // Pictures are cached by path; the next look re-reads them.
    state
        .editor
        .images
        .update(|images| images.retain(|path, _| moved(path).is_none()));
    remember_tabs(state);
}

/// Move an entry into a directory (`""` for the root): a drop on the tree,
/// or Cut followed by Paste. The backend answers with where it landed.
pub fn move_entry(state: AppState, from: String, into: String, is_dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        into: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args {
        from: from.clone(),
        into,
    };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::MOVE, &args).await },
        move |to| {
            if to != from {
                follow_move(state, &from, &to, is_dir);
            }
            refresh_tree(state);
        },
    );
}

/// Copy an entry into a directory. The copy takes a free name where its
/// own is taken, so pasting beside the original is ` copy`.
pub fn copy_entry(state: AppState, from: String, into: String) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        into: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args { from, into };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::COPY, &args).await },
        move |_created| refresh_tree(state),
    );
}

/// Rename an entry in place. The open tabs follow, as after a move.
pub fn rename_entry(state: AppState, from: String, name: String, is_dir: bool) {
    #[derive(serde::Serialize)]
    struct Args {
        from: String,
        name: String,
    }

    if !state.has_project_now() {
        return;
    }
    let args = Args {
        from: from.clone(),
        name,
    };
    track(
        state,
        async move { ipc::call::<_, String>(cmd::files::RENAME, &args).await },
        move |to| {
            if to != from {
                follow_move(state, &from, &to, is_dir);
            }
            refresh_tree(state);
        },
    );
}

/// Move an entry to the recycle bin, after asking. The question names the
/// bin the platform has, and every tab under the entry closes with it.
pub fn delete_entry(state: AppState, path: String, is_dir: bool) {
    if !state.has_project_now() {
        return;
    }
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let question = if host_is_windows() {
        t!("tree.delete-confirm-bin", name = name)
    } else {
        t!("tree.delete-confirm-trash", name = name)
    };
    spawn_local(async move {
        // Through `ipc::confirm`, never `window.confirm`: see `close_tab`.
        if !ipc::confirm(&question).await {
            return;
        }
        let args = PathArg { path: path.clone() };
        track(
            state,
            async move { ipc::call::<_, ()>(cmd::files::DELETE, &args).await },
            move |()| {
                close_under(state, &path, is_dir);
                refresh_tree(state);
            },
        );
    });
}

/// Close every tab at or under a path that is gone. Without asking: the
/// question was asked about the file, and a draft of a deleted file has
/// nowhere left to be saved.
fn close_under(state: AppState, path: &str, is_dir: bool) {
    for group in state.open_groups() {
        let doomed: Vec<String> = group
            .editor
            .tabs
            .get_untracked()
            .into_iter()
            .filter(|tab| retarget(tab, path, "", is_dir).is_some())
            .collect();
        for tab in doomed {
            remove_tab(group, tab);
        }
    }
}

/// Select the entry in the platform's file manager.
pub fn reveal_entry(state: AppState, path: String) {
    track(
        state,
        async move { ipc::call::<_, ()>(cmd::files::REVEAL, &PathArg { path }).await },
        |()| {},
    );
}

/// Re-read the project tree, and which of its files the module tree reaches.
pub fn refresh_tree(state: AppState) {
    if !state.has_project_now() {
        return;
    }
    track(
        state,
        async move { ipc::call::<_, Vec<Entry>>(cmd::files::TREE, &()).await },
        move |entries| state.editor.tree.set(entries),
    );
    refresh_unlinked(state);
}

/// Which `.rs` files no `mod` declaration reaches, for the tree to dim.
///
/// Not through `track`: a project whose module tree cannot be read is not a
/// failure worth a banner — the answer is a shade of grey. `None` is the
/// refusal (`rusty_edit::modules` claims nothing where it cannot be sure)
/// and travels as itself, so nothing here can turn a doubt into a dim, and
/// a call that fails outright leaves the last claim alone rather than
/// silently un-dimming every file in the project.
pub fn refresh_unlinked(state: AppState) {
    spawn_local(async move {
        if let Ok(claim) = ipc::call::<_, Option<Vec<String>>>(cmd::files::UNLINKED, &()).await {
            set_if_changed(state.editor.unlinked, claim);
        }
    });
}

/// Fold the file tree away or bring it back: the Files switcher's second
/// click, Ctrl+B and the View menu.
pub fn toggle_tree(state: AppState) {
    let hidden = !state.layout.tree_hidden.get_untracked();
    state.layout.tree_hidden.set(hidden);
    crate::state::remember_tree_hidden(hidden);
}
