//! The code surface's Rust half: the open documents, the one on screen, and
//! everything a key, a click or a paste does to it.
//!
//! Slint draws; this measures, places and decides. A row is handed over as
//! pieces of one token at the x they start at and marks behind them
//! (`rows.rs`), so the view does no text layout, and the caret, a click and
//! a selection are placed by the arithmetic that placed the text
//! (`metrics.rs`). What an edit does is pure arithmetic over the text and
//! the cursors (`cursors.rs`, `pairs.rs`, `indent.rs`, `clip.rs`,
//! `lineops.rs`, under their tests); this module applies it to a
//! [`Document`] and tells the view, the painter and the app what changed.

mod analysis;
mod asks;
pub mod backup;
mod brackets;
mod clip;
mod completion;
mod cursors;
mod document;
mod find;
mod finding;
mod history;
mod host;
mod hover;
mod indent;
mod inlays;
mod keymap;
mod lineops;
mod lines;
mod link;
mod metrics;
mod modal;
mod overview;
mod pairs;
mod problems;

pub use problems::is_problem;
mod resting;
mod rows;
mod shade;
pub(crate) use rows::kind as token_kind;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    ops::Range,
    rc::{Rc, Weak},
    sync::Arc,
    time::{Duration, Instant},
};

use rusty_edit::{
    Files,
    text_file::{Charset, LineEnding},
};
use slint::{ComponentHandle, ModelRc};

use crate::{
    AppWindow, Code, Find, TornWindow, language,
    lsp::{Problems, Servers},
    work,
};
use cursors::Cursor;
use document::Change;
pub use document::Document;
use history::Kind;
use host::{Host, Strong};
use keymap::Command;
pub use metrics::Metrics;
pub use rows::Gutter;
use rows::Rows;

type Shared = Rc<RefCell<Option<Rc<RefCell<Document>>>>>;

/// The open documents by path, shared by every surface: one file open in
/// two views is one document.
type Open = Rc<RefCell<HashMap<String, Rc<RefCell<Document>>>>>;

/// Every surface the process has — the main window's, and one per window a
/// tab was moved into — so that what one view does to a document the others
/// showing it draw too, and an answer finds the surface that asked.
#[derive(Default)]
struct Views {
    next: Cell<usize>,
    all: RefCell<Vec<(usize, Weak<Surface>)>>,
}

impl Views {
    fn join(&self, surface: Weak<Surface>) -> usize {
        let id = self.next.get();
        self.next.set(id + 1);
        let mut all = self.all.borrow_mut();
        all.retain(|(_, view)| view.strong_count() > 0);
        all.push((id, surface));
        id
    }

    fn live(&self) -> Vec<Rc<Surface>> {
        self.all
            .borrow()
            .iter()
            .filter_map(|(_, view)| view.upgrade())
            .collect()
    }

    fn find(&self, id: usize) -> Option<Rc<Surface>> {
        self.all
            .borrow()
            .iter()
            .find(|(view, _)| *view == id)
            .and_then(|(_, view)| view.upgrade())
    }
}

/// What every surface shares with the others.
struct Common {
    files: Arc<Files>,
    open: Open,
    metrics: Rc<RefCell<Metrics>>,
    gutter: Rc<RefCell<Gutter>>,
    painting: Rc<RefCell<HashSet<String>>>,
    servers: Rc<Servers>,
    problems: Problems,
    views: Rc<Views>,
}

/// Who is told after an edit, with the edited document's path.
type OnEdit = RefCell<Option<Box<dyn Fn(&str)>>>;
/// Who is told when the caret moves.
type OnCaret = RefCell<Option<Box<dyn Fn()>>>;
/// Asked first about every key the code takes: the window's, while a
/// two-stroke chord is half pressed — its second stroke is the chord's, not
/// a key for the editor (`Ctrl+K Ctrl+C` is no copy).
type OnChord = RefCell<Option<Box<dyn Fn(&str, bool, bool, bool, bool) -> bool>>>;
/// Who is told which lines an edit replaced: the path, the first line, how
/// many lines were there and how many are there now.
type OnLines = RefCell<Option<Box<dyn Fn(&str, usize, usize, usize)>>>;
type OnBreakpoint = RefCell<Option<Box<dyn Fn(String, u32)>>>;
type OnLens = RefCell<Option<Box<dyn Fn(rusty_edit::Runnable, String)>>>;
/// Who applies a fix the hover card offered: the text's version, the
/// server's answer, and which of its actions.
type OnFix = RefCell<Option<Box<dyn Fn(u64, rusty_lsp::CodeActions, usize)>>>;
/// Who does an action the view asked for by its id — a Ctrl+click's
/// definition, Vim's `:w` — for the window the view is in.
type OnAct = RefCell<Option<Box<dyn Fn(&str)>>>;

/// A press being dragged: what the press itself selected — a point, a word
/// or a line — which every move extends from.
#[derive(Clone, Copy, Debug)]
struct Drag {
    span: (usize, usize),
    unit: Unit,
    /// Which cursor the drag moves: the primary one, or one Alt added.
    cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Point,
    Word,
    Line,
}

/// The last press, for counting a double and a triple click.
#[derive(Clone, Copy, Debug)]
struct Press {
    at: Instant,
    line: usize,
    x: f32,
    count: u8,
}

/// How long two presses may be apart, and how far, to be one click count.
const CLICK_GAP: Duration = Duration::from_millis(450);
const CLICK_SLOP: f32 = 4.0;

/// Whether the code takes `stroke` for its own — Ctrl+D, Ctrl+Z — before
/// the window's shortcuts hear it, while the code has the keys.
pub fn takes(stroke: &crate::keys::Stroke) -> bool {
    keymap::command(&stroke.text(), stroke.ctrl, stroke.alt, stroke.shift, 1).is_some()
}

pub struct Surface {
    /// Itself, for the timers that call back into it.
    this: Weak<Surface>,
    /// Which of the process's surfaces this is: an answer from another
    /// thread names it, since a surface cannot travel there.
    id: usize,
    views: Rc<Views>,
    files: Arc<Files>,
    open: Open,
    current: Shared,
    metrics: Rc<RefCell<Metrics>>,
    rows: Rc<Rows>,
    /// The margin's breakpoints and the debugger's stop, shared with the
    /// rows that draw them.
    gutter: Rc<RefCell<Gutter>>,
    /// Told the line a click beside its number asked a breakpoint for.
    on_breakpoint: OnBreakpoint,
    /// Told what a lens's verb asked to run.
    on_lens: OnLens,
    /// Told which fix the hover card's click picked.
    on_fix: OnFix,
    /// Told what action the view asked for; the app's own when nobody is.
    on_act: OnAct,
    window: Host,
    on_edit: OnEdit,
    on_caret: OnCaret,
    on_chord: OnChord,
    on_lines: OnLines,
    /// Paths with a repaint on its way: one at a time each, whichever
    /// surface asked for it.
    painting: Rc<RefCell<HashSet<String>>>,
    drag: Cell<Option<Drag>>,
    press: Cell<Option<Press>>,
    /// The last line copied with nothing selected, which pastes as a line.
    copied_line: RefCell<Option<String>>,
    /// The lines drawn with marks, to redraw when the cursors leave them.
    marked: RefCell<Range<usize>>,
    /// The widest line seen, for how far the view scrolls sideways.
    widest: Cell<f32>,
    /// The find bar, and what it found, which the rows draw.
    finding: RefCell<finding::Finding>,
    found: Rc<RefCell<find::Marks>>,
    /// The servers told about every document, and what they found.
    servers: Rc<Servers>,
    problems: Problems,
    /// The lines squiggles are drawn on in the document on screen.
    squiggled: RefCell<Vec<usize>>,
    hovering: hover::Hovering,
    /// The name under the pointer while Ctrl is held, drawn as a link.
    linking: link::Linking,
    /// Where the caret rests: the bracket pair outlined and the name's
    /// occurrences washed, shared with the rows that draw them.
    resting: Rc<RefCell<resting::Resting>>,
    occurring: resting::Occurring,
    completing: completion::Completing,
    /// Sticky scroll and the minimap.
    overview: overview::Overview,
    /// When the server is next asked for colours and hints.
    analysis: analysis::Analysis,
    /// Vim's keys, when they are on.
    modal: modal::Modal,
    /// The other editor group's file, drawn beside this one.
    shade: shade::Shade,
}

impl Surface {
    pub fn new(
        window: &AppWindow,
        files: Arc<Files>,
        servers: Rc<Servers>,
        problems: Problems,
    ) -> Rc<Surface> {
        let strong = Strong::Main(window.clone_strong());
        let common = Common {
            files,
            open: Rc::default(),
            metrics: Rc::new(RefCell::new(Metrics::measure(&strong))),
            gutter: Rc::default(),
            painting: Rc::default(),
            servers,
            problems,
            views: Rc::default(),
        };
        Surface::build(&strong, Host::Main(window.as_weak()), common)
    }

    /// A surface for a window a tab was moved into, beside this one: the
    /// same documents, servers, painting and margin, a view of its own.
    pub fn beside(&self, window: &TornWindow) -> Rc<Surface> {
        let strong = Strong::Torn(window.clone_strong());
        let common = Common {
            files: self.files.clone(),
            open: self.open.clone(),
            metrics: self.metrics.clone(),
            gutter: self.gutter.clone(),
            painting: self.painting.clone(),
            servers: self.servers.clone(),
            problems: self.problems.clone(),
            views: self.views.clone(),
        };
        Surface::build(&strong, Host::Torn(window.as_weak()), common)
    }

    fn build(strong: &Strong, host: Host, common: Common) -> Rc<Surface> {
        let Common {
            files,
            open,
            metrics,
            gutter,
            painting,
            servers,
            problems,
            views,
        } = common;
        let current: Shared = Rc::default();
        let found: Rc<RefCell<find::Marks>> = Rc::default();
        let resting: Rc<RefCell<resting::Resting>> = Rc::default();
        let rows = Rc::new(Rows::new(
            current.clone(),
            metrics.clone(),
            found.clone(),
            resting.clone(),
            problems.clone(),
            gutter.clone(),
        ));
        strong
            .global::<Code>()
            .set_rows(ModelRc::from(rows.clone()));
        let shade_doc: Shared = Rc::default();
        let shade_rows = Rc::new(Rows::new(
            shade_doc.clone(),
            metrics.clone(),
            Rc::default(),
            Rc::default(),
            problems.clone(),
            gutter.clone(),
        ));
        // The other editor group is the main window's alone.
        if let Strong::Main(window) = strong {
            window
                .global::<crate::Shade>()
                .set_rows(ModelRc::from(shade_rows.clone()));
        }
        let surface = Rc::new_cyclic(|this| Surface {
            this: this.clone(),
            id: views.join(this.clone()),
            views,
            files,
            open,
            current,
            metrics,
            rows,
            gutter,
            on_breakpoint: RefCell::default(),
            on_lens: RefCell::default(),
            on_fix: RefCell::default(),
            on_act: RefCell::default(),
            window: host,
            on_edit: RefCell::default(),
            on_caret: RefCell::default(),
            on_chord: RefCell::default(),
            on_lines: RefCell::default(),
            painting,
            drag: Cell::new(None),
            press: Cell::new(None),
            copied_line: RefCell::default(),
            marked: RefCell::new(0..0),
            widest: Cell::new(0.0),
            finding: RefCell::default(),
            found,
            servers,
            problems,
            squiggled: RefCell::default(),
            hovering: hover::Hovering::default(),
            linking: link::Linking::default(),
            resting,
            occurring: resting::Occurring::default(),
            completing: completion::Completing::default(),
            overview: overview::Overview::default(),
            analysis: analysis::Analysis::default(),
            modal: modal::Modal::default(),
            shade: shade::Shade {
                doc: shade_doc,
                rows: shade_rows,
            },
        });
        surface.wire(strong);
        surface
    }

    /// Which surface this is, for an answer to find it by.
    pub fn id(&self) -> usize {
        self.id
    }

    /// The surface `id`, while it is there: one in another window's, or
    /// this one.
    pub fn view(&self, id: usize) -> Option<Rc<Surface>> {
        self.views.find(id)
    }

    /// The surface `id`, or this one when that one has gone: an answer that
    /// must land somewhere — a repaint, which frees its document for the
    /// next — lands here.
    fn view_or_this(&self, id: usize) -> Rc<Surface> {
        self.view(id)
            .or_else(|| self.this.upgrade())
            .expect("a surface answering is alive")
    }

    /// Every other surface still open.
    fn others(&self) -> Vec<Rc<Surface>> {
        self.views
            .live()
            .into_iter()
            .filter(|view| view.id != self.id)
            .collect()
    }

    /// Who does what the view asks for by an action's id.
    pub fn on_act(&self, tell: impl Fn(&str) + 'static) {
        *self.on_act.borrow_mut() = Some(Box::new(tell));
    }

    /// An action the view asked for: its window's to do.
    fn act(&self, id: &str) {
        match self.on_act.borrow().as_ref() {
            Some(tell) => tell(id),
            None => work::app().act(id),
        }
    }

    pub fn on_edit(&self, tell: impl Fn(&str) + 'static) {
        *self.on_edit.borrow_mut() = Some(Box::new(tell));
    }

    /// Who decides what a click beside a line's number does: the debugger
    /// holds the breakpoints.
    /// What a lens runs, and which verb: `run` or `debug`.
    pub fn on_lens(&self, tell: impl Fn(rusty_edit::Runnable, String) + 'static) {
        *self.on_lens.borrow_mut() = Some(Box::new(tell));
    }

    /// The last run's verdicts, by test filter; the lenses redrawn.
    pub fn set_verdicts(
        &self,
        verdicts: std::collections::HashMap<String, rusty_edit::testrun::Verdict>,
    ) {
        self.gutter.borrow_mut().verdicts = verdicts;
        self.gutter_changed();
    }

    /// Where an example's `main` may be run from: not a root that is its own
    /// firmware, and not inside the excluded firmware crate.
    pub fn set_examples(&self, runs: bool, firmware: Option<String>) {
        let mut gutter = self.gutter.borrow_mut();
        gutter.runs_examples = runs;
        gutter.firmware = firmware;
    }

    /// Who applies the fix a click on the hover card picked.
    pub fn on_fix(&self, tell: impl Fn(u64, rusty_lsp::CodeActions, usize) + 'static) {
        *self.on_fix.borrow_mut() = Some(Box::new(tell));
    }

    pub fn on_breakpoint(&self, tell: impl Fn(String, u32) + 'static) {
        *self.on_breakpoint.borrow_mut() = Some(Box::new(tell));
    }

    /// The margin's state, to change; [`Surface::gutter_changed`] redraws it.
    pub fn gutter(&self) -> Rc<RefCell<Gutter>> {
        self.gutter.clone()
    }

    /// The margin changed: every row on screen is drawn again, in every
    /// window, since the margin is every view's.
    pub fn gutter_changed(&self) {
        for view in self.views.live() {
            view.rows.reset();
        }
    }

    pub fn on_caret(&self, tell: impl Fn() + 'static) {
        *self.on_caret.borrow_mut() = Some(Box::new(tell));
    }

    pub fn on_chord(&self, ask: impl Fn(&str, bool, bool, bool, bool) -> bool + 'static) {
        *self.on_chord.borrow_mut() = Some(Box::new(ask));
    }

    /// Who follows a line through the edits: told the path, the first line
    /// an edit replaced, how many lines it replaced and how many it wrote.
    pub fn on_lines(&self, tell: impl Fn(&str, usize, usize, usize) + 'static) {
        *self.on_lines.borrow_mut() = Some(Box::new(tell));
    }

    // ---- documents -------------------------------------------------------

    pub fn holds(&self, path: &str) -> bool {
        self.open.borrow().contains_key(path)
    }

    pub fn is_dirty(&self, path: &str) -> bool {
        self.open
            .borrow()
            .get(path)
            .is_some_and(|doc| doc.borrow().dirty())
    }

    /// The path of the document on screen.
    pub fn current_path(&self) -> Option<String> {
        Some(self.doc()?.borrow().path.clone())
    }

    /// A document as it is to be written: text, charset, line endings.
    pub fn draft_of(&self, path: &str) -> Option<(String, Charset, LineEnding)> {
        let open = self.open.borrow();
        let doc = open.get(path)?.borrow();
        Some((doc.text.clone(), doc.charset, doc.ending))
    }

    /// A document's draft as it would be backed up, when it differs from
    /// the disk's.
    pub fn backup_of(&self, path: &str) -> Option<backup::Backup> {
        let open = self.open.borrow();
        backup_of(&open.get(path)?.borrow())
    }

    /// Every open document with its draft to keep, or `None` where its
    /// text is the disk's and any old backup is to go. Not a macro's
    /// expansion, which has no disk to differ from.
    pub fn backups(&self) -> Vec<(String, Option<backup::Backup>)> {
        self.open
            .borrow()
            .values()
            .filter(|doc| !rusty_edit::is_expansion(&doc.borrow().path))
            .map(|doc| {
                let doc = doc.borrow();
                (doc.path.clone(), backup_of(&doc))
            })
            .collect()
    }

    /// Which text of `path` an answer about it was asked about.
    pub fn version_of(&self, path: &str) -> Option<u64> {
        Some(self.open.borrow().get(path)?.borrow().version)
    }

    /// Replace `path`'s text — a format's answer — when it is still the
    /// text at `version`; the caret keeps its line and column. Undone in
    /// one step, as VS Code's format is.
    pub fn replace(&self, path: &str, version: u64, text: String) -> bool {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return false;
        };
        let change = {
            let mut d = doc.borrow_mut();
            if d.version != version {
                return false;
            }
            let (line, col) = d.lines.position(&d.text, d.primary().head);
            let lines = lines::Lines::of(&text);
            let at = lines.byte(&text, line, col);
            d.edit(text, vec![Cursor::caret(at)], Kind::Other)
        };
        if let Some(change) = change {
            self.edited(&doc, change);
        }
        true
    }

    /// A draft put back over a document just opened: the disk's text is
    /// what it was saved as, and the draft is one undo away from it.
    pub fn restore(&self, path: &str, draft: String) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let change = doc
            .borrow_mut()
            .edit(draft, vec![Cursor::caret(0)], Kind::Other);
        if let Some(change) = change {
            self.edited(&doc, change);
        }
    }

    /// A document changed by something other than a key: on screen it is
    /// redrawn as a key's edit is; off screen it is painted and said.
    fn edited(&self, doc: &Rc<RefCell<Document>>, change: Change) {
        if self.doc().is_some_and(|current| Rc::ptr_eq(&current, doc)) {
            self.after_edit(doc, change);
        } else {
            self.paint(doc);
            let path = doc.borrow().path.clone();
            self.servers.changed(&path, &doc.borrow().text);
            self.tell_edit(&path);
            self.tell_lines(&path, change);
            self.follows(doc, Some(change));
        }
    }

    /// The disk now holds `text` for `path`.
    pub fn saved(&self, path: &str, text: &str) {
        if let Some(doc) = self.open.borrow().get(path) {
            doc.borrow_mut().mark_saved(text);
        }
        self.servers.saved(path);
        self.tell_edit(path);
    }

    /// Open a document and put it on screen; its colours follow from a
    /// painting off the UI thread.
    pub fn show_new(&self, document: Document) {
        let path = document.path.clone();
        self.add(document);
        self.show(&path);
    }

    /// Open a document without putting it on screen.
    pub fn add(&self, document: Document) {
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, &document.text);
        }
        let path = document.path.clone();
        self.servers.opened(&path, &document.text);
        let doc = Rc::new(RefCell::new(document));
        self.open.borrow_mut().insert(path, doc.clone());
        self.paint(&doc);
    }

    /// Where `path`'s view was left: where the next view to show it finds
    /// it (`show`, and the other group's view when a tab is moved there).
    pub fn kept_scroll(&self, path: &str) -> Option<(f32, f32)> {
        self.open.borrow().get(path).map(|doc| doc.borrow().scroll)
    }

    /// `path`'s view left at `at`, by a view about to show something else
    /// — the other group's, whose file a dragged tab took the place of.
    pub fn keep_scroll(&self, path: &str, at: (f32, f32)) {
        if let Some(doc) = self.open.borrow().get(path) {
            doc.borrow_mut().scroll = (shade::within(at.0), at.1);
        }
    }

    pub fn show(&self, path: &str) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        self.widest.set(self.metrics.borrow().widest(&doc.borrow()));
        // The view leaves its place with the document it showed, and
        // finds the one this document was left at.
        if let Some(leaving) = self.doc() {
            leaving.borrow_mut().scroll = self.scroll();
        }
        let scroll = doc.borrow().scroll;
        *self.current.borrow_mut() = Some(doc);
        *self.marked.borrow_mut() = 0..0;
        self.squiggled.borrow_mut().clear();
        self.close_completion();
        self.close_signature();
        self.finding.borrow_mut().scope = None;
        self.research(None);
        self.rows.reset();
        self.rest_nowhere();
        self.caret_rested();
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            code.set_has_document(true);
            code.set_content_width(self.widest.get() + 200.0);
            code.set_scroll_y(shade::within(scroll.0));
            code.set_scroll_x(scroll.1);
            self.show_caret();
            window.invoke_focus_code();
        }
        self.view_changed();
        self.analyse_soon();
    }

    /// The text drawn at `scale` of its own size — Ctrl and the wheel over
    /// the code, in any window that shows it. The font, the one row height
    /// and the margin change together, every character is measured again at
    /// the new size and every row placed again, and each view keeps the
    /// line at its top.
    ///
    /// **One size for every window.** The windows a tab was moved into
    /// measure their characters into the same table as this one and keep
    /// one place per document, both in pixels of the size the text is
    /// drawn at, so a window at a size of its own would be measured wrong
    /// by every other. So the table and the places are done once, here,
    /// and then every view is drawn again at the new size.
    pub fn set_scale(&self, scale: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let (font, row, gutter) = metrics::text_size(scale);
        // What this window had: the size the documents' places are kept in.
        let before = window.global::<Code>().get_row_height().max(1.0);
        let views = self.views.live();
        let resized: Vec<(Rc<Surface>, f32)> = views
            .into_iter()
            .filter_map(|view| view.fit_text(font, row, gutter).map(|old| (view, old)))
            .collect();
        if resized.is_empty() {
            return;
        }
        // Measured through this window, whose text is at the new size now.
        let mut metrics = Metrics::measure(&window);
        for doc in self.open.borrow().values() {
            let d = doc.borrow();
            metrics.learn(&window, &d.text);
            let labels: String = d.hints.iter().map(|h| h.label.as_str()).collect();
            metrics.learn(&window, &labels);
        }
        *self.metrics.borrow_mut() = metrics;
        // Every document keeps its top line and how far across it was.
        for doc in self.open.borrow().values() {
            let mut d = doc.borrow_mut();
            d.scroll = (d.scroll.0 * row / before, d.scroll.1 * row / before);
        }
        for (view, old) in resized {
            view.refit(old, row);
        }
    }

    /// A new view takes the size every other view has: a window a tab was
    /// moved into, before it shows anything. The table and the documents'
    /// places are at that size already.
    pub fn take_scale(&self, scale: f32) {
        let (font, row, gutter) = metrics::text_size(scale);
        if let Some(old) = self.fit_text(font, row, gutter) {
            self.refit(old, row);
        }
    }

    /// This view's text set to `font` on rows of `row` beside a margin of
    /// `gutter`, what was drawn over the text at the old size put away; the
    /// old row height, or nothing when the view had that size already.
    fn fit_text(&self, font: f32, row: f32, gutter: f32) -> Option<f32> {
        let window = self.window.upgrade()?;
        let code = window.global::<Code>();
        let before = code.get_row_height().max(1.0);
        if code.get_font_size() == font && before == row && code.get_gutter() == gutter {
            return None;
        }
        self.hide_hover();
        self.close_completion();
        self.close_signature();
        code.set_font_size(font);
        code.set_row_height(row);
        code.set_gutter(gutter);
        Some(before)
    }

    /// This view drawn again at the new size: where it was scrolled to,
    /// kept at the same line and the same share across, the width of what
    /// it shows, and every row.
    fn refit(&self, before: f32, row: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let across = |x: f32| x * row / before;
        let code = window.global::<Code>();
        code.set_scroll_y(shade::within(across(code.get_scroll_y())));
        code.set_scroll_x(across(code.get_scroll_x()));
        let widths = |doc: &Shared| {
            doc.borrow()
                .as_ref()
                .map(|doc| self.metrics.borrow().widest(&doc.borrow()))
        };
        if let Some(widest) = widths(&self.current) {
            self.widest.set(widest);
            code.set_content_width(widest + 200.0);
        }
        // The other group is the main window's alone.
        if let Strong::Main(main) = &window {
            let shade = main.global::<crate::Shade>();
            shade.set_scroll_y(shade::within(across(shade.get_scroll_y())));
            shade.set_scroll_x(across(shade.get_scroll_x()));
            if let Some(widest) = widths(&self.shade.doc) {
                shade.set_content_width(widest + 200.0);
            }
        }
        self.overview.forget_stuck();
        self.rows.reset();
        self.shade.rows.reset();
        if self.doc().is_some() {
            self.hints_changed();
        }
        self.view_changed();
    }

    pub fn show_nothing(&self) {
        *self.current.borrow_mut() = None;
        self.rest_nowhere();
        self.rows.reset();
        if let Some(window) = self.window.upgrade() {
            window.global::<Code>().set_has_document(false);
        }
    }

    pub fn close(&self, path: &str) {
        if self.open.borrow_mut().remove(path).is_some() {
            self.servers.closed(path);
        }
        for other in self.others() {
            other.let_go(path);
        }
        self.let_go(path);
    }

    /// `path` is no longer open: neither view of this surface shows it.
    fn let_go(&self, path: &str) {
        let shaded = self
            .shade
            .doc
            .borrow()
            .as_ref()
            .is_some_and(|doc| doc.borrow().path == path);
        if shaded {
            self.show_shade(None, (0.0, 0.0));
        }
        let showing = self
            .current
            .borrow()
            .as_ref()
            .is_some_and(|doc| doc.borrow().path == path);
        if showing {
            self.show_nothing();
        }
    }

    /// `from` — a file, or a folder and everything open below it — is now
    /// `to` on disk: every document it held goes with it, its text, cursors,
    /// folds and history and all, renamed in place. The servers hear the old
    /// name closed and the new one opened; a document whose extension
    /// changed is painted again, since the grammar is read off the name.
    /// Answers with each document's old path and new.
    pub fn moved(&self, from: &str, to: &str) -> Vec<(String, String)> {
        let moving: Vec<(String, String)> = self
            .open
            .borrow()
            .keys()
            .filter_map(|path| Some((path.clone(), rusty_edit::moved_path(path, from, to)?)))
            .collect();
        for (old, new) in &moving {
            let Some(doc) = self.open.borrow_mut().remove(old) else {
                continue;
            };
            let text = {
                let mut d = doc.borrow_mut();
                d.path = new.clone();
                d.text.clone()
            };
            self.open.borrow_mut().insert(new.clone(), doc.clone());
            self.servers.closed(old);
            self.servers.opened(new, &text);
            let extension = |path: &str| {
                std::path::Path::new(path)
                    .extension()
                    .map(|e| e.to_ascii_lowercase())
            };
            if extension(old) != extension(new) {
                {
                    let mut d = doc.borrow_mut();
                    d.paint = None;
                    d.unpainted = None;
                }
                self.paint(&doc);
            }
        }
        moving
    }

    /// Ctrl+G's line and column, counted from one: the caret there, and the
    /// line in the middle of the view when it was not on screen.
    pub fn go_to(&self, line: usize, col: Option<usize>) {
        let Some(doc) = self.doc() else { return };
        let (at, line) = {
            let d = doc.borrow();
            let line = line.saturating_sub(1).min(d.lines.count() - 1);
            let col = col.map_or(0, |col| col.saturating_sub(1));
            (d.lines.byte(&d.text, line, col), line)
        };
        if doc.borrow_mut().unfold_over([line]) {
            self.rows.reset();
        }
        if let Some(window) = self.window.upgrade() {
            let code = window.global::<Code>();
            let (row, view) = (code.get_row_height(), code.get_view_height());
            let top = doc.borrow().row_of(line) as f32 * row;
            let scrolled = -code.get_scroll_y();
            if top < scrolled || top + row > scrolled + view {
                code.set_scroll_y(-(top - (view - row) / 2.0).max(0.0));
            }
        }
        self.set_cursors(vec![Cursor::caret(at)]);
        if let Some(window) = self.window.upgrade() {
            window.invoke_focus_code();
        }
    }

    /// Where the caret is, for a question about the name there: the file,
    /// its line and column, and the text as the question should find it.
    pub fn caret_question(&self) -> Option<(String, u32, u32, rusty_lsp::Draft)> {
        let doc = self.doc()?;
        let d = doc.borrow();
        let (line, col) = d.lines.position(&d.text, d.primary().head);
        Some((
            d.path.clone(),
            line as u32,
            col as u32,
            self.servers.draft(&d.text),
        ))
    }

    /// The document on screen as it goes with a question to the assistant:
    /// its path, its draft — unsaved edits included — and the lines the
    /// selection covers, counted from zero; the caret's line twice when
    /// nothing is selected.
    pub fn attachment(&self) -> Option<(String, String, (usize, usize))> {
        let doc = self.doc()?;
        let d = doc.borrow();
        let cursor = d.primary();
        let (first, _) = d.lines.position(&d.text, cursor.anchor.min(cursor.head));
        let (last, _) = d.lines.position(&d.text, cursor.anchor.max(cursor.head));
        Some((d.path.clone(), d.text.clone(), (first, last)))
    }

    /// The word the caret is in, for a rename to start from.
    pub fn caret_word(&self) -> Option<String> {
        let doc = self.doc()?;
        let d = doc.borrow();
        let (from, to) = cursors::word_at(&d.text, d.primary().head)?;
        Some(d.text[from..to].to_string())
    }

    /// Open a file outside the project, to be read and not written.
    pub fn show_read_only(&self, mut document: Document) {
        document.read_only = true;
        self.show_new(document);
    }

    /// The text a file has on disk now, put in its document as an edit —
    /// a rename or a fix wrote it there — and the document saved as that.
    pub fn reload(&self, path: &str, text: String) {
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let head = doc.borrow().primary().head.min(text.len());
        let head = (0..=head)
            .rev()
            .find(|&at| text.is_char_boundary(at))
            .unwrap_or(0);
        let change = doc
            .borrow_mut()
            .edit(text.clone(), vec![Cursor::caret(head)], Kind::Other);
        doc.borrow_mut().mark_saved(&text);
        if let Some(change) = change {
            self.edited(&doc, change);
        }
        self.tell_edit(path);
    }

    /// Edits the server sent for the document on screen — a fix — applied
    /// while it is still the text they were made for (`version`).
    pub fn apply_server_edits(&self, version: u64, edits: Vec<rusty_lsp::ActionEdit>) {
        self.apply_resolved(version, edits);
    }

    pub fn version_now(&self) -> Option<u64> {
        Some(self.doc()?.borrow().version)
    }

    /// Every open document whose text differs from the disk's, as it is to
    /// be written.
    pub fn unsaved(&self) -> Vec<(String, String, Charset, LineEnding)> {
        self.open
            .borrow()
            .values()
            .filter_map(|doc| {
                let d = doc.borrow();
                (d.dirty() && !d.read_only)
                    .then(|| (d.path.clone(), d.text.clone(), d.charset, d.ending))
            })
            .collect()
    }

    /// How many lines the document on screen has.
    pub fn line_count(&self) -> Option<usize> {
        Some(self.doc()?.borrow().lines.count())
    }

    /// The caret on screen as people number it, and what the document is
    /// written as: line and column from one, charset, line endings.
    pub fn status(&self) -> Option<(usize, usize, &'static str, &'static str)> {
        let doc = self.current.borrow().clone()?;
        let doc = doc.borrow();
        let at = self.vim_cursor(&doc).unwrap_or(doc.primary().head);
        let (line, col) = doc.lines.position(&doc.text, at);
        Some((line + 1, col + 1, doc.charset.label(), doc.ending.label()))
    }

    fn doc(&self) -> Option<Rc<RefCell<Document>>> {
        self.current.borrow().clone()
    }

    // ---- telling ---------------------------------------------------------

    fn tell_edit(&self, path: &str) {
        if let Some(tell) = self.on_edit.borrow().as_ref() {
            tell(path);
        }
    }

    fn tell_caret(&self) {
        if let Some(tell) = self.on_caret.borrow().as_ref() {
            tell();
        }
    }

    fn tell_lines(&self, path: &str, change: Change) {
        if let Some(tell) = self.on_lines.borrow().as_ref() {
            tell(path, change.first, change.removed, change.added);
        }
    }

    // ---- the view's callbacks --------------------------------------------

    fn wire(self: &Rc<Self>, window: &Strong) {
        self.wire_find(&window.global::<Find>());
        let code = window.global::<Code>();
        self.wire_hover(&code);
        let this = Rc::downgrade(self);
        code.on_typed(move |text| {
            if let Some(this) = this.upgrade() {
                this.hide_hover();
                this.link_away();
                this.type_text(&text);
            }
        });
        let this = Rc::downgrade(self);
        code.on_key(move |event| {
            let Some(this) = this.upgrade() else {
                return false;
            };
            let m = event.modifiers;
            // Ctrl on its own draws the name under the pointer as a link;
            // no modifier on its own puts the card away, as in VS Code.
            this.link_key(&event.text, true);
            if link::is_modifier(&event.text) {
                return false;
            }
            // While a two-stroke chord is half pressed the key is the
            // window's, before the editor can take it for its own.
            if let Some(chord) = this.on_chord.borrow().as_ref()
                && chord(&event.text, m.control, m.alt, m.shift, m.meta)
            {
                return true;
            }
            // Any other key puts the card away: it is about the text as it
            // was.
            this.hide_hover();
            if this.completion_key(&event.text, m.control || m.alt || m.meta) {
                return true;
            }
            // Vim's keys come after the popup's — Escape closes the popup
            // before Vim sees it — and before the editor's own.
            if this.vim_key(&event.text, m.control || m.meta, m.alt) {
                return true;
            }
            let page = this.page();
            match keymap::command(&event.text, m.control || m.meta, m.alt, m.shift, page) {
                Some(command) if this.doc().is_some() => {
                    this.run(command);
                    true
                }
                // What the editor does not take goes to the window's
                // bindings from here, not by bubbling up: the TextInput the
                // keys arrive in moves its own cursor on an arrow, so
                // Alt+Left never reached Back. Its own window's: a window a
                // tab was moved into has bindings of its own to ask.
                _ => this
                    .window
                    .upgrade()
                    .is_some_and(|window| window.invoke_key(event)),
            }
        });
        let this = Rc::downgrade(self);
        code.on_key_released(move |event| {
            if let Some(this) = this.upgrade() {
                this.link_key(&event.text, false);
            }
        });
        let this = Rc::downgrade(self);
        code.on_blurred(move || {
            if let Some(this) = this.upgrade() {
                this.link_blurred();
            }
        });
        let this = Rc::downgrade(self);
        code.on_link_over(move |row, x, y, ctrl, window_x, window_y| {
            if let Some(this) = this.upgrade() {
                this.link_over(row, x, y, ctrl, (window_x, window_y));
            }
        });
        let this = Rc::downgrade(self);
        code.on_link_left(move |row| {
            if let Some(this) = this.upgrade() {
                this.link_left(row);
            }
        });
        let this = Rc::downgrade(self);
        code.on_navigate(move |back| {
            if let Some(this) = this.upgrade() {
                this.act(if back { "nav.back" } else { "nav.forward" });
            }
        });
        let this = Rc::downgrade(self);
        code.on_completion_pick(move |row| {
            if let Some(this) = this.upgrade() {
                this.completion_pick(row as usize);
            }
        });
        let this = Rc::downgrade(self);
        code.on_lens(move |row, verb| {
            let Some(this) = this.upgrade() else { return };
            let Some(runnable) = this.rows.runnable_at(row.max(0) as usize) else {
                return;
            };
            if let Some(tell) = this.on_lens.borrow().as_ref() {
                tell(runnable, verb.to_string());
            }
        });
        let this = Rc::downgrade(self);
        code.on_toggle_breakpoint(move |row| {
            let Some(this) = this.upgrade() else { return };
            let Some(doc) = this.doc() else { return };
            let (path, line) = {
                let d = doc.borrow();
                (d.path.clone(), d.line_at_row(row.max(0) as usize))
            };
            if let Some(tell) = this.on_breakpoint.borrow().as_ref() {
                tell(path, line as u32);
            }
        });
        let this = Rc::downgrade(self);
        code.on_fold(move |row| {
            if let Some(this) = this.upgrade() {
                this.fold_row(row.max(0) as usize);
            }
        });
        let this = Rc::downgrade(self);
        code.on_view_changed(move || {
            if let Some(this) = this.upgrade() {
                this.view_changed();
            }
        });
        if let Strong::Main(main) = window {
            let this = Rc::downgrade(self);
            main.global::<crate::Shade>().on_scrolled(move || {
                if let Some(this) = this.upgrade() {
                    this.shade_scrolled();
                }
            });
        }
        let this = Rc::downgrade(self);
        code.on_sticky_pick(move |index| {
            if let Some(this) = this.upgrade() {
                this.sticky_pick(index.max(0) as usize);
            }
        });
        let this = Rc::downgrade(self);
        code.on_minimap_to(move |y| {
            if let Some(this) = this.upgrade() {
                this.minimap_to(y);
            }
        });
        let this = Rc::downgrade(self);
        code.on_context_menu(move |row, x, y, at_x, at_y| {
            if let Some(this) = this.upgrade() {
                this.context_press(row.max(0) as usize, x, y, (at_x, at_y));
            }
        });
        let this = Rc::downgrade(self);
        code.on_pointer(move |kind, row, x, y, modifiers| {
            if let Some(this) = this.upgrade() {
                let held = (
                    modifiers.shift,
                    modifiers.alt,
                    modifiers.control || modifiers.meta,
                );
                this.pointer(kind, row as usize, x, y, held);
            }
        });
    }

    /// How many rows a page is.
    fn page(&self) -> isize {
        let Some(window) = self.window.upgrade() else {
            return 20;
        };
        let code = window.global::<Code>();
        ((code.get_view_height() / code.get_row_height()) as isize - 1).max(1)
    }

    // ---- commands --------------------------------------------------------

    fn run(&self, command: Command) {
        let Some(doc) = self.doc() else { return };
        let (text, cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let primary = cursors[0];
        let single = cursors.len() == 1;
        // Each arm answers an edit (the new text, the cursors, what kind),
        // or moves the cursors itself and answers nothing.
        let edit: Option<(String, Vec<Cursor>, Kind)> = match command {
            Command::Move(motion, extend) => {
                self.set_cursors(cursors::moved(&text, &cursors, motion, extend));
                None
            }
            Command::Vertical(lines, extend) => {
                self.vertical(lines, extend);
                None
            }
            Command::Edge { end, extend } => {
                let at = if end { text.len() } else { 0 };
                let anchor = if extend { primary.anchor } else { at };
                self.set_cursors(vec![Cursor { anchor, head: at }]);
                None
            }
            Command::SelectAll => {
                self.set_cursors(vec![Cursor {
                    anchor: 0,
                    head: text.len(),
                }]);
                None
            }
            Command::SingleCursor => {
                if single && self.finding_open() {
                    self.find_close();
                } else {
                    self.set_cursors(vec![primary]);
                }
                None
            }
            Command::Backspace { word } => {
                let pair = (single && primary.is_caret() && !word)
                    .then(|| pairs::on_backspace(&text, primary.head))
                    .flatten();
                Some(match pair {
                    Some(edit) => applied(&text, edit, Kind::Deleting),
                    None => {
                        let (t, c) = cursors::erased(&text, &cursors, false, word);
                        (t, c, Kind::Deleting)
                    }
                })
            }
            Command::Delete { word } => {
                let (t, c) = cursors::erased(&text, &cursors, true, word);
                Some((t, c, Kind::Deleting))
            }
            Command::Enter => Some(if single && primary.is_caret() {
                applied(&text, pairs::on_enter(&text, primary.head), Kind::Other)
            } else {
                let (t, c) = cursors::broken(&text, &cursors);
                (t, c, Kind::Other)
            }),
            Command::Tab => {
                let shifted = (single
                    && indent::shifts_lines(&text, primary.start(), primary.end(), false))
                .then(|| indent::shift(&text, primary.start(), primary.end(), false))
                .flatten();
                Some(match shifted {
                    Some(edit) => applied(&text, edit, Kind::Other),
                    None => {
                        let (t, c) = cursors::typed(&text, &cursors, "    ");
                        (t, c, Kind::Typing)
                    }
                })
            }
            Command::Indent | Command::Outdent => {
                let back = command == Command::Outdent;
                indent::shift(&text, primary.start(), primary.end(), back)
                    .map(|edit| applied(&text, edit, Kind::Other))
            }
            Command::Copy => {
                self.copy(&text, &cursors);
                None
            }
            Command::Cut => Some(if single && primary.is_caret() {
                let (line, edit) = clip::cut_line(&text, primary.head);
                clipboard_set(&line);
                *self.copied_line.borrow_mut() = Some(line);
                applied(&text, edit, Kind::Other)
            } else {
                clipboard_set(&cursors::copied(&text, &cursors));
                let (t, c) = cursors::cut(&text, &cursors);
                (t, c, Kind::Other)
            }),
            Command::Paste => clipboard_get().map(|clip| {
                let remembered = self.copied_line.borrow().clone();
                if single
                    && primary.is_caret()
                    && clip::is_copied_line(&clip, remembered.as_deref())
                {
                    applied(
                        &text,
                        clip::paste_line(&text, primary.head, &clip),
                        Kind::Other,
                    )
                } else {
                    let (t, c) = cursors::pasted(&text, &cursors, &clip);
                    (t, c, Kind::Other)
                }
            }),
            Command::Undo | Command::Redo => {
                let change = if command == Command::Undo {
                    doc.borrow_mut().undo()
                } else {
                    doc.borrow_mut().redo()
                };
                if let Some(change) = change {
                    self.after_edit(&doc, change);
                }
                None
            }
            Command::NextMatch => {
                let last = *cursors.last().unwrap_or(&primary);
                if last.is_caret() {
                    // The first press takes the word the caret is in.
                    if let Some((start, end)) = cursors::word_at(&text, last.head) {
                        let mut next = cursors.clone();
                        if let Some(slot) = next.last_mut() {
                            *slot = Cursor {
                                anchor: start,
                                head: end,
                            };
                        }
                        self.set_cursors(next);
                    }
                } else if let Some(found) = cursors::next_match(&text, &cursors) {
                    let mut next = cursors.clone();
                    next.push(found);
                    self.set_cursors(next);
                    self.reveal(found.head);
                }
                None
            }
            Command::AllMatches => {
                let target = if primary.is_caret() {
                    cursors::word_at(&text, primary.head)
                        .map(|(anchor, head)| Cursor { anchor, head })
                } else {
                    Some(primary)
                };
                if let Some(target) = target {
                    self.set_cursors(cursors::every_match(&text, target));
                }
                None
            }
            Command::AddCursor { up } => {
                if let Some(added) = cursors::beside_vertically(&text, &cursors, up) {
                    let mut next = cursors.clone();
                    next.push(added);
                    self.set_cursors(next);
                    self.reveal(added.head);
                }
                None
            }
            Command::MoveLines { up } => {
                lineops::moved(&text, &cursors, up).map(|(t, c)| (t, c, Kind::Other))
            }
            Command::CopyLines { up } => {
                let (t, c) = lineops::copied(&text, &cursors, up);
                Some((t, c, Kind::Other))
            }
            Command::DeleteLines => {
                let (t, c) = lineops::deleted(&text, &cursors);
                Some((t, c, Kind::Other))
            }
            Command::OpenLine { above } => {
                let (t, c) = lineops::opened(&text, &cursors, above);
                Some((t, c, Kind::Other))
            }
            Command::SelectLine => {
                self.set_cursors(lineops::line_selected(&text, &cursors));
                None
            }
            Command::Suggest => {
                self.suggest();
                None
            }
            Command::Fold(fold) => {
                self.fold_at_caret(fold);
                None
            }
            Command::ToggleComment => {
                let path = doc.borrow().path.clone();
                language::of(&path)
                    .and_then(|language| language.comment)
                    .and_then(|marker| lineops::commented(&text, &cursors, marker))
                    .map(|(t, c)| (t, c, Kind::Other))
            }
        };
        if let Some((text, cursors, kind)) = edit {
            let change = doc.borrow_mut().edit(text, cursors::merged(&cursors), kind);
            match change {
                Some(change) => self.after_edit(&doc, change),
                None => self.cursors_moved(),
            }
        }
        // Deleting widens the popup; anything else that is not typing
        // leaves the word, and the popup goes. The signature is asked
        // again, and goes when the caret has left the call.
        match command {
            Command::Backspace { .. } | Command::Delete { .. } => self.after_typing(false),
            Command::Suggest => {}
            _ => {
                self.close_completion();
                if self.completing_signature() {
                    self.after_typing(false);
                }
            }
        }
    }

    /// Text typed or composed: at every cursor, with the bracket pairs' rules
    /// for one character at one cursor.
    fn type_text(&self, typed: &str) {
        // What an input method commits in Vim's normal mode is not text.
        if self.vim_commands() {
            return;
        }
        let Some(doc) = self.doc() else { return };
        if let Some(window) = self.window.upgrade() {
            self.metrics.borrow_mut().learn(&window, typed);
        }
        let (text, cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let mut chars = typed.chars();
        let one = match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        };
        let pair = match one {
            Some(ch) if cursors.len() == 1 => {
                pairs::on_type(&text, cursors[0].start(), cursors[0].end(), ch)
            }
            _ => None,
        };
        let (text, cursors) = match pair {
            Some(edit) => {
                let (t, c, _) = applied(&text, edit, Kind::Typing);
                (t, c)
            }
            None => cursors::typed(&text, &cursors, typed),
        };
        let change = doc.borrow_mut().edit(text, cursors, Kind::Typing);
        if let Some(change) = change {
            self.after_edit(&doc, change);
        }
        self.after_typing(true);
    }

    fn copy(&self, text: &str, cursors: &[Cursor]) {
        let copied = if cursors.len() == 1 && cursors[0].is_caret() {
            let line = clip::copy_line(text, cursors[0].head);
            *self.copied_line.borrow_mut() = Some(line.clone());
            line
        } else {
            cursors::copied(text, cursors)
        };
        clipboard_set(&copied);
    }

    /// Up and Down by `lines`, to the column the run started at — measured
    /// in pixels, so a caret below a `中` lands under it and not a column
    /// of ASCII off.
    fn vertical(&self, lines: isize, extend: bool) {
        let Some(doc) = self.doc() else { return };
        {
            let metrics = self.metrics.borrow();
            let mut d = doc.borrow_mut();
            let last = d.rows() as isize - 1;
            let x_of = |d: &Document, at: usize| {
                let (line, col) = d.lines.position(&d.text, at);
                inlays::caret_on(&metrics, d, line, col)
            };
            let want = d.want_x.unwrap_or_else(|| x_of(&d, d.cursors[0].head));
            let moved: Vec<Cursor> = d
                .cursors
                .iter()
                .enumerate()
                .map(|(index, cursor)| {
                    let x = if index == 0 {
                        want
                    } else {
                        x_of(&d, cursor.head)
                    };
                    let target = d.row_of(d.lines.line_of(cursor.head)) as isize + lines;
                    let head = if target < 0 {
                        0
                    } else if target > last {
                        d.text.len()
                    } else {
                        let target = d.line_at_row(target as usize);
                        let col = inlays::column_on(&metrics, &d, target, x);
                        d.lines.byte(&d.text, target, col)
                    };
                    let anchor = if extend { cursor.anchor } else { head };
                    Cursor { anchor, head }
                })
                .collect();
            d.set_cursors(cursors::merged(&moved));
            d.want_x = Some(want);
        }
        self.cursors_moved();
    }

    fn set_cursors(&self, cursors: Vec<Cursor>) {
        let Some(doc) = self.doc() else { return };
        doc.borrow_mut().set_cursors(cursors::merged(&cursors));
        self.cursors_moved();
    }

    // ---- the pointer -----------------------------------------------------

    /// The line a point of the view is on, and the byte the caret would
    /// stand at there: `y` from the top of `row`, `x` from the code's left.
    fn point_at(&self, doc: &Rc<RefCell<Document>>, row: usize, x: f32, y: f32) -> (usize, usize) {
        let row_height = self
            .window
            .upgrade()
            .map_or(19.0, |w| w.global::<Code>().get_row_height());
        let d = doc.borrow();
        let last = d.rows() as isize - 1;
        let screen = (row as isize + (y / row_height).floor() as isize).clamp(0, last);
        let line = d.line_at_row(screen as usize);
        let col = inlays::column_on(&self.metrics.borrow(), &d, line, x.max(0.0));
        (line, d.lines.byte(&d.text, line, col))
    }

    /// A right-click: the caret to the point first, unless it is inside a
    /// selection, as VS Code does — so what the menu offers is about what
    /// was clicked — and the menu at `at` in the window.
    fn context_press(&self, row: usize, x: f32, y: f32, at: (f32, f32)) {
        let Some(doc) = self.doc() else { return };
        self.hide_hover();
        self.close_completion();
        self.close_signature();
        let (_, point) = self.point_at(&doc, row, x, y);
        let selected = doc
            .borrow()
            .cursors
            .iter()
            .any(|c| c.start() < c.end() && c.start() <= point && point <= c.end());
        if !selected {
            self.set_cursors(vec![Cursor::caret(point)]);
        }
        // The menu is the main window's; a window of its own has none yet.
        if self.window.is_main() {
            crate::work::app().editor_menu(at.0, at.1);
        }
    }

    /// `held` is Shift, Alt and Ctrl (or Cmd).
    fn pointer(&self, kind: i32, row: usize, x: f32, y: f32, held: (bool, bool, bool)) {
        let (shift, alt, ctrl) = held;
        let Some(doc) = self.doc() else { return };
        let (line, at) = self.point_at(&doc, row, x, y);
        if kind == 0 {
            self.hide_hover();
            self.close_completion();
            self.close_signature();
            self.vim_pressed();
        }
        // Ctrl+click: the definition of the name clicked, the caret put
        // there first so the jump is about what was clicked — where the
        // link drawn on it said, when it is drawn, so what is underlined is
        // what the click follows.
        if kind == 0 && ctrl && !alt {
            let leads = self.link_under(row, x, y);
            self.link_away();
            self.set_cursors(vec![Cursor::caret(at)]);
            match leads {
                Some(location) => crate::work::app().follow_link(self.id, location),
                None => self.act("editor.definition"),
            }
            return;
        }
        if kind == 0 {
            self.link_away();
        }
        match kind {
            0 => self.press(&doc, line, x, at, shift, alt),
            1 => self.drag_to(&doc, at),
            _ => self.drag.set(None),
        }
    }

    fn press(
        &self,
        doc: &Rc<RefCell<Document>>,
        line: usize,
        x: f32,
        at: usize,
        shift: bool,
        alt: bool,
    ) {
        let now = Instant::now();
        let count = match self.press.get() {
            Some(last)
                if now.duration_since(last.at) < CLICK_GAP
                    && last.line == line
                    && (last.x - x).abs() < CLICK_SLOP =>
            {
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.press.set(Some(Press {
            at: now,
            line,
            x,
            count,
        }));
        let unit = match count {
            2 => Unit::Word,
            3 => Unit::Line,
            _ => Unit::Point,
        };
        let (text, mut cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let span = unit_span(&text, at, unit);
        let drag = if alt && unit == Unit::Point {
            // Alt adds a cursor, and dragging moves only it.
            cursors.push(Cursor::caret(at));
            Drag {
                span,
                unit,
                cursor: cursors.len() - 1,
            }
        } else if shift {
            // Shift extends the primary selection from where it was begun.
            cursors.truncate(1);
            cursors[0].head = at;
            Drag {
                span: (cursors[0].anchor, cursors[0].anchor),
                unit,
                cursor: 0,
            }
        } else {
            cursors = vec![Cursor {
                anchor: span.0,
                head: span.1,
            }];
            Drag {
                span,
                unit,
                cursor: 0,
            }
        };
        self.drag.set(Some(drag));
        doc.borrow_mut().set_cursors(cursors);
        self.cursors_moved();
    }

    fn drag_to(&self, doc: &Rc<RefCell<Document>>, at: usize) {
        let Some(drag) = self.drag.get() else { return };
        let (text, mut cursors) = {
            let d = doc.borrow();
            (d.text.clone(), d.cursors.clone())
        };
        let here = unit_span(&text, at, drag.unit);
        let cursor = if here.0 < drag.span.0 {
            Cursor {
                anchor: drag.span.1,
                head: here.0,
            }
        } else {
            Cursor {
                anchor: drag.span.0,
                head: here.1.max(drag.span.1),
            }
        };
        if let Some(slot) = cursors.get_mut(drag.cursor) {
            *slot = cursor;
        }
        doc.borrow_mut().set_cursors(cursors);
        self.cursors_moved();
    }

    // ---- after a change --------------------------------------------------

    fn after_edit(&self, doc: &Rc<RefCell<Document>>, change: Change) {
        let refolded = std::mem::take(&mut doc.borrow_mut().folds_moved);
        if refolded {
            self.rows.reset();
        } else if change.added > change.removed {
            self.rows
                .added(change.first + change.removed, change.added - change.removed);
        } else if change.removed > change.added {
            self.rows
                .removed(change.first + change.added, change.removed - change.added);
        }
        self.rows.changed(change.first, change.first + change.added);
        let widest = {
            let d = doc.borrow();
            let metrics = self.metrics.borrow();
            (change.first..change.first + change.added)
                .map(|line| inlays::end_on(&metrics, &d, line))
                .fold(self.widest.get(), f32::max)
        };
        if widest > self.widest.get() {
            self.widest.set(widest);
            if let Some(window) = self.window.upgrade() {
                window.global::<Code>().set_content_width(widest + 200.0);
            }
        }
        self.paint(doc);
        let path = doc.borrow().path.clone();
        self.servers.changed(&path, &doc.borrow().text);
        self.tell_edit(&path);
        self.tell_lines(&path, change);
        self.research(Some(change));
        self.rest_edited(change);
        self.cursors_moved();
        self.view_changed();
        self.analyse_soon();
        self.follows(doc, Some(change));
    }

    /// The cursors moved (or the text under them changed): redraw the rows
    /// their marks were and are on, and put the caret on screen.
    fn cursors_moved(&self) {
        let Some(doc) = self.doc() else { return };
        // A cursor put on a hidden line — a find, a jump, an undo — opens
        // what hides it rather than stand where nobody can see.
        let opened = {
            let mut d = doc.borrow_mut();
            let lines: Vec<usize> = d
                .cursors
                .iter()
                .flat_map(|c| [d.lines.line_of(c.anchor), d.lines.line_of(c.head)])
                .collect();
            d.unfold_over(lines)
        };
        if opened {
            self.rows.reset();
        }
        let now = {
            let d = doc.borrow();
            let first = d
                .cursors
                .iter()
                .map(|c| d.lines.line_of(c.start()))
                .min()
                .unwrap_or(0);
            let last = d
                .cursors
                .iter()
                .map(|c| d.lines.line_of(c.end()))
                .max()
                .unwrap_or(0);
            first..last + 1
        };
        let was = self.marked.replace(now.clone());
        let span = was.start.min(now.start)..was.end.max(now.end);
        if span.len() > 400 {
            self.rows.reset();
        } else {
            self.rows.changed(was.start, was.end);
            self.rows.changed(now.start, now.end);
        }
        self.find_follow_selection();
        self.show_caret();
        self.caret_rested();
        self.cursors_follow(&doc);
    }

    /// Scroll so `at` is on screen.
    fn reveal(&self, at: usize) {
        let Some(doc) = self.doc() else { return };
        let (line, x) = {
            let d = doc.borrow();
            let (line, col) = d.lines.position(&d.text, at);
            (
                line,
                inlays::caret_on(&self.metrics.borrow(), &d, line, col),
            )
        };
        self.scroll_to(line, x);
    }

    fn show_caret(&self) {
        if let Some((line, x)) = self.place_caret() {
            self.scroll_to(line, x);
            self.tell_caret();
        }
    }

    /// The caret drawn where its column now is — the text's size, or the
    /// hints before it, moved it — without the view moving to it: an
    /// answer from the server landing while the caret is scrolled out of
    /// view is no reason to scroll back to it. Its line and x.
    pub(super) fn place_caret(&self) -> Option<(usize, f32)> {
        let window = self.window.upgrade()?;
        let doc = self.doc()?;
        // In Vim's command modes the caret is a block on the character the
        // cursor stands on, which in visual mode is not the selection's head.
        let (line, x, block) = {
            let d = doc.borrow();
            let vim = self.vim_cursor(&d);
            let (line, col) = d.lines.position(&d.text, vim.unwrap_or(d.primary().head));
            let x = match vim {
                Some(_) => inlays::char_on(&self.metrics.borrow(), &d, line, col),
                None => inlays::caret_on(&self.metrics.borrow(), &d, line, col),
            };
            (line, x, vim.map(|at| self.block_width(&d, at)))
        };
        let code = window.global::<Code>();
        code.set_caret_line(doc.borrow().row_of(line) as i32);
        code.set_caret_x(x);
        code.set_block_caret(block.is_some());
        code.set_caret_width(block.unwrap_or(2.0));
        Some((line, x))
    }

    fn scroll_to(&self, line: usize, x: f32) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let code = window.global::<Code>();
        let row = code.get_row_height();
        let screen = self.doc().map_or(line, |doc| doc.borrow().row_of(line));
        let top = screen as f32 * row;
        let (view, scrolled) = (code.get_view_height(), -code.get_scroll_y());
        if top < scrolled {
            code.set_scroll_y(-top);
        } else if view > 0.0 && top + row > scrolled + view {
            code.set_scroll_y(-(top + row - view));
        }
        // Sideways: keep the caret a few characters in from either edge,
        // past the gutter.
        let gutter = code.get_gutter();
        let margin = 4.0 * self.metrics.borrow().ascii();
        let (width, across) = (code.get_view_width(), -code.get_scroll_x());
        if width > 0.0 {
            if x < across + margin {
                code.set_scroll_x(-(x - margin).max(0.0));
            } else if x + gutter + margin > across + width {
                code.set_scroll_x(-(x + gutter + margin - width));
            }
        }
    }

    // ---- folding ---------------------------------------------------------

    /// The chevron on `row`: its block collapsed, or opened.
    fn fold_row(&self, row: usize) {
        let Some(doc) = self.doc() else { return };
        let line = doc.borrow().line_at_row(row);
        let region = {
            let d = doc.borrow();
            self.rows
                .regions(&d)
                .iter()
                .find(|r| r.header == line as u32)
                .copied()
        };
        if let Some(region) = region {
            doc.borrow_mut().folds.toggle(region);
            self.refolded(&doc);
        }
    }

    /// Ctrl+Shift+[ and ]: the innermost block the caret is in collapsed, or
    /// the one collapsed on its line opened.
    pub fn fold_at_caret(&self, fold: bool) {
        let Some(doc) = self.doc() else { return };
        let changed = {
            let regions = self.rows.regions(&doc.borrow());
            let mut d = doc.borrow_mut();
            let line = d.lines.line_of(d.primary().head) as u32;
            if fold {
                let inner = regions
                    .iter()
                    .rev()
                    .find(|r| r.header <= line && line <= r.last && !d.folds.is_folded(r.header))
                    .copied();
                inner.is_some_and(|region| {
                    d.folds.fold(region);
                    d.folds.is_folded(region.header)
                })
            } else if d.folds.is_folded(line) {
                d.folds.unfold(line);
                true
            } else {
                false
            }
        };
        if changed {
            self.refolded(&doc);
        }
    }

    /// Every block collapsed — the outermost, since a fold inside a fold
    /// hides nothing more — or every block opened.
    pub fn fold_all(&self, fold: bool) {
        let Some(doc) = self.doc() else { return };
        {
            let regions = self.rows.regions(&doc.borrow());
            let mut d = doc.borrow_mut();
            d.folds.clear();
            if fold {
                for region in regions.iter() {
                    d.folds.fold(*region);
                }
            }
        }
        self.refolded(&doc);
    }

    /// The folds changed: a cursor they now hide goes to the end of its
    /// fold's header, as VS Code moves it, and every row is drawn again.
    fn refolded(&self, doc: &Rc<RefCell<Document>>) {
        {
            let mut d = doc.borrow_mut();
            let moved: Vec<Cursor> = d
                .cursors
                .iter()
                .map(|c| {
                    let line = d.lines.line_of(c.head);
                    if d.folds.hides(line as u32) {
                        let header = d.line_at_row(d.row_of(line));
                        Cursor::caret(d.lines.end(header))
                    } else {
                        *c
                    }
                })
                .collect();
            if moved != d.cursors {
                d.set_cursors(cursors::merged(&moved));
            }
        }
        self.rows.reset();
        self.cursors_moved();
        self.view_changed();
    }

    // ---- painting --------------------------------------------------------

    /// Repaint `doc` off the UI thread: one repaint on its way per document,
    /// and when it lands on a text since edited, another for the new text.
    fn paint(&self, doc: &Rc<RefCell<Document>>) {
        let (path, text, base, stale, version) = {
            let d = doc.borrow();
            let stale = d
                .unpainted
                .clone()
                .map(|lines| (lines.start as u32, lines.end as u32));
            (d.path.clone(), d.text.clone(), d.paint, stale, d.version)
        };
        if !self.painting.borrow_mut().insert(path.clone()) {
            return;
        }
        let files = self.files.clone();
        let asker = self.id;
        // The grammar is read off the name; a macro's expansion is named
        // after its macro, and what rust-analyzer expands it to is Rust.
        let asked = if rusty_edit::is_expansion(&path) {
            format!("{path}.rs")
        } else {
            path.clone()
        };
        work::spawn(
            move || files.repaint(&asked, &text, base, stale),
            move |app, repaint| {
                app.editor()
                    .view_or_this(asker)
                    .painted(&path, version, repaint)
            },
        );
    }

    /// A repaint has come back for `path`, painted for the text at
    /// `version`.
    pub fn painted(&self, path: &str, version: u64, repaint: rusty_edit::Repaint) {
        self.painting.borrow_mut().remove(path);
        let Some(doc) = self.open.borrow().get(path).cloned() else {
            return;
        };
        let from = repaint.from as usize;
        let count = repaint.lines.len();
        let placed = doc
            .borrow_mut()
            .painted_from(version, repaint.version, from, repaint.lines);
        if !placed {
            self.paint(&doc);
            return;
        }
        self.follows(&doc, None);
        let showing = self.doc().is_some_and(|current| Rc::ptr_eq(&current, &doc));
        if showing {
            self.rows.changed(from, from + count);
            // What is a string or a comment may have changed under the
            // brackets the caret is beside.
            self.match_brackets();
            self.overview
                .paintings
                .set(self.overview.paintings.get() + 1);
            self.view_changed();
        }
    }
}

/// What a press selected at `at`: the point, its word, or its line.
fn unit_span(text: &str, at: usize, unit: Unit) -> (usize, usize) {
    match unit {
        Unit::Point => (at, at),
        Unit::Word => cursors::word_at(text, at).unwrap_or((at, at)),
        Unit::Line => {
            let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
            let end = text[at..].find('\n').map_or(text.len(), |i| at + i + 1);
            (start, end)
        }
    }
}

/// A [`pairs::Edit`] applied: the text after it and the one cursor it
/// leaves, which is a selection when the edit says what to select.
fn applied(text: &str, edit: pairs::Edit, kind: Kind) -> (String, Vec<Cursor>, Kind) {
    let mut out = String::with_capacity(text.len() + edit.text.len());
    out.push_str(&text[..edit.range.0]);
    out.push_str(&edit.text);
    out.push_str(&text[edit.range.1..]);
    let cursor = match edit.select {
        Some((anchor, head)) => Cursor { anchor, head },
        None => Cursor::caret(edit.caret),
    };
    (out, vec![cursor], kind)
}

fn backup_of(doc: &Document) -> Option<backup::Backup> {
    (doc.dirty() && !rusty_edit::is_expansion(&doc.path)).then(|| backup::Backup {
        path: doc.path.clone(),
        text: doc.text.clone(),
        charset: doc.charset,
        ending: doc.ending,
    })
}

fn clipboard_set(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}
