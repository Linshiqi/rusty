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
mod calls;
mod clone;
mod context;
mod crates;
mod debug;
mod device;
mod drafts;
mod draw;
mod environment;
mod expansion;
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
mod new_window;
mod output;
mod part;
mod pictures;
mod playground;
mod plot;
mod registers;
mod saving;
mod search;
mod servers;
mod sheet;
mod shortcuts;
mod sim;
mod strip;
mod symbols;
mod tab_drag;
mod tab_menu;
mod terminal;
mod testing;
mod torn;
mod tree;
mod unlinked;
mod updating;
mod watch;
mod waves;
mod wizard;

use saving::After;

use crate::{
    AppWindow, Icons, Note, PanelTab, PickRow, Recent, Shell, TreeRow, actions,
    docks::{Dock, Glyph},
    editor::{
        Document, Surface,
        backup::{self, Keeper},
    },
    language,
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
    /// A picture file of the project, drawn: never a document, so never
    /// text in the editor and never a language server's.
    Picture(String),
}

impl Tab {
    /// The tab a project file opens as: a picture, drawn, or a file the
    /// editor holds — an SVG is the second, with its picture over it.
    fn of(path: String) -> Tab {
        if crate::picture::is_picture(&path) && !crate::picture::is_svg(&path) {
            Tab::Picture(path)
        } else {
            Tab::File(path)
        }
    }

    fn id(&self) -> String {
        match self {
            Tab::File(path) => format!("file:{path}"),
            Tab::Page(kind) => format!("page:{kind}"),
            Tab::Picture(path) => format!("picture:{path}"),
        }
    }

    fn title(&self) -> String {
        match self {
            Tab::File(path) | Tab::Picture(path) => {
                path.rsplit('/').next().unwrap_or(path).to_string()
            }
            Tab::Page("welcome") => t!("desk.welcome"),
            Tab::Page("settings") => t!("desk.settings"),
            Tab::Page("environment") => rusty_i18n::lookup("panel.toolchain"),
            Tab::Page("wizard") => t!("wizard.title"),
            Tab::Page("git") => t!("git.history"),
            Tab::Page("gitdiff") => t!("git.changes"),
            Tab::Page("memory") => t!("panel.memory"),
            Tab::Page("crates") => t!("panel.crates"),
            Tab::Page("math") => t!("panel.math"),
            Tab::Page("keys") => t!("shortcuts.title"),
            Tab::Page(kind) => (*kind).to_string(),
        }
    }

    fn kind(&self) -> &str {
        match self {
            Tab::File(_) => "file",
            Tab::Page(kind) => kind,
            Tab::Picture(_) => "picture",
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
    /// What the editor draws around the code (Settings ▸ Editor).
    view: rusty_embed::EditorView,
    /// Vim's keys in the editor.
    vim: bool,
    /// Where updating rusty itself stands.
    update: updating::Stage,
    /// Split: the other editor group's tabs and its front one — `tabs` and
    /// `active` are always the group with the keys — and which side that
    /// group is on, 0 left and 1 right.
    split: bool,
    other: Vec<Tab>,
    other_active: Option<usize>,
    focus: usize,
    project: Option<Project>,
    recents: Vec<String>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    picker: Option<(Mode, Vec<Pick>)>,
    notes: Vec<NoteItem>,
    next_note: i32,
    server: servers::ServerStatus,
    jumps: navigate::Jumps,
    /// The call hierarchy the Calls panel shows.
    calls: calls::Calls,
    /// The finder's `@` and `#`: the outline asked for, the workspace
    /// searched, the view as it was before a symbol was shown in it.
    symbols: symbols::Symbols,
    /// The fixes the picker lists, with the version of the text they are for.
    /// And which surface asked for them, which applies the one picked.
    fixes: Option<(u64, rusty_lsp::CodeActions, usize)>,
    /// A rename being typed: where the symbol is, and its name now.
    renaming: Option<(String, u32, u32, String)>,
    /// What the picker lists when a list was handed to it.
    listed: Vec<(String, String, Pick)>,
    environment: environment::Environment,
    wizard: wizard::Wizard,
    activities: activity::Activities,
    build_log: build::BuildLog,
    /// What the Problems panel's filter box and switches say; Output's is
    /// the runner's, beside the lines it narrows.
    problems_filter: crate::filter::Filter,
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
    /// What a surface's menu was opened on, and the wheel's travel.
    context: context::Context,
    /// The tab a right-click menu is open for: whether it is the other
    /// group's, and its id.
    tab_menu: Option<(bool, String)>,
    memory: memory::Memory,
    crates: crates::Crates,
    assistant: assistant::Assistant,
    math: math::Math,
    /// Markdown files (and SVGs) shown as their source rather than as a
    /// page.
    markdown_source: HashSet<String>,
    /// The Markdown page in front: what it was read as, its formulas and
    /// the widths its paragraphs were laid out for.
    page: markdown_view::Page,
    /// Picture files read, for the picture page and the pages naming them.
    pictures: pictures::Pictures,
    watching: watch::Watching,
    testing: testing::Testing,
    term: terminal::Term,
    /// The keys in force, a chord half pressed, and the shortcuts page.
    keys: shortcuts::Keys,
    /// The project tree's selection, clipboard, naming box and drag.
    tree: tree::Tree,
    /// The Rust files no `mod` declares, drawn dimmed.
    unlinked: unlinked::Unlinked,
    /// A tab being dragged (`tab_drag.rs`).
    tab_drag: Option<tab_drag::Drag>,
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
    /// The project's remembered strip has been put back, and changes to
    /// the strip are written from now on (`strip.rs`).
    strip_back: Cell<bool>,
    /// The strip as it was last written.
    strip_saved: RefCell<Option<rusty_embed::model::ProjectTabs>>,
    /// The windows tabs were moved into (`torn.rs`).
    torn: RefCell<Vec<torn::Torn>>,
    /// The surface of the window of its own an action was asked for in,
    /// while the action runs (`App::keyed`).
    keyed: RefCell<Option<Rc<Surface>>>,
    /// The project tree's rows, changed in place (`tree::patch`).
    tree_rows: Rc<VecModel<TreeRow>>,
    /// The editor groups' strips, changed in place, and what their tabs'
    /// words measure (`tab_drag.rs`).
    strips: tab_drag::Strips,
}

impl App {
    pub fn new(window: &AppWindow) -> Rc<App> {
        let saved = rusty_embed::config::workbench();
        let files = Arc::new(rusty_edit::Files::new());
        let runner = Runner::new();
        window.global::<Shell>().set_output(runner.view());
        let tree_rows = Rc::new(VecModel::<TreeRow>::default());
        window
            .global::<Shell>()
            .set_tree(ModelRc::from(tree_rows.clone()));
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
            strip_back: Cell::new(false),
            strip_saved: RefCell::default(),
            torn: RefCell::default(),
            keyed: RefCell::default(),
            tree_rows,
            strips: tab_drag::Strips::new(window),
            state: RefCell::new(State {
                layout: saved.layout.clone(),
                view: saved.editor,
                vim: saved.vim,
                recents: saved.recent_projects.clone(),
                keys: shortcuts::Keys::new(&saved.keybinds),
                tabs: vec![Tab::Page("welcome")],
                active: Some(0),
                ..State::default()
            }),
        });
        app.wire(window);
        let weak = Rc::downgrade(&app);
        app.runner.on_counts(move |shown, total| {
            if let Some(app) = weak.upgrade() {
                app.sync_output_filter((shown, total));
            }
        });
        app.picture_ground();
        app.editor.set_view(saved.editor);
        app.editor.set_vim(saved.vim);
        app.sync_all();
        app.update_soon();
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
        on!(on_tab_dragged, |app, id, other, begun, x, y| app
            .tab_dragged(&id, other, begun, x, y));
        on!(on_tab_dropped, |app, id, other, x, y| app
            .tab_dropped(&id, other, x, y));
        on!(on_tab_drag_again, |app| app.tab_drag_again());
        on!(on_tab_drag_cancel, |app| app.tab_drag_cancel());
        on!(on_shade_pick, |app, id| app.shade_pick(&id));
        on!(on_shade_close, |app, id| {
            app.close_tab_in(true, &id);
        });
        on!(on_tab_menu, |app, id, other, x, y| app
            .tab_menu(&id, other, x, y));
        on!(on_split_resized, |app, share| app.split_resized(share));
        let weak = Rc::downgrade(self);
        window
            .global::<crate::Shade>()
            .on_pressed(move |row, x, y| {
                if let Some(app) = weak.upgrade() {
                    app.shade_pressed(row.max(0) as usize, x, y);
                }
            });
        on!(on_tree_click, |app, index| app.tree_click(index));
        on!(on_tree_double_click, |app, index| app
            .tree_double_click(index));
        on!(on_tree_menu, |app, index, x, y| app.tree_menu(index, x, y));
        on!(on_tree_name_edited, |app, text| app.tree_name_edited(&text));
        on!(on_tree_name_done, |app, ok| app.tree_name_done(ok));
        on!(on_tree_name_left, |app| app.tree_name_left());
        on!(on_tree_drag, |app, from, over| app.tree_drag(from, over));
        on!(on_tree_drop, |app, from, over| app.tree_drop(from, over));
        let weak = Rc::downgrade(self);
        shell.on_tree_key(move |event| {
            weak.upgrade().is_some_and(|app| {
                let m = event.modifiers;
                app.tree_key(&event.text, m.control || m.meta, m.alt, m.shift)
            })
        });
        on!(on_open_recent, |app, path| app
            .open_project(PathBuf::from(path.as_str()), None));
        on!(on_picker_query, |app, query| app.picker_query(&query));
        on!(on_picker_pick, |app, index| app.picker_pick(index as usize));
        on!(on_picker_moved, |app, index| app.picker_moved(index));
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
        on!(on_filter_edited, |app, panel, text| app
            .filter_edited(&panel, &text));
        on!(on_filter_level, |app, panel, level| app
            .filter_level(&panel, &level));
        let weak = Rc::downgrade(self);
        shell.on_panel_key(move |panel, event| {
            let m = event.modifiers;
            weak.upgrade().is_some_and(|app| {
                app.panel_key(&panel, &event.text, (m.control, m.alt, m.shift, m.meta))
            })
        });
        on!(on_plot_solo, |app, name| app.plot_solo(&name));
        on!(on_waves_resized, |app, width| app.waves_resized(width));
        on!(on_draw_resized, |app, w, h| app.draw_resized(w, h));
        on!(on_lab_pick_source, |app, index| app.lab_pick_source(index));
        on!(on_debug_frame, |app, level| app.debug_frame(level));
        on!(on_calls_toggle, |app, id| app
            .calls_toggle(id.max(0) as u64));
        on!(on_calls_go, |app, id, function| app
            .calls_go(id.max(0) as u64, function));
        on!(on_calls_sites, |app, id| app.calls_sites(id.max(0) as u64));
        on!(on_calls_direction, |app, incoming| app
            .calls_direction(incoming));
        on!(on_registers_pick, |app, name| app.registers_pick(&name));
        on!(on_search_edited, |app, what, text| app
            .search_edited(&what, &text));
        on!(on_search_open, |app, index| app.search_open(index));
        on!(on_term_resized, |app, cols, rows| app
            .term_resized(cols, rows));
        on!(on_term_scroll, |app, lines| app.term_scroll(lines));
        on!(on_term_select, |app, kind, col, row| app
            .term_select(kind, col, row));
        on!(on_content_zoom, |app, what, delta| app
            .content_zoom(&what, delta));
        on!(on_page_click, |app, x, y| app.page_click(x, y));
        on!(on_surface_menu, |app, surface, what, row, x, y| app
            .surface_menu(&surface, &what, row, x, y));
        let weak = Rc::downgrade(self);
        window.on_resized(move || {
            if let Some(app) = weak.upgrade()
                && let Some(window) = app.window()
            {
                crate::zoom::apply(&window, app.zoom());
            }
        });
        let weak = Rc::downgrade(self);
        window.global::<crate::Code>().on_zoom(move |delta| {
            if let Some(app) = weak.upgrade() {
                app.content_zoom("editor", delta);
            }
        });
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
        on!(on_md_flow_width, |app, key, width, size| app
            .md_flow_width(key, width, size));
        on!(on_math_select, |app, index| app.math_select(index));
        on!(on_clone_edited, |app| app.clone_edited());
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
        on!(on_set_view, |app, which, on| app.set_view(&which, on));
        on!(on_set_vim, |app, on| app.set_vim(on));
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
            let m = event.modifiers;
            weak.upgrade()
                .is_some_and(|app| app.window_key(&event.text, m.control, m.alt, m.shift, m.meta))
        });
        let weak = Rc::downgrade(self);
        self.editor.on_chord(move |text, ctrl, alt, shift, meta| {
            weak.upgrade()
                .is_some_and(|app| app.chord_key(text, ctrl, alt, shift, meta))
        });
        let weak = Rc::downgrade(self);
        shell.on_keys_stroke(move |event| {
            let m = event.modifiers;
            weak.upgrade()
                .is_some_and(|app| app.keys_stroke(&event.text, m.control, m.alt, m.shift, m.meta))
        });
        let weak = Rc::downgrade(self);
        shell.on_keys_search_stroke(move |event| {
            let m = event.modifiers;
            weak.upgrade().is_some_and(|app| {
                app.keys_search_stroke(&event.text, m.control, m.alt, m.shift, m.meta)
            })
        });
        let weak = Rc::downgrade(self);
        shell.on_key_of(move |id, _version| {
            weak.upgrade()
                .map(|app| app.key_suffix(&id))
                .unwrap_or_default()
                .into()
        });
        on!(on_keys_query_edited, |app, query| app.keys_query(&query));
        on!(on_keys_record, |app, id| app.keys_record(&id));
        on!(on_keys_reset, |app, id| app.keys_reset(&id));
        on!(on_keys_lost, |app| app.keys_lost());
        on!(on_open_recent_new_window, |app, path| app
            .new_window(Some(PathBuf::from(path.as_str()))));
        on!(on_picker_pick_new_window, |app, index| app
            .picker_pick_with(index as usize, true));
        let weak = Rc::downgrade(self);
        self.editor.on_breakpoint(move |path, line| {
            if let Some(app) = weak.upgrade() {
                app.debug_toggle(path, line);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_lens(move |runnable, verb| {
            if let Some(app) = weak.upgrade() {
                app.lens(runnable, &verb);
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_fix(move |version, actions, index| {
            if let Some(app) = weak.upgrade() {
                app.apply_fix_from(version, actions, index);
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
                app.caret_moved();
            }
        });
        let weak = Rc::downgrade(self);
        self.editor.on_lines(move |path, first, removed, added| {
            if let Some(app) = weak.upgrade() {
                app.lines_moved(path, first, removed, added);
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
            "project.open-new-window" => self.choose_project_new_window(),
            "window.new" => self.new_window(None),
            "keys.open" => self.open_shortcuts(),
            "keys.reset-all" => self.keys_reset_all(),
            "keys.search-keys" => self.keys_search_keys(),
            "settings.open" => self.open_tab(Tab::Page("settings")),
            "environment.open" => self.open_tab(Tab::Page("environment")),
            "environment.refresh" => self.probe_environment(),
            "environment.install-missing" => self.install_missing(),
            "project.new" => self.open_wizard(),
            "project.clone" => self.clone_act("open"),
            "run.test" => self.run_test(String::new()),
            other if other.starts_with("clone.") => self.clone_act(&other["clone.".len()..]),
            "playground.reset" => self.reset_playground(),
            "playground.keep" => self.keep_playground(),
            "wizard.create" => self.wizard_create(),
            "wizard.install" => self.wizard_install(),
            "welcome.open" => {
                self.open_tab(Tab::Page("welcome"));
                self.reread_workbench();
            }
            "file.save" => self.save(),
            "find.open" => self.keyed().find_open(false),
            "find.replace" => self.keyed().find_open(true),
            "find.next" => self.keyed().find_next(true),
            "find.previous" => self.keyed().find_next(false),
            "editor.go-to-line" => self.open_picker_with(Mode::Files, ":"),
            "finder.symbols" => self.open_picker_with(Mode::Files, "@"),
            "finder.workspace-symbols" => self.open_picker_with(Mode::Files, "#"),
            "editor.show-hover" => self.keyed().hover_caret(),
            "editor.definition" => self.go_to_definition(),
            "editor.references" => self.find_places(navigate::Places::References),
            "editor.implementations" => self.find_places(navigate::Places::Implementations),
            "editor.type-definition" => self.find_places(navigate::Places::TypeDefinitions),
            "editor.call-hierarchy" => self.show_call_hierarchy(),
            "editor.expand-macro" => self.expand_macro(),
            "editor.quick-fix" => self.quick_fix(),
            "editor.rename" => self.rename_prompt(),
            "editor.fold" => self.keyed().fold_at_caret(true),
            "editor.unfold" => self.keyed().fold_at_caret(false),
            "editor.fold-all" => self.keyed().fold_all(true),
            "editor.unfold-all" => self.keyed().fold_all(false),
            "editor.split" => self.split(),
            "editor.detach" => self.detach(None, None),
            "update.check" => self.update_check(true),
            "update.download" => self.update_download(),
            "update.restart" => self.update_restart(),
            "update.skip" => self.update_skip(),
            "update.releases" => markdown_view::open_link(rusty_embed::model::REPO_RELEASES),
            "editor.vim" => {
                let on = !self.state.borrow().vim;
                self.set_vim(on);
            }
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
            "tab.close-others"
            | "tab.close-right"
            | "tab.close-saved"
            | "tab.close-all"
            | "tab.copy-path"
            | "tab.copy-relative-path"
            | "tab.reveal"
            | "tab.split-right" => self.tab_act(id),
            "dock.left" => self.toggle_dock(Dock::Left),
            "dock.bottom" => self.toggle_dock(Dock::Bottom),
            "dock.right" => self.toggle_dock(Dock::Right),
            // From anywhere but the terminal, which hears its own Ctrl+`
            // first (`term_key`) — so the keys are elsewhere.
            "terminal.toggle" => self.toggle_terminal(false),
            "problems.show" => self.show_panel("bottom", "problems"),
            "view.zoom-in" => self.set_zoom(self.zoom() + 0.1),
            "view.zoom-out" => self.set_zoom(self.zoom() - 0.1),
            "view.zoom-reset" => self.set_zoom(1.0),
            "editor.zoom-reset" => self.text_zoom("editor", None),
            "page.zoom-reset" => self.text_zoom("page", None),
            "terminal.zoom-reset" => self.text_zoom("terminal", None),
            "theme.system" => self.set_theme("system"),
            "theme.dark" => self.set_theme("dark"),
            "theme.light" => self.set_theme("light"),
            "project.menu" => {
                self.open_picker(Mode::Projects);
                // What another window opened since is a recent project too.
                self.reread_workbench();
            }
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
            other if other.starts_with("tree.") => self.tree_act(&other["tree.".len()..]),
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

    /// Open a project, and then `file` in it when one is named. One open in
    /// another rusty window is said rather than opened twice: two windows
    /// on one project are two language servers at one build directory, and
    /// two hot exits keeping drafts of the same files.
    pub fn open_project(self: &Rc<Self>, root: PathBuf, file: Option<String>) {
        work::spawn(
            move || {
                if crate::instances::holding(&root) {
                    let name = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    return Err((1, t!("windows.open-elsewhere", name = name)));
                }
                Project::read(root.clone()).map_err(|reason| {
                    let path = root.display().to_string();
                    (2, t!("desk.open-failed", path = path, reason = reason))
                })
            },
            move |app, read| match read {
                Ok(project) => {
                    app.project_opened(project);
                    if let Some(file) = file {
                        app.open_file(file);
                    }
                }
                Err((level, said)) => app.note(level, said),
            },
        );
    }

    fn project_opened(self: &Rc<Self>, project: Project) {
        // The project going keeps its drafts, as a closing window does.
        self.keep_drafts();
        // Its windows of their own go with it.
        self.torn_all_gone();
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
            let mut gone = std::mem::take(&mut state.tabs);
            gone.append(&mut state.other);
            for tab in gone {
                if let Tab::File(path) = tab {
                    self.editor.close(&path);
                }
            }
            state.tabs = vec![Tab::Page("welcome")];
            state.active = Some(0);
            (state.split, state.other_active, state.focus) = (false, None, 0);
            if !playground {
                state.recents.retain(|path| path != &root);
                state.recents.insert(0, root);
            }
            state.project = Some(project);
            // A hierarchy is about the project it was asked in.
            state.calls.forget();
            state.unlinked = state.unlinked.next();
            if !state.layout.left.open {
                state.layout.left.open = true;
            }
        }
        self.editor.set_dimmed(Default::default());
        self.sync_all();
        self.sync_problems();
        self.sync_calls();
        // Where an example's main may run: not where `cargo run` flashes.
        {
            let state = self.state.borrow();
            let embedded = state
                .project
                .as_ref()
                .and_then(|p| p.embedded.as_ref().ok());
            let runs = embedded.is_none_or(|e| !e.root_is_firmware());
            let firmware = embedded.and_then(|e| e.firmware_dir.clone());
            self.editor.set_examples(runs, firmware);
        }
        self.start_servers();
        self.watch_project();
        self.scan_unlinked();
        self.probe_environment();
        self.load_board(None);
        self.bring_back(opened);
        self.tell_instances();
    }

    /// Open a file of the project: in front if it is open, read off the
    /// UI thread if it is not.
    pub fn open_file(self: &Rc<Self>, path: String) {
        self.open_file_at(path, None);
    }

    /// Open a file and put the caret at a line and column, counted from
    /// zero, when one is given.
    pub fn open_file_at(self: &Rc<Self>, path: String, at: Option<(usize, usize)>) {
        self.open_file_then(path, move |app| {
            if let Some((line, col)) = at {
                app.editor.go_to(line + 1, Some(col + 1));
            }
        });
    }

    /// Open a file in front, and then `then` — the tree, which opens a
    /// file and keeps the keys, takes them back there.
    pub(super) fn open_file_then(
        self: &Rc<Self>,
        path: String,
        then: impl FnOnce(&Rc<App>) + Send + 'static,
    ) {
        // A picture is drawn, not read as text; an SVG is both, and opens
        // as text with its picture over it.
        if let Tab::Picture(path) = Tab::of(path.clone()) {
            self.open_picture(path);
            then(self);
            return;
        }
        self.hold_then(path, move |app, path| {
            app.open_tab(Tab::File(path));
            then(app);
        });
    }

    /// `path` held by the editor — read off the UI thread when it is not —
    /// and then `then`, with no tab opened for it yet.
    pub(super) fn hold_then(
        self: &Rc<Self>,
        path: String,
        then: impl FnOnce(&Rc<App>, String) + Send + 'static,
    ) {
        if self.editor.holds(&path) {
            then(self, path);
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
                    then(app, path);
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
                        app.editor.add(document);
                        then(app, path);
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

    /// Close a tab, of the group with the keys when that group has it.
    fn close_tab(self: &Rc<Self>, id: &str) {
        let other = !self.state.borrow().tabs.iter().any(|t| t.id() == id);
        self.close_tab_in(other, id);
    }

    /// Close a tab of one group — `other` for the group without the keys,
    /// since one file can be a tab in both. A file with changes not saved
    /// asks first — save, throw the changes away, or keep the tab — as
    /// every editor does; false when the answer was to keep it.
    fn close_tab_in(self: &Rc<Self>, other: bool, id: &str) -> bool {
        let tab = self.tab_in(other, id);
        // The last view of a file with changes asks; one of two does not,
        // since the other still holds them.
        if let Some(Tab::File(path)) = tab
            && self.editor.is_dirty(&path)
            && self.views_of(&path) < 2
        {
            match ask_to_save(&path) {
                Some(true) => self.save_file(path, After::Close),
                Some(false) => {
                    if let Some(root) = self.root() {
                        self.keeper.remove(&root, &path);
                    }
                    self.drop_tab(other, id);
                }
                None => return false,
            }
            return true;
        }
        self.drop_tab(other, id);
        true
    }

    /// Drop a tab from whichever group has it, the keys group's first: for
    /// a file gone from the disk, whose tabs go from both groups, asking
    /// nothing — the same id twice drops both.
    fn drop_tab_anywhere(self: &Rc<Self>, id: &str) {
        let other = !self.state.borrow().tabs.iter().any(|t| t.id() == id);
        self.drop_tab(other, id);
    }

    fn drop_tab(self: &Rc<Self>, other: bool, id: &str) {
        let closed = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let (tabs, active) = if other {
                (&mut state.other, &mut state.other_active)
            } else {
                (&mut state.tabs, &mut state.active)
            };
            let Some(at) = tabs.iter().position(|t| t.id() == id) else {
                return;
            };
            let closed = tabs.remove(at);
            *active = match *active {
                _ if tabs.is_empty() => None,
                Some(front) if front > at => Some(front - 1),
                Some(front) if front == at => Some(at.min(tabs.len() - 1)),
                other => other,
            };
            closed
        };
        if let Tab::File(path) = &closed
            && self.views_of(path) == 0
        {
            self.backup_timers.borrow_mut().remove(path);
            self.editor.close(path);
        }
        self.settle_groups();
        self.front_tab();
    }

    // ---- editor groups ---------------------------------------------------

    /// How many tabs, across both groups and every window of its own,
    /// show `path`.
    fn views_of(&self, path: &str) -> usize {
        let state = self.state.borrow();
        state
            .tabs
            .iter()
            .chain(&state.other)
            .filter(|tab| matches!(tab, Tab::File(p) if p == path))
            .count()
            + self.torn_views_of(path)
    }

    /// Ctrl+\: the file in front beside itself — the new group on the
    /// right, with the keys, the left keeping every tab it had. Split
    /// already, the keys go to the other group.
    fn split(self: &Rc<Self>) {
        if self.state.borrow().split {
            self.swap_groups();
            return;
        }
        let Some(Tab::File(path)) = self.active_tab() else {
            return;
        };
        let scroll = self.editor.scroll();
        {
            let mut state = self.state.borrow_mut();
            state.other = std::mem::take(&mut state.tabs);
            state.other_active = state.active.take();
            state.tabs = vec![Tab::File(path)];
            state.active = Some(0);
            state.split = true;
            state.focus = 1;
        }
        self.editor.show_shade(self.shade_file().as_deref(), scroll);
        self.front_tab();
        self.editor.set_scroll(scroll);
    }

    /// The keys to the other group: it is put in front where its view was
    /// scrolled, and the group leaving is drawn where it was.
    fn swap_groups(self: &Rc<Self>) {
        if !self.state.borrow().split {
            return;
        }
        let (leaving, coming) = (self.editor.scroll(), self.editor.shade_scroll());
        {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            std::mem::swap(&mut state.tabs, &mut state.other);
            std::mem::swap(&mut state.active, &mut state.other_active);
            state.focus = 1 - state.focus;
        }
        self.editor
            .show_shade(self.shade_file().as_deref(), leaving);
        self.front_tab();
        self.editor.set_scroll(coming);
    }

    /// The file the other group has in front, while split.
    fn shade_file(&self) -> Option<String> {
        let state = self.state.borrow();
        if !state.split {
            return None;
        }
        match state.other_active.and_then(|at| state.other.get(at)) {
            Some(Tab::File(path)) => Some(path.clone()),
            _ => None,
        }
    }

    /// A group left with no tabs ends the split: the other's tabs are the
    /// window's, on the left — never an empty pane beside the work.
    fn settle_groups(&self) {
        let ended = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            if !state.split || (!state.tabs.is_empty() && !state.other.is_empty()) {
                false
            } else {
                if state.tabs.is_empty() {
                    state.tabs = std::mem::take(&mut state.other);
                    state.active = state.other_active.take();
                }
                state.other.clear();
                (state.split, state.other_active, state.focus) = (false, None, 0);
                true
            }
        };
        let scroll = self.editor.shade_scroll();
        self.editor.show_shade(self.shade_file().as_deref(), scroll);
        if ended {
            self.sync_tabs();
        }
    }

    /// A tab on the other group's strip: its group to front, and that tab.
    fn shade_pick(self: &Rc<Self>, id: &str) {
        self.swap_groups();
        self.pick_tab(id);
    }

    /// A press in the other group's view: its group to front, and the
    /// press made there.
    fn shade_pressed(self: &Rc<Self>, row: usize, x: f32, y: f32) {
        self.swap_groups();
        self.editor.press_at(row, x, y);
    }

    /// Put the active tab's document on screen and say so.
    fn front_tab(self: &Rc<Self>) {
        match self.active_tab() {
            Some(Tab::File(path)) => self.editor.show(&path),
            _ => self.editor.show_nothing(),
        }
        self.sync_tabs();
        self.strip_changed();
        self.sync_tree();
        self.sync_status();
        self.sync_markdown();
        self.sync_picture();
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
            // Shown is back on the strip, wherever it was taken off.
            layout.hidden.retain(|hidden| hidden != panel);
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

    /// The divider between the editor groups let go of: where the groups
    /// part is kept with the docks' sizes.
    fn split_resized(self: &Rc<Self>, share: f32) {
        if !share.is_finite() {
            return;
        }
        self.state.borrow_mut().layout.split = share.clamp(0.05, 0.95);
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

    /// One of Settings ▸ Editor's switches: drawn at once, and kept in
    /// `workbench.toml` so a second window draws the editor the same way.
    fn set_view(self: &Rc<Self>, which: &str, on: bool) {
        let view = {
            let mut state = self.state.borrow_mut();
            let view = &mut state.view;
            match which {
                "hints" => view.inlay_hints = on,
                "minimap" => view.minimap = on,
                "sticky" => view.sticky_scroll = on,
                "guides" => view.indent_guides = on,
                _ => return,
            }
            *view
        };
        for surface in self.surfaces() {
            surface.set_view(view);
        }
        self.sync_settings();
        let _ = rusty_embed::config::update(move |saved| saved.editor = view);
    }

    /// Vim's keys on or off: at once, and in `workbench.toml`, so a second
    /// window boots into the same mode — landing in the wrong one is twenty
    /// keystrokes doing something else.
    fn set_vim(self: &Rc<Self>, on: bool) {
        self.state.borrow_mut().vim = on;
        for surface in self.surfaces() {
            surface.set_vim(on);
        }
        self.sync_settings();
        self.sync_status();
        let _ = rusty_embed::config::update(move |saved| saved.vim = on);
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
        self.symbols_opened();
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
        // The finder's `@` and `#`: symbols, with an icon for each kind.
        let symbolic = (mode == Mode::Files)
            .then(|| crate::symbols::target(query))
            .flatten();
        if !matches!(symbolic, Some(crate::symbols::Target::Outline { .. })) {
            self.preview_ends(true);
        }
        if let Some(target) = symbolic {
            let (rows, picks): (Vec<PickRow>, Vec<Pick>) = self
                .symbol_rows(target)
                .into_iter()
                .map(|row| {
                    let shown = PickRow {
                        title: row.title.into(),
                        detail: row.detail.into(),
                        chord: row.right.into(),
                        windowed: false,
                        glyph: row.glyph,
                        tint: row.tint,
                        group: row.group.into(),
                    };
                    (shown, row.pick)
                })
                .unzip();
            self.picker_show(mode, rows, picks);
            return;
        }
        let found = match mode {
            Mode::Commands => picker::commands(query, &self.state.borrow().keys.map),
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
            .map(|(title, detail, pick)| PickRow {
                // A project opens here, or with Ctrl+Enter or its button in
                // a window of its own, as in VS Code's recents.
                windowed: matches!(pick, Pick::Project(_)),
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
                glyph: 0,
                tint: 0,
                group: SharedString::new(),
            })
            .collect();
        let picks = found.into_iter().map(|(.., p)| p).collect();
        self.picker_show(mode, rows, picks);
    }

    /// The picker's rows on screen, the first selected — and, for the
    /// finder's `@`, shown in the editor.
    fn picker_show(&self, mode: Mode, rows: Vec<PickRow>, picks: Vec<Pick>) {
        self.state.borrow_mut().picker = Some((mode, picks));
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_picker_rows(ModelRc::new(VecModel::from(rows)));
            shell.set_picker_listed(shell.get_picker_listed() + 1);
            shell.set_picker_selected(0);
        }
        self.picker_moved(0);
    }

    fn picker_pick(self: &Rc<Self>, index: usize) {
        self.picker_pick_with(index, false);
    }

    /// A row picked, a project's opening in a new window when `windowed`.
    fn picker_pick_with(self: &Rc<Self>, index: usize, windowed: bool) {
        let pick = self
            .state
            .borrow_mut()
            .picker
            .take()
            .and_then(|(_, picks)| picks.into_iter().nth(index));
        if let Some(window) = self.window() {
            window.global::<Shell>().set_picker_open(false);
        }
        // What `@` showed in the editor is put back, unless it is gone to.
        self.symbols_closed(matches!(pick, Some(Pick::Outline(_))));
        match pick {
            Some(Pick::Action(id)) => self.act(id),
            Some(Pick::File(path)) => self.open_file(path),
            Some(Pick::Project(path)) if windowed => self.new_window(Some(PathBuf::from(path))),
            Some(Pick::Project(path)) => self.open_project(PathBuf::from(path), None),
            Some(Pick::Line(line, col)) => self.editor.go_to(line, col),
            Some(Pick::Place(location)) | Some(Pick::Outline(location)) => self.jump(&location),
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
        // Every window of its own goes with this one: their drafts are
        // kept above, and their files are on the strip written for next time.
        self.torn_all_gone();
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
        self.sync_update();
        self.sync_calls();
        self.sync_keys();
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
                Glyph::Calls => icons.get_calls(),
            }
        };
        let state = self.state.borrow();
        let has_content = |id: &str| has_content(&state, id);
        let panels = |dock: Dock| -> ModelRc<PanelTab> {
            let front = dock.front(&state.layout);
            let hidden = &dock.of(&state.layout).hidden;
            let tabs: Vec<PanelTab> = dock
                .panels()
                .iter()
                .filter(|(id, ..)| crate::docks::listed(id, has_content, front, hidden))
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
        shell.set_split_share(layout.split);
    }

    fn sync_tabs(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        // Changed in place, never replaced: a tab under a held pointer
        // keeps its drag (`tab_drag.rs`).
        self.strips
            .show(self.doc_tabs(&state.tabs), self.doc_tabs(&state.other));
        let active = state.active.and_then(|at| state.tabs.get(at));
        let shade = state
            .other_active
            .and_then(|at| state.other.get(at))
            .filter(|_| state.split);
        shell.set_split(state.split);
        shell.set_focus_group(state.focus as i32);
        shell.set_shade_active(shade.map(Tab::id).unwrap_or_default().into());
        shell.set_shade_kind(
            shade
                .map(|t| t.kind().to_string())
                .unwrap_or_default()
                .into(),
        );
        shell.set_shade_title(shade.map(Tab::title).unwrap_or_default().into());
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
        drop(state);
        self.sync_torn();
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
        let (mode, pending) = self
            .editor
            .vim_status()
            .filter(|_| file)
            .unwrap_or_default();
        shell.set_vim_mode(mode.into());
        shell.set_vim_hint(pending.into());
        shell.set_encoding(encoding.into());
        shell.set_eol(eol.into());
        let language = match self.active_tab() {
            // What rust-analyzer expands a macro to is Rust.
            Some(Tab::File(path)) if rusty_edit::is_expansion(&path) => "Rust",
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
        shell.set_page_zoom(state.layout.page_zoom);
        shell.set_term_font_size(terminal::FONT * state.layout.terminal_zoom);
        self.editor.set_scale(state.layout.editor_zoom);
        shell.set_view_hints(state.view.inlay_hints);
        shell.set_view_minimap(state.view.minimap);
        shell.set_view_sticky(state.view.sticky_scroll);
        shell.set_view_guides(state.view.indent_guides);
        shell.set_vim_on(state.vim);
        drop(state);
        self.sync_torn_settings();
    }
}

/// Whether a dock's panel has something to show: a run's panels arrive
/// with what they read.
fn has_content(state: &State, id: &str) -> bool {
    match id {
        "plot" => !state.sim.plot.channels.is_empty() || !state.sim.params.is_empty(),
        "waves" => !state.sim.trace.events.is_empty(),
        "draw" => !state.draw.is_empty(),
        "flight" => !state.sim.sensors.is_empty(),
        "debug" | "registers" => state.debug.state.is_some(),
        "calls" => !state.calls.is_idle(),
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
