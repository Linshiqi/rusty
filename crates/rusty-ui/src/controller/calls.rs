//! Who calls a function, and what it calls: the call hierarchy in the dock's
//! Calls tab, a level at a time (`crate::calls` is the tree, `view/dock/
//! calls.rs` draws it).
//!
//! The dock belongs to no editor group, so everything here after the first
//! ask works on `state.layout.calls` alone, and a jump goes to the group in
//! focus.

use leptos::prelude::*;
use leptos::task::spawn_local;

use rusty_i18n::t;

use super::*;
use crate::{
    calls::CallTree,
    ipc::{self, cmd},
    state::{AppState, CallsView, DockTab, LspStatus, PlaceList},
};

/// Every ask a number, across trees, so an answer knows which it is for.
fn next_serial() -> u64 {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Start a hierarchy at the function under this group's caret, and show it:
/// VS Code's "Show Call Hierarchy". Calls in first, as VS Code opens it, or
/// whichever way the last hierarchy was being read.
pub fn show_call_hierarchy(state: AppState) {
    #[derive(serde::Serialize)]
    struct Args {
        path: String,
        line: u32,
        col: u32,
    }

    let Some(path) = state.active_path_now().filter(|path| path.ends_with(".rs")) else {
        return;
    };
    if state.lsp.status.get_untracked() != LspStatus::Ready {
        return;
    }
    let Some((line, col)) = caret_position(state) else {
        return;
    };
    let word = state
        .editor
        .draft
        .with_untracked(|text| word_around(text, line, col));
    let incoming = state.layout.calls.with_untracked(|view| match view {
        CallsView::Tree(tree) => tree.incoming,
        _ => true,
    });
    let serial = next_serial();
    state.layout.calls.set(CallsView::Asking(serial));
    state.show_dock(DockTab::Calls);
    let args = Args { path, line, col };
    spawn_local(async move {
        let answer =
            ipc::call::<_, Vec<rusty_lsp::CallItem>>(cmd::lsp::CALL_HIERARCHY, &args).await;
        if state
            .layout
            .calls
            .with_untracked(|view| *view != CallsView::Asking(serial))
        {
            return;
        }
        match answer.ok().and_then(|items| items.into_iter().next()) {
            Some(root) => {
                state
                    .layout
                    .calls
                    .set(CallsView::Tree(CallTree::new(serial, root, incoming)));
                open_call(state, 0);
            }
            None => state.layout.calls.set(CallsView::NoFunction(word)),
        }
    });
}

/// Read the same function's calls the other way: who calls it, or what it
/// calls. The tree starts again from the function, opened.
pub fn set_call_direction(state: AppState, incoming: bool) {
    let root = state.layout.calls.with_untracked(|view| match view {
        CallsView::Tree(tree) if tree.incoming != incoming => Some(tree.root().clone()),
        _ => None,
    });
    let Some(root) = root else {
        return;
    };
    let tree = CallTree::new(next_serial(), root, incoming);
    state.layout.calls.set(CallsView::Tree(tree));
    open_call(state, 0);
}

/// Open a row of the tree: ask for its function's calls, and put them under
/// it when they come — if the tree is still the one that asked.
pub fn open_call(state: AppState, id: u64) {
    #[derive(serde::Serialize)]
    struct Args {
        item: String,
        incoming: bool,
    }

    let mut ask = None;
    state.layout.calls.update(|view| {
        if let CallsView::Tree(tree) = view {
            ask = tree.open(id).map(|item| {
                (
                    tree.serial,
                    Args {
                        item,
                        incoming: tree.incoming,
                    },
                )
            });
        }
    });
    let Some((serial, args)) = ask else {
        return;
    };
    spawn_local(async move {
        let answer = ipc::call::<_, Vec<rusty_lsp::Call>>(cmd::lsp::CALLS, &args).await;
        state.layout.calls.update(|view| {
            if let CallsView::Tree(tree) = view
                && tree.serial == serial
            {
                match answer {
                    Ok(calls) => tree.answer(id, calls),
                    // The server warming up, most likely: closed again, and
                    // a second click asks again.
                    Err(_) => tree.failed(id),
                }
            }
        });
    });
}

/// Close a row, and everything opened under it.
pub fn close_call(state: AppState, id: u64) {
    state.layout.calls.update(|view| {
        if let CallsView::Tree(tree) = view {
            tree.close(id);
        }
    });
}

/// Go where a row's call is made — in the caller, which is the row for calls
/// in and the row above it for calls out. The first row made no call, so it
/// goes to its function.
pub fn go_to_call(state: AppState, id: u64) {
    let target = state.layout.calls.with_untracked(|view| match view {
        CallsView::Tree(tree) => tree.rows.iter().find(|row| row.id == id).map(|row| {
            row.call
                .sites
                .first()
                .unwrap_or(&row.call.item.place)
                .location
                .clone()
        }),
        _ => None,
    });
    if let Some(location) = target {
        go_to(state.focused(), location);
    }
}

/// Go to a row's function itself.
pub fn go_to_called(state: AppState, id: u64) {
    let target = state.layout.calls.with_untracked(|view| match view {
        CallsView::Tree(tree) => tree
            .rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.call.item.place.location.clone()),
        _ => None,
    });
    if let Some(location) = target {
        go_to(state.focused(), location);
    }
}

/// Every place a row's calls are made, in the finder — for a row calling
/// more than once, whose click can go to only the first.
pub fn list_call_sites(state: AppState, id: u64) {
    let list = state.layout.calls.with_untracked(|view| {
        let CallsView::Tree(tree) = view else {
            return None;
        };
        let at = tree.rows.iter().position(|row| row.id == id)?;
        let row = &tree.rows[at];
        // The row it is under: the depth above it, nearest first.
        let parent = tree.rows[..at]
            .iter()
            .rev()
            .find(|above| above.depth < row.depth)?;
        let (caller, callee) = if tree.incoming {
            (&row.call.item.name, &parent.call.item.name)
        } else {
            (&parent.call.item.name, &row.call.item.name)
        };
        Some(PlaceList {
            title: t!(
                "calls.sites-title",
                caller = caller.clone(),
                callee = callee.clone()
            ),
            places: row.call.sites.clone(),
        })
    });
    if let Some(list) = list {
        state.layout.quick_places.set(Some(list));
        state.layout.quick_seed.set(String::new());
        state.layout.quick_open.set(true);
    }
}
