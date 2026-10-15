//! A right-click menu: one for the window, filled by whichever surface was
//! clicked, which also hears what was picked. There is no menu but the ones
//! made here.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use super::App;
use crate::{MenuItem, Shell};

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
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let rows: Vec<MenuItem> = items
            .into_iter()
            .map(|(id, label, danger)| MenuItem {
                id: id.into(),
                label: label.into(),
                danger,
            })
            .collect();
        self.state.borrow_mut().menu = Some(owner);
        shell.set_menu_items(ModelRc::new(VecModel::from(rows)));
        shell.set_menu_x(x);
        shell.set_menu_y(y);
        shell.set_menu_open(true);
    }

    pub(super) fn menu_picked(self: &Rc<Self>, id: &str) {
        let owner = self.menu_close();
        if id.is_empty() {
            return;
        }
        if owner == Some("git") {
            self.git_menu_picked(id);
        }
    }

    pub(super) fn menu_dismissed(self: &Rc<Self>) {
        self.menu_close();
    }

    fn menu_close(&self) -> Option<&'static str> {
        if let Some(window) = self.window() {
            window.global::<Shell>().set_menu_open(false);
        }
        self.state.borrow_mut().menu.take()
    }
}
