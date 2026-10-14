//! The app: the window, what it holds, and what it does when asked.
//!
//! State lives here and in the parts it owns (the editor's documents); the
//! `.slint` files draw it. Every change goes through a method that changes
//! the state and then pushes what it changed into the `Shell` global — one
//! `sync_*` per region, so what is on screen is always a function of the
//! state rather than of whichever callback ran last.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use rusty_edit::text_file::TextFile;
use rusty_embed::WindowLayout;
use rusty_i18n::{Locale, t};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, VecModel};

mod activity;
mod assistant;
mod board;
mod build;
mod crates;
mod debug;
mod device;
mod drafts;
mod draw;
mod environment;
mod flight;
mod git;
mod git_view;
mod lab;
mod lab_words;
mod markdown_view;
mod math;
mod math_words;
mod memory;
mod menu;
mod navigate;
mod part;
mod playground;
mod plot;
mod registers;
mod saving;
mod search;
mod servers;
mod sheet;
mod sim;
mod terminal;
mod waves;
mod wizard;

use saving::After;

use crate::{
    AppWindow, DocTab, Icons, Note, PanelTab, PickRow, Recent, Shell, TreeRow, actions,
    docks::{Dock, Glyph},
    editor::{
        Document, Surface,
        backup::{self, Keeper},
    },
    keys, language,
    lsp::{Problems, Servers},
    picker::{self, Mode, Pick},
    project::Project,
    run::{Ended, Runner},
    work,
};

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
            Tab::Page("environment") => rusty_i18n::lookup("panel.toolchain"),
            Tab::Page("wizard") => t!("wizard.title"),
            Tab::Page("git") => t!("git.history"),
            Tab::Page("gitdiff") => t!("git.changes"),
            Tab::Page("memory") => t!("panel.memory"),
            Tab::Page("crates") => t!("panel.crates"),
            Tab::Page("math") => t!("panel.math"),
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
    action: Option<NoteAction>,
}

/// What a note's button does.
#[derive(Clone, Debug)]
pub enum NoteAction {
    /// Run a command line into the Output dock: a problem's fix.
    Run(String),
    /// An action of the window's, by its id.
    Act(&'static str),
    /// Install these tools, in turn, as the Environment page would.
    Install(Vec<String>),
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
    server: servers::ServerStatus,
    jumps: navigate::Jumps,
    /// The fixes the picker lists, with the version of the text they are for.
    fixes: Option<(u64, rusty_lsp::CodeActions)>,
    /// A rename being typed: where the symbol is, and its name now.
    renaming: Option<(String, u32, u32, String)>,
    /// What the picker lists when a list was handed to it.
    listed: Vec<(String, String, Pick)>,
    environment: environment::Environment,
    wizard: wizard::Wizard,
    activities: activity::Activities,
    build_log: build::BuildLog,
    devices: device::Devices,
    sim: sim::Sim,
    draw: draw::Drawings,
    flight: flight::Flight,
    lab: lab::Lab,
    debug: debug::Debug,
    registers: registers::Registers,
    search: search::Search,
    git: git::Git,
    menu: Option<&'static str>,
    memory: memory::Memory,
    crates: crates::Crates,
    assistant: assistant::Assistant,
    math: math::Math,
    /// Markdown files shown as their source rather than as a page.
    markdown_source: HashSet<String>,
    term: terminal::Term,
}

pub struct App {
    window: slint::Weak<AppWindow>,
    /// The grammars, which the editor paints with and a page's code blocks
    /// are painted with too.
    files: Arc<rusty_edit::Files>,
    editor: Rc<Surface>,
    state: RefCell<State>,
    keeper: Keeper,
    /// A timer per draft, restarted by every edit, that backs it up.
    backup_timers: RefCell<HashMap<String, Timer>>,
    /// A backup that failed is said once, not once a second.
    backup_warned: Cell<bool>,
    /// The language servers, and what they found in every file.
    servers: Rc<Servers>,
    problems: Problems,
    problems_timer: Timer,
    /// What has been said once this session, and is not said again.
    noted: RefCell<HashSet<String>>,
    /// What the window runs, one at a time, into the Output dock.
    runner: Runner,
    /// The board beside the code; apart from `state`, since the protocol
    /// reader holds `state` while it tells the board the pins moved.
    board: RefCell<board::Board>,
    /// The board's live layer, changed row by row.
    board_models: board::Models,
    /// A Plot redraw is on its way.
    plot_due: Cell<bool>,
    /// A Flight redraw is on its way.
    flight_due: Cell<bool>,
    /// Steps the plant while the loop is closed.
    plant_clock: slint::Timer,
    /// When the window started: the activity view's clock counts from it.
    started: std::time::Instant,
    /// Ticks while something runs, so the view's clock moves.
    clock: slint::Timer,
}

impl App {
    pub fn new(window: &AppWindow) -> Rc<App> {
        let saved = rusty_embed::config::workbench();
        let files = Arc::new(rusty_edit::Files::new());
        let output = Rc::new(VecModel::<crate::OutputLine>::default());
        window
            .global::<Shell>()
            .set_output(ModelRc::from(output.clone()));
        let runner = Runner::new(output);
        let servers = Servers::new();
        let problems = Problems::default();
        let editor = Surface::new(window, files.clone(), servers.clone(), problems.clone());
        let keeper = Keeper::start(backup::directory(), |reason| {
            work::deliver(reason, |app, reason| app.backup_failed(reason))
        });
        let app = Rc::new(App {
            window: window.as_weak(),
            files,
            editor,
            keeper,
            backup_timers: RefCell::default(),
            backup_warned: Cell::new(false),
            servers,
            problems,
            problems_timer: Timer::default(),
            noted: RefCell::default(),
            runner,
            board: RefCell::default(),
            board_models: board::Models::new(window),
            plot_due: Cell::new(false),
            flight_due: Cell::new(false),
            plant_clock: slint::Timer::default(),
            started: std::time::Instant::now(),
            clock: slint::Timer::default(),
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

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    /// Run `plan` in `dir` into the Output dock, which comes forward, and
    /// hand how it ended to `then`. Refused, and said, while another run
    /// is going.
    pub fn run(
        self: &Rc<Self>,
        plan: &rusty_embed::CommandPlan,
        dir: Option<PathBuf>,
        then: impl FnOnce(&Rc<App>, Ended) + Send + 'static,
    ) -> bool {
        self.show_panel("bottom", "output");
        let began = std::time::Instant::now();
        let started = self.runner.run(plan, dir, move |app, ended| {
            app.said_end(began, ended);
            then(app, ended);
        });
        if !started {
            self.note_transient(1, t!("desk.already-running"));
        }
        self.sync_running();
        started
    }

    /// A session opened some other way — rusty's serial link — run as
    /// [`App::run`] runs a plan, typed into from the dock.
    pub fn run_session(
        self: &Rc<Self>,
        display: &str,
        session: rusty_embed::process::Session,
        then: impl FnOnce(&Rc<App>, Ended) + Send + 'static,
    ) -> bool {
        self.show_panel("bottom", "output");
        let began = std::time::Instant::now();
        let started = self
            .runner
            .run_session(display, session, move |app, ended| {
                app.said_end(began, ended);
                then(app, ended);
            });
        if !started {
            self.note_transient(1, t!("desk.already-running"));
        }
        self.sync_running();
        started
    }

    /// How a run ended, said in the dock.
    fn said_end(self: &Rc<Self>, began: std::time::Instant, ended: Ended) {
        self.drawing_ends();
        let seconds = format!("{:.1}", began.elapsed().as_secs_f32());
        match ended {
            Ended::Done => self.runner.say(
                &t!("desk.run-done", seconds = seconds),
                crate::run::kind::DONE,
            ),
            Ended::Failed(code) => self.runner.say(
                &t!(
                    "desk.run-failed",
                    code = code.map_or_else(|| "?".to_string(), |c| c.to_string())
                ),
                crate::run::kind::ERROR,
            ),
            Ended::Stopped => self
                .runner
                .say(&t!("desk.run-stopped"), crate::run::kind::PLAIN),
            Ended::Refused => {}
        }
        self.sync_running();
    }

    /// A line typed in the dock, to what runs: shown, then sent.
    fn output_send(self: &Rc<Self>, text: &str) {
        self.runner
            .say(&format!("> {text}"), crate::run::kind::COMMAND);
        self.runner.send(text);
    }

    fn sync_running(&self) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_running(self.runner.is_running());
            shell.set_output_input(self.runner.takes_input());
        }
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
        on!(on_problem_click, |app, index| app
            .problem_click(index as usize));
        on!(on_note_action, |app, id| app.note_action(id));
        on!(on_env_install, |app, name, command| app
            .env_install(&command, &name));
        on!(on_open_playground, |app, id| app.open_playground(&id));
        on!(on_output_click, |app, index| app
            .output_click(index as usize));
        on!(on_output_send, |app, text| app.output_send(&text));
        on!(on_plot_solo, |app, name| app.plot_solo(&name));
        on!(on_waves_resized, |app, width| app.waves_resized(width));
        on!(on_draw_resized, |app, w, h| app.draw_resized(w, h));
        on!(on_lab_pick_source, |app, index| app.lab_pick_source(index));
        on!(on_debug_frame, |app, level| app.debug_frame(level));
        on!(on_registers_pick, |app, name| app.registers_pick(&name));
        on!(on_search_edited, |app, what, text| app
            .search_edited(&what, &text));
        on!(on_search_open, |app, index| app.search_open(index));
        on!(on_term_resized, |app, cols, rows| app
            .term_resized(cols, rows));
        on!(on_term_scroll, |app, lines| app.term_scroll(lines));
        on!(on_git_file, |app, verb, path, staged| app.git_file(
            &verb,
            path.to_string(),
            staged
        ));
        on!(on_git_file_menu, |app, path, staged, x, y| app
            .git_change_menu(&path, staged, x, y));
        on!(on_git_identity_save, |app, name, email, local| app
            .git_identity_save(
                name.to_string(),
                email.to_string(),
                local
            ));
        on!(on_git_stash, |app, verb, index| app.git_stash(&verb, index));
        on!(on_git_search, |app, text| app.git_search(&text));
        on!(on_git_search_step, |app, forward| app
            .git_search_step(forward));
        on!(on_git_ref_filter, |app, text| app.git_ref_filter(&text));
        on!(on_git_ref, |app, index, double| app
            .git_ref(index.max(0) as usize, double));
        on!(on_git_ref_menu, |app, index, x, y| app
            .git_ref_menu(index, x, y));
        on!(on_git_pick, |app, index| app
            .git_pick(index.max(0) as usize));
        on!(on_git_step, |app, step| app.git_step(step));
        on!(on_git_commit_menu, |app, index, x, y| app
            .git_commit_menu(index, x, y));
        on!(on_git_detail_file, |app, index| app
            .git_detail_file(index.max(0) as usize));
        on!(on_git_detail_file_menu, |app, index, x, y| app
            .git_file_menu(index, x, y));
        on!(on_git_prompt_edited, |app| app.git_prompt_edited());
        on!(on_git_prompt_done, |app, ok| app.git_prompt_done(ok));
        on!(on_menu_picked, |app, id| app.menu_picked(&id));
        on!(on_mem_pick, |app, path| app.memory_pick(&path));
        on!(on_crates_upgrade, |app, name, version| app
            .crates_upgrade(&name, &version));
        on!(on_feat_pick, |app, package| app.features_pick(&package));
        on!(on_feat_toggle, |app, feature| app.features_toggle(&feature));
        on!(on_disk_tree, |app, verb, index| app.disk_tree(&verb, index));
        on!(on_disk_item, |app, index, cache| app
            .disk_item(index, cache));
        on!(on_disk_idle_set, |app, days| app.disk_idle(days));
        on!(on_disk_auto_set, |app, on| app.disk_auto(on));
        on!(on_copy_text, |_app, text| git::copy_text(&text));
        on!(on_ai_fold, |app, at| app.ai_fold(at));
        on!(on_ai_preset, |app, index| app.ai_preset(index));
        on!(on_ai_pick_model, |app, model| app.ai_pick_model(&model));
        on!(on_open_link, |_app, link| markdown_view::open_link(&link));
        on!(on_math_select, |app, index| app.math_select(index));
        on!(on_math_toggle, |app, index| app.math_toggle(index));
        on!(on_math_row, |app, index, text| app.math_row(index, &text));
        on!(on_math_remove, |app, index| app.math_remove(index));
        on!(on_math_example, |app, index| app.math_example(index));
        on!(on_math_slider, |app, index, value| app
            .math_slider(index, value));
        on!(on_math_step, |app, index| app.math_step(index));
        on!(on_math_scrub, |app, at| app.math_scrub(at));
        on!(on_math_resized, |app, w, h| app.math_resized(w, h));
        on!(on_math_orbit, |app, dx, dy| app.math_orbit(dx, dy));
        on!(on_math_zoom, |app, delta| app.math_zoom(delta));
        let weak = Rc::downgrade(self);
        shell.on_math_row_key(move |index, key| {
            weak.upgrade()
                .is_some_and(|app| app.math_row_key(index, &key))
        });
        on!(on_menu_dismissed, |app| app.menu_dismissed());
        let weak = Rc::downgrade(self);
        shell.on_term_key(move |event| {
            weak.upgrade().is_some_and(|app| {
                let m = event.modifiers;
                app.term_key(&event.text, m.control, m.alt, m.shift)
            })
        });
        on!(on_registers_toggle, |app, name| app.registers_toggle(&name));
        on!(on_lab_source_text, |app, index, text| app
            .lab_source_text(index, &text));
        on!(on_lab_preset, |app, index, preset| app
            .lab_preset(index, preset));
        on!(on_lab_console_text, |app, name, axis, text| app
            .lab_console_text(&name, axis, &text));
        on!(on_lab_set, |app, what, index| app.lab_set(&what, index));
        on!(on_lab_pick_record, |app, what, index| app
            .lab_pick_record(&what, index));
        on!(on_lab_field, |app, what, index, text| app
            .lab_field(&what, index, &text));
        on!(on_draw_orbit, |app, dx, dy| app.draw_orbit(dx, dy));
        on!(on_draw_zoom, |app, delta| app.draw_zoom(delta));
        on!(on_draw_pick, |app, title| app.draw_pick(&title));
        on!(on_draw_hover, |app, index| app.draw_hover(index));
        on!(on_waves_grab, |app| app.waves_grab());
        on!(on_waves_drag, |app, by| app.waves_drag(by));
        on!(on_waves_zoom, |app, at, factor| app.waves_zoom(at, factor));
        let weak = Rc::downgrade(self);
        shell.on_waves_time_at(move |at| {
            weak.upgrade()
                .map(|app| app.waves_time_at(at))
                .unwrap_or_default()
                .into()
        });
        on!(on_plot_set, |app, name, value| app.plot_set(&name, value));
        on!(on_board_press, |app, index, key, down| app.board_press(
            index as usize,
            key,
            down
        ));
        on!(on_board_control, |app, index, key, value| app
            .board_control(index as usize, &key, value));
        on!(on_wiz_go, |app, step| app.wizard_go(step));
        on!(on_wiz_filter, |app, query| app.wizard_filter(&query));
        on!(on_wiz_pick, |app, id| app.wizard_pick(&id));
        on!(on_wiz_hover, |app, id| app.wizard_hover(&id));
        on!(on_wiz_name_edited, |app, name| app.wizard_name(&name));
        on!(on_wiz_set_layout, |app, layout| app.wizard_layout(&layout));
        on!(on_set_theme, |app, theme| app.set_theme(&theme));
        on!(on_set_language, |app, tag| app.set_language(&tag));
        on!(on_set_zoom, |app, zoom| app.set_zoom(zoom));
        let weak = Rc::downgrade(self);
        shell.on_board_key(move |event| {
            weak.upgrade().is_some_and(|app| {
                app.board_key(&event.text, event.modifiers.control, event.modifiers.shift)
            })
        });
        let weak = Rc::downgrade(self);
        shell.on_board_down(move |x, y, shift| {
            weak.upgrade()
                .is_some_and(|app| app.board_down(x, y, shift))
        });
        on!(on_board_drag, |app, x, y| app.board_drag(x, y));
        on!(on_board_up, |app, x, y| app.board_up(x, y));
        on!(on_board_hover, |app, x, y| app.board_hover(x, y));
        on!(on_board_rename, |app, name| app.board_rename(&name));
        on!(on_board_revalue, |app, value| app.board_revalue(&value));
        on!(on_board_reprop, |app, key, value| app
            .board_reprop(&key, &value));
        on!(on_board_library_query, |app, query| app
            .board_library_query(&query));
        on!(on_board_add, |app, id| app.board_add(&id));
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
        self.editor.on_breakpoint(move |path, line| {
            if let Some(app) = weak.upgrade() {
                app.debug_toggle(path, line);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_edit(move |path| {
            if let Some(app) = weak.upgrade() {
                app.sync_tabs();
                app.sync_markdown();
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
            "environment.open" => self.open_tab(Tab::Page("environment")),
            "environment.refresh" => self.probe_environment(),
            "environment.install-missing" => self.install_missing(),
            "project.new" => self.open_wizard(),
            "playground.reset" => self.reset_playground(),
            "playground.keep" => self.keep_playground(),
            "wizard.create" => self.wizard_create(),
            "wizard.install" => self.wizard_install(),
            "welcome.open" => self.open_tab(Tab::Page("welcome")),
            "file.save" => self.save(),
            "find.open" => self.editor.find_open(false),
            "find.replace" => self.editor.find_open(true),
            "find.next" => self.editor.find_next(true),
            "find.previous" => self.editor.find_next(false),
            "editor.go-to-line" => self.open_picker_with(Mode::Files, ":"),
            "editor.show-hover" => self.editor.hover_caret(),
            "editor.definition" => self.go_to_definition(),
            "editor.references" => self.find_places(false),
            "editor.implementations" => self.find_places(true),
            "editor.quick-fix" => self.quick_fix(),
            "editor.rename" => self.rename_prompt(),
            "nav.back" => self.walk(true),
            "nav.forward" => self.walk(false),
            "problem.next" => self.next_problem(true),
            "problem.previous" => self.next_problem(false),
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
            "run.stop" => {
                // Debugging stops with the run that booted the target.
                if self.state.borrow().debug.live() {
                    self.debug_stop();
                }
                self.runner.stop();
            }
            "run.debug" => self.debug_verb(),
            "run.build" => self.build(Box::new(|_, _| {})),
            "run.run" => self.run_verb(),
            "run.flash" => self.device_action(device::Verb::Flash),
            "run.monitor" => self.device_action(device::Verb::Monitor),
            "run.choose-target" => self.choose_target(None),
            "run.rescan" => {
                let pending = self.state.borrow().devices.pending;
                self.choose_target(pending);
            }
            "output.show" => self.show_panel("bottom", "output"),
            id if id.starts_with("board.") => self.board_act(id),
            "waves.follow" => self.waves_follow(),
            "waves.fit" => self.waves_fit(),
            "waves.export" => self.waves_export(),
            "draw.clear" => self.draw_clear(),
            "draw.iso" => self.draw_look(Some(rusty_embed::scene::Look::Iso)),
            "draw.top" => self.draw_look(Some(rusty_embed::scene::Look::Top)),
            "draw.front" => self.draw_look(Some(rusty_embed::scene::Look::Front)),
            "draw.side" => self.draw_look(Some(rusty_embed::scene::Look::Side)),
            "draw.fit" => self.draw_look(None),
            "flight.close" => self.flight_close(true),
            "flight.open" => self.flight_close(false),
            "registers.fetch" => self.fetch_svd(),
            "terminal.restart" => self.term_start(),
            "terminal.ensure" => self.term_ensure(),
            "terminal.redraw" => self.sync_term(),
            "debug.pause" => self.debug_control("pause"),
            "debug.over" => self.debug_control("over"),
            "debug.into" => self.debug_control("into"),
            "debug.out" => self.debug_control("out"),
            "lab.sweep-start" => self.lab_sweep_start(),
            "lab.sweep-stop" => self.lab_sweep_stop(),
            "lab.measure-recorded" => self.lab_measure_recorded(),
            "output.clear" => self.runner.clear(),
            "git.branches" => {
                self.show_panel("left", "git");
                self.git_shown();
            }
            other if other.starts_with("git.") => self.git_act(&other["git.".len()..]),
            other if other.starts_with("memory.") => self.memory_act(&other["memory.".len()..]),
            other if other.starts_with("crates.") => self.crates_act(&other["crates.".len()..]),
            other if other.starts_with("ai.") => self.ai_act(&other["ai.".len()..]),
            "markdown.toggle" => self.markdown_toggle(),
            other if other.starts_with("math.") => self.math_act(&other["math.".len()..]),
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
        // A playground has its own doors; the recents are the projects
        // somebody works on.
        let playground = project
            .embedded
            .as_ref()
            .is_ok_and(|found| found.playground.is_some());
        if !playground {
            rusty_embed::config::record_recent(&root);
        }
        {
            let mut state = self.state.borrow_mut();
            for tab in std::mem::take(&mut state.tabs) {
                if let Tab::File(path) = tab {
                    self.editor.close(&path);
                }
            }
            state.tabs = vec![Tab::Page("welcome")];
            state.active = Some(0);
            if !playground {
                state.recents.retain(|path| path != &root);
                state.recents.insert(0, root);
            }
            state.project = Some(project);
            if !state.layout.left.open {
                state.layout.left.open = true;
            }
        }
        self.sync_all();
        self.sync_problems();
        self.start_servers();
        self.probe_environment();
        self.load_board(None);
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
        self.open_file_at(path, None);
    }

    /// Open a file and put the caret at a line and column, counted from
    /// zero, when one is given.
    pub fn open_file_at(self: &Rc<Self>, path: String, at: Option<(usize, usize)>) {
        let go = move |app: &Rc<App>| {
            if let Some((line, col)) = at {
                app.editor.go_to(line + 1, Some(col + 1));
            }
        };
        if self.editor.holds(&path) {
            self.open_tab(Tab::File(path));
            go(self);
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
            move |app, (path, read)| {
                // Opened meanwhile — a draft brought back from last time is
                // the case — and the document that is open is the one to
                // keep: read again it would replace the draft with the disk.
                if app.editor.holds(&path) {
                    app.open_tab(Tab::File(path));
                    go(app);
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
                        let document = Document::new(path.clone(), text, charset, ending);
                        app.editor.show_new(document);
                        app.open_tab(Tab::File(path));
                        go(app);
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

    /// The open project's directory, for the probe.
    pub fn root_path(&self) -> Option<PathBuf> {
        self.root()
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
        self.sync_markdown();
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
        self.open_picker_titled(mode, text, None);
    }

    /// The picker listing `rows` under `heading`, to choose one from.
    pub(super) fn open_list(
        self: &Rc<Self>,
        mode: Mode,
        heading: String,
        rows: Vec<(String, String, Pick)>,
    ) {
        self.state.borrow_mut().listed = rows;
        self.open_picker_titled(mode, "", Some(heading));
    }

    fn open_picker_titled(self: &Rc<Self>, mode: Mode, text: &str, heading: Option<String>) {
        self.state.borrow_mut().picker = Some((mode, Vec::new()));
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_picker_text(text.into());
            // A name to replace is selected whole, as VS Code's rename box
            // selects it; anything else is typed after.
            shell.set_picker_select_all(mode == Mode::Rename);
            let hint = heading.unwrap_or_else(|| match mode {
                Mode::Commands => t!("desk.palette-hint"),
                Mode::Files => t!("desk.finder-hint"),
                Mode::Projects => t!("desk.projects-hint"),
                Mode::Rename => t!("desk.rename-hint"),
                Mode::Places | Mode::Fixes | Mode::Targets => String::new(),
            });
            shell.set_picker_hint(SharedString::from(hint));
            shell.set_picker_open(true);
            shell.set_picker_opened(shell.get_picker_opened() + 1);
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
            Mode::Places | Mode::Fixes | Mode::Targets => {
                let state = self.state.borrow();
                let titles: Vec<String> = state
                    .listed
                    .iter()
                    .map(|(title, detail, _)| format!("{title} {detail}"))
                    .collect();
                picker::rank(query, &titles)
                    .into_iter()
                    .map(|at| state.listed[at].clone())
                    .collect()
            }
            Mode::Rename => self.rename_rows(query),
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
            Some(Pick::Place(location)) => self.jump(&location),
            // The keys back in the code the fix or the rename changes.
            Some(Pick::Fix(index)) => {
                self.front_focus();
                self.apply_fix(index);
            }
            Some(Pick::Rename(name)) => {
                self.front_focus();
                self.rename_to(name);
            }
            Some(Pick::Target(index)) => {
                self.front_focus();
                self.pick_target(index);
            }
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
        self.note_for(level, text, level == 0);
    }

    /// A note that goes by itself whatever its level: an answer to a key,
    /// not news that waits to be read.
    pub fn note_transient(self: &Rc<Self>, level: i32, text: String) {
        self.note_for(level, text, true);
    }

    /// A note said once this session: a missing tool is the same sentence
    /// at every save.
    pub fn note_once(self: &Rc<Self>, level: i32, text: String) {
        self.note_once_with(level, text, None);
    }

    pub fn note_once_with(self: &Rc<Self>, level: i32, text: String, action: Option<NoteAction>) {
        if self.noted.borrow_mut().insert(text.clone()) {
            self.note_with(level, text, action);
        }
    }

    /// A note with a button that does something.
    pub fn note_with(self: &Rc<Self>, level: i32, text: String, action: Option<NoteAction>) {
        self.note_full(level, text, level == 0, action);
    }

    fn note_for(self: &Rc<Self>, level: i32, text: String, transient: bool) {
        self.note_full(level, text, transient, None);
    }

    fn note_full(
        self: &Rc<Self>,
        level: i32,
        text: String,
        transient: bool,
        action: Option<NoteAction>,
    ) {
        let id = {
            let mut state = self.state.borrow_mut();
            state.next_note += 1;
            let id = state.next_note;
            state.notes.push(NoteItem {
                id,
                level,
                text,
                action,
            });
            if state.notes.len() > 4 {
                state.notes.remove(0);
            }
            id
        };
        self.sync_notes();
        if transient {
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

    /// A note's button: what it says it does, and the note goes.
    fn note_action(self: &Rc<Self>, id: i32) {
        let action = {
            let state = self.state.borrow();
            state
                .notes
                .iter()
                .find(|n| n.id == id)
                .and_then(|n| n.action.clone())
        };
        self.dismiss_note(id);
        match action {
            Some(NoteAction::Run(line)) => {
                let mut words = line.split_whitespace();
                let Some(program) = words.next() else { return };
                let plan = rusty_embed::CommandPlan::new(
                    program,
                    words.map(str::to_string).collect(),
                    String::new(),
                );
                let dir = self.root();
                self.run(&plan, dir, |app, _| app.reread_project());
            }
            Some(NoteAction::Act(id)) => self.act(id),
            Some(NoteAction::Install(names)) => self.install_tools(names),
            None => {}
        }
    }

    // ---- pushing the state into the window -------------------------------

    fn sync_all(&self) {
        self.sync_project();
        self.sync_part();
        self.sync_playground();
        self.sync_activity();
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
                Glyph::Plot => icons.get_plot(),
                Glyph::Waves => icons.get_waves(),
                Glyph::Draw => icons.get_draw(),
                Glyph::Flight => icons.get_flight(),
                Glyph::Signals => icons.get_signals(),
                Glyph::Debug => icons.get_bug(),
                Glyph::Registers => icons.get_chip(),
            }
        };
        let state = self.state.borrow();
        let has_content = |id: &str| match id {
            "plot" => !state.sim.plot.channels.is_empty() || !state.sim.params.is_empty(),
            "waves" => !state.sim.trace.events.is_empty(),
            "draw" => !state.draw.is_empty(),
            "flight" => !state.sim.sensors.is_empty(),
            "debug" | "registers" => state.debug.state.is_some(),
            "signals" => {
                state.lab.has_signals()
                    || state
                        .sim
                        .plan
                        .as_ref()
                        .and_then(|plan| plan.board.as_ref())
                        .is_some_and(|sheet| !rusty_embed::lab::sources_of(sheet).is_empty())
            }
            _ => true,
        };
        let panels = |dock: Dock| -> ModelRc<PanelTab> {
            let front = dock.front(&state.layout);
            let tabs: Vec<PanelTab> = dock
                .panels()
                .iter()
                .filter(|(id, ..)| crate::docks::listed(id, has_content, front))
                .map(|(id, title, glyph)| PanelTab {
                    id: (*id).into(),
                    title: rusty_i18n::lookup(title).into(),
                    icon: icon(*glyph),
                })
                .collect();
            ModelRc::new(VecModel::from(tabs))
        };
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
            Some(Tab::File(path)) => language::of(&path).map_or("Plain text", |l| l.name),
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
                action: match &note.action {
                    Some(NoteAction::Run(_)) => t!("desk.note-run-fix"),
                    Some(NoteAction::Install(_)) => t!("toolchain.install"),
                    Some(NoteAction::Act(id)) => actions::by_id(id)
                        .map(|a| rusty_i18n::lookup(a.title))
                        .unwrap_or_default(),
                    None => String::new(),
                }
                .into(),
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
