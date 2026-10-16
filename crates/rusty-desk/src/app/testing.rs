//! Tests and examples, run from the editor's lenses and the title bar.
//!
//! A test runs where the testable crates are — the opened project, never
//! the firmware crate, which has no test harness — as `cargo test <filter>
//! -- --show-output`, rust-analyzer's own default for its lens: each test's
//! output and its panic printed under its name at the end, rather than
//! among every other test's lines as they happen. What the run says is read
//! as it goes (`rusty_edit::testrun`) and each lens carries a tick or a
//! cross until the next run of it.
//!
//! A failed test's panic is marked where it happened (`editor/failed.rs`):
//! the place as rustc printed it, resolved against the workspace's root —
//! which cargo is asked for as the run starts, since rustc runs there and
//! names every member's file from there — and then made the project's.
//! A place that is not one of the project's files is not marked; a run
//! whose workspace cargo could not name marks nothing.
//!
//! Debug builds the test binaries visibly, asks each binary which holds the
//! test (`rusty_embed::host_debug` says why the path cannot), and runs that
//! one under gdb — or LLDB's debug adapter where the debug information is a
//! PDB, which gdb cannot read.

use std::{collections::HashMap, path::PathBuf, rc::Rc};

use rusty_dbg::{AnySession, DapLaunch, DapSession, Debugger, Events, Launch, Target};
use rusty_edit::testrun::{Failure, Heard, Reader, Verdict};
use rusty_edit::{Runnable, RunnableKind};
use rusty_embed::{CommandPlan, host_debug};
use rusty_i18n::t;

use super::App;
use crate::{editor::Failed, run::Ended, work};

#[derive(Default)]
pub(super) struct Testing {
    /// Reading a test run's output while one goes.
    reader: Option<Reader>,
    /// Every test's last verdict, by the harness's name for it.
    results: HashMap<String, Verdict>,
    /// Which run is being read: cargo's answer about the workspace is for
    /// one run, and an answer for another is dropped.
    run: u64,
    /// The directory rustc ran in for that run — the workspace's root —
    /// once cargo has said it; `Some(None)` when it could not.
    base: Option<Option<PathBuf>>,
    /// Failures heard before cargo said it.
    pending: Vec<Failure>,
    /// The debug session a test is being run under, by its epoch: what
    /// the program prints is read as a test run's output is.
    debugged: Option<u64>,
}

impl App {
    /// A lens's verb.
    pub(super) fn lens(self: &Rc<Self>, runnable: Runnable, verb: &str) {
        match (runnable.kind, verb) {
            (RunnableKind::Example, _) => self.run_example(runnable.filter),
            (_, "debug") => self.debug_test(runnable.filter),
            _ => self.run_test(runnable.filter),
        }
    }

    /// Where tests run: the opened project — refused, out loud, where that
    /// is the firmware itself and `cargo test` would build for the chip.
    fn test_root(self: &Rc<Self>) -> Option<PathBuf> {
        let root = self.root()?;
        let blocked = self
            .state
            .borrow()
            .project
            .as_ref()
            .and_then(|p| p.embedded.as_ref().ok())
            .is_some_and(|e| e.root_is_firmware());
        if blocked {
            self.runner
                .say(&t!("toolbar.test-blocked"), crate::run::kind::ERROR);
            self.show_panel("bottom", "output");
            self.note(2, t!("toolbar.test-blocked"));
            return None;
        }
        Some(root)
    }

    /// One test or module of them, or the whole suite for an empty filter —
    /// every draft written first, since what runs is what is on screen.
    pub(super) fn run_test(self: &Rc<Self>, filter: String) {
        let Some(root) = self.test_root() else { return };
        self.write_all_then(move |app| {
            let mut args = vec!["test".to_string()];
            if !filter.is_empty() {
                args.push(filter.clone());
                args.extend(["--".into(), "--show-output".into()]);
            }
            let plan = CommandPlan::new("cargo", args, String::new());
            // A run of its own in an empty dock, as rust-analyzer's
            // runnables clear their terminal.
            app.runner.clear();
            app.tests_begin(&root);
            app.run(&plan, Some(root), |app, _| app.test_ended());
        });
    }

    /// A run of tests begins, from what is on disk now: its output read
    /// afresh, and cargo asked where the workspace's root is — beside the
    /// run, which is building meanwhile — so a panic's place can be named.
    fn tests_begin(self: &Rc<Self>, root: &std::path::Path) {
        let run = {
            let mut state = self.state.borrow_mut();
            let testing = &mut state.testing;
            testing.run += 1;
            testing.base = None;
            testing.pending.clear();
            testing.reader = Some(Reader::default());
            // The reader is this run's: a test still being debugged is
            // read no more.
            testing.debugged = None;
            testing.run
        };
        self.editor.tests_began();
        let dir = root.to_path_buf();
        work::spawn(
            move || host_debug::workspace_root(&dir).ok(),
            move |app, base| {
                {
                    let mut state = app.state.borrow_mut();
                    if state.testing.run != run {
                        return;
                    }
                    state.testing.base = Some(base);
                }
                app.tests_place();
            },
        );
    }

    /// Every failure heard and not yet marked, marked now that the
    /// workspace's root is known: where its place is one of the project's
    /// files, and nowhere when it is not, or when cargo named no root.
    fn tests_place(&self) {
        let (base, pending) = {
            let mut state = self.state.borrow_mut();
            let Some(base) = state.testing.base.clone() else {
                return;
            };
            (base, std::mem::take(&mut state.testing.pending))
        };
        let (Some(base), Some(root)) = (base, self.root()) else {
            return;
        };
        let (base, root) = (base.to_string_lossy(), root.to_string_lossy());
        for failure in pending {
            let Some((printed, line, col)) = failure.at else {
                continue;
            };
            let Some(path) = rusty_edit::testrun::file_in(&printed, &base, &root) else {
                continue;
            };
            self.editor.test_failed(Failed {
                test: failure.test,
                path,
                line,
                col,
                message: failure.message,
            });
        }
    }

    /// Another project: the verdicts and the marks were about this one's
    /// files, and an answer still coming for its run is dropped.
    pub(super) fn forget_tests(&self) {
        {
            let mut state = self.state.borrow_mut();
            let testing = &mut state.testing;
            testing.run += 1;
            testing.reader = None;
            testing.results.clear();
            testing.base = None;
            testing.pending.clear();
            testing.debugged = None;
        }
        self.editor.forget_tests();
    }

    fn run_example(self: &Rc<Self>, name: String) {
        let Some(root) = self.root() else { return };
        self.write_all_then(move |app| {
            let plan = CommandPlan::new(
                "cargo",
                vec!["run".into(), "--example".into(), name],
                String::new(),
            );
            app.runner.clear();
            app.run(&plan, Some(root), |_, _| {});
        });
    }

    /// A line of a test run: its verdicts kept, the lenses redrawn.
    pub(super) fn testing_heard(&self, text: &str) {
        let heard = {
            let mut state = self.state.borrow_mut();
            let Some(reader) = state.testing.reader.as_mut() else {
                return;
            };
            reader.read(text)
        };
        self.test_heard(heard);
    }

    /// What a debugged test printed: read as a run's lines are, when the
    /// session is the one a test's Debug started.
    pub(super) fn testing_debug_heard(&self, epoch: u64, lines: &[String]) {
        if self.state.borrow().testing.debugged != Some(epoch) {
            return;
        }
        for text in lines {
            self.testing_heard(text);
        }
    }

    /// That session ended: the run with it.
    pub(super) fn testing_debug_ended(self: &Rc<Self>, epoch: u64) {
        let ours = {
            let mut state = self.state.borrow_mut();
            let ours = state.testing.debugged == Some(epoch);
            if ours {
                state.testing.debugged = None;
            }
            ours
        };
        if ours {
            self.test_ended();
        }
    }

    fn test_ended(self: &Rc<Self>) {
        let heard = self
            .state
            .borrow_mut()
            .testing
            .reader
            .take()
            .map(|mut reader| reader.finish())
            .unwrap_or_default();
        self.test_heard(heard);
    }

    fn test_heard(&self, heard: Vec<Heard>) {
        let mut verdicts = Vec::new();
        let mut failed = false;
        {
            let mut state = self.state.borrow_mut();
            for item in heard {
                match item {
                    Heard::Verdict(name, verdict) => {
                        state.testing.results.insert(name.clone(), verdict);
                        verdicts.push(name);
                    }
                    Heard::Failure(failure) => {
                        state.testing.pending.push(failure);
                        failed = true;
                    }
                }
            }
        }
        if !verdicts.is_empty() {
            // A verdict takes away what an earlier run marked for the test
            // — a test that passes now is marked no more.
            for name in &verdicts {
                self.editor.test_verdict(name);
            }
            let results = self.state.borrow().testing.results.clone();
            self.editor.set_verdicts(results);
        }
        if failed {
            self.tests_place();
        }
    }

    /// Debug one test on this machine: refusals before the build, the build
    /// in the dock, the binary that holds the test found by asking, and that
    /// binary run under the debugger with the margin's breakpoints.
    pub(super) fn debug_test(self: &Rc<Self>, filter: String) {
        let Some(root) = self.test_root() else { return };
        let probe = root.clone();
        work::spawn(
            move || -> Result<Backend, String> {
                let host = host_debug::host_triple(&probe).map_err(|e| e.to_string())?;
                match host_debug::gdb_reads(&host) {
                    Ok(()) => host_debug::host_gdb()
                        .map(Backend::Gdb)
                        .ok_or_else(|| t!("desk.debug-no-host-gdb")),
                    Err(why) => {
                        let adapters = host_debug::host_adapters();
                        if adapters.is_empty() {
                            Err(why.to_string())
                        } else {
                            Ok(Backend::Adapters(adapters))
                        }
                    }
                }
            },
            move |app, backend| {
                let backend = match backend {
                    Ok(backend) => backend,
                    Err(why) => return app.note(2, why),
                };
                let plan = host_debug::build_plan();
                let built_root = root.clone();
                app.write_all_then(move |app| {
                    app.runner.clear();
                    app.run(&plan, Some(built_root), move |app, ended| {
                        if ended != Ended::Done {
                            return;
                        }
                        let breakpoints = app.editor.gutter().borrow().all();
                        let read_in = root.clone();
                        work::spawn(
                            move || start_host(&root, &filter, backend, breakpoints),
                            move |app, started| match started {
                                Ok((session, events)) => {
                                    // Read as a run of the test is: its
                                    // verdict on the lens, its panic on
                                    // its line, where the program prints
                                    // them.
                                    app.tests_begin(&read_in);
                                    app.debug_own(session, events);
                                    let epoch = app.debug_epoch();
                                    app.state.borrow_mut().testing.debugged = Some(epoch);
                                }
                                Err(why) => app.note(2, why),
                            },
                        );
                    });
                });
            },
        );
    }
}

/// Which debugger reads what this machine's toolchain builds.
enum Backend {
    Gdb(PathBuf),
    Adapters(Vec<PathBuf>),
}

/// The test's binary found and started under the debugger: one thread, so
/// a step is a step and not a switch to whichever test ran next, and
/// `--nocapture` because what the test prints is usually why it is being
/// run this way.
fn start_host(
    root: &std::path::Path,
    filter: &str,
    backend: Backend,
    breakpoints: Vec<(String, u32)>,
) -> Result<(AnySession, Events), String> {
    let json = host_debug::built_json(root).map_err(|e| e.to_string())?;
    let built = host_debug::test_executables(&json);
    let exe = host_debug::binary_holding(&built, filter).map_err(|e| e.to_string())?;
    let args = vec![
        filter.to_string(),
        "--nocapture".to_string(),
        "--test-threads=1".to_string(),
    ];
    match backend {
        Backend::Gdb(gdb) => {
            let launch = Launch {
                gdb,
                elf: exe,
                target: Target::Host { args },
                root: root.to_path_buf(),
            };
            Debugger::start(&launch)
                .map(|(session, events)| (AnySession::Gdb(session), events))
                .map_err(|e| e.to_string())
        }
        // Each in turn: "the adapter exists" and "the adapter answers" are
        // different facts, and the first that talks is the one used.
        Backend::Adapters(adapters) => {
            let mut refused = Vec::new();
            for adapter in adapters {
                let launch = DapLaunch {
                    adapter: adapter.clone(),
                    program: exe.clone(),
                    args: args.clone(),
                    root: root.to_path_buf(),
                    breakpoints: breakpoints.clone(),
                    board: None,
                };
                match DapSession::start(&launch) {
                    Ok((session, events)) => return Ok((AnySession::Dap(session), events)),
                    Err(e) => refused.push(format!("{}: {e}", adapter.display())),
                }
            }
            Err(t!("desk.debug-no-adapter", tried = refused.join("\n")))
        }
    }
}
