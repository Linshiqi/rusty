//! What the window runs — an install, a build, a flash — one at a time,
//! its lines into the Output dock as they come and its end said.
//!
//! One at a time because what it runs fights over the same locks: two
//! `cargo install`s over the package cache, a build over the build
//! directory, a flash over the port. A second run asked for while one is
//! going waits for the first to be stopped; the caller decides whether to.

use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
};

use rusty_embed::{CommandPlan, process::Stopper};
use slint::{Model, VecModel};

use crate::{OutputLine, app::App, work};

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

/// How many lines the dock keeps; the oldest go first.
const KEPT: usize = 20_000;

struct Running {
    generation: u64,
    stopper: Stopper,
    stopped: Cell<bool>,
}

pub struct Runner {
    output: Rc<VecModel<OutputLine>>,
    running: RefCell<Option<Running>>,
    generation: Cell<u64>,
}

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

impl Runner {
    pub fn new(output: Rc<VecModel<OutputLine>>) -> Runner {
        Runner {
            output,
            running: RefCell::default(),
            generation: Cell::new(0),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.borrow().is_some()
    }

    /// A line in the dock that no process printed: a command line, how a
    /// run ended, what rusty did between two runs.
    pub fn say(&self, text: &str, kind: i32) {
        self.output.push(OutputLine {
            text: text.into(),
            kind,
        });
        let over = self.output.row_count().saturating_sub(KEPT);
        if over > 0 {
            // From the front, in one go: one removal per line kept the
            // model busy longer than the run that printed them.
            let kept: Vec<OutputLine> = self.output.iter().skip(over).collect();
            self.output.set_vec(kept);
        }
    }

    pub fn clear(&self) {
        self.output.set_vec(Vec::new());
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
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.say(&format!("$ {}", plan.display), kind::COMMAND);
        let session = match rusty_embed::process::spawn(plan, dir.as_deref()) {
            Ok(session) => session,
            Err(error) => {
                self.say(&error.to_string(), kind::ERROR);
                then(&work::app(), Ended::Refused);
                return true;
            }
        };
        *self.running.borrow_mut() = Some(Running {
            generation,
            stopper: session.stopper(),
            stopped: Cell::new(false),
        });
        std::thread::Builder::new()
            .name("run".into())
            .spawn(move || {
                while let Some(line) = session.recv() {
                    work::deliver(plain(&line.text), |app, text| {
                        app.runner().say(&text, kind_of(&text));
                    });
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
        true
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
}
