//! A tab's right-click menu, VS Code's: close it, the others, those to its
//! right, the saved ones or all of them; copy its path, whole or from the
//! project; show it in the file manager; open it in the group beside.
//! Every row is an action (`actions.rs`), named and keyed as the palette
//! names and keys it, and the palette's way in acts on the tab in front.
//! What a tab and a row of the tree both do — copy a path, reveal it, open
//! it beside — is done the tree's way (`app/tree.rs`, `crate::reveal`), so
//! the two menus cannot disagree about it.
//!
//! A tab is named by its group as well as its id, because one file can be
//! a tab in both groups and a verb on one must not land on the other.

use std::rc::Rc;

use rusty_i18n::t;

use super::{App, Tab};

/// The menu's rows, in VS Code's order and groups; an empty id is a
/// divider. Another row is one more id here and one more arm in
/// [`App::tab_verb`].
const ROWS: &[&str] = &[
    "tab.close",
    "tab.close-others",
    "tab.close-right",
    "tab.close-saved",
    "tab.close-all",
    "",
    "tab.copy-path",
    "tab.copy-relative-path",
    "",
    "tab.reveal",
    "",
    "tab.split-right",
];

/// The verbs about the file on disk behind a tab, which a page and a
/// macro's expansion have none of.
const ON_DISK: &[&str] = &["tab.copy-path", "tab.copy-relative-path", "tab.reveal"];

/// Which rows a tab's menu shows: what would do nothing is left out rather
/// than offered — nothing to close beside or right of it, nothing on disk,
/// no group to its right — and the dividers that leaves at an edge or
/// beside another go too.
pub(super) fn rows_for(
    tab: &Tab,
    at: usize,
    count: usize,
    has_right_group_to_open: bool,
) -> Vec<&'static str> {
    let on_disk = matches!(tab, Tab::File(path) if !rusty_edit::is_expansion(path))
        || matches!(tab, Tab::Picture(_));
    let file = matches!(tab, Tab::File(_) | Tab::Picture(_));
    let kept: Vec<&'static str> = ROWS
        .iter()
        .copied()
        .filter(|id| match *id {
            "tab.close-others" => count > 1,
            "tab.close-right" => at + 1 < count,
            "tab.split-right" => file && has_right_group_to_open,
            id if ON_DISK.contains(&id) => on_disk,
            _ => true,
        })
        .collect();
    let mut rows: Vec<&'static str> = Vec::new();
    for id in kept {
        if id.is_empty() && rows.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        rows.push(id);
    }
    while rows.last() == Some(&"") {
        rows.pop();
    }
    rows
}

/// What each close verb closes of a group, by position: every tab but the
/// one at `at`, those after it, those with nothing unsaved, or all.
pub(super) fn doomed(verb: &str, at: usize, dirty: &[bool]) -> Vec<usize> {
    (0..dirty.len())
        .filter(|&index| match verb {
            "tab.close-others" => index != at,
            "tab.close-right" => index > at,
            "tab.close-saved" => !dirty[index],
            "tab.close-all" => true,
            _ => false,
        })
        .collect()
}

impl App {
    /// A tab of one group by its id: the keys group's, or with `other` the
    /// other group's.
    pub(super) fn tab_in(&self, other: bool, id: &str) -> Option<Tab> {
        let state = self.state.borrow();
        let tabs = if other { &state.other } else { &state.tabs };
        tabs.iter().find(|tab| tab.id() == id).cloned()
    }

    /// Whether a tab of this group has a group to its right to open in:
    /// the window is not split, or the group is the one on the left.
    fn opens_beside(&self, other: bool) -> bool {
        let state = self.state.borrow();
        let side = if other { 1 - state.focus } else { state.focus };
        !state.split || side == 0
    }

    /// A right-click on a tab: its menu, for the group it is in.
    pub(super) fn tab_menu(self: &Rc<Self>, id: &str, other: bool, x: f32, y: f32) {
        let (tab, at, count) = {
            let state = self.state.borrow();
            let tabs = if other { &state.other } else { &state.tabs };
            let Some(at) = tabs.iter().position(|tab| tab.id() == id) else {
                return;
            };
            (tabs[at].clone(), at, tabs.len())
        };
        let rows = rows_for(&tab, at, count, self.opens_beside(other));
        self.state.borrow_mut().tab_menu = Some((other, id.to_string()));
        self.open_action_menu("tab", &rows, x, y);
    }

    /// The tab a menu is open for, for the probe: whether it is the other
    /// group's, and its id.
    pub fn tab_menu_for(&self) -> Option<(bool, String)> {
        self.state.borrow().tab_menu.clone()
    }

    pub(super) fn tab_menu_picked(self: &Rc<Self>, verb: &str) {
        let Some((other, id)) = self.state.borrow_mut().tab_menu.take() else {
            return;
        };
        self.tab_verb(verb, other, &id);
    }

    /// A tab verb from the palette or a key: for the tab in front of the
    /// group with the keys.
    pub(super) fn tab_act(self: &Rc<Self>, verb: &str) {
        if let Some(tab) = self.active_tab() {
            self.tab_verb(verb, false, &tab.id());
        }
    }

    fn tab_verb(self: &Rc<Self>, verb: &str, other: bool, id: &str) {
        let Some(tab) = self.tab_in(other, id) else {
            return;
        };
        match verb {
            "tab.close" => {
                self.close_tab_in(other, id);
            }
            "tab.close-others" | "tab.close-right" | "tab.close-saved" | "tab.close-all" => {
                self.close_around(verb, other, id);
            }
            "tab.copy-path" | "tab.copy-relative-path" | "tab.reveal" => {
                let (Tab::File(path) | Tab::Picture(path)) = &tab else {
                    return self.not_on_disk(&tab);
                };
                if rusty_edit::is_expansion(path) {
                    return self.not_on_disk(&tab);
                }
                match verb {
                    "tab.copy-path" => self.tree_copy_path(path, false),
                    "tab.copy-relative-path" => self.tree_copy_path(path, true),
                    _ => self.tree_reveal(path.clone()),
                }
            }
            "tab.split-right" => {
                if let Tab::File(path) | Tab::Picture(path) = tab {
                    self.split_right(other, path);
                }
            }
            _ => {}
        }
    }

    fn not_on_disk(self: &Rc<Self>, tab: &Tab) {
        self.note_transient(1, t!("strip.not-a-file", name = tab.title()));
    }

    /// Close the tabs a verb names around the one clicked, in its group,
    /// left to right — each through the one close path, so a file with
    /// changes asks, and keeping one ends the rest. The group comes to
    /// front first, as a click on one of its tabs brings it, and for Close
    /// others and Close to the right the clicked tab with it.
    fn close_around(self: &Rc<Self>, verb: &str, other: bool, id: &str) {
        if other {
            self.swap_groups();
        }
        if matches!(verb, "tab.close-others" | "tab.close-right") {
            self.pick_tab(id);
        }
        let ids: Vec<String> = {
            let state = self.state.borrow();
            let Some(at) = state.tabs.iter().position(|tab| tab.id() == id) else {
                return;
            };
            let dirty: Vec<bool> = state
                .tabs
                .iter()
                .map(|tab| matches!(tab, Tab::File(path) if self.editor.is_dirty(path)))
                .collect();
            doomed(verb, at, &dirty)
                .into_iter()
                .map(|index| state.tabs[index].id())
                .collect()
        };
        for doomed in ids {
            if !self.close_tab_in(false, &doomed) {
                break;
            }
        }
    }

    /// Split right: `path` in the group to the right of the one its tab is
    /// in, which gets the keys — a new group when the window is not split,
    /// as the tree's Open to the side makes one. Split already, with the
    /// keys on the right, the tab clicked is on the left and the group
    /// with the keys is the one beside it.
    fn split_right(self: &Rc<Self>, other: bool, path: String) {
        if !self.opens_beside(other) {
            return;
        }
        let keys_on_right = {
            let state = self.state.borrow();
            state.split && state.focus == 1
        };
        if keys_on_right {
            self.open_tab(Tab::of(path));
        } else {
            self.open_to_side(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Tab {
        Tab::File(path.into())
    }

    #[test]
    fn a_file_in_the_middle_is_offered_every_row() {
        assert_eq!(rows_for(&file("src/main.rs"), 1, 3, true), ROWS);
    }

    /// Nothing that would do nothing: a lone tab has no others and nothing
    /// to its right, and a group on the right has no group beyond it.
    #[test]
    fn rows_that_would_do_nothing_are_left_out() {
        assert_eq!(
            rows_for(&file("src/main.rs"), 0, 1, false),
            [
                "tab.close",
                "tab.close-saved",
                "tab.close-all",
                "",
                "tab.copy-path",
                "tab.copy-relative-path",
                "",
                "tab.reveal",
            ]
        );
        let last = rows_for(&file("src/main.rs"), 2, 3, true);
        assert!(!last.contains(&"tab.close-right"));
        assert!(last.contains(&"tab.close-others"));
    }

    /// A page and a macro's expansion are no file on disk: nothing to copy
    /// or reveal, and no stray divider where those rows were.
    #[test]
    fn what_is_not_on_disk_has_no_path_rows() {
        assert_eq!(
            rows_for(&Tab::Page("settings"), 0, 2, true),
            [
                "tab.close",
                "tab.close-others",
                "tab.close-right",
                "tab.close-saved",
                "tab.close-all",
            ]
        );
        let expansion = file(&rusty_edit::expansion_path("src/main.rs", 7, "println!"));
        let rows = rows_for(&expansion, 0, 1, true);
        assert!(!rows.contains(&"tab.copy-path") && !rows.contains(&"tab.reveal"));
        assert_eq!(rows.last(), Some(&"tab.split-right"));
        assert!(!rows.windows(2).any(|pair| pair == ["", ""]));
    }

    /// A picture is a file on disk like any other: its path is copied,
    /// revealed and opened beside.
    #[test]
    fn a_picture_tab_has_the_file_rows() {
        let picture = Tab::Picture("book/src/figures/fig.png".into());
        assert_eq!(rows_for(&picture, 1, 3, true), ROWS);
    }

    #[test]
    fn each_close_verb_closes_what_it_names() {
        let dirty = [false, true, false, false];
        assert_eq!(doomed("tab.close-others", 1, &dirty), [0, 2, 3]);
        assert_eq!(doomed("tab.close-right", 1, &dirty), [2, 3]);
        assert_eq!(doomed("tab.close-saved", 1, &dirty), [0, 2, 3]);
        assert_eq!(doomed("tab.close-all", 1, &dirty), [0, 1, 2, 3]);
        assert!(doomed("tab.close-right", 3, &dirty).is_empty());
    }

    /// Every row is an action the palette knows, so its name and key come
    /// from the one table.
    #[test]
    fn every_row_is_an_action() {
        for id in ROWS.iter().filter(|id| !id.is_empty()) {
            assert!(crate::actions::by_id(id).is_some(), "{id} is no action");
        }
    }
}
