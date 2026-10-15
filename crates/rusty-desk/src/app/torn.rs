//! A tab moved into a window of its own — VS Code's Move into New Window,
//! from the palette (`editor.detach`) or by dragging a tab beyond the main
//! window's edge.
//!
//! One process and a second Slint window: `TornWindow` (`ui/torn.slint`)
//! carries the same code view as the main window, and a surface of its own
//! drives it (`Surface::beside`) over the documents the main window's
//! surface holds. So the split's rule holds across windows: one file open in
//! two views is one document, an edit draws both, the history is the
//! document's, the servers hear of the file once, and the last view to close
//! is the one that asks about changes not saved.
//!
//! One file a window. What is about that file happens there — typing,
//! completion, the hover card and its fixes, find, folds, save, undo;
//! what is about the project — the palette, the finder, a jump to another
//! file, a list of places, a run — is the main window's, which comes
//! forward to show it. The main window closing closes these through its own
//! exit, which keeps every draft (hot exit), and a torn file is written to
//! the remembered strip with the main window's, so a restart opens it there.

use std::{cell::Cell, rc::Rc, time::Duration};

use rusty_i18n::t;
use slint::{CloseRequestResponse, ComponentHandle, PhysicalPosition, Timer, TimerMode};

use super::{App, Tab, ask_to_save, saving::After, strip};
use crate::{AppWindow, Shell, TornWindow, actions, editor::Surface, keys};

/// A window a tab was moved into.
pub(super) struct Torn {
    pub window: TornWindow,
    pub surface: Rc<Surface>,
    pub path: String,
    /// Puts away what the last key here could not do.
    message: Timer,
    /// The interface's zoom last put on this window, over the display's
    /// own factor, which is the window's factor divided by it.
    zoomed: Cell<f32>,
}

impl Torn {
    fn id(&self) -> usize {
        self.surface.id()
    }
}

/// Actions a window of its own does not do, because they act on the main
/// window's editor after a list there: Go to Line jumps in the file in front
/// there, and a split is the main window's two groups.
const NOT_HERE: &[&str] = &["editor.go-to-line", "editor.split"];

/// Actions about the file in this window, done here; every other action is
/// the main window's, which comes forward.
const DONE_HERE: &[&str] = &[
    "file.save",
    "tab.close",
    "find.open",
    "find.replace",
    "find.next",
    "find.previous",
    "editor.show-hover",
    "editor.fold",
    "editor.unfold",
    "editor.fold-all",
    "editor.unfold-all",
    "problem.next",
    "problem.previous",
    "editor.vim",
    "editor.detach",
    "view.zoom-in",
    "view.zoom-out",
    "view.zoom-reset",
    "theme.system",
    "theme.dark",
    "theme.light",
];

impl App {
    /// The surface an action is about: a window of its own's, while an
    /// action asked for there runs, else the main window's.
    pub(super) fn keyed(&self) -> Rc<Surface> {
        self.keyed
            .borrow()
            .clone()
            .unwrap_or_else(|| self.editor.clone())
    }

    /// Every surface: the main window's and each torn window's.
    pub(super) fn surfaces(&self) -> Vec<Rc<Surface>> {
        std::iter::once(self.editor.clone())
            .chain(self.torn.borrow().iter().map(|torn| torn.surface.clone()))
            .collect()
    }

    /// How many windows of their own show `path`.
    pub(super) fn torn_views_of(&self, path: &str) -> usize {
        self.torn
            .borrow()
            .iter()
            .filter(|torn| torn.path == path)
            .count()
    }

    /// Each window of its own and the surface in it, for the probe.
    pub fn torn_windows(&self) -> Vec<(TornWindow, Rc<Surface>)> {
        self.torn
            .borrow()
            .iter()
            .map(|torn| (torn.window.clone_strong(), torn.surface.clone()))
            .collect()
    }

    /// The files windows of their own show, for the remembered strip.
    pub(super) fn torn_paths(&self) -> Vec<String> {
        self.torn
            .borrow()
            .iter()
            .map(|torn| torn.path.clone())
            .collect()
    }

    /// Move into New Window: the tab `id` — the file in front when none is
    /// named — into a window of its own, put at `at` on the screen (where a
    /// dragged tab was let go) or beside the main window.
    pub(super) fn detach(self: &Rc<Self>, id: Option<String>, at: Option<PhysicalPosition>) {
        let (tab, in_front, in_shade) = {
            let state = self.state.borrow();
            let front = state.active.and_then(|at| state.tabs.get(at)).cloned();
            let shade = state
                .other_active
                .and_then(|at| state.other.get(at))
                .filter(|_| state.split)
                .cloned();
            let tab = match &id {
                Some(id) => state
                    .tabs
                    .iter()
                    .chain(&state.other)
                    .find(|tab| tab.id() == *id)
                    .cloned(),
                None => front.clone(),
            };
            let in_front = tab.is_some() && tab == front;
            let in_shade = tab.is_some() && tab == shade;
            (tab, in_front, in_shade)
        };
        let path = match tab {
            Some(Tab::File(path)) if !rusty_edit::is_expansion(&path) => path,
            Some(_) => {
                self.note_transient(0, t!("desk.detach-file-only"));
                return;
            }
            None => return,
        };
        // Where its view was scrolled, to be found there again.
        let scroll = if in_front {
            Some(self.editor.scroll())
        } else if in_shade {
            Some(self.editor.shade_scroll())
        } else {
            None
        };
        let window = match TornWindow::new() {
            Ok(window) => window,
            Err(error) => {
                self.note(2, error.to_string());
                return;
            }
        };
        let surface = self.editor.beside(&window);
        self.wire_torn(&window, &surface);
        self.torn.borrow_mut().push(Torn {
            window: window.clone_strong(),
            surface: surface.clone(),
            path: path.clone(),
            message: Timer::default(),
            zoomed: Cell::new(1.0),
        });
        // Off the main window's strip; the document stays open, since the
        // new window is a view of it now.
        self.drop_tab(&Tab::File(path.clone()).id());
        let (view, vim) = {
            let state = self.state.borrow();
            (state.view, state.vim)
        };
        surface.set_view(view);
        surface.set_vim(vim);
        self.place_torn(&window, at);
        if let Err(error) = window.show() {
            self.note(2, error.to_string());
        }
        if let Some(torn) = self.torn.borrow().last() {
            self.dress_torn(torn);
        }
        surface.show(&path);
        if let Some(scroll) = scroll {
            surface.set_scroll(scroll);
        }
        self.sync_torn();
        self.strip_changed();
    }

    /// Where a new window goes: its tab under the pointer that let it go,
    /// or down and to the right of the main window's corner.
    fn place_torn(&self, window: &TornWindow, at: Option<PhysicalPosition>) {
        let Some(main) = self.window() else { return };
        let scale = main.window().scale_factor();
        let at = at.unwrap_or_else(|| {
            let corner = main.window().position();
            let step = (48.0 * scale) as i32 * (self.torn.borrow().len() as i32);
            PhysicalPosition::new(corner.x + step + (120.0 * scale) as i32, corner.y + step)
        });
        // The pointer over the tab, below the system's title bar.
        let offset = (60.0 * scale) as i32;
        window
            .window()
            .set_position(PhysicalPosition::new(at.x - offset, at.y - offset / 2));
    }

    /// A window of its own takes the main window's theme and zoom: the
    /// zoom over its own display's factor, which may not be the main
    /// window's display.
    fn dress_torn(&self, torn: &Torn) {
        let (theme, zoom) = {
            let state = self.state.borrow();
            (state.layout.theme.clone(), state.layout.zoom)
        };
        torn.window.global::<Shell>().set_theme_choice(theme.into());
        let window = torn.window.window();
        let display = window.scale_factor() / torn.zoomed.get();
        let wanted = display * zoom;
        if (window.scale_factor() - wanted).abs() > 0.001 {
            window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
                scale_factor: wanted,
            });
        }
        torn.zoomed.set(zoom);
    }

    fn wire_torn(self: &Rc<Self>, window: &TornWindow, surface: &Rc<Surface>) {
        let id = surface.id();
        let weak = Rc::downgrade(self);
        window.on_key(move |event| {
            let Some(app) = weak.upgrade() else {
                return false;
            };
            let m = event.modifiers;
            let Some(chord) = keys::chord(&event.text, m.control, m.alt, m.shift, m.meta) else {
                return false;
            };
            match actions::bound_to(&chord) {
                Some(action) => {
                    app.torn_act(id, action.id);
                    true
                }
                None => false,
            }
        });
        let weak = Rc::downgrade(self);
        window.on_close_tab(move || {
            if let Some(app) = weak.upgrade() {
                app.torn_close(id);
            }
        });
        let weak = Rc::downgrade(self);
        window.on_move_back(move || {
            if let Some(app) = weak.upgrade() {
                app.move_back(id);
            }
        });
        let weak = Rc::downgrade(self);
        window.window().on_close_requested(move || {
            let Some(app) = weak.upgrade() else {
                return CloseRequestResponse::HideWindow;
            };
            if app.torn_may_close(id) {
                app.torn_forget(id);
                CloseRequestResponse::HideWindow
            } else {
                CloseRequestResponse::KeepWindowShown
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_act(move |action| {
            if let Some(app) = weak.upgrade() {
                app.torn_act(id, action);
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_edit(move |path| {
            if let Some(app) = weak.upgrade() {
                app.sync_tabs();
                app.back_up_soon(path);
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_caret(move || {
            if let Some(app) = weak.upgrade() {
                app.sync_torn();
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_breakpoint(move |path, line| {
            if let Some(app) = weak.upgrade() {
                app.debug_toggle(path, line);
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_lens(move |runnable, verb| {
            if let Some(app) = weak.upgrade() {
                app.lens(runnable, &verb);
                app.bring_forward();
            }
        });
        let weak = Rc::downgrade(self);
        surface.on_fix(move |version, actions, index| {
            if let Some(app) = weak.upgrade()
                && let Some(surface) = app.editor.view(id)
            {
                app.apply_fix_in(&surface, version, actions, index);
            }
        });
    }

    fn torn_surface(&self, id: usize) -> Option<(Rc<Surface>, String)> {
        self.torn
            .borrow()
            .iter()
            .find(|torn| torn.id() == id)
            .map(|torn| (torn.surface.clone(), torn.path.clone()))
    }

    /// An action asked for in a window of its own: what is about its file
    /// is done there, a few things are refused there and said, and the rest
    /// is the main window's, which comes forward.
    pub(super) fn torn_act(self: &Rc<Self>, id: usize, action: &str) {
        let Some((surface, path)) = self.torn_surface(id) else {
            return;
        };
        match action {
            "tab.close" => self.torn_close(id),
            "file.save" => self.save_file(path, After::Stay),
            // Already a window of its own.
            "editor.detach" => {}
            refused if NOT_HERE.contains(&refused) => {
                let name = actions::by_id(refused)
                    .map(|a| rusty_i18n::lookup(a.title))
                    .unwrap_or_else(|| refused.to_string());
                self.torn_say(id, t!("desk.torn-not-here", name = name));
            }
            _ => {
                *self.keyed.borrow_mut() = Some(surface);
                self.act(action);
                *self.keyed.borrow_mut() = None;
                if !DONE_HERE.contains(&action) && action != "editor.definition" {
                    self.bring_forward();
                }
            }
        }
    }

    /// Something said in a window of its own's status line, for a moment.
    fn torn_say(&self, id: usize, text: String) {
        let torn = self.torn.borrow();
        let Some(torn) = torn.iter().find(|torn| torn.id() == id) else {
            return;
        };
        torn.window.set_message(text.into());
        let window = torn.window.as_weak();
        torn.message
            .start(TimerMode::SingleShot, Duration::from_secs(4), move || {
                if let Some(window) = window.upgrade() {
                    window.set_message("".into());
                }
            });
    }

    /// Whether a window of its own may close: its file's last view with
    /// changes asks — save, throw them away, or keep the window.
    fn torn_may_close(self: &Rc<Self>, id: usize) -> bool {
        let Some((_, path)) = self.torn_surface(id) else {
            return true;
        };
        if !self.editor.is_dirty(&path) || self.views_of(&path) > 1 {
            return true;
        }
        match ask_to_save(&path) {
            Some(true) => {
                self.save_file(path, After::CloseWindow(id));
                false
            }
            Some(false) => {
                if let Some(root) = self.root() {
                    self.keeper.remove(&root, &path);
                }
                true
            }
            None => false,
        }
    }

    /// The window's × on its tab, Ctrl+W there, Vim's `:q`.
    pub(super) fn torn_close(self: &Rc<Self>, id: usize) {
        if self.torn_may_close(id) {
            self.torn_forget(id);
        }
    }

    /// A window of its own goes: hidden now, let go of once the event that
    /// closed it is over, and its file closed when nothing else shows it.
    pub(super) fn torn_forget(self: &Rc<Self>, id: usize) {
        let gone = {
            let mut list = self.torn.borrow_mut();
            let Some(at) = list.iter().position(|torn| torn.id() == id) else {
                return;
            };
            list.remove(at)
        };
        let _ = gone.window.hide();
        let path = gone.path.clone();
        // Not inside the window's own callback, which is what is running.
        Timer::single_shot(Duration::ZERO, move || drop(gone));
        if self.views_of(&path) == 0 {
            self.backup_timers.borrow_mut().remove(&path);
            self.editor.close(&path);
        }
        self.sync_tabs();
        self.strip_changed();
    }

    /// The tab back among the main window's, in the group with the keys,
    /// where its view was scrolled.
    pub(super) fn move_back(self: &Rc<Self>, id: usize) {
        let Some((surface, path)) = self.torn_surface(id) else {
            return;
        };
        let scroll = surface.scroll();
        self.open_tab(Tab::File(path));
        self.torn_forget(id);
        self.editor.set_scroll(scroll);
        self.bring_forward();
    }

    /// Every window of its own closed without a question — the main window
    /// is going, or the project is, and both keep every draft first. The
    /// remembered strip is left as it was, with their files on it, so the
    /// project opened again has them back.
    pub(super) fn torn_all_gone(self: &Rc<Self>) {
        let gone = std::mem::take(&mut *self.torn.borrow_mut());
        let paths: Vec<String> = gone.iter().map(|torn| torn.path.clone()).collect();
        for torn in &gone {
            let _ = torn.window.hide();
        }
        Timer::single_shot(Duration::ZERO, move || drop(gone));
        for path in paths {
            if self.views_of(&path) == 0 {
                self.backup_timers.borrow_mut().remove(&path);
                self.editor.close(&path);
            }
        }
    }

    /// A tab let go of after a drag, at `x`, `y` in the main window: beyond
    /// its edge, it moves into a window of its own there.
    pub(super) fn tab_dropped(self: &Rc<Self>, id: &str, x: f32, y: f32) {
        let Some(main) = self.window() else { return };
        let window = main.window();
        let scale = window.scale_factor();
        let size = window.size();
        let (width, height) = (size.width as f32 / scale, size.height as f32 / scale);
        if (0.0..width).contains(&x) && (0.0..height).contains(&y) {
            return;
        }
        let corner = window.position();
        let at =
            PhysicalPosition::new(corner.x + (x * scale) as i32, corner.y + (y * scale) as i32);
        self.detach(Some(id.to_string()), Some(at));
    }

    /// Each window of its own told what its file is and where its caret
    /// stands: its title, its tab — told from the main window's files of
    /// the same name — and its status line.
    pub(super) fn sync_torn(&self) {
        let torn = self.torn.borrow();
        if torn.is_empty() {
            return;
        }
        let project = self
            .state
            .borrow()
            .project
            .as_ref()
            .map(|project| project.name.clone());
        let mut paths: Vec<String> = {
            let state = self.state.borrow();
            state
                .tabs
                .iter()
                .chain(&state.other)
                .filter_map(|tab| match tab {
                    Tab::File(path) if !rusty_edit::is_expansion(path) => Some(path.clone()),
                    _ => None,
                })
                .collect()
        };
        for each in torn.iter() {
            if !paths.contains(&each.path) {
                paths.push(each.path.clone());
            }
        }
        let hints = strip::tab_hints(&paths);
        for each in torn.iter() {
            let name = each.path.rsplit('/').next().unwrap_or(&each.path);
            let hint = paths
                .iter()
                .position(|path| *path == each.path)
                .and_then(|at| hints.get(at).cloned())
                .unwrap_or_default();
            let caption = match &project {
                Some(project) => format!("{name} — {project} — rusty"),
                None => format!("{name} — rusty"),
            };
            let window = &each.window;
            window.set_caption(caption.into());
            window.set_name(name.into());
            window.set_hint(hint.into());
            window.set_dirty(self.editor.is_dirty(&each.path));
            let caret = each
                .surface
                .status()
                .map(|(line, col, ..)| {
                    t!("desk.caret", line = line.to_string(), col = col.to_string())
                })
                .unwrap_or_default();
            window.set_caret(caret.into());
            let mode = each
                .surface
                .vim_status()
                .map(|(mode, _)| mode)
                .unwrap_or_default();
            window.set_mode(mode.into());
        }
    }

    /// The theme and zoom the main window has, on every window of its own.
    pub(super) fn sync_torn_settings(&self) {
        let torn = self.torn.borrow();
        for each in torn.iter() {
            self.dress_torn(each);
        }
    }

    /// The main window in front: an action from a window of its own landed
    /// there — the palette, a jump to another file, a list of places.
    pub(super) fn bring_forward(&self) {
        if let Some(main) = self.window() {
            bring_to_front(&main);
        }
    }
}

/// A window to the front, with the keys. Slint has no word for it but
/// winit's, behind its `unstable-winit-030` feature: the one place rusty
/// reaches past Slint's own API.
fn bring_to_front(window: &AppWindow) {
    use slint::winit_030::WinitWindowAccessor;
    window.window().with_winit_window(|winit| {
        winit.set_minimized(false);
        winit.focus_window();
    });
}
