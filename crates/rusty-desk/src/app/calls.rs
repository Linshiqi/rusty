//! Who calls a function, and what it calls: the call hierarchy in the
//! bottom dock's Calls panel, a level at a time — VS Code's "Show Call
//! Hierarchy". `crate::calls` is the tree, `ui/calls.slint` draws it.
//!
//! Each level is asked of the server when its row is opened, handing back
//! the item the server sent exactly as it sent it: the server puts in it
//! what it needs to answer the next level. The panel belongs to no editor
//! group, so a jump from it goes to the group with the keys.

use std::rc::Rc;

use rusty_i18n::t;
use rusty_lsp::{Call, CallItem, Location};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{
    CallLine, Shell,
    calls::{CallTree, Openness},
    picker::Mode,
};

/// What the panel shows.
#[derive(Default)]
enum View {
    /// Nothing asked yet: how to ask.
    #[default]
    Idle,
    /// The function at the caret asked for, under this number.
    Asking(u64),
    /// A sentence in place of a tree: no function there, or the server's
    /// refusal.
    Said(String),
    Tree(CallTree),
}

pub(super) struct Calls {
    view: View,
    /// Which way the next hierarchy reads: calls in, as VS Code opens it,
    /// or whichever way the last one was turned.
    incoming: bool,
    /// The file the hierarchy was asked from: its server answers every
    /// level, wherever the functions in it are.
    path: String,
    /// Every ask a number, so an answer knows which tree it is for.
    serial: u64,
}

impl Default for Calls {
    fn default() -> Self {
        Calls {
            view: View::Idle,
            incoming: true,
            path: String::new(),
            serial: 0,
        }
    }
}

impl Calls {
    /// Whether there is anything to show: the panel comes to the dock's
    /// strip once a hierarchy has been asked for.
    pub(super) fn is_idle(&self) -> bool {
        matches!(self.view, View::Idle)
    }

    /// Another project opened: a hierarchy is about the project it was
    /// asked in. Which way the next one reads is kept.
    pub(super) fn forget(&mut self) {
        self.view = View::Idle;
        self.path.clear();
    }

    fn tree(&self) -> Option<&CallTree> {
        match &self.view {
            View::Tree(tree) => Some(tree),
            _ => None,
        }
    }

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }
}

impl App {
    /// "Show call hierarchy": the function at the caret, the calls into it
    /// (or out of it) asked for at once, in the Calls panel.
    pub(super) fn show_call_hierarchy(self: &Rc<Self>) {
        let Some((path, line, col, draft)) = self.editor.caret_question() else {
            return;
        };
        if !self.servers.serves(&path) {
            self.note_transient(0, t!("desk.nothing-found"));
            return;
        }
        let word = self.editor.caret_word().unwrap_or_default();
        let serial = {
            let mut state = self.state.borrow_mut();
            let calls = &mut state.calls;
            let serial = calls.next_serial();
            calls.view = View::Asking(serial);
            calls.path = path.clone();
            serial
        };
        self.show_panel("bottom", "calls");
        self.sync_calls();
        self.servers.ask_always(
            &path,
            Some(draft),
            move |client, path| client.call_hierarchy(path, line, col),
            move |app, items| app.calls_rooted(serial, word, items),
        );
    }

    /// The function the hierarchy starts at — the first the server named —
    /// and its calls asked for; or why there is none.
    fn calls_rooted(
        self: &Rc<Self>,
        serial: u64,
        word: String,
        answer: Result<Vec<CallItem>, String>,
    ) {
        let rooted = {
            let mut state = self.state.borrow_mut();
            let calls = &mut state.calls;
            if !matches!(calls.view, View::Asking(asked) if asked == serial) {
                return;
            }
            let (view, rooted) = match answer {
                Ok(items) => match items.into_iter().next() {
                    Some(root) => (
                        View::Tree(CallTree::new(serial, root, calls.incoming)),
                        true,
                    ),
                    None if word.is_empty() => (View::Said(t!("calls.no-function")), false),
                    None => (View::Said(t!("calls.not-a-function", name = word)), false),
                },
                Err(reason) => (
                    View::Said(t!(
                        "desk.server-could-not",
                        server = server_of(&calls.path),
                        reason = reason
                    )),
                    false,
                ),
            };
            calls.view = view;
            rooted
        };
        if rooted {
            self.calls_open(0);
        } else {
            self.sync_calls();
        }
    }

    /// A row's chevron: its calls asked for, or put away.
    pub(super) fn calls_toggle(self: &Rc<Self>, id: u64) {
        let open = self
            .state
            .borrow()
            .calls
            .tree()
            .and_then(|tree| tree.row(id))
            .map(|row| row.open);
        match open {
            Some(Openness::Closed) => self.calls_open(id),
            Some(Openness::Open) => {
                if let View::Tree(tree) = &mut self.state.borrow_mut().calls.view {
                    tree.close(id);
                }
                self.sync_calls();
            }
            Some(Openness::Asking) | None => {}
        }
    }

    /// Ask for a row's calls, and put them under it when they come — if the
    /// tree is still the one that asked.
    fn calls_open(self: &Rc<Self>, id: u64) {
        let ask = {
            let mut state = self.state.borrow_mut();
            let calls = &mut state.calls;
            let path = calls.path.clone();
            match &mut calls.view {
                View::Tree(tree) => tree
                    .open(id)
                    .map(|item| (tree.serial, tree.incoming, item, path)),
                _ => None,
            }
        };
        self.sync_calls();
        let Some((serial, incoming, item, path)) = ask else {
            return;
        };
        let asked = self.servers.ask_always(
            &path,
            None,
            move |client, _| {
                if incoming {
                    client.incoming_calls(&item)
                } else {
                    client.outgoing_calls(&item)
                }
            },
            move |app, answer| app.calls_answered(serial, id, answer),
        );
        // The server went — a project switch, a restart — and the row is
        // closed again rather than left asking.
        if !asked {
            self.calls_answered(serial, id, Err(String::new()));
        }
    }

    fn calls_answered(self: &Rc<Self>, serial: u64, id: u64, answer: Result<Vec<Call>, String>) {
        {
            let mut state = self.state.borrow_mut();
            let View::Tree(tree) = &mut state.calls.view else {
                return;
            };
            if tree.serial != serial {
                return;
            }
            match answer {
                Ok(calls) => tree.answer(id, calls),
                // The server warming up, most likely: closed again, and the
                // chevron asks again.
                Err(_) => tree.failed(id),
            }
        }
        self.sync_calls();
    }

    /// A row clicked: where its call is made — in the caller, which is the
    /// row for calls in and the row above for calls out. Double-clicked:
    /// its function.
    pub(super) fn calls_go(self: &Rc<Self>, id: u64, function: bool) {
        let target: Option<Location> = {
            let state = self.state.borrow();
            state.calls.tree().and_then(|tree| tree.row(id)).map(|row| {
                if function {
                    row.function().clone()
                } else {
                    row.call_site().clone()
                }
            })
        };
        if let Some(location) = target {
            self.jump(&location);
        }
    }

    /// Every place a row's calls are made, in the picker: a click goes to
    /// the first.
    pub(super) fn calls_sites(self: &Rc<Self>, id: u64) {
        let list = {
            let state = self.state.borrow();
            state.calls.tree().and_then(|tree| {
                let row = tree.row(id)?;
                let (caller, called) = tree.ends(id)?;
                Some((
                    t!(
                        "calls.sites-title",
                        caller = caller.name.clone(),
                        callee = called.name.clone()
                    ),
                    row.call.sites.clone(),
                ))
            })
        };
        if let Some((heading, places)) = list {
            self.open_list(Mode::Places, heading, super::navigate::place_rows(places));
        }
    }

    /// Read the calls the other way: who calls the function, or what it
    /// calls. A tree there starts again from its function, opened; and the
    /// next hierarchy reads the same way.
    pub(super) fn calls_direction(self: &Rc<Self>, incoming: bool) {
        let restarted = {
            let mut state = self.state.borrow_mut();
            let calls = &mut state.calls;
            calls.incoming = incoming;
            let root = calls
                .tree()
                .filter(|tree| tree.incoming != incoming)
                .map(|tree| tree.root().clone());
            match root {
                Some(root) => {
                    let serial = calls.next_serial();
                    calls.view = View::Tree(CallTree::new(serial, root, incoming));
                    true
                }
                None => false,
            }
        };
        if restarted {
            self.calls_open(0);
        } else {
            self.sync_calls();
        }
    }

    pub(super) fn sync_calls(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let calls = &state.calls;
        let server = server_of(&calls.path);
        let mut rows = Vec::new();
        let mut empty = String::new();
        let said = match &calls.view {
            View::Idle => t!("calls.idle"),
            View::Asking(_) => t!("calls.asking", server = server),
            View::Said(text) => text.clone(),
            View::Tree(tree) => {
                rows = tree
                    .rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let at = row.call_site();
                        let file = at.path.rsplit(['/', '\\']).next().unwrap_or(&at.path);
                        CallLine {
                            id: row.id as i32,
                            depth: row.depth as i32,
                            open: match row.open {
                                Openness::Closed => 0,
                                Openness::Asking => 1,
                                // Opened, and nothing came back: no chevron,
                                // as VS Code draws a leaf.
                                Openness::Open
                                    if tree
                                        .rows
                                        .get(index + 1)
                                        .is_none_or(|next| next.depth <= row.depth) =>
                                {
                                    3
                                }
                                Openness::Open => 2,
                            },
                            name: row.call.item.name.as_str().into(),
                            detail: row.call.item.detail.clone().unwrap_or_default().into(),
                            place: format!("{file}:{}", at.line + 1).into(),
                            whole: format!("{}:{}", at.path, at.line + 1).into(),
                            recursive: tree.recursive(index),
                            count: row.call.sites.len() as i32,
                            count_tip: t!("calls.sites", count = row.call.sites.len().to_string())
                                .into(),
                            first: index == 0,
                        }
                    })
                    .collect();
                if tree.is_empty() {
                    let name = tree.root().name.clone();
                    empty = if tree.incoming {
                        t!("calls.none-in", name = name)
                    } else {
                        t!("calls.none-out", name = name)
                    };
                }
                String::new()
            }
        };
        shell.set_calls_incoming(calls.incoming);
        shell.set_calls_said(SharedString::from(said));
        shell.set_calls_empty(SharedString::from(empty));
        shell.set_calls_rows(ModelRc::new(VecModel::from(rows)));
    }
}

/// The server that answers for `path`, by the name the status bar gives it.
fn server_of(path: &str) -> &'static str {
    crate::language::of(path)
        .and_then(|language| language.server)
        .map_or("", rusty_lsp::ServerKind::label)
}
