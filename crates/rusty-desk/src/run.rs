//! What the window runs — an install, a build, a flash — one at a time,
//! its lines into the Output dock as they come and its end said.
//!
//! One at a time because what it runs fights over the same locks: two
//! `cargo install`s over the package cache, a build over the build
//! directory, a flash over the port. A second run asked for while one is
//! going waits for the first to be stopped; the caller decides whether to.
//!
//! Every run somebody asks for starts on an empty dock ([`starts_afresh`]),
//! once for the whole of it: a flash's build and its write are one run, as
//! a simulation's build and its boot are. The dock draws what the filter
//! over it lets through ([`crate::filter`]); the lines themselves are kept
//! whole under it.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use rusty_embed::{
    CommandPlan,
    activity::Kind,
    process::{Input, Session, Stopper},
};
use slint::{FilterModel, MapModel, Model, ModelRc, VecModel};

use crate::{
    OutputLine, OutputPiece,
    app::App,
    filter::{Filter, Level},
    work,
};

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Exit status zero.
    Done,
    /// Anything else, with the status when there was one.
    Failed(Option<i32>),
    /// Somebody stopped it.
    Stopped,
    /// It never started: the program is not there, or would not run.
    Refused,
}

/// The kinds a line of output is drawn as.
pub mod kind {
    pub const PLAIN: i32 = 0;
    /// The command line itself, as run.
    pub const COMMAND: i32 = 1;
    pub const ERROR: i32 = 2;
    pub const WARNING: i32 = 3;
    /// How the run ended, when it ended well.
    pub const DONE: i32 = 4;
}

/// How many lines the dock keeps at most; the oldest go first, a tenth of
/// them at a time.
const KEPT: usize = 20_000;

/// Whether a run of this kind starts on an empty dock, with the last
/// build's problems gone from Problems. A run somebody asked for does — a
/// build, a simulation and its restart, a flash, a monitor, a test, an
/// example, a debug session: what is on screen is then what this run said,
/// never the last one's errors over code that has since been mended. A
/// command between runs — a git write, an install, a `cargo add` — is
/// added below what is there, as a terminal's history is. A run's later
/// steps are not asked about at all: a chain begins once.
pub fn starts_afresh(kind: Kind) -> bool {
    !matches!(kind, Kind::Install | Kind::Command)
}

/// The filter function the dock's lines pass through.
type Passes = Box<dyn Fn(&OutputLine) -> bool>;

struct Running {
    generation: u64,
    stopper: Stopper,
    stopped: Cell<bool>,
    /// Where a line typed in the dock goes, for a session that reads it.
    input: Option<Input>,
}

pub struct Runner {
    /// Every line, as printed.
    output: Rc<VecModel<OutputLine>>,
    /// What the filter box and its switches say, and the lines they let
    /// through, which is what the dock draws.
    filter: Rc<RefCell<Filter>>,
    shown: Rc<FilterModel<Rc<VecModel<OutputLine>>, Passes>>,
    running: RefCell<Option<Running>>,
    generation: Cell<u64>,
    /// Where the last run ran: what a relative path it printed is relative
    /// to.
    dir: RefCell<Option<PathBuf>>,
    /// How many lines have gone from the front — trimmed, or cleared —
    /// since the first: what turns a line's number (`said`) into its row.
    gone: Cell<u64>,
    /// Told the counts whenever they may have moved (`on_counts`).
    counted: RefCell<Option<Counted>>,
}

type Counted = Box<dyn Fn(usize, usize)>;

/// What kind a line is, by what it says: a compiler's `error` and
/// `warning`, and the lines a tool prints when it fails.
pub fn kind_of(text: &str) -> i32 {
    let lower = text.trim_start().to_ascii_lowercase();
    if lower.starts_with("error") || lower.starts_with("fatal") || lower.contains("panicked at") {
        kind::ERROR
    } else if lower.starts_with("warning") {
        kind::WARNING
    } else {
        kind::PLAIN
    }
}

/// A line as the dock shows it: a terminal's colour codes taken out, since
/// the dock colours by kind and a raw escape is noise.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner {
    pub fn new() -> Runner {
        let output = Rc::new(VecModel::<OutputLine>::default());
        let filter = Rc::new(RefCell::new(Filter::default()));
        let passes: Passes = {
            let filter = filter.clone();
            Box::new(move |line: &OutputLine| {
                filter
                    .borrow()
                    .shows(&line.text, Level::of_output(line.kind))
            })
        };
        Runner {
            shown: Rc::new(FilterModel::new(output.clone(), passes)),
            output,
            filter,
            running: RefCell::default(),
            generation: Cell::new(0),
            dir: RefCell::default(),
            gone: Cell::new(0),
            counted: RefCell::default(),
        }
    }

    /// The number of the line said last, for [`Runner::relink`]: every line
    /// ever said has one, counted from the first, whatever has gone since.
    pub fn said(&self) -> u64 {
        (self.gone.get() + self.output.row_count() as u64).saturating_sub(1)
    }

    /// The line numbered `said` names a place after all, now that what its
    /// path is relative to is known — or nothing, if it is gone.
    pub fn relink(&self, said: u64, place: (String, i32, i32)) {
        let Some(row) = said.checked_sub(self.gone.get()) else {
            return;
        };
        let row = row as usize;
        let Some(mut line) = self.output.row_data(row) else {
            return;
        };
        (line.path, line.line, line.col) = (place.0.into(), place.1, place.2);
        self.output.set_row_data(row, line);
    }

    /// Where the last run ran.
    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.borrow().clone()
    }

    /// What the dock draws: the lines the filter lets through, each with
    /// what the box holds marked in it.
    pub fn view(&self) -> ModelRc<OutputLine> {
        let filter = self.filter.clone();
        ModelRc::new(MapModel::new(
            self.shown.clone(),
            move |mut line: OutputLine| {
                let pieces: Vec<OutputPiece> = filter
                    .borrow()
                    .pieces(&line.text)
                    .into_iter()
                    .map(|(text, hit)| OutputPiece {
                        text: text.into(),
                        hit,
                    })
                    .collect();
                if !pieces.is_empty() {
                    line.pieces = ModelRc::new(VecModel::from(pieces));
                }
                line
            },
        ))
    }

    /// The line drawn `index`th, with the filter as it is now.
    pub fn line(&self, index: usize) -> Option<OutputLine> {
        (index < self.shown.row_count())
            .then(|| self.output.row_data(self.shown.unfiltered_row(index)))
            .flatten()
    }

    /// Every line kept, filtered or not.
    pub fn all_text(&self) -> String {
        let lines: Vec<String> = self.output.iter().map(|l| l.text.to_string()).collect();
        lines.join("\n")
    }

    /// The filter, as the box and its switches show it.
    pub fn filter(&self) -> Filter {
        self.filter.borrow().clone()
    }

    /// The filter changed: every line judged again.
    pub fn set_filter(&self, change: impl FnOnce(&mut Filter)) {
        change(&mut self.filter.borrow_mut());
        self.shown.reset();
        self.recount();
    }

    /// How many lines are drawn, and how many are kept.
    pub fn counts(&self) -> (usize, usize) {
        (self.shown.row_count(), self.output.row_count())
    }

    pub fn is_running(&self) -> bool {
        self.running.borrow().is_some()
    }

    /// A line in the dock that no process printed: a command line, how a
    /// run ended, what rusty did between two runs.
    pub fn say(&self, text: &str, kind: i32) {
        self.say_at(text, kind, None);
    }

    /// A line that names a place — a path as the project names it, a line
    /// and a column counted from zero — which a click goes to.
    pub fn say_at(&self, text: &str, kind: i32, place: Option<(String, i32, i32)>) {
        let (path, line, col) = place.unwrap_or_default();
        self.output.push(OutputLine {
            text: text.into(),
            kind,
            path: path.into(),
            line,
            col,
            ..Default::default()
        });
        let over = self.output.row_count().saturating_sub(KEPT);
        if over > 0 {
            // From the front, in one go: one removal per line kept the
            // model busy longer than the run that printed them. And a
            // tenth more than is over, so the next trim is two thousand
            // lines away: every trim is the whole model set again, and the
            // filter over it judges every line again.
            let dropped = (over + KEPT / 10).min(self.output.row_count());
            let kept: Vec<OutputLine> = self.output.iter().skip(dropped).collect();
            self.gone.set(self.gone.get() + dropped as u64);
            self.output.set_vec(kept);
        }
        // The count beside the box moves with every line while it narrows;
        // with nothing typed it says nothing, and is not told.
        if self.filter.borrow().narrows() {
            self.recount();
        }
    }

    pub fn clear(&self) {
        self.gone
            .set(self.gone.get() + self.output.row_count() as u64);
        self.output.set_vec(Vec::new());
        self.recount();
    }

    /// Who hears how many lines are drawn out of how many kept.
    pub fn on_counts(&self, heard: impl Fn(usize, usize) + 'static) {
        self.counted.replace(Some(Box::new(heard)));
    }

    fn recount(&self) {
        if let Some(heard) = self.counted.borrow().as_ref() {
            let (shown, kept) = self.counts();
            heard(shown, kept);
        }
    }

    /// Run `plan` in `dir`, its lines into the dock, and hand how it ended
    /// to `then`. Refused — `false` — while another run is going.
    pub fn run(
        &self,
        plan: &CommandPlan,
        dir: Option<PathBuf>,
        then: impl FnOnce(&Rc<App>, Ended) + Send + 'static,
    ) -> bool {
        if self.is_running() {
            return false;
        }
        self.say(&format!("$ {}", plan.display), kind::COMMAND);
        self.dir.replace(dir.clone());
        // `launch`, not `spawn`: a UF2 copy and a DFU image are written in
        // this process, and everything else is spawned as `spawn` would.
        match rusty_embed::flash::launch(plan, dir.as_deref()) {
            Ok(session) => self.watch(session, plan.program.clone(), false, then),
            Err(error) => {
                self.say(&error.to_string(), kind::ERROR);
                then(&work::app(), Ended::Refused);
            }
        }
        true
    }

    /// A session opened some other way — rusty's own serial link — run as
    /// a plan's is, and typed into from the dock.
    pub fn run_session(
        &self,
        display: &str,
        session: Session,
        then: impl FnOnce(&Rc<App>, Ended) + Send + 'static,
    ) -> bool {
        if self.is_running() {
            return false;
        }
        self.say(&format!("$ {display}"), kind::COMMAND);
        self.dir.replace(None);
        self.watch(session, String::new(), true, then);
        true
    }

    fn watch(
        &self,
        session: Session,
        program: String,
        typed: bool,
        then: impl FnOnce(&Rc<App>, Ended) + Send + 'static,
    ) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        *self.running.borrow_mut() = Some(Running {
            generation,
            stopper: session.stopper(),
            stopped: Cell::new(false),
            input: typed.then(|| session.input()),
        });
        std::thread::Builder::new()
            .name("run".into())
            .spawn(move || {
                while let Some(line) = session.recv() {
                    // wlink stamps every line with the time; a protocol
                    // line is read without it.
                    let text = plain(rusty_embed::flash::firmware_line(&program, &line.text));
                    work::deliver(text, |app, text| app.heard(&text));
                }
                let code = session.wait();
                work::deliver(code, move |app, code| {
                    let stopped = app.runner().finish(generation);
                    let ended = if stopped {
                        Ended::Stopped
                    } else if code == Some(0) {
                        Ended::Done
                    } else {
                        Ended::Failed(code)
                    };
                    then(app, ended);
                });
            })
            .expect("the run's thread starts");
    }

    /// Whether what runs can be typed into from the dock.
    pub fn takes_input(&self) -> bool {
        self.running
            .borrow()
            .as_ref()
            .is_some_and(|running| running.input.is_some())
    }

    /// A line typed in the dock, to what runs.
    pub fn send(&self, text: &str) {
        if let Some(input) = self
            .running
            .borrow()
            .as_ref()
            .and_then(|r| r.input.as_ref())
        {
            input.send_line(text);
        }
    }

    /// Stop what is running; its end is said as any end is.
    pub fn stop(&self) {
        if let Some(running) = self.running.borrow().as_ref() {
            running.stopped.set(true);
            running.stopper.stop();
        }
    }

    /// The run numbered `generation` is over: whether it was stopped.
    fn finish(&self, generation: u64) -> bool {
        let mut running = self.running.borrow_mut();
        match running.as_ref() {
            Some(r) if r.generation == generation => {
                let stopped = r.stopped.get();
                *running = None;
                stopped
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_coloured_by_what_it_says_and_not_by_its_escapes() {
        assert_eq!(plain("\u{1b}[1m\u{1b}[31merror\u{1b}[0m: no"), "error: no");
        assert_eq!(kind_of("error[E0308]: mismatched types"), kind::ERROR);
        assert_eq!(kind_of("  warning: unused variable"), kind::WARNING);
        assert_eq!(
            kind_of("thread 'main' panicked at src/main.rs:3"),
            kind::ERROR
        );
        assert_eq!(kind_of("   Compiling esp-hal v1.0.0"), kind::PLAIN);
    }

    /// Every run somebody asks for starts on an empty dock; a command
    /// between runs is added to what is there.
    #[test]
    fn a_run_starts_afresh_and_a_command_does_not() {
        for kind in [
            Kind::Build,
            Kind::Test,
            Kind::Flash,
            Kind::Monitor,
            Kind::Simulate,
            Kind::Debug,
            Kind::Link,
        ] {
            assert!(starts_afresh(kind), "{kind:?}");
        }
        assert!(!starts_afresh(Kind::Install));
        assert!(!starts_afresh(Kind::Command));
    }

    /// Past the limit the oldest go a tenth at a time, the newest kept,
    /// and a filter in force goes on narrowing what is left.
    #[test]
    fn the_oldest_lines_go_a_tenth_at_a_time() {
        let runner = Runner::new();
        runner.set_filter(|f| f.set_text("7"));
        for at in 0..=KEPT {
            runner.say(&format!("line {at}"), kind::PLAIN);
        }
        let (shown, kept) = runner.counts();
        assert_eq!(kept, KEPT - KEPT / 10);
        assert_eq!(
            runner.all_text().lines().last(),
            Some(format!("line {KEPT}").as_str())
        );
        assert!(shown > 0 && shown < kept);
        assert!(drawn(&runner).iter().all(|line| line.contains('7')));
    }

    fn drawn(runner: &Runner) -> Vec<String> {
        (0..runner.counts().0)
            .filter_map(|at| runner.line(at))
            .map(|line| line.text.to_string())
            .collect()
    }

    /// The filter decides what is drawn and keeps everything: a line that
    /// arrives under it is judged as it arrives, a click on a drawn line
    /// finds the line drawn, and an empty box draws them all again.
    #[test]
    fn the_filter_narrows_what_is_drawn_and_keeps_every_line() {
        let runner = Runner::new();
        runner.say("$ cargo build", kind::COMMAND);
        runner.say_at(
            "error[E0412]: cannot find type `Vector3d`",
            kind::ERROR,
            Some(("src/lib.rs".into(), 6, 11)),
        );
        runner.say("warning: unused import", kind::WARNING);
        runner.set_filter(|f| f.set_text("VECTOR"));
        assert_eq!(
            drawn(&runner),
            ["error[E0412]: cannot find type `Vector3d`"]
        );
        assert_eq!(
            runner.line(0).map(|l| l.path.to_string()).as_deref(),
            Some("src/lib.rs")
        );
        assert!(runner.line(1).is_none());
        runner.say("error: could not compile, vector or not", kind::ERROR);
        assert_eq!(runner.counts(), (2, 4));
        let marked: Vec<(String, bool)> = runner
            .view()
            .row_data(0)
            .unwrap()
            .pieces
            .iter()
            .map(|p| (p.text.to_string(), p.hit))
            .collect();
        assert_eq!(
            marked,
            [
                ("error[E0412]: cannot find type `".to_string(), false),
                ("Vector".to_string(), true),
                ("3d`".to_string(), false),
            ]
        );
        runner.set_filter(|f| {
            f.set_text("");
            f.toggle(Level::Warning);
        });
        assert_eq!(drawn(&runner), ["warning: unused import"]);
        runner.set_filter(|f| f.toggle(Level::Warning));
        assert_eq!(runner.counts(), (4, 4));
        assert_eq!(runner.view().row_data(0).unwrap().pieces.row_count(), 0);
        assert!(runner.all_text().starts_with("$ cargo build\nerror[E0412]"));
        runner.clear();
        assert_eq!(runner.counts(), (0, 0));
    }
}
