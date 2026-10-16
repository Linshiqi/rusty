//! The finder's `@` and `#` on the window's side: the file in front's
//! outline asked of the server that reads it, the project's symbols asked
//! as a name is typed, and the editor showing where the selected `@` row
//! leads. `crate::symbols` is the pure half — the rows, and which answer
//! to a search may be shown.
//!
//! Where nothing can answer the finder says which: no project, no file in
//! front, no server for the file's language, a server not running or still
//! starting or loading, and an answer of nothing — never an empty list.

use std::{rc::Rc, time::Duration};

use rusty_i18n::t;
use rusty_lsp::{Location, Symbol};
use slint::{ComponentHandle, Timer, TimerMode};

use super::{App, Tab};
use crate::{
    Shell,
    picker::{Mode, Pick},
    symbols::{self, Row, Search, Target, View},
};

/// How long the typing pauses before the name typed is asked of the
/// server: a search per keystroke would be answers to names nobody stopped
/// at.
const PAUSE: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct Symbols {
    /// The outline of the file in front, as last asked.
    outline: Option<Outline>,
    serial: u64,
    /// The project's symbols, as asked while a name is typed.
    search: Search,
    /// Started by every change to a `#` name (`Search::wants`); when it
    /// fires, the name is asked.
    pause: Timer,
    /// The view as it was before `@` showed a symbol in it: the file, and
    /// where it was scrolled — what Escape puts back.
    before: Option<(String, (f32, f32))>,
    /// The probe's: the next workspace answer held this long before it
    /// lands, to be the slow reply an answer must never be shown after.
    hold: Option<Duration>,
}

/// One file's outline, about one version of its text.
struct Outline {
    path: String,
    version: u64,
    serial: u64,
    answer: Answer,
}

enum Answer {
    Asking,
    Got(Vec<Symbol>),
    Failed(String),
}

impl Outline {
    /// An answer that may only have been "not yet": nothing, or a refusal.
    fn is_empty(&self) -> bool {
        match &self.answer {
            Answer::Asking => false,
            Answer::Got(symbols) => symbols.is_empty(),
            Answer::Failed(_) => true,
        }
    }
}

impl App {
    /// The finder's rows for a query in one of the symbol modes.
    pub(super) fn symbol_rows(self: &Rc<Self>, target: Target) -> Vec<Row> {
        match target {
            Target::Outline { filter, grouped } => self.outline_rows(&filter, grouped),
            Target::Workspace { query } => self.workspace_rows(&query),
        }
    }

    /// The file the keys are in, when a file is what is in front.
    fn file_in_front(&self) -> Option<String> {
        match self.active_tab() {
            Some(Tab::File(path)) => Some(path),
            _ => None,
        }
    }

    /// Whether an empty answer may be "not yet": the server said it is
    /// still loading — or, from one that never says (clangd), the status
    /// bar's server is starting or busy.
    fn still_loading(&self, settled: Option<bool>) -> bool {
        match settled {
            Some(settled) => !settled,
            None => self.state.borrow().server.is_busy(),
        }
    }

    /// Why no server will be asked about `path`: none reads its language,
    /// or the one that does is not running — or is still being started.
    fn symbols_refused(&self, path: &str) -> Option<String> {
        if self.servers.serves(path) {
            return None;
        }
        if self.state.borrow().server.is_starting() {
            return Some(t!("symbols.starting"));
        }
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        Some(match self.servers.named_for(path) {
            Some(server) => t!("symbols.not-running", server = server.label()),
            None => t!("symbols.no-server", file = name),
        })
    }

    fn outline_rows(self: &Rc<Self>, filter: &str, grouped: bool) -> Vec<Row> {
        if self.root().is_none() {
            return vec![Row::said(t!("symbols.no-project"))];
        }
        let Some(path) = self.file_in_front() else {
            return vec![Row::said(t!("symbols.no-file"))];
        };
        if let Some(refused) = self.symbols_refused(&path) {
            return vec![Row::said(refused)];
        }
        let version = self.editor.version_now().unwrap_or_default();
        let asked = self
            .state
            .borrow()
            .symbols
            .outline
            .as_ref()
            .is_some_and(|o| o.path == path && o.version == version);
        if !asked {
            self.ask_outline(&path, version);
        }
        let server = self
            .servers
            .named_for(&path)
            .map_or("", |server| server.label());
        let name = path.rsplit('/').next().unwrap_or(&path);
        let loading = self.still_loading(self.servers.settled_for(&path));
        let state = self.state.borrow();
        let Some(outline) = state.symbols.outline.as_ref() else {
            return Vec::new();
        };
        match &outline.answer {
            Answer::Asking => vec![Row::said(t!("symbols.asking", server = server))],
            Answer::Failed(reason) => vec![Row::said(t!(
                "symbols.failed",
                server = server,
                reason = reason
            ))],
            Answer::Got(list) if list.is_empty() && loading => {
                vec![Row::said(t!("symbols.loading", server = server))]
            }
            Answer::Got(list) if list.is_empty() => {
                vec![Row::said(t!("symbols.none-file", file = name))]
            }
            Answer::Got(list) => symbols::outline_rows(list, filter, grouped),
        }
    }

    /// The outline of `path` asked for, about the text on screen.
    fn ask_outline(self: &Rc<Self>, path: &str, version: u64) {
        let Some((asked, _, _, draft)) = self.editor.caret_question() else {
            return;
        };
        if asked != path {
            return;
        }
        let serial = {
            let mut state = self.state.borrow_mut();
            let symbols = &mut state.symbols;
            symbols.serial += 1;
            symbols.outline = Some(Outline {
                path: path.to_string(),
                version,
                serial: symbols.serial,
                answer: Answer::Asking,
            });
            symbols.serial
        };
        self.servers.ask_always(
            path,
            Some(draft),
            |client, path| client.document_symbols(path),
            move |app, answer| app.outline_answered(serial, answer),
        );
    }

    fn outline_answered(self: &Rc<Self>, serial: u64, answer: Result<Vec<Symbol>, String>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(outline) = state
                .symbols
                .outline
                .as_mut()
                .filter(|outline| outline.serial == serial)
            else {
                return;
            };
            outline.answer = match answer {
                Ok(list) => Answer::Got(list),
                Err(reason) => Answer::Failed(reason),
            };
        }
        self.symbols_requery();
    }

    fn workspace_rows(self: &Rc<Self>, query: &str) -> Vec<Row> {
        if self.root().is_none() {
            return vec![Row::said(t!("symbols.no-project"))];
        }
        let near = self.file_in_front();
        let Some(server) = self.servers.project_kind(near.as_deref()) else {
            let said = if self.state.borrow().server.is_starting() {
                t!("symbols.starting")
            } else {
                t!("symbols.no-servers")
            };
            return vec![Row::said(said)];
        };
        let server = server.label();
        if query.is_empty() {
            let mut state = self.state.borrow_mut();
            state.symbols.pause.stop();
            state.symbols.search.forget();
            return vec![Row::said(t!("symbols.type-name"))];
        }
        let loading = self.still_loading(self.servers.project_settled(near.as_deref()));
        let ask = self.state.borrow_mut().symbols.search.wants(query);
        if ask {
            self.search_soon(query, near);
        }
        let state = self.state.borrow();
        match state.symbols.search.view(query) {
            View::Answer([]) if loading => {
                vec![Row::said(t!("symbols.loading", server = server))]
            }
            View::Answer([]) => vec![Row::said(t!("symbols.none-workspace", name = query))],
            View::Answer(list) => symbols::workspace_rows(list),
            View::Failed(reason) => vec![Row::said(t!(
                "symbols.failed",
                server = server,
                reason = reason
            ))],
            View::Narrowed(list) => {
                let list: Vec<Symbol> = list.into_iter().cloned().collect();
                symbols::workspace_rows(&list)
            }
            View::Waiting => vec![Row::said(t!("symbols.asking", server = server))],
        }
    }

    /// `query` asked once the typing pauses on it.
    fn search_soon(self: &Rc<Self>, query: &str, near: Option<String>) {
        let weak = Rc::downgrade(self);
        let query = query.to_string();
        self.state
            .borrow()
            .symbols
            .pause
            .start(TimerMode::SingleShot, PAUSE, move || {
                if let Some(app) = weak.upgrade() {
                    app.search_now(&query, near.clone());
                }
            });
    }

    fn search_now(self: &Rc<Self>, query: &str, near: Option<String>) {
        self.state.borrow_mut().symbols.search.unpause();
        // Only the name still typed: the finder closed, or moved on.
        let still = self.typed_now().is_some_and(|typed| {
            symbols::target(&typed)
                == Some(Target::Workspace {
                    query: query.into(),
                })
        });
        if !still {
            return;
        }
        let (asked, hold) = {
            let mut state = self.state.borrow_mut();
            let symbols = &mut state.symbols;
            (symbols.search.ask(query), symbols.hold.take())
        };
        let name = query.to_string();
        let sent = self.servers.ask_project(
            near.as_deref(),
            move |client| client.workspace_symbols(&name),
            move |app, answer| match hold {
                Some(hold) => {
                    let weak = Rc::downgrade(app);
                    Timer::single_shot(hold, move || {
                        if let Some(app) = weak.upgrade() {
                            app.searched(asked, answer);
                        }
                    });
                }
                None => app.searched(asked, answer),
            },
        );
        if !sent {
            self.symbols_requery();
        }
    }

    /// The answer to ask `asked`, shown only while that ask is the newest.
    fn searched(self: &Rc<Self>, asked: u64, answer: Result<Vec<Symbol>, String>) {
        let kept = self
            .state
            .borrow_mut()
            .symbols
            .search
            .answered(asked, answer);
        if kept {
            self.symbols_requery();
        }
    }

    /// What the finder's field holds, while the finder is open.
    fn typed_now(&self) -> Option<String> {
        if !matches!(self.state.borrow().picker, Some((Mode::Files, _))) {
            return None;
        }
        let window = self.window()?;
        Some(window.global::<Shell>().get_picker_text().to_string())
    }

    /// An answer landed: the finder's rows again, if it still shows symbols.
    fn symbols_requery(self: &Rc<Self>) {
        if let Some(typed) = self.typed_now()
            && symbols::target(&typed).is_some()
        {
            self.picker_query(&typed);
        }
    }

    /// The server settled: what came back empty while it loaded is asked
    /// again. Only then are the rows made again — a server says it settled
    /// often, and rows made again go back to the first.
    pub(super) fn symbols_settled(self: &Rc<Self>) {
        let forgot = {
            let mut state = self.state.borrow_mut();
            let symbols = &mut state.symbols;
            let outline = symbols.outline.as_ref().is_some_and(Outline::is_empty);
            if outline {
                symbols.outline = None;
            }
            symbols.search.forget_empty() || outline
        };
        if forgot {
            self.symbols_requery();
        }
    }

    /// The picker opened: nothing asked for an earlier one is waited on,
    /// and an outline that came back empty is asked again.
    pub(super) fn symbols_opened(&self) {
        self.preview_ends(true);
        let mut state = self.state.borrow_mut();
        let symbols = &mut state.symbols;
        symbols.pause.stop();
        symbols.search.forget();
        if symbols.outline.as_ref().is_some_and(Outline::is_empty) {
            symbols.outline = None;
        }
    }

    /// The picker closed, on a symbol gone to (`went`) or not: the view is
    /// put back unless it is going there, and no answer is waited on.
    pub(super) fn symbols_closed(&self, went: bool) {
        self.preview_ends(!went);
        let mut state = self.state.borrow_mut();
        state.symbols.pause.stop();
        state.symbols.search.forget();
    }

    /// The picker's selected row moved to `index`: an `@` row shows where it
    /// leads in the editor.
    pub(super) fn picker_moved(&self, index: i32) {
        let pick = self.state.borrow().picker.as_ref().and_then(|(_, picks)| {
            usize::try_from(index)
                .ok()
                .and_then(|at| picks.get(at).cloned())
        });
        match pick {
            Some(Pick::Outline(location)) => self.preview_symbol(&location),
            _ if self.state.borrow().symbols.before.is_some() => self.editor.preview(None, 0.0),
            _ => {}
        }
    }

    fn preview_symbol(&self, location: &Location) {
        if self.editor.current_path().as_deref() != Some(location.path.as_str()) {
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            if state.symbols.before.is_none() {
                state.symbols.before = Some((location.path.clone(), self.editor.scroll()));
            }
        }
        // How far the finder hangs over the code: the line goes below it.
        let covered = self.window().map_or(0.0, |window| {
            let foot = window.global::<Shell>().get_picker_foot();
            foot - window.global::<crate::Code>().get_view_top()
        });
        self.editor.preview(Some(location.line as usize), covered);
    }

    /// The editor no longer shows a symbol: its line unlit, and with
    /// `restore` the view back where it was before.
    pub(super) fn preview_ends(&self, restore: bool) {
        let Some((path, scroll)) = self.state.borrow_mut().symbols.before.take() else {
            return;
        };
        self.editor.preview(None, 0.0);
        if restore && self.editor.current_path().as_deref() == Some(path.as_str()) {
            self.editor.set_scroll(scroll);
        }
    }

    /// The probe's: the next answer to a workspace search lands `by` late,
    /// as a slow server's would.
    pub fn hold_next_symbol_answer(&self, by: Duration) {
        self.state.borrow_mut().symbols.hold = Some(by);
    }

    /// The probe's: the name the workspace was asked about last, the name
    /// the rows on screen answer, and how many answers came too late to be
    /// shown.
    pub fn symbol_search(&self) -> (Option<String>, Option<String>, u32) {
        let state = self.state.borrow();
        let search = &state.symbols.search;
        (
            search.asked_for().map(str::to_string),
            search.shown_for().map(str::to_string),
            search.dropped(),
        )
    }
}
