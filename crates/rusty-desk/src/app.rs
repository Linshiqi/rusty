//! The app: the window, what it holds, and what it does when asked.
//!
//! State lives here and in the parts it owns (the editor's documents); the
//! `.slint` files draw it. Every change goes through a method that changes
//! the state and then pushes what it changed into the `Shell` global — one
//! `sync_*` per region, so what is on screen is always a function of the
//! state rather than of whichever callback ran last.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

use rusty_edit::text_file::{self, TextFile};
use rusty_embed::WindowLayout;
use rusty_i18n::{Locale, t};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::{
    AppWindow, DocTab, Icons, Note, PanelTab, PickRow, Recent, Shell, TreeRow, actions,
    docks::{Dock, Glyph},
    editor::{
        self, Document, Surface,
        backup::{self, Backup, Keeper},
    },
    keys,
    picker::{self, Mode, Pick},
    project::Project,
    work,
};

/// How long after the last keystroke a draft is backed up.
const BACKUP_AFTER: Duration = Duration::from_secs(1);
/// How long a save waits for rustfmt before saving what it has — VS Code's
/// budget for its own format-on-save.
const FORMAT_BUDGET: Duration = Duration::from_secs(1);
/// How long a closing window waits for its drafts to reach the disk.
const KEEP_BUDGET: Duration = Duration::from_secs(5);

/// What happens once a file is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Stay,
    Close,
}

/// What rustfmt made of a file being saved.
enum Format {
    Changed(String),
    Unchanged,
    /// It did not answer within [`FORMAT_BUDGET`]; the save goes ahead.
    Late,
    Failed(String),
}

/// A document on the strip: a file, or a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    File(String),
    Page(&'static str),
}

impl Tab {
    fn id(&self) -> String {
        match self {
            Tab::File(path) => format!("file:{path}"),
            Tab::Page(kind) => format!("page:{kind}"),
        }
    }

    fn title(&self) -> String {
        match self {
            Tab::File(path) => path.rsplit('/').next().unwrap_or(path).to_string(),
            Tab::Page("welcome") => t!("desk.welcome"),
            Tab::Page("settings") => t!("desk.settings"),
            Tab::Page(kind) => (*kind).to_string(),
        }
    }

    fn kind(&self) -> &str {
        match self {
            Tab::File(_) => "file",
            Tab::Page(kind) => kind,
        }
    }
}

struct NoteItem {
    id: i32,
    level: i32,
    text: String,
}

#[derive(Default)]
struct State {
    layout: WindowLayout,
    project: Option<Project>,
    recents: Vec<String>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    picker: Option<(Mode, Vec<Pick>)>,
    notes: Vec<NoteItem>,
    next_note: i32,
}

pub struct App {
    window: slint::Weak<AppWindow>,
    editor: Rc<Surface>,
    state: RefCell<State>,
    keeper: Keeper,
    /// A timer per draft, restarted by every edit, that backs it up.
    backup_timers: RefCell<HashMap<String, Timer>>,
    /// A backup that failed is said once, not once a second.
    backup_warned: Cell<bool>,
}

impl App {
    pub fn new(window: &AppWindow) -> Rc<App> {
        let saved = rusty_embed::config::workbench();
        let files = Arc::new(rusty_edit::Files::new());
        let editor = Surface::new(window, files);
        let keeper = Keeper::start(backup::directory(), |reason| {
            work::deliver(reason, |app, reason| app.backup_failed(reason))
        });
        let app = Rc::new(App {
            window: window.as_weak(),
            editor,
            keeper,
            backup_timers: RefCell::default(),
            backup_warned: Cell::new(false),
            state: RefCell::new(State {
                layout: saved.layout.clone(),
                recents: saved.recent_projects.clone(),
                tabs: vec![Tab::Page("welcome")],
                active: Some(0),
                ..State::default()
            }),
        });
        app.wire(window);
        app.sync_all();
        app
    }

    pub fn editor(&self) -> &Rc<Surface> {
        &self.editor
    }

    fn window(&self) -> Option<AppWindow> {
        self.window.upgrade()
    }

    fn wire(self: &Rc<Self>, window: &AppWindow) {
        let shell = window.global::<Shell>();
        let weak = Rc::downgrade(self);
        let with = move |f: &dyn Fn(&Rc<App>)| {
            if let Some(app) = weak.upgrade() {
                f(&app);
            }
        };
        macro_rules! on {
            ($method:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
                let with = with.clone();
                shell.$method(move |$($arg),*| with(&|$app| $body));
            }};
        }
        on!(on_act, |app, id| app.act(&id));
        on!(on_show_panel, |app, dock, panel| app
            .show_panel(&dock, &panel));
        on!(on_dock_resized, |app, dock, size| app
            .dock_resized(&dock, size));
        on!(on_pick_tab, |app, id| app.pick_tab(&id));
        on!(on_close_tab, |app, id| app.close_tab(&id));
        on!(on_tree_click, |app, index| app.tree_click(index as usize));
        on!(on_open_recent, |app, path| app
            .open_project(PathBuf::from(path.as_str()), None));
        on!(on_picker_query, |app, query| app.picker_query(&query));
        on!(on_picker_pick, |app, index| app.picker_pick(index as usize));
        on!(on_dismiss_note, |app, id| app.dismiss_note(id));
        on!(on_note_action, |app, id| app.dismiss_note(id));
        on!(on_set_theme, |app, theme| app.set_theme(&theme));
        on!(on_set_language, |app, tag| app.set_language(&tag));
        on!(on_set_zoom, |app, zoom| app.set_zoom(zoom));
        let weak = Rc::downgrade(self);
        shell.on_key(move |event| {
            let Some(app) = weak.upgrade() else {
                return false;
            };
            let modifiers = event.modifiers;
            let Some(chord) = keys::chord(
                &event.text,
                modifiers.control,
                modifiers.alt,
                modifiers.shift,
                modifiers.meta,
            ) else {
                return false;
            };
            match actions::bound_to(&chord) {
                Some(action) => {
                    app.act(action.id);
                    true
                }
                None => false,
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_edit(move |path| {
            if let Some(app) = weak.upgrade() {
                app.sync_tabs();
                app.back_up_soon(path);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_caret(move || {
            if let Some(app) = weak.upgrade() {
                app.sync_status();
            }
        });
    }

    // ---- actions ---------------------------------------------------------

    /// Do what an action names. The one door for buttons, menus, keys and
    /// the palette.
    pub fn act(self: &Rc<Self>, id: &str) {
        match id {
            "palette.open" => self.open_picker(Mode::Commands),
            "finder.open" => self.open_picker(Mode::Files),
            "project.open" => self.choose_project(),
            "settings.open" => self.open_tab(Tab::Page("settings")),
            "welcome.open" => self.open_tab(Tab::Page("welcome")),
            "file.save" => self.save(),
            "find.open" => self.editor.find_open(false),
            "find.replace" => self.editor.find_open(true),
            "find.next" => self.editor.find_next(true),
            "find.previous" => self.editor.find_next(false),
            "editor.go-to-line" => self.open_picker_with(Mode::Files, ":"),
            "tab.close" => {
                let id = self.active_tab().map(|tab| tab.id());
                if let Some(id) = id {
                    self.close_tab(&id);
                }
            }
            "dock.left" => self.toggle_dock(Dock::Left),
            "dock.bottom" => self.toggle_dock(Dock::Bottom),
            "dock.right" => self.toggle_dock(Dock::Right),
            "problems.show" => self.show_panel("bottom", "problems"),
            "view.zoom-in" => self.set_zoom(self.zoom() + 0.1),
            "view.zoom-out" => self.set_zoom(self.zoom() - 0.1),
            "view.zoom-reset" => self.set_zoom(1.0),
            "theme.system" => self.set_theme("system"),
            "theme.dark" => self.set_theme("dark"),
            "theme.light" => self.set_theme("light"),
            "project.menu" => self.open_picker(Mode::Projects),
            "app.menu" => self.open_picker(Mode::Commands),
            "app.exit" => {
                self.keep_drafts_and_wait();
                let _ = slint::quit_event_loop();
            }
            other => {
                // Not moved to this window yet: say so rather than do nothing.
                let name = actions::by_id(other)
                    .map(|action| rusty_i18n::lookup(action.title))
                    .unwrap_or_else(|| other.to_string());
                self.note(0, t!("desk.not-yet-action", name = name));
            }
        }
    }

    // ---- projects --------------------------------------------------------

    fn choose_project(self: &Rc<Self>) {
        work::spawn(
            || rfd::FileDialog::new().pick_folder(),
            |app, picked| {
                if let Some(root) = picked {
                    app.open_project(root, None);
                }
            },
        );
    }

    /// Open a project, and then `file` in it when one is named.
    pub fn open_project(self: &Rc<Self>, root: PathBuf, file: Option<String>) {
        work::spawn(
            move || Project::read(root.clone()).map_err(|reason| (root, reason)),
            move |app, read| match read {
                Ok(project) => {
                    app.project_opened(project);
                    if let Some(file) = file {
                        app.open_file(file);
                    }
                }
                Err((root, reason)) => app.note(
                    2,
                    t!(
                        "desk.open-failed",
                        path = root.display().to_string(),
                        reason = reason
                    ),
                ),
            },
        );
    }

    fn project_opened(self: &Rc<Self>, project: Project) {
        // The project going keeps its drafts, as a closing window does.
        self.keep_drafts();
        self.backup_timers.borrow_mut().clear();
        let opened = project.root.clone();
        let root = project.root.to_string_lossy().into_owned();
        rusty_embed::config::record_recent(&root);
        {
            let mut state = self.state.borrow_mut();
            for tab in std::mem::take(&mut state.tabs) {
                if let Tab::File(path) = tab {
                    self.editor.close(&path);
                }
            }
            state.tabs = vec![Tab::Page("welcome")];
            state.active = Some(0);
            state.recents.retain(|path| path != &root);
            state.recents.insert(0, root);
            state.project = Some(project);
            if !state.layout.left.open {
                state.layout.left.open = true;
            }
        }
        self.sync_all();
        self.bring_back_drafts(opened);
    }

    fn tree_click(self: &Rc<Self>, index: usize) {
        let row = {
            let state = self.state.borrow();
            state
                .project
                .as_ref()
                .and_then(|project| project.rows().into_iter().nth(index))
        };
        let Some(row) = row else { return };
        if row.dir {
            if let Some(project) = self.state.borrow_mut().project.as_mut()
                && !project.expanded.remove(&row.path)
            {
                project.expanded.insert(row.path.clone());
            }
            self.sync_tree();
        } else {
            self.open_file(row.path);
        }
    }

    /// Open a file of the project: in front if it is open, read off the
    /// UI thread if it is not.
    pub fn open_file(self: &Rc<Self>, path: String) {
        if self.editor.holds(&path) {
            self.open_tab(Tab::File(path));
            return;
        }
        let Some(root) = self.state.borrow().project.as_ref().map(|p| p.root.clone()) else {
            return;
        };
        work::spawn(
            move || {
                let read = rusty_edit::text_file::read_text(&root, &path);
                (path, read.map_err(|error| error.to_string()))
            },
            |app, (path, read)| {
                // Opened meanwhile — a draft brought back from last time is
                // the case — and the document that is open is the one to
                // keep: read again it would replace the draft with the disk.
                if app.editor.holds(&path) {
                    app.open_tab(Tab::File(path));
                    return;
                }
                let refused =
                    |why: String| t!("desk.open-failed", path = path.clone(), reason = why);
                match read {
                    Ok(TextFile::Text {
                        text,
                        charset,
                        ending,
                    }) => {
                        let mut document = Document::new(path.clone(), text, charset, ending);
                        document.language = editor::language_of(&path).map(str::to_string);
                        app.editor.show_new(document);
                        app.open_tab(Tab::File(path));
                    }
                    Ok(TextFile::Binary) => app.note(1, refused(t!("desk.binary"))),
                    Ok(TextFile::TooLarge) => app.note(1, refused(t!("desk.too-large"))),
                    Err(reason) => app.note(2, refused(reason)),
                }
            },
        );
    }

    fn root(&self) -> Option<PathBuf> {
        self.state.borrow().project.as_ref().map(|p| p.root.clone())
    }

    // ---- saving ----------------------------------------------------------

    fn save(self: &Rc<Self>) {
        if let Some(path) = self.editor.current_path() {
            self.save_file(path, After::Stay);
        }
    }

    /// Save a file: Rust through rustfmt first, within a budget, and the
    /// answer used only while the draft is still the text it was sent.
    fn save_file(self: &Rc<Self>, path: String, after: After) {
        let (Some(root), Some((text, ..)), Some(version)) = (
            self.root(),
            self.editor.draft_of(&path),
            self.editor.version_of(&path),
        ) else {
            return;
        };
        if !path.ends_with(".rs") {
            self.write_file(path, after);
            return;
        }
        work::spawn(
            move || {
                let format = format_within(&root, &path, text);
                (path, format)
            },
            move |app, (path, format)| {
                match format {
                    Format::Changed(text) => {
                        app.editor.replace(&path, version, text);
                    }
                    Format::Failed(reason) => {
                        app.note(1, t!("desk.format-failed", reason = reason));
                    }
                    Format::Unchanged | Format::Late => {}
                }
                app.write_file(path, after);
            },
        );
    }

    /// Write a file's draft as it stands now, in what it was read as.
    fn write_file(self: &Rc<Self>, path: String, after: After) {
        let (Some(root), Some((text, charset, ending))) =
            (self.root(), self.editor.draft_of(&path))
        else {
            return;
        };
        work::spawn(
            move || {
                text_file::save_text(&root, &path, &text, charset, ending)
                    .map(|()| (path.clone(), text))
                    .map_err(|error| (path, error.to_string()))
            },
            move |app, saved| match saved {
                Ok((path, text)) => {
                    app.editor.saved(&path, &text);
                    // Gone if nothing was typed while it was written; kept
                    // anew if something was.
                    app.back_up(&path);
                    app.sync_tabs();
                    if after == After::Close {
                        app.close_tab(&Tab::File(path).id());
                    }
                }
                Err((path, reason)) => {
                    app.note(2, t!("desk.save-failed", path = path, reason = reason))
                }
            },
        );
    }

    // ---- drafts kept between runs ----------------------------------------

    /// A draft was edited: back it up once typing has stopped for a moment.
    fn back_up_soon(self: &Rc<Self>, path: &str) {
        let weak = Rc::downgrade(self);
        let owned = path.to_string();
        self.backup_timers
            .borrow_mut()
            .entry(path.to_string())
            .or_default()
            .start(TimerMode::SingleShot, BACKUP_AFTER, move || {
                if let Some(app) = weak.upgrade() {
                    app.back_up(&owned);
                }
            });
    }

    /// Keep `path`'s draft, or forget it where the text is the disk's.
    fn back_up(&self, path: &str) {
        let Some(root) = self.root() else { return };
        match self.editor.backup_of(path) {
            Some(draft) => self.keeper.write(&root, draft),
            None => self.keeper.remove(&root, path),
        }
    }

    /// Keep every open draft and forget every backup of a file now saved:
    /// the project is going, or the window is.
    fn keep_drafts(&self) {
        let Some(root) = self.root() else { return };
        for (path, draft) in self.editor.backups() {
            match draft {
                Some(draft) => self.keeper.write(&root, draft),
                None => self.keeper.remove(&root, &path),
            }
        }
    }

    fn keep_drafts_and_wait(&self) {
        self.keep_drafts();
        if !self.keeper.flush(KEEP_BUDGET) {
            eprintln!("rusty: the drafts were still being written when the window closed");
        }
    }

    /// Put back the drafts kept for the project at `root`, each over the
    /// disk's text so that it is unsaved, marked so, and one undo from it.
    fn bring_back_drafts(self: &Rc<Self>, root: PathBuf) {
        let asked = root.clone();
        self.keeper.read(&asked.clone(), move |kept| {
            let found: Vec<(Backup, String)> = kept
                .into_iter()
                .map(|draft| {
                    // A file gone or no longer text has nothing under the
                    // draft: saving it writes the draft whole.
                    let disk = match text_file::read_text(&asked, &draft.path) {
                        Ok(TextFile::Text { text, .. }) => text,
                        _ => String::new(),
                    };
                    (draft, disk)
                })
                .collect();
            work::deliver(found, move |app, found| app.drafts_back(&root, found));
        });
    }

    fn drafts_back(self: &Rc<Self>, root: &Path, found: Vec<(Backup, String)>) {
        // Another project opened meanwhile: these stay kept for next time.
        if found.is_empty() || self.root().as_deref() != Some(root) {
            return;
        }
        let mut first = None;
        for (draft, disk) in found {
            let path = draft.path.clone();
            if !self.editor.holds(&path) {
                let mut document = Document::new(path.clone(), disk, draft.charset, draft.ending);
                document.language = editor::language_of(&path).map(str::to_string);
                self.editor.add(document);
            }
            self.editor.restore(&path, draft.text);
            let mut state = self.state.borrow_mut();
            let tab = Tab::File(path);
            if !state.tabs.contains(&tab) {
                state.tabs.push(tab.clone());
            }
            first.get_or_insert(tab);
        }
        // In front only when nothing else is: a file opened meanwhile stays
        // where it is, caret and scroll included.
        let on_a_page = !matches!(self.active_tab(), Some(Tab::File(_)));
        if on_a_page {
            let mut state = self.state.borrow_mut();
            state.active = first.and_then(|tab| state.tabs.iter().position(|t| *t == tab));
        }
        if on_a_page {
            self.front_tab();
        } else {
            self.sync_tabs();
        }
    }

    fn backup_failed(self: &Rc<Self>, reason: String) {
        if !self.backup_warned.replace(true) {
            self.note(2, t!("desk.backup-failed", reason = reason));
        }
    }

    // ---- tabs ------------------------------------------------------------

    fn active_tab(&self) -> Option<Tab> {
        let state = self.state.borrow();
        state.active.and_then(|at| state.tabs.get(at).cloned())
    }

    fn open_tab(self: &Rc<Self>, tab: Tab) {
        {
            let mut state = self.state.borrow_mut();
            let at = match state.tabs.iter().position(|t| *t == tab) {
                Some(at) => at,
                None => {
                    state.tabs.push(tab.clone());
                    state.tabs.len() - 1
                }
            };
            state.active = Some(at);
        }
        self.front_tab();
    }

    fn pick_tab(self: &Rc<Self>, id: &str) {
        let found = self.state.borrow().tabs.iter().position(|t| t.id() == id);
        if let Some(at) = found {
            self.state.borrow_mut().active = Some(at);
            self.front_tab();
        }
    }

    /// Close a tab; a file with changes not saved asks first — save, throw
    /// the changes away, or keep the tab — as every editor does.
    fn close_tab(self: &Rc<Self>, id: &str) {
        let tab = self
            .state
            .borrow()
            .tabs
            .iter()
            .find(|t| t.id() == id)
            .cloned();
        if let Some(Tab::File(path)) = tab
            && self.editor.is_dirty(&path)
        {
            match ask_to_save(&path) {
                Some(true) => self.save_file(path, After::Close),
                Some(false) => {
                    if let Some(root) = self.root() {
                        self.keeper.remove(&root, &path);
                    }
                    self.drop_tab(id);
                }
                None => {}
            }
            return;
        }
        self.drop_tab(id);
    }

    fn drop_tab(self: &Rc<Self>, id: &str) {
        let closed = {
            let mut state = self.state.borrow_mut();
            let Some(at) = state.tabs.iter().position(|t| t.id() == id) else {
                return;
            };
            let closed = state.tabs.remove(at);
            state.active = match state.active {
                _ if state.tabs.is_empty() => None,
                Some(active) if active > at => Some(active - 1),
                Some(active) if active == at => Some(at.min(state.tabs.len() - 1)),
                other => other,
            };
            closed
        };
        if let Tab::File(path) = closed {
            self.backup_timers.borrow_mut().remove(&path);
            self.editor.close(&path);
        }
        self.front_tab();
    }

    /// Put the active tab's document on screen and say so.
    fn front_tab(self: &Rc<Self>) {
        match self.active_tab() {
            Some(Tab::File(path)) => self.editor.show(&path),
            _ => self.editor.show_nothing(),
        }
        self.sync_tabs();
        self.sync_tree();
        self.sync_status();
        if let Some(window) = self.window()
            && !matches!(self.active_tab(), Some(Tab::File(_)))
        {
            window.invoke_focus_window();
        }
    }

    // ---- docks -----------------------------------------------------------

    fn toggle_dock(self: &Rc<Self>, dock: Dock) {
        {
            let mut state = self.state.borrow_mut();
            let layout = dock.of_mut(&mut state.layout);
            layout.open = !layout.open;
        }
        self.sync_docks();
        self.save_layout();
    }

    fn show_panel(self: &Rc<Self>, dock: &str, panel: &str) {
        let Some(dock) = Dock::parse(dock) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let layout = dock.of_mut(&mut state.layout);
            layout.open = true;
            layout.front = Some(panel.to_string());
        }
        self.sync_docks();
        self.save_layout();
    }

    fn dock_resized(self: &Rc<Self>, dock: &str, size: f32) {
        let Some(dock) = Dock::parse(dock) else {
            return;
        };
        dock.of_mut(&mut self.state.borrow_mut().layout).size = size;
        self.save_layout();
    }

    fn save_layout(&self) {
        let layout = self.state.borrow().layout.clone();
        let _ = rusty_embed::config::update(move |saved| saved.layout = layout);
    }

    // ---- appearance ------------------------------------------------------

    fn set_theme(self: &Rc<Self>, theme: &str) {
        self.state.borrow_mut().layout.theme = theme.to_string();
        self.sync_settings();
        self.save_layout();
    }

    fn zoom(&self) -> f32 {
        self.state.borrow().layout.zoom
    }

    fn set_zoom(self: &Rc<Self>, zoom: f32) {
        let zoom = (zoom * 100.0).round().clamp(70.0, 160.0) / 100.0;
        self.state.borrow_mut().layout.zoom = zoom;
        self.sync_settings();
        self.save_layout();
    }

    fn set_language(self: &Rc<Self>, tag: &str) {
        apply_locale(tag);
        let _ = rusty_embed::config::update(|saved| saved.locale = Some(tag.to_string()));
        self.sync_all();
    }

    // ---- the picker ------------------------------------------------------

    fn open_picker(self: &Rc<Self>, mode: Mode) {
        self.open_picker_with(mode, "");
    }

    /// The picker open with `text` already typed: `:` for a line.
    fn open_picker_with(self: &Rc<Self>, mode: Mode, text: &str) {
        self.state.borrow_mut().picker = Some((mode, Vec::new()));
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_picker_text(text.into());
            shell.set_picker_hint(SharedString::from(match mode {
                Mode::Commands => t!("desk.palette-hint"),
                Mode::Files => t!("desk.finder-hint"),
                Mode::Projects => t!("desk.projects-hint"),
            }));
            shell.set_picker_open(true);
        }
        self.picker_query(text);
    }

    fn picker_query(self: &Rc<Self>, query: &str) {
        let Some(mode) = self.state.borrow().picker.as_ref().map(|(mode, _)| *mode) else {
            return;
        };
        let found = match mode {
            Mode::Commands => picker::commands(query),
            Mode::Files if picker::line_target(query).is_some() => self.line_rows(query),
            Mode::Files => {
                let state = self.state.borrow();
                let files = state.project.as_ref().map_or(&[][..], |p| &p.files);
                picker::files(query, files)
            }
            Mode::Projects => picker::projects(query, &self.state.borrow().recents),
        };
        let rows: Vec<PickRow> = found
            .iter()
            .map(|(title, detail, _)| PickRow {
                title: title.into(),
                detail: if mode == Mode::Commands {
                    SharedString::new()
                } else {
                    detail.into()
                },
                chord: if mode == Mode::Commands {
                    detail.into()
                } else {
                    SharedString::new()
                },
            })
            .collect();
        self.state.borrow_mut().picker = Some((mode, found.into_iter().map(|(.., p)| p).collect()));
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_picker_rows(ModelRc::new(VecModel::from(rows)));
            shell.set_picker_selected(0);
        }
    }

    fn picker_pick(self: &Rc<Self>, index: usize) {
        let pick = self
            .state
            .borrow_mut()
            .picker
            .take()
            .and_then(|(_, picks)| picks.into_iter().nth(index));
        if let Some(window) = self.window() {
            window.global::<Shell>().set_picker_open(false);
        }
        match pick {
            Some(Pick::Action(id)) => self.act(id),
            Some(Pick::File(path)) => self.open_file(path),
            Some(Pick::Project(path)) => self.open_project(PathBuf::from(path), None),
            Some(Pick::Line(line, col)) => self.editor.go_to(line, col),
            Some(Pick::Nothing) => self.front_focus(),
            None => {
                // Escape or a click outside: back to whatever had the keys.
                self.front_focus();
            }
        }
    }

    /// The finder's rows for `:` and a line: the line to go to, or with
    /// nothing typed yet, where the caret is and how far the file goes.
    fn line_rows(&self, query: &str) -> Vec<(String, String, Pick)> {
        let Some((Some(Tab::File(path)), Some((line, col, ..)), Some(count))) = Some((
            self.active_tab(),
            self.editor.status(),
            self.editor.line_count(),
        )) else {
            return Vec::new();
        };
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        match picker::line_target(query) {
            Some((Some(to), Some(at))) => vec![(
                t!(
                    "desk.go-to-column",
                    line = to.min(count).to_string(),
                    col = at.to_string()
                ),
                name,
                Pick::Line(to, Some(at)),
            )],
            Some((Some(to), None)) => vec![(
                t!("desk.go-to-line", line = to.min(count).to_string()),
                name,
                Pick::Line(to, None),
            )],
            _ => vec![(
                t!(
                    "desk.go-to-prompt",
                    line = line.to_string(),
                    col = col.to_string(),
                    count = count.to_string()
                ),
                name,
                Pick::Nothing,
            )],
        }
    }

    /// The keys back to the document in front, or to the window.
    fn front_focus(&self) {
        if let Some(window) = self.window() {
            match self.active_tab() {
                Some(Tab::File(_)) => window.invoke_focus_code(),
                _ => window.invoke_focus_window(),
            }
        }
    }

    // ---- closing ---------------------------------------------------------

    /// The window is asked to close, and closes: VS Code's hot exit. What
    /// is not saved is kept as a draft and is back, unsaved, the next time
    /// the project opens — so closing asks nothing.
    pub fn closing(self: &Rc<Self>) -> slint::CloseRequestResponse {
        self.keep_drafts_and_wait();
        slint::CloseRequestResponse::HideWindow
    }

    // ---- notifications ---------------------------------------------------

    /// Say something in the corner: 0 information, 1 a warning, 2 an error.
    /// Information goes by itself after a while; the rest wait to be read.
    pub fn note(self: &Rc<Self>, level: i32, text: String) {
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_note += 1;
            let id = state.next_note;
            state.notes.push(NoteItem { id, level, text });
            if state.notes.len() > 4 {
                state.notes.remove(0);
            }
            id
        };
        self.sync_notes();
        if level == 0 {
            let weak = Rc::downgrade(self);
            slint::Timer::single_shot(std::time::Duration::from_secs(6), move || {
                if let Some(app) = weak.upgrade() {
                    app.dismiss_note(id);
                }
            });
        }
    }

    fn dismiss_note(self: &Rc<Self>, id: i32) {
        self.state.borrow_mut().notes.retain(|note| note.id != id);
        self.sync_notes();
    }

    // ---- pushing the state into the window -------------------------------

    fn sync_all(&self) {
        self.sync_project();
        self.sync_docks();
        self.sync_tabs();
        self.sync_tree();
        self.sync_status();
        self.sync_notes();
        self.sync_settings();
    }

    fn sync_project(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        shell.set_has_project(state.project.is_some());
        shell.set_project(
            state
                .project
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_default()
                .into(),
        );
        shell.set_target(SharedString::from(t!("desk.simulator")));
        shell.set_activity(SharedString::from(t!("desk.ready")));
        let recents: Vec<Recent> = state
            .recents
            .iter()
            .take(8)
            .map(|path| Recent {
                name: path
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap_or(path)
                    .into(),
                path: path.into(),
            })
            .collect();
        shell.set_recents(ModelRc::new(VecModel::from(recents)));
    }

    fn sync_docks(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let icons = window.global::<Icons>();
        let icon = |glyph: Glyph| -> SharedString {
            match glyph {
                Glyph::Files => icons.get_files(),
                Glyph::Search => icons.get_search(),
                Glyph::Branch => icons.get_branch(),
                Glyph::Output => icons.get_output(),
                Glyph::Warn => icons.get_warn(),
                Glyph::Monitor => icons.get_monitor(),
                Glyph::Board => icons.get_board(),
                Glyph::Assistant => icons.get_assistant(),
            }
        };
        let panels = |dock: Dock| -> ModelRc<PanelTab> {
            let tabs: Vec<PanelTab> = dock
                .panels()
                .iter()
                .map(|(id, title, glyph)| PanelTab {
                    id: (*id).into(),
                    title: rusty_i18n::lookup(title).into(),
                    icon: icon(*glyph),
                })
                .collect();
            ModelRc::new(VecModel::from(tabs))
        };
        let state = self.state.borrow();
        let layout = &state.layout;
        shell.set_left_open(layout.left.open);
        shell.set_left_size(layout.left.size);
        shell.set_left_panels(panels(Dock::Left));
        shell.set_left_front(Dock::Left.front(layout).into());
        shell.set_bottom_open(layout.bottom.open);
        shell.set_bottom_size(layout.bottom.size);
        shell.set_bottom_panels(panels(Dock::Bottom));
        shell.set_bottom_front(Dock::Bottom.front(layout).into());
        shell.set_right_open(layout.right.open);
        shell.set_right_size(layout.right.size);
        shell.set_right_panels(panels(Dock::Right));
        shell.set_right_front(Dock::Right.front(layout).into());
    }

    fn sync_tabs(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let tabs: Vec<DocTab> = state
            .tabs
            .iter()
            .map(|tab| DocTab {
                id: tab.id().into(),
                title: tab.title().into(),
                hint: SharedString::new(),
                dirty: matches!(tab, Tab::File(path) if self.editor.is_dirty(path)),
                kind: tab.kind().into(),
            })
            .collect();
        let active = state.active.and_then(|at| state.tabs.get(at));
        shell.set_tabs(ModelRc::new(VecModel::from(tabs)));
        shell.set_active_tab(active.map(Tab::id).unwrap_or_default().into());
        shell.set_active_kind(
            active
                .map(|t| t.kind().to_string())
                .unwrap_or_default()
                .into(),
        );
        shell.set_active_title(active.map(Tab::title).unwrap_or_default().into());
        let title = match (state.project.as_ref(), active) {
            (Some(project), Some(tab)) => format!("{} — {} — rusty", tab.title(), project.name),
            (Some(project), None) => format!("{} — rusty", project.name),
            _ => "rusty".to_string(),
        };
        window.set_caption(title.into());
    }

    fn sync_tree(&self) {
        let Some(window) = self.window() else { return };
        let state = self.state.borrow();
        let active = match state.active.and_then(|at| state.tabs.get(at)) {
            Some(Tab::File(path)) => Some(path.clone()),
            _ => None,
        };
        let rows: Vec<TreeRow> = state.project.as_ref().map_or(Vec::new(), |project| {
            project
                .rows()
                .into_iter()
                .map(|row| TreeRow {
                    depth: row.depth as i32,
                    selected: active.as_deref() == Some(row.path.as_str()),
                    name: row.name.into(),
                    dir: row.dir,
                    open: row.open,
                })
                .collect()
        });
        window
            .global::<Shell>()
            .set_tree(ModelRc::new(VecModel::from(rows)));
    }

    fn sync_status(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let file = matches!(self.active_tab(), Some(Tab::File(_)));
        let (caret, encoding, eol) = match self.editor.status().filter(|_| file) {
            Some((line, col, charset, ending)) => (
                t!("desk.caret", line = line.to_string(), col = col.to_string()),
                charset,
                ending,
            ),
            None => (String::new(), "", ""),
        };
        shell.set_caret(caret.into());
        shell.set_encoding(encoding.into());
        shell.set_eol(eol.into());
        let language = match self.active_tab() {
            Some(Tab::File(path)) => editor::language_of(&path).unwrap_or("Plain text"),
            _ => "",
        };
        shell.set_language(language.into());
    }

    fn sync_notes(&self) {
        let Some(window) = self.window() else { return };
        let notes: Vec<Note> = self
            .state
            .borrow()
            .notes
            .iter()
            .map(|note| Note {
                id: note.id,
                level: note.level,
                text: note.text.clone().into(),
                action: SharedString::new(),
            })
            .collect();
        window
            .global::<Shell>()
            .set_notes(ModelRc::new(VecModel::from(notes)));
    }

    fn sync_settings(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        shell.set_theme_choice(state.layout.theme.clone().into());
        shell.set_language_choice(rusty_i18n::locale().tag().into());
        shell.set_zoom(state.layout.zoom);
        crate::zoom::apply(&window, state.layout.zoom);
    }
}

/// Ask whether to save `path` before its tab closes: `Some(true)` save,
/// `Some(false)` throw the changes away, `None` keep the tab open.
fn ask_to_save(path: &str) -> Option<bool> {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    let name = path.rsplit('/').next().unwrap_or(path);
    let answer = MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(t!("desk.unsaved-title"))
        .set_description(t!("desk.unsaved-detail", name = name))
        .set_buttons(MessageButtons::YesNoCancel)
        .show();
    match answer {
        MessageDialogResult::Yes => Some(true),
        MessageDialogResult::No => Some(false),
        _ => None,
    }
}

/// rustfmt's answer for a file being saved, or [`Format::Late`] once
/// [`FORMAT_BUDGET`] has passed — the format runs on, and its answer is
/// dropped when it comes.
fn format_within(root: &Path, path: &str, text: String) -> Format {
    let (answer, answered) = mpsc::channel();
    let (root, path) = (root.to_path_buf(), path.to_string());
    std::thread::spawn(move || {
        let _ = answer.send(rusty_edit::format_rust(&root, &path, &text).map(|f| {
            if f.changed {
                Format::Changed(f.text)
            } else {
                Format::Unchanged
            }
        }));
    });
    match answered.recv_timeout(FORMAT_BUDGET) {
        Ok(Ok(format)) => format,
        Ok(Err(error)) => Format::Failed(error.to_string()),
        Err(_) => Format::Late,
    }
}

/// The language to start in: the one chosen, or the system's when nothing
/// was — and English when the system's is one rusty does not speak.
pub fn chosen_locale(saved: Option<&str>) -> String {
    saved
        .and_then(Locale::parse)
        .or_else(|| sys_locale::get_locale().as_deref().and_then(Locale::parse))
        .unwrap_or_default()
        .tag()
        .to_string()
}

/// Switch both halves of the window to a language: the catalogue the Rust
/// side reads and the bundle the `.slint` files read.
pub fn apply_locale(tag: &str) {
    let locale = Locale::parse(tag).unwrap_or_default();
    rusty_i18n::set_locale(locale);
    let _ = slint::select_bundled_translation(locale.tag());
}
