//! A right-click menu: one for the window, filled by whichever surface was
//! clicked, which also hears what was picked. There is no menu but the ones
//! made here.

use std::rc::Rc;

use rusty_lsp::ServerKind;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::App;
use crate::{MenuItem, Shell, actions, language};

impl App {
    /// Show `items` — an id, a label, whether it destroys something; an
    /// empty id is a divider — at a point of the window, for `owner`.
    pub(super) fn open_menu(
        self: &Rc<Self>,
        owner: &'static str,
        items: Vec<(String, String, bool)>,
        x: f32,
        y: f32,
    ) {
        let rows = items
            .into_iter()
            .map(|(id, label, danger)| MenuItem {
                id: id.into(),
                label: label.into(),
                danger,
                chord: Default::default(),
            })
            .collect();
        self.show_menu(owner, rows, x, y);
    }

    pub(super) fn show_menu(
        self: &Rc<Self>,
        owner: &'static str,
        rows: Vec<MenuItem>,
        x: f32,
        y: f32,
    ) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        self.state.borrow_mut().menu = Some(owner);
        shell.set_menu_items(ModelRc::new(VecModel::from(rows)));
        shell.set_menu_x(x);
        shell.set_menu_y(y);
        shell.set_menu_open(true);
    }

    /// The editor's right-click menu, VS Code's: where the name at the
    /// caret leads, then what changes it, then the palette. Each row is an
    /// action, named and keyed as the palette names and keys it. What only
    /// a language server answers is offered where one reads the file, and
    /// a macro's expansion only where rust-analyzer does.
    pub fn editor_menu(self: &Rc<Self>, x: f32, y: f32) {
        let path = self.editor.current_path().unwrap_or_default();
        let served = self.servers.serves(&path);
        let rust = language::of(&path).and_then(|l| l.server) == Some(ServerKind::RustAnalyzer);
        let mut ids: Vec<&'static str> = Vec::new();
        if served {
            ids.extend([
                "editor.definition",
                "editor.type-definition",
                "editor.references",
                "editor.implementations",
                "editor.call-hierarchy",
            ]);
            if rust {
                ids.push("editor.expand-macro");
            }
            ids.extend(["", "editor.rename", "editor.quick-fix", ""]);
        }
        ids.push("palette.open");
        self.open_action_menu("editor", &ids, x, y);
    }

    /// A menu of actions by their ids, each row named and keyed as the
    /// palette names and keys it — the key in force, so a rebound key is
    /// the key shown; an empty id is a divider.
    pub(super) fn open_action_menu(
        self: &Rc<Self>,
        owner: &'static str,
        ids: &[&str],
        x: f32,
        y: f32,
    ) {
        let keys = self.state.borrow().keys.map.clone();
        let rows = ids
            .iter()
            .map(|&id| {
                let action = actions::by_id(id);
                MenuItem {
                    id: id.into(),
                    label: action
                        .map(|a| rusty_i18n::lookup(a.title))
                        .unwrap_or_default()
                        .into(),
                    danger: false,
                    // The key in force: a rebound key is the key shown.
                    chord: crate::platform::pretty(
                        keys.chord(id).unwrap_or_default(),
                        crate::platform::is_macos(),
                    )
                    .into(),
                }
            })
            .collect();
        self.show_menu(owner, rows, x, y);
    }

    /// The status bar's language-server item: VS Code's rust-analyzer
    /// status menu, cut down to the one row that undoes a stale analysis —
    /// Restart server, the palette's action — so the restart has a door
    /// where the server's state is read, and a click there never restarts
    /// anything by itself.
    pub(super) fn server_menu(self: &Rc<Self>, x: f32, y: f32) {
        self.open_action_menu("server", &["lsp.restart"], x, y);
    }

    pub(super) fn menu_picked(self: &Rc<Self>, id: &str) {
        let owner = self.menu_close();
        if id.is_empty() {
            return;
        }
        match owner {
            Some("server") => self.act(id),
            Some("git") => self.git_menu_picked(id),
            Some("surface") => self.surface_menu_picked(id),
            Some("tree") => self.tree_menu_picked(id),
            // The keys back in the code, then what was picked.
            Some("editor") => {
                self.front_focus();
                self.act(id);
            }
            Some("tab") => {
                self.front_focus();
                self.tab_menu_picked(id);
            }
            _ => {}
        }
    }

    pub(super) fn menu_dismissed(self: &Rc<Self>) {
        let owner = self.menu_close();
        match owner {
            Some("editor") => self.front_focus(),
            Some("surface") => self.surface_menu_dismissed(),
            // The keys back in the tree, its menu gone.
            Some("tree") => self.tree_menu_dismissed(),
            Some("tab") => {
                self.state.borrow_mut().tab_menu = None;
                self.front_focus();
            }
            // A diff's keys back to it.
            Some("git") => self.git_menu_dismissed(),
            _ => {}
        }
    }

    fn menu_close(&self) -> Option<&'static str> {
        if let Some(window) = self.window() {
            window.global::<Shell>().set_menu_open(false);
        }
        self.state.borrow_mut().menu.take()
    }
}
