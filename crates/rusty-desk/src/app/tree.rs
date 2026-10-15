//! The project tree's own verbs, VS Code's explorer: a new file or folder
//! named in a box where it will be, rename in place, cut, copy and paste,
//! delete to the recycle bin, drag and drop, reveal in the file manager,
//! the paths copied, a file opened to the side — from the right-click menu,
//! the keys while the tree has them, and the dock's buttons.
//!
//! The writes are `rusty_edit::entries`', confined to the project and
//! refusing what they refuse; the arithmetic — where the box goes, what an
//! arrow does, what a name is refused for — is `crate::tree`, pure. What
//! is here is the order: a write runs off this thread, the tree is read
//! again in the same job, and a path that moved takes its state with it
//! before the watcher has heard (`follow_move`).

use std::rc::Rc;

use rusty_i18n::t;
use slint::{ComponentHandle, Model, VecModel};

use super::{App, Tab, git};
use crate::{
    MenuItem, Shell, TreeRow, keys,
    tree::{self, Arrow, Naming, Refusal, Shown, Step},
    work,
};

/// What Cut or Copy took, for Paste.
#[derive(Clone, Debug)]
struct Clip {
    path: String,
    cut: bool,
}

/// An entry being carried from row `from`, and the folder it would drop
/// into (`""` the root) — `None` where a drop moves nothing.
#[derive(Clone, Debug)]
struct Carried {
    from: i32,
    path: String,
    into: Option<String>,
}

#[derive(Default)]
pub(super) struct Tree {
    /// The entry the tree has selected.
    selected: Option<String>,
    clip: Option<Clip>,
    /// The box a name is being typed in.
    naming: Option<Naming>,
    drag: Option<Carried>,
    /// A drag given up with Escape, by the row it started on, so the
    /// moves still on their way do not take it up again.
    dropped_drag: Option<i32>,
    /// The file in front the last time the tree followed it.
    followed: Option<String>,
    /// What a right-click was on — `""` is the space below the rows, the
    /// root — and whether it is a folder.
    menu: Option<(String, bool)>,
    /// Counters the view reads: the keys to the tree, and the selected
    /// row into view.
    focus: i32,
    reveal: i32,
    reveal_due: bool,
}

impl App {
    /// The rows as drawn: the project's open rows with the box among them.
    fn tree_shown(&self) -> Vec<Shown> {
        let state = self.state.borrow();
        state.project.as_ref().map_or_else(Vec::new, |project| {
            tree::with_box(project.rows(), state.tree.naming.as_ref())
        })
    }

    /// The entry drawn on row `index`, when it is an entry.
    fn tree_entry(&self, index: i32) -> Option<crate::project::ShownRow> {
        let index = usize::try_from(index).ok()?;
        match self.tree_shown().into_iter().nth(index)? {
            Shown::Entry(row) => Some(row),
            Shown::Name { .. } => None,
        }
    }

    /// The row `path` is drawn on, for the probe to point at.
    pub fn tree_row_of(&self, path: &str) -> Option<i32> {
        self.tree_shown()
            .iter()
            .position(|shown| matches!(shown, Shown::Entry(row) if row.path == path))
            .map(|at| at as i32)
    }

    /// The entry the tree has selected, for the probe.
    pub fn tree_selected(&self) -> Option<String> {
        self.state.borrow().tree.selected.clone()
    }

    /// The keys to the tree.
    pub(super) fn focus_tree(&self) {
        let focus = {
            let mut state = self.state.borrow_mut();
            state.tree.focus += 1;
            state.tree.focus
        };
        if let Some(window) = self.window() {
            window.global::<Shell>().set_tree_focus(focus);
        }
    }

    /// The selection, and the folders above it open, brought into view.
    fn tree_select(&self, path: Option<String>) {
        let mut state = self.state.borrow_mut();
        if let (Some(path), Some(project)) = (&path, state.project.as_mut()) {
            open_above(&mut project.expanded, path);
        }
        state.tree.selected = path;
        state.tree.reveal_due = true;
    }

    // ---- the pointer -------------------------------------------------------

    /// A click on a row — a folder opens or closes, a file opens and the
    /// keys stay in the tree, as VS Code's single click does — or below the
    /// rows (`-1`), which selects nothing, so that a paste goes to the root.
    pub(super) fn tree_click(self: &Rc<Self>, index: i32) {
        self.tree_name_left();
        if index < 0 {
            self.state.borrow_mut().tree.selected = None;
            self.focus_tree();
            self.sync_tree();
            return;
        }
        let Some(row) = self.tree_entry(index) else {
            return;
        };
        self.state.borrow_mut().tree.selected = Some(row.path.clone());
        self.focus_tree();
        if row.dir {
            if let Some(project) = self.state.borrow_mut().project.as_mut()
                && !project.expanded.remove(&row.path)
            {
                project.expanded.insert(row.path.clone());
            }
            self.sync_tree();
        } else {
            self.sync_tree();
            self.open_file_then(row.path, |app| app.focus_tree());
        }
    }

    /// A double-click on a file: open, and the keys to the code.
    pub(super) fn tree_double_click(self: &Rc<Self>, index: i32) {
        if let Some(row) = self.tree_entry(index)
            && !row.dir
        {
            self.open_file(row.path);
        }
    }

    // ---- the keys ------------------------------------------------------------

    /// A key while the tree has the keys; whether the tree took it. Keys it
    /// does not take go on to the window's bindings, so Ctrl+P is Ctrl+P.
    pub(super) fn tree_key(
        self: &Rc<Self>,
        text: &str,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> bool {
        if self.state.borrow().tree.naming.is_some() {
            return false;
        }
        let Some(chord) = keys::chord(text, ctrl, alt, shift, false) else {
            return false;
        };
        let selected = self.state.borrow().tree.selected.clone();
        let entry = selected
            .as_deref()
            .and_then(|path| self.tree_entry_at(path));
        let arrow = match chord.as_str() {
            "ArrowUp" => Some(Arrow::Up),
            "ArrowDown" => Some(Arrow::Down),
            "ArrowLeft" => Some(Arrow::Left),
            "ArrowRight" => Some(Arrow::Right),
            "Home" => Some(Arrow::Home),
            "End" => Some(Arrow::End),
            _ => None,
        };
        if let Some(arrow) = arrow {
            let rows = self
                .state
                .borrow()
                .project
                .as_ref()
                .map(|p| p.rows())
                .unwrap_or_default();
            match tree::arrow(&rows, selected.as_deref(), arrow) {
                Step::Select(path) => self.tree_select(Some(path)),
                Step::Expand(path) => self.tree_expand(&path, true),
                Step::Collapse(path) => self.tree_expand(&path, false),
                Step::Stay => {}
            }
            self.sync_tree();
            return true;
        }
        match chord.as_str() {
            // Enter opens a file and gives it the keys; Space opens it and
            // keeps them here. Either opens or closes a folder.
            "Enter" | "Space" => match entry {
                Some((path, true)) => {
                    let open = self
                        .state
                        .borrow()
                        .project
                        .as_ref()
                        .is_some_and(|p| p.expanded.contains(&path));
                    self.tree_expand(&path, !open);
                    self.sync_tree();
                }
                Some((path, false)) if chord == "Enter" => self.open_file(path),
                Some((path, false)) => self.open_file_then(path, |app| app.focus_tree()),
                None => return false,
            },
            "F2" => match entry {
                Some((path, dir)) => self.tree_rename(path, dir),
                None => return false,
            },
            "Delete" | "Ctrl+Backspace" => match entry {
                Some((path, dir)) => self.tree_delete(path, dir),
                None => return false,
            },
            "Ctrl+X" | "Ctrl+C" => match entry {
                Some((path, _)) => self.tree_clip(path, chord == "Ctrl+X"),
                None => return false,
            },
            "Ctrl+V" => {
                let into = match entry {
                    Some((path, true)) => path,
                    Some((path, false)) => rusty_edit::parent_dir(&path).to_string(),
                    None => String::new(),
                };
                self.tree_paste(into);
            }
            // Escape gives up a drag, then a cut.
            "Escape" => {
                let mut state = self.state.borrow_mut();
                if let Some(carried) = state.tree.drag.take() {
                    state.tree.dropped_drag = Some(carried.from);
                } else if state.tree.clip.as_ref().is_some_and(|c| c.cut) {
                    state.tree.clip = None;
                } else {
                    return false;
                }
                drop(state);
                self.sync_tree();
            }
            _ => return false,
        }
        true
    }

    /// The selected path's entry, when it is on screen: its path and
    /// whether it is a folder.
    fn tree_entry_at(&self, path: &str) -> Option<(String, bool)> {
        let state = self.state.borrow();
        state
            .project
            .as_ref()?
            .rows()
            .into_iter()
            .find(|row| row.path == path)
            .map(|row| (row.path, row.dir))
    }

    fn tree_expand(&self, path: &str, open: bool) {
        if let Some(project) = self.state.borrow_mut().project.as_mut() {
            if open {
                project.expanded.insert(path.to_string());
            } else {
                project.expanded.remove(path);
            }
        }
    }

    // ---- the menu ------------------------------------------------------------

    /// A right-click on a row, or below the rows (`-1`): VS Code's
    /// explorer menu for a folder, a file or the root.
    pub(super) fn tree_menu(self: &Rc<Self>, index: i32, x: f32, y: f32) {
        self.tree_name_left();
        if self.state.borrow().project.is_none() {
            return;
        }
        let target = if index < 0 {
            (String::new(), true)
        } else {
            match self.tree_entry(index) {
                Some(row) => (row.path, row.dir),
                None => return,
            }
        };
        {
            let mut state = self.state.borrow_mut();
            state.tree.selected = (!target.0.is_empty()).then(|| target.0.clone());
            state.tree.menu = Some(target.clone());
        }
        self.sync_tree();
        let pasting = self.state.borrow().tree.clip.is_some();
        let (path, dir) = target;
        let mut rows: Vec<(&str, String, &str, bool)> = Vec::new();
        let divider = || ("", String::new(), "", false);
        if dir {
            rows.push(("new-file", t!("context.tree-new-file"), "", false));
            rows.push(("new-folder", t!("context.tree-new-folder"), "", false));
        } else {
            rows.push(("open-side", t!("context.tree-open-side"), "", false));
        }
        if !path.is_empty() {
            rows.push(divider());
            rows.push((
                "reveal",
                rusty_i18n::lookup(crate::reveal::label_key()),
                "",
                false,
            ));
            rows.push(divider());
            rows.push(("cut", t!("context.tree-cut"), "Ctrl+X", false));
            rows.push(("copy", t!("context.tree-copy"), "Ctrl+C", false));
        }
        if dir && pasting {
            if path.is_empty() {
                rows.push(divider());
            }
            rows.push(("paste", t!("context.tree-paste"), "Ctrl+V", false));
        }
        if !path.is_empty() {
            rows.push(divider());
            rows.push(("copy-path", t!("context.tree-copy-path"), "", false));
            rows.push((
                "copy-relative-path",
                t!("context.tree-copy-relative-path"),
                "",
                false,
            ));
            rows.push(divider());
            rows.push(("rename", t!("context.tree-rename"), "F2", false));
            rows.push(("delete", t!("context.tree-delete"), DELETE_KEY, true));
        }
        let rows = rows
            .into_iter()
            .map(|(id, label, chord, danger)| MenuItem {
                id: id.into(),
                label: label.into(),
                danger,
                chord: chord.into(),
            })
            .collect();
        self.show_menu("tree", rows, x, y);
    }

    /// A row of the tree's menu picked.
    pub(super) fn tree_menu_picked(self: &Rc<Self>, id: &str) {
        let Some((path, dir)) = self.state.borrow_mut().tree.menu.take() else {
            return;
        };
        let folder = if dir {
            path.clone()
        } else {
            rusty_edit::parent_dir(&path).to_string()
        };
        match id {
            "new-file" => return self.tree_new(folder, false),
            "new-folder" => return self.tree_new(folder, true),
            "rename" => return self.tree_rename(path, dir),
            "open-side" => return self.open_to_side(path),
            "reveal" => self.tree_reveal(path),
            "cut" | "copy" => self.tree_clip(path, id == "cut"),
            "paste" => self.tree_paste(folder),
            "copy-path" | "copy-relative-path" => {
                self.tree_copy_path(&path, id == "copy-relative-path")
            }
            "delete" => self.tree_delete(path, dir),
            _ => {}
        }
        self.focus_tree();
    }

    /// The menu put away with nothing picked: the keys back in the tree.
    pub(super) fn tree_menu_dismissed(self: &Rc<Self>) {
        self.state.borrow_mut().tree.menu = None;
        self.focus_tree();
    }

    /// The dock's buttons and the palette: a new file or folder where the
    /// selection is — in it, beside it, or at the root — and the tree read
    /// again.
    pub(super) fn tree_act(self: &Rc<Self>, verb: &str) {
        if self.state.borrow().project.is_none() {
            return;
        }
        let folder = {
            let selected = self.state.borrow().tree.selected.clone();
            match selected.as_deref().and_then(|path| self.tree_kind(path)) {
                Some((path, true)) => path,
                Some((path, false)) => rusty_edit::parent_dir(&path).to_string(),
                None => String::new(),
            }
        };
        match verb {
            // From the palette the tree may be hidden: it is shown first,
            // since the name is typed there.
            "new-file" | "new-folder" => {
                self.show_panel("left", "project");
                self.tree_new(folder, verb == "new-folder");
            }
            "refresh" => self.reread_project(),
            _ => {}
        }
    }

    /// Whether `path` is in the project's tree, and a folder.
    fn tree_kind(&self, path: &str) -> Option<(String, bool)> {
        fn find(entries: &[rusty_edit::Entry], path: &str) -> Option<bool> {
            for entry in entries {
                if entry.path == path {
                    return Some(entry.is_dir);
                }
                if entry.is_dir && rusty_edit::is_within(path, &entry.path) {
                    return find(&entry.children, path);
                }
            }
            None
        }
        let state = self.state.borrow();
        let dir = find(&state.project.as_ref()?.entries, path)?;
        Some((path.to_string(), dir))
    }

    // ---- the box -------------------------------------------------------------

    /// A new file or folder in `dir`, named in a box among its future
    /// siblings, the folder opened first.
    fn tree_new(self: &Rc<Self>, dir: String, folder: bool) {
        self.tree_name_left();
        if !dir.is_empty() {
            let mut state = self.state.borrow_mut();
            if let Some(project) = state.project.as_mut() {
                open_above(&mut project.expanded, &format!("{dir}/x"));
            }
        }
        self.tree_naming(Naming::New { dir, folder }, String::new());
    }

    /// The entry renamed in its own row, its name before the extension
    /// selected.
    fn tree_rename(self: &Rc<Self>, path: String, dir: bool) {
        self.tree_name_left();
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.tree_naming(Naming::Rename { path, dir }, name);
    }

    fn tree_naming(self: &Rc<Self>, naming: Naming, name: String) {
        let select = match &naming {
            Naming::Rename { dir, .. } => tree::stem_bytes(&name, *dir),
            Naming::New { .. } => 0,
        };
        {
            let mut state = self.state.borrow_mut();
            state.tree.naming = Some(naming);
            state.tree.reveal_due = true;
        }
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_tree_name(name.into());
            shell.set_tree_name_select(select as i32);
            shell.set_tree_name_problem(Default::default());
        }
        self.sync_tree();
    }

    /// The name changed, committed by whatever input method made it: what
    /// is wrong with it, said under the box as it is typed.
    pub(super) fn tree_name_edited(self: &Rc<Self>, text: &str) {
        let problem = self.tree_refusal(text).map(|r| refusal_text(&r));
        if let Some(window) = self.window() {
            window
                .global::<Shell>()
                .set_tree_name_problem(problem.unwrap_or_default().into());
        }
    }

    fn tree_refusal(&self, text: &str) -> Option<Refusal> {
        let state = self.state.borrow();
        let naming = state.tree.naming.as_ref()?;
        let entries = &state.project.as_ref()?.entries;
        tree::refusal(naming, text, entries, FOLD)
    }

    /// Enter makes what the box names, unless the name is refused, which
    /// the box goes on saying; Escape puts the box away.
    pub(super) fn tree_name_done(self: &Rc<Self>, ok: bool) {
        if ok {
            let text = self.tree_name_text();
            match self.tree_refusal(&text) {
                Some(Refusal::Empty) => {}
                Some(refused) => {
                    if let Some(window) = self.window() {
                        window
                            .global::<Shell>()
                            .set_tree_name_problem(refusal_text(&refused).into());
                    }
                    return;
                }
                None => return self.tree_name_commit(text),
            }
        }
        self.tree_name_close();
        self.focus_tree();
    }

    /// The box lost the keys — a click elsewhere: what it names is made
    /// when the name can be, and the box goes otherwise, as in VS Code.
    pub(super) fn tree_name_left(self: &Rc<Self>) {
        if self.state.borrow().tree.naming.is_none() {
            return;
        }
        let text = self.tree_name_text();
        match self.tree_refusal(&text) {
            None => self.tree_name_commit(text),
            Some(_) => self.tree_name_close(),
        }
    }

    fn tree_name_text(&self) -> String {
        self.window()
            .map(|w| w.global::<Shell>().get_tree_name().to_string())
            .unwrap_or_default()
    }

    fn tree_name_close(&self) {
        self.state.borrow_mut().tree.naming = None;
        if let Some(window) = self.window() {
            window
                .global::<Shell>()
                .set_tree_name_problem(Default::default());
        }
        self.sync_tree();
    }

    fn tree_name_commit(self: &Rc<Self>, text: String) {
        let Some(naming) = self.state.borrow().tree.naming.clone() else {
            return;
        };
        self.tree_name_close();
        let Some(root) = self.root() else { return };
        let to = tree::target(&naming, &text);
        match naming {
            Naming::New { folder, .. } => {
                let made = to.clone();
                self.tree_write(
                    move || rusty_edit::create(&root, &made, folder).map(|()| made),
                    move |app, made| {
                        app.tree_select(Some(made.clone()));
                        app.sync_tree();
                        if folder {
                            app.focus_tree();
                        } else {
                            // A new file opens in the editor, as VS Code's does.
                            app.open_file(made);
                        }
                    },
                    move |reason| t!("tree.create-failed", name = to.clone(), reason = reason),
                );
            }
            Naming::Rename { path, .. } => {
                let name = text.trim().to_string();
                if to == path {
                    self.focus_tree();
                    return;
                }
                let from = path.clone();
                self.tree_write(
                    move || rusty_edit::rename_entry(&root, &from, &name),
                    move |app, renamed| {
                        app.follow_move(&path, &renamed);
                        app.tree_select(Some(renamed));
                        app.sync_tree();
                        app.focus_tree();
                    },
                    move |reason| t!("tree.rename-failed", name = to.clone(), reason = reason),
                );
            }
        }
    }

    // ---- writes --------------------------------------------------------------

    /// Run a write off this thread, read the tree again in the same job, and
    /// hand what the write answered to `then` once the tree is the new one;
    /// a refusal is said in the user's words.
    fn tree_write<T: Send + 'static>(
        self: &Rc<Self>,
        write: impl FnOnce() -> rusty_edit::Result<T> + Send + 'static,
        then: impl FnOnce(&Rc<App>, T) + Send + 'static,
        failed: impl FnOnce(String) -> String + Send + 'static,
    ) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || {
                let written = write();
                let entries = rusty_edit::read_tree(&root).ok();
                (root, written, entries)
            },
            move |app, (root, written, entries)| {
                if let Some(entries) = entries {
                    let mut state = app.state.borrow_mut();
                    if let Some(project) = state.project.as_mut()
                        && project.root == root
                    {
                        project.set_entries(entries);
                    }
                }
                match written {
                    Ok(answer) if app.root().as_deref() == Some(root.as_path()) => {
                        then(app, answer)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        app.sync_tree();
                        app.note(2, failed(error_text(&error)));
                        app.focus_tree();
                    }
                }
            },
        );
    }

    fn tree_clip(&self, path: String, cut: bool) {
        self.state.borrow_mut().tree.clip = Some(Clip { path, cut });
        self.sync_tree();
    }

    /// What Cut or Copy took, into `into`: a cut moves and is used up, a
    /// copy stays to be pasted again and takes a free name where its own
    /// is taken (` copy`, ` copy 2`).
    fn tree_paste(self: &Rc<Self>, into: String) {
        let Some(clip) = self.state.borrow().tree.clip.clone() else {
            return;
        };
        let Some(root) = self.root() else { return };
        let name = clip.path.clone();
        if clip.cut {
            let from = clip.path.clone();
            self.tree_write(
                move || rusty_edit::move_entry(&root, &from, &into),
                move |app, moved| {
                    app.state.borrow_mut().tree.clip = None;
                    app.follow_move(&clip.path, &moved);
                    app.tree_select(Some(moved));
                    app.sync_tree();
                    app.focus_tree();
                },
                move |reason| t!("tree.move-failed", name = name, reason = reason),
            );
        } else {
            let from = clip.path.clone();
            self.tree_write(
                move || rusty_edit::copy_entry(&root, &from, &into),
                |app, copied| {
                    app.tree_select(Some(copied));
                    app.sync_tree();
                    app.focus_tree();
                },
                move |reason| t!("tree.copy-failed", name = name, reason = reason),
            );
        }
    }

    /// To the recycle bin after asking — the tabs under it closed without
    /// a second question, since what they would save has nowhere to go.
    fn tree_delete(self: &Rc<Self>, path: String, dir: bool) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let asked = match (dir, cfg!(windows)) {
            (true, true) => t!("tree.delete-folder-bin", name = name.clone()),
            (true, false) => t!("tree.delete-folder-trash", name = name.clone()),
            (false, true) => t!("tree.delete-bin", name = name.clone()),
            (false, false) => t!("tree.delete-trash", name = name.clone()),
        };
        if !git::confirm(&asked) {
            return;
        }
        // The row after it takes the selection, as in VS Code.
        let at = self
            .state
            .borrow()
            .project
            .as_ref()
            .and_then(|p| p.rows().iter().position(|r| r.path == path));
        let Some(root) = self.root() else { return };
        let gone = path.clone();
        self.tree_write(
            move || rusty_edit::delete_entry(&root, &gone),
            move |app, ()| {
                app.follow_delete(&path);
                let next = at.and_then(|at| {
                    let state = app.state.borrow();
                    let rows = state.project.as_ref()?.rows();
                    rows.get(at.min(rows.len().saturating_sub(1)))
                        .map(|r| r.path.clone())
                });
                app.tree_select(next);
                app.sync_tree();
                app.focus_tree();
            },
            move |reason| t!("tree.delete-failed", name = name, reason = reason),
        );
    }

    /// Explorer, Finder or the desktop's file manager, with the entry
    /// selected (`crate::reveal`).
    pub(super) fn tree_reveal(self: &Rc<Self>, path: String) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || {
                let target = rusty_edit::absolute(&root, &path).map_err(|e| error_text(&e))?;
                crate::reveal::in_file_manager(&target).map_err(|e| e.to_string())
            },
            |app, opened| {
                if let Err(reason) = opened {
                    app.note(2, t!("tree.reveal-failed", reason = reason));
                }
            },
        );
    }

    /// The path onto the clipboard, written the way this system writes
    /// one — VS Code's: backslashes on Windows, the relative one too.
    pub(super) fn tree_copy_path(&self, path: &str, relative: bool) {
        let Some(root) = self.root() else { return };
        let joined = if relative {
            path.to_string()
        } else if path.is_empty() {
            rusty_embed::tools::plain(&root)
                .to_string_lossy()
                .into_owned()
        } else {
            rusty_embed::tools::plain(&root.join(path))
                .to_string_lossy()
                .into_owned()
        };
        let text = if cfg!(windows) {
            joined.replace('/', "\\")
        } else {
            joined
        };
        git::copy_text(&text);
    }

    // ---- drag and drop -------------------------------------------------------

    /// Row `from` carried over row `over` — past the last row is the space
    /// below them, the root: the folder it would land in lit, by the drop
    /// rule (`rusty_edit::drop_target`).
    pub(super) fn tree_drag(self: &Rc<Self>, from: i32, over: i32) {
        if self.state.borrow().tree.dropped_drag == Some(from) {
            return;
        }
        self.tree_name_left();
        let Some(carried) = self.tree_carried(from, over) else {
            return;
        };
        let changed = {
            let mut state = self.state.borrow_mut();
            let was = state.tree.drag.as_ref().map(|d| d.into.clone());
            let changed = was != Some(carried.into.clone());
            state.tree.drag = Some(carried);
            changed
        };
        if changed {
            self.sync_tree();
        }
    }

    /// Let go: moved into the folder lit, if one was.
    pub(super) fn tree_drop(self: &Rc<Self>, from: i32, over: i32) {
        let given_up = self.state.borrow_mut().tree.dropped_drag.take() == Some(from);
        let carried = self.tree_carried(from, over);
        self.state.borrow_mut().tree.drag = None;
        self.sync_tree();
        if given_up {
            return;
        }
        let Some(Carried {
            path,
            into: Some(into),
            ..
        }) = carried
        else {
            return;
        };
        let Some(root) = self.root() else { return };
        let (from_path, name) = (path.clone(), path.clone());
        self.tree_write(
            move || rusty_edit::move_entry(&root, &from_path, &into),
            move |app, moved| {
                app.follow_move(&path, &moved);
                app.tree_select(Some(moved));
                app.sync_tree();
                app.focus_tree();
            },
            move |reason| t!("tree.move-failed", name = name, reason = reason),
        );
    }

    /// What row `from` carried over row `over` is, and where it would land:
    /// nowhere above the first row, the root below the last.
    fn tree_carried(&self, from: i32, over: i32) -> Option<Carried> {
        let row = self.tree_entry(from)?;
        let shown = self.tree_shown();
        let into = match usize::try_from(over) {
            Err(_) => None,
            Ok(over) => match shown.get(over) {
                None => rusty_edit::drop_target(&row.path, row.dir, None),
                Some(Shown::Entry(target)) => rusty_edit::drop_target(
                    &row.path,
                    row.dir,
                    Some((target.path.as_str(), target.dir)),
                ),
                Some(Shown::Name { .. }) => None,
            },
        };
        Some(Carried {
            from,
            path: row.path,
            into,
        })
    }

    // ---- paths that moved take their state with them -------------------------

    /// `from` is `to` on disk now. Everything the window keeps by path goes
    /// with it before the watcher hears — the tabs in both groups, the
    /// documents (their text, cursors, folds and history), the servers
    /// (the old name closed, the new opened), the drafts kept, the open
    /// folders, the breakpoints, the selection and what is cut.
    pub(super) fn follow_move(self: &Rc<Self>, from: &str, to: &str) {
        let moved = self.editor.moved(from, to);
        let mv = |path: &mut String| {
            if let Some(new) = rusty_edit::moved_path(path, from, to) {
                *path = new;
            }
        };
        {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            for tab in state.tabs.iter_mut().chain(state.other.iter_mut()) {
                if let Tab::File(path) = tab {
                    mv(path);
                }
            }
            if let Some(project) = state.project.as_mut() {
                project.expanded = std::mem::take(&mut project.expanded)
                    .into_iter()
                    .map(|mut path| {
                        mv(&mut path);
                        path
                    })
                    .collect();
            }
            state.markdown_source = std::mem::take(&mut state.markdown_source)
                .into_iter()
                .map(|mut path| {
                    mv(&mut path);
                    path
                })
                .collect();
            for path in [&mut state.tree.selected, &mut state.tree.followed]
                .into_iter()
                .flatten()
            {
                mv(path);
            }
            if let Some(clip) = state.tree.clip.as_mut() {
                mv(&mut clip.path);
            }
        }
        {
            let gutter = self.editor.gutter();
            let mut gutter = gutter.borrow_mut();
            let keys: Vec<String> = gutter.breakpoints.keys().cloned().collect();
            for key in keys {
                if let Some(new) = rusty_edit::moved_path(&key, from, to)
                    && let Some(lines) = gutter.breakpoints.remove(&key)
                {
                    gutter.breakpoints.insert(new, lines);
                }
            }
        }
        if let Some(root) = self.root() {
            for (old, new) in &moved {
                self.backup_timers.borrow_mut().remove(old);
                self.keeper.remove(&root, old);
                if self.editor.is_dirty(new) {
                    self.back_up(new);
                }
            }
        }
        self.torn_moved(from, to);
        self.editor.gutter_changed();
        self.front_tab();
    }

    /// `path` went to the recycle bin: every tab under it closes without
    /// asking, its drafts with it, and nothing is kept for it.
    fn follow_delete(self: &Rc<Self>, path: &str) {
        let tabs: Vec<String> = {
            let state = self.state.borrow();
            state
                .tabs
                .iter()
                .chain(&state.other)
                .filter(|tab| matches!(tab, Tab::File(p) if rusty_edit::is_within(p, path)))
                .map(Tab::id)
                .collect()
        };
        let root = self.root();
        for id in tabs {
            if let Some(root) = &root
                && let Some(file) = id.strip_prefix("file:")
            {
                self.keeper.remove(root, file);
            }
            self.drop_tab_anywhere(&id);
        }
        self.torn_deleted(path);
        let mut state = self.state.borrow_mut();
        if let Some(project) = state.project.as_mut() {
            project
                .expanded
                .retain(|open| !rusty_edit::is_within(open, path));
        }
        if state
            .tree
            .clip
            .as_ref()
            .is_some_and(|clip| rusty_edit::is_within(&clip.path, path))
        {
            state.tree.clip = None;
        }
        state
            .markdown_source
            .retain(|open| !rusty_edit::is_within(open, path));
    }

    // ---- open to the side ----------------------------------------------------

    /// The file in the other editor group — a group made on the right when
    /// there is only one — with the keys, as VS Code's *Open to the Side*
    /// gives them; a macro's expansion opened beside (`open_beside`) leaves
    /// them in the code it was asked from.
    pub(super) fn open_to_side(self: &Rc<Self>, path: String) {
        self.hold_then(path, |app, path| {
            if app.state.borrow().split {
                app.swap_groups();
                app.open_tab(Tab::File(path));
                return;
            }
            if app.state.borrow().tabs.is_empty() {
                app.open_tab(Tab::File(path));
                return;
            }
            let scroll = app.editor.scroll();
            {
                let mut state = app.state.borrow_mut();
                state.other = std::mem::take(&mut state.tabs);
                state.other_active = state.active.take();
                state.tabs = vec![Tab::File(path)];
                state.active = Some(0);
                state.split = true;
                state.focus = 1;
            }
            app.editor.show_shade(app.shade_file().as_deref(), scroll);
            app.front_tab();
        });
    }

    // ---- drawing -------------------------------------------------------------

    /// The tree into the window: the rows changed in place, so the box a
    /// name is being typed in is never made again under the keys typing
    /// into it; the folder a drag would drop into lit; the selection
    /// following the file in front, and brought into view when asked.
    pub(super) fn sync_tree(&self) {
        self.tree_follow_active();
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let (rows, reveal, name_row, root_lit) = {
            let mut state = self.state.borrow_mut();
            let shown = state.project.as_ref().map_or_else(Vec::new, |project| {
                tree::with_box(project.rows(), state.tree.naming.as_ref())
            });
            let tree = &state.tree;
            let into = tree.drag.as_ref().and_then(|d| d.into.clone());
            let cut = tree.clip.as_ref().filter(|c| c.cut).map(|c| c.path.clone());
            let mut reveal = None;
            let mut name_row = -1;
            let rows: Vec<TreeRow> = shown
                .into_iter()
                .enumerate()
                .map(|(at, shown)| match shown {
                    Shown::Entry(row) => {
                        let selected = tree.selected.as_deref() == Some(row.path.as_str());
                        if selected {
                            reveal = Some(at);
                        }
                        TreeRow {
                            depth: row.depth as i32,
                            selected,
                            cut: cut
                                .as_deref()
                                .is_some_and(|c| rusty_edit::is_within(&row.path, c)),
                            drop: into.as_deref().is_some_and(|into| {
                                !into.is_empty() && rusty_edit::is_within(&row.path, into)
                            }),
                            name: row.name.into(),
                            dir: row.dir,
                            open: row.open,
                            naming: false,
                        }
                    }
                    Shown::Name { depth, folder } => {
                        name_row = at as i32;
                        TreeRow {
                            depth: depth as i32,
                            dir: folder,
                            naming: true,
                            ..TreeRow::default()
                        }
                    }
                })
                .collect();
            if name_row >= 0 {
                reveal = Some(name_row as usize);
            }
            let due = std::mem::take(&mut state.tree.reveal_due);
            let reveal = reveal.filter(|_| due).map(|row| {
                state.tree.reveal += 1;
                (state.tree.reveal, row as i32)
            });
            (rows, reveal, name_row, into.as_deref() == Some(""))
        };
        patch(&self.tree_rows, rows);
        shell.set_tree_name_row(name_row);
        shell.set_tree_drop_root(root_lit);
        if let Some((count, row)) = reveal {
            shell.set_tree_reveal_row(row);
            shell.set_tree_reveal(count);
        }
    }

    /// The file put in front is selected in the tree, the folders above it
    /// opened and it brought into view — VS Code's `explorer.autoReveal` —
    /// once each time it changes, so a folder closed over it stays closed.
    fn tree_follow_active(&self) {
        let mut state = self.state.borrow_mut();
        let state = &mut *state;
        let active = match state.active.and_then(|at| state.tabs.get(at)) {
            Some(Tab::File(path)) => Some(path.clone()),
            _ => None,
        };
        if state.tree.followed == active {
            return;
        }
        state.tree.followed = active.clone();
        let Some(path) = active else { return };
        let Some(project) = state.project.as_mut() else {
            return;
        };
        if !project.files.iter().any(|file| file == &path) {
            return;
        }
        open_above(&mut project.expanded, &path);
        state.tree.selected = Some(path);
        state.tree.reveal_due = true;
    }
}

/// The keys a menu row names for Delete: Delete, or Cmd+Backspace on a Mac,
/// whose Delete key is a backspace.
const DELETE_KEY: &str = if cfg!(target_os = "macos") {
    "Ctrl+Backspace"
} else {
    "Delete"
};

/// Names are compared without case where the disk does: Windows and macOS.
const FOLD: bool = cfg!(any(windows, target_os = "macos"));

/// Every folder above `path` opened.
fn open_above(expanded: &mut std::collections::BTreeSet<String>, path: &str) {
    let mut at = 0;
    while let Some(slash) = path[at..].find('/') {
        expanded.insert(path[..at + slash].to_string());
        at += slash + 1;
    }
}

/// A refusal of a name, in the reader's language.
fn refusal_text(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Empty => t!("tree.name-empty"),
        Refusal::Taken(name) => t!("tree.name-taken", name = name.clone()),
        Refusal::Separator => t!("tree.name-separator"),
        Refusal::Edge => t!("tree.name-edge"),
        Refusal::Dots(name) => t!("tree.name-dots", name = name.clone()),
        Refusal::Invalid(name) => t!("tree.name-invalid", name = name.clone()),
        Refusal::Blocked(name) => t!("tree.name-blocked", name = name.clone()),
    }
}

/// Why a write was refused, in the reader's language where `rusty_edit`
/// named the reason, and in the system's own words where the system did.
fn error_text(error: &rusty_edit::Error) -> String {
    use rusty_edit::Error;
    match error {
        Error::Exists { path } => t!("tree.exists", name = path.clone()),
        Error::NotFound { path } => t!("tree.gone", name = path.clone()),
        Error::IntoItself { path } => t!("tree.into-itself", name = path.clone()),
        Error::BadName { name } => t!("tree.name-dots", name = name.clone()),
        Error::Outside { path } => t!("tree.outside", name = path.clone()),
        Error::Read { source, .. } | Error::Write { source, .. } => source.to_string(),
        other => other.to_string(),
    }
}

/// Make `model` hold `rows`, changing only the rows that differ: the rows
/// before and after a change keep the elements drawing them, the box among
/// them.
fn patch(model: &VecModel<TreeRow>, rows: Vec<TreeRow>) {
    let old = model.row_count();
    let same = |at: usize, row: &TreeRow| model.row_data(at).as_ref() == Some(row);
    let mut head = 0;
    while head < old.min(rows.len()) && same(head, &rows[head]) {
        head += 1;
    }
    let mut tail = 0;
    while tail < (old - head).min(rows.len() - head)
        && same(old - 1 - tail, &rows[rows.len() - 1 - tail])
    {
        tail += 1;
    }
    let (was, now) = (old - head - tail, rows.len() - head - tail);
    let both = was.min(now);
    for at in 0..both {
        model.set_row_data(head + at, rows[head + at].clone());
    }
    for at in both..now {
        model.insert(head + at, rows[head + at].clone());
    }
    for _ in both..was {
        model.remove(head + both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> TreeRow {
        TreeRow {
            name: name.into(),
            ..TreeRow::default()
        }
    }

    fn names(model: &VecModel<TreeRow>) -> Vec<String> {
        model.iter().map(|r| r.name.to_string()).collect()
    }

    #[test]
    fn a_patch_ends_with_the_rows_asked_for() {
        let model = VecModel::from(vec![row("a"), row("b"), row("c")]);
        for wanted in [
            vec!["a", "x", "b", "c"],
            vec!["a", "c"],
            vec!["z"],
            vec![],
            vec!["a", "b", "c", "d"],
            vec!["a", "b", "c", "d"],
            vec!["b", "c", "d", "a"],
        ] {
            patch(&model, wanted.iter().map(|n| row(n)).collect());
            assert_eq!(names(&model), wanted);
        }
    }

    #[test]
    fn the_folders_above_a_path_open_and_no_more() {
        let mut open = std::collections::BTreeSet::new();
        open_above(&mut open, "core/src/驱动/mod.rs");
        assert_eq!(
            open.into_iter().collect::<Vec<_>>(),
            ["core", "core/src", "core/src/驱动"]
        );
        let mut none = std::collections::BTreeSet::new();
        open_above(&mut none, "Cargo.toml");
        assert!(none.is_empty());
    }
}
