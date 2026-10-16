//! Debugging: the firmware booted frozen in the simulator with gdb attached
//! to its gdbstub, or flashed through a probe and debugged on the board by
//! probe-rs's own debug adapter — one handle over both (`rusty_dbg::
//! AnySession`), and one panel.
//!
//! The breakpoints live in the editor's margin (`editor::Gutter`): set
//! before a session they are placed when it attaches, set during one they
//! are placed at once, and a breakpoint the debugger moved to the next line
//! with code moves its dot there. A stop opens the file at the frame and
//! marks the line; the Debug panel lists the stack and the frame's
//! variables; the transport over the editor steps.
//!
//! State is pushed rather than polled: a thread sits on the session's
//! events and hands each state to the window.

use std::{path::PathBuf, rc::Rc, sync::Arc};

use rusty_dbg::{
    AnySession, Board, DapLaunch, DapSession, DebugState, Debugger, Events, Launch, StopReason,
    Target,
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{DebugFrame, DebugVariable, Shell, work};

/// What the debugger holds between states.
#[derive(Default)]
pub(super) struct Debug {
    session: Option<Arc<AnySession>>,
    pub(super) state: Option<DebugState>,
    /// Counts sessions, so a state from one already replaced is dropped.
    epoch: u64,
    /// The standing breakpoints have been placed in this session.
    placed: bool,
    /// Where the last stop was, so a stop is only followed into the editor
    /// once.
    stopped_at: Option<String>,
    /// The session stopped at least once for a reason of its own.
    halted: bool,
}

impl Debug {
    pub(super) fn live(&self) -> bool {
        self.session.is_some()
    }
}

/// QEMU's gdbstub on the plan's port, the CPU frozen at reset, and a clock
/// that counts instructions, so stepping is the same every time.
pub(super) fn gdbstub_args(port: u16) -> Vec<String> {
    vec![
        "-gdb".to_string(),
        format!("tcp::{port}"),
        "-S".to_string(),
        "-icount".to_string(),
        "shift=auto,sleep=on".to_string(),
    ]
}

impl App {
    /// F5: continue a stopped session, or start one — on the board when a
    /// probe is the target and the part has a name probe-rs knows, in the
    /// simulator otherwise.
    pub(super) fn debug_verb(self: &Rc<Self>) {
        let stopped = self
            .state
            .borrow()
            .debug
            .state
            .as_ref()
            .is_some_and(DebugState::stopped);
        if stopped {
            return self.debug_control("resume");
        }
        if self.state.borrow().debug.live() {
            return;
        }
        match self.run_target() {
            Some(super::device::Chosen::Board(rusty_embed::Transport::Probe { identifier })) => {
                self.debug_board(identifier)
            }
            _ => self.simulate_as(true),
        }
    }

    /// The simulator's emulator is up and frozen: attach gdb to it.
    pub(super) fn debug_attach(self: &Rc<Self>, elf: String, port: u16) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || -> Result<(Debugger, Events), String> {
                let firmware = rusty_embed::project::firmware_root(&root);
                let project = rusty_embed::project::detect(&firmware).map_err(|e| e.to_string())?;
                // Which gdb: the chip's architecture decides, and refusing
                // beats guessing — an Xtensa gdb cannot debug a RISC-V image.
                let gdb = rusty_embed::simulate::gdb_for(&project)
                    .ok_or_else(|| t!("desk.debug-no-gdb"))?;
                let launch = Launch {
                    gdb,
                    elf: PathBuf::from(elf),
                    target: Target::Qemu { port },
                    root,
                };
                Debugger::start(&launch).map_err(|e| e.to_string())
            },
            |app, started| match started {
                Ok((debugger, events)) => app.debug_own(AnySession::Gdb(debugger), events),
                Err(reason) => app.note(2, reason),
            },
        );
    }

    /// Build, then flash through the probe and debug on the board.
    fn debug_board(self: &Rc<Self>, probe: Option<String>) {
        if probe.as_deref().is_some_and(|p| p.starts_with("wlink:")) {
            self.note(2, t!("desk.debug-wlink"));
            return;
        }
        self.build(Box::new(move |app, built| {
            if !built {
                return;
            }
            let Some(root) = app.root() else { return };
            let breakpoints = app.editor.gutter().borrow().all();
            work::spawn(
                move || -> Result<(DapSession, Events), String> {
                    let firmware_root = rusty_embed::project::firmware_root(&root);
                    let project =
                        rusty_embed::project::detect(&firmware_root).map_err(|e| e.to_string())?;
                    let chip = project.chip.clone().unwrap_or_default();
                    let target = project
                        .probe_chip
                        .clone()
                        .or_else(|| rusty_embed::chip::by_id(&chip).and_then(|c| c.probe_rs_target))
                        .ok_or_else(|| {
                            rusty_embed::Error::UnknownProbeTarget { chip: chip.clone() }
                                .to_string()
                        })?;
                    let image = rusty_embed::firmware::newest_in_project(&firmware_root)
                        .ok_or_else(|| t!("device.debug-no-image"))?;
                    let launch = DapLaunch {
                        adapter: rusty_embed::tools::find("probe-rs")
                            .unwrap_or_else(|| "probe-rs".into()),
                        program: PathBuf::from(image.path),
                        args: Vec::new(),
                        root,
                        breakpoints,
                        board: Some(Board {
                            chip: target,
                            probe,
                        }),
                    };
                    DapSession::start(&launch).map_err(|e| e.to_string())
                },
                |app, started| match started {
                    Ok((session, events)) => app.debug_own(AnySession::Dap(session), events),
                    Err(reason) => app.note(2, reason),
                },
            );
        }));
    }

    /// A session started: hold it, and read its states on a thread of their
    /// own until it ends.
    pub(super) fn debug_own(self: &Rc<Self>, session: AnySession, events: Events) {
        let epoch = {
            let mut state = self.state.borrow_mut();
            let debug = &mut state.debug;
            debug.epoch += 1;
            debug.session = Some(Arc::new(session));
            debug.state = Some(DebugState::default());
            debug.placed = false;
            debug.stopped_at = None;
            debug.halted = false;
            debug.epoch
        };
        self.show_panel("bottom", "debug");
        self.sync_debug();
        // The Registers tab comes with the session; the chip's SVD is read
        // once.
        if self.state.borrow().registers_unread() {
            self.load_registers();
        }
        std::thread::spawn(move || {
            while let Some(update) = events.next() {
                let ended = update.exited.is_some();
                work::deliver(update, move |app, update| app.debug_heard(epoch, update));
                if ended {
                    break;
                }
            }
            work::deliver((), move |app, ()| app.debug_ended(epoch));
        });
    }

    /// Which session is the debugger's now: what it sends is named by it.
    pub(super) fn debug_epoch(&self) -> u64 {
        self.state.borrow().debug.epoch
    }

    fn debug_heard(self: &Rc<Self>, epoch: u64, update: DebugState) {
        if self.state.borrow().debug.epoch != epoch {
            return;
        }
        // A link where a line names a place, as a run's are: a panic the
        // program wrote goes to the line it happened on.
        for text in &update.output {
            self.say_linked(text);
        }
        self.testing_debug_heard(epoch, &update.output);
        // Attached: the standing breakpoints are placed, and the frozen
        // target runs to them.
        let first = {
            let mut state = self.state.borrow_mut();
            let first = update.attached && !state.debug.placed;
            if first {
                state.debug.placed = true;
            }
            first
        };
        if first {
            let session = self.state.borrow().debug.session.clone();
            let standing = self.editor.gutter().borrow().all();
            if let Some(session) = session {
                std::thread::spawn(move || {
                    for (file, line) in standing {
                        let _ = session.add_breakpoint(&file, line);
                    }
                    let _ = session.resume();
                });
            }
        }
        // A breakpoint the debugger moved to the next line with code moves
        // its dot there; one it could not place is drawn hollow.
        {
            let gutter = self.editor.gutter();
            let mut gutter = gutter.borrow_mut();
            for placed in &update.breakpoints {
                let lines = gutter.breakpoints.entry(placed.file.clone()).or_default();
                if let Some(asked) = placed.requested
                    && asked != placed.line
                {
                    lines.remove(&asked);
                }
                lines.insert(placed.line, placed.verified);
            }
        }
        let follow = {
            let mut state = self.state.borrow_mut();
            let debug = &mut state.debug;
            if update
                .reason
                .is_some_and(|reason| reason != StopReason::Exited)
            {
                debug.halted = true;
            }
            if update.running {
                debug.stopped_at = None;
                None
            } else {
                update.stack.first().and_then(|frame| {
                    let arrived = debug.stopped_at.as_deref() != Some(&frame.address);
                    arrived.then(|| {
                        debug.stopped_at = Some(frame.address.clone());
                        (frame.file.clone(), frame.line)
                    })
                })
            }
        };
        {
            let gutter = self.editor.gutter();
            let mut gutter = gutter.borrow_mut();
            if update.running {
                gutter.stopped = None;
            } else if let Some(frame) = update
                .stack
                .iter()
                .find(|f| f.level == update.frame)
                .or(update.stack.first())
                && let (Some(file), Some(line)) = (&frame.file, frame.line)
            {
                gutter.stopped = Some((file.clone(), line));
            }
        }
        let running = update.running;
        self.state.borrow_mut().debug.state = Some(update);
        if let Some((Some(file), Some(line))) = follow {
            self.open_file_at(file, Some((line as usize, 0)));
        }
        self.editor.gutter_changed();
        self.sync_debug();
        // The registers are read again at every stop: the same address in a
        // loop holds new values each time round.
        if running {
            self.registers_running();
        } else {
            self.read_peripheral();
        }
        self.sync_registers();
    }

    /// Read `bytes` of the target's memory from `address`; the answer comes
    /// back in the session's next state.
    pub(super) fn debug_read(&self, address: u64, bytes: u32) {
        let Some(session) = self.state.borrow().debug.session.clone() else {
            return;
        };
        std::thread::spawn(move || {
            let _ = session.read_memory(address, bytes);
        });
    }

    fn debug_ended(self: &Rc<Self>, epoch: u64) {
        self.testing_debug_ended(epoch);
        let (exited, halted, any) = {
            let state = self.state.borrow();
            if state.debug.epoch != epoch {
                return;
            }
            (
                state.debug.state.as_ref().and_then(|s| s.exited),
                state.debug.halted,
                !self
                    .editor
                    .gutter()
                    .borrow()
                    .breakpoints
                    .values()
                    .all(|l| l.is_empty()),
            )
        };
        // A program that ran to its end without once stopping says so: the
        // panel it leaves behind is the one a session that never started
        // leaves.
        if let Some(code) = exited
            && !halted
        {
            let text = if any {
                t!("dock.debug.ran-past", code = code)
            } else {
                t!("dock.debug.ran-unstopped", code = code)
            };
            self.runner.say(&text, crate::run::kind::WARNING);
        }
        {
            let mut state = self.state.borrow_mut();
            state.debug.session = None;
        }
        self.editor.gutter().borrow_mut().stopped = None;
        self.editor.gutter_changed();
        self.sync_debug();
    }

    /// A step, a continue or a pause: sent off the window's thread, since a
    /// debugger answers when it answers.
    pub(super) fn debug_control(self: &Rc<Self>, action: &'static str) {
        let ready = self.state.borrow().debug.state.as_ref().is_some_and(|s| {
            if action == "pause" {
                s.running
            } else {
                s.stopped()
            }
        });
        let Some(session) = self.state.borrow().debug.session.clone() else {
            return;
        };
        if !ready {
            return;
        }
        std::thread::spawn(move || {
            let _ = match action {
                "pause" => session.pause(),
                "over" => session.step_over(),
                "into" => session.step_into(),
                "out" => session.step_out(),
                _ => session.resume(),
            };
        });
    }

    /// A frame of the stack picked: its variables, and its place.
    pub(super) fn debug_frame(self: &Rc<Self>, level: i32) {
        let Some(session) = self.state.borrow().debug.session.clone() else {
            return;
        };
        let level = level.max(0) as u32;
        let place = self.state.borrow().debug.state.as_ref().and_then(|s| {
            let frame = s.stack.iter().find(|f| f.level == level)?;
            Some((frame.file.clone()?, frame.line?))
        });
        std::thread::spawn(move || {
            let _ = session.refresh(level);
        });
        if let Some((file, line)) = place {
            self.open_file_at(file, Some((line as usize, 0)));
        }
    }

    /// Stop debugging, and the run that booted the target.
    pub(super) fn debug_stop(self: &Rc<Self>) {
        let session = {
            let mut state = self.state.borrow_mut();
            state.debug.epoch += 1;
            state.debug.state = None;
            state.debug.session.take()
        };
        if let Some(session) = session {
            std::thread::spawn(move || session.stop());
        }
        self.editor.gutter().borrow_mut().stopped = None;
        self.editor.gutter_changed();
        self.sync_debug();
    }

    /// A click beside a line's number: a breakpoint there, or not — and,
    /// during a session, placed or cleared at once.
    pub(super) fn debug_toggle(self: &Rc<Self>, file: String, line: u32) {
        // A macro's expansion is no file a debugger has lines of.
        if rusty_edit::is_expansion(&file) {
            return;
        }
        let existed = {
            let gutter = self.editor.gutter();
            let mut gutter = gutter.borrow_mut();
            let lines = gutter.breakpoints.entry(file.clone()).or_default();
            let existed = lines.remove(&line).is_some();
            if !existed {
                lines.insert(line, true);
            }
            existed
        };
        self.editor.gutter_changed();
        let (session, number) = {
            let state = self.state.borrow();
            let number = state.debug.state.as_ref().and_then(|s| {
                s.breakpoints
                    .iter()
                    .find(|b| b.file == file && b.line == line)
                    .and_then(|b| b.number)
            });
            (state.debug.session.clone(), number)
        };
        if let Some(session) = session {
            std::thread::spawn(move || {
                let _ = if existed {
                    number.map_or(Ok(()), |number| session.remove_breakpoint(number))
                } else {
                    session.add_breakpoint(&file, line)
                };
            });
        }
    }

    pub(super) fn sync_debug(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let debug = &state.debug;
        let live = debug.live();
        let session = debug.state.as_ref();
        shell.set_debug_live(live);
        shell.set_debug_running(session.is_some_and(|s| s.running));
        shell.set_debug_stopped(session.is_some_and(DebugState::stopped));
        let status = match session {
            None => t!("dock.debug.idle"),
            Some(s) if s.exited.is_some() => {
                t!("desk.debug-exited", code = s.exited.unwrap_or_default())
            }
            Some(s) if !s.attached => t!("desk.debug-attaching"),
            Some(s) if s.running => t!("dock.debug.running"),
            Some(s) => match s.reason {
                Some(StopReason::Breakpoint) => t!("desk.debug-at-breakpoint"),
                Some(StopReason::Step) => t!("desk.debug-stepped"),
                Some(StopReason::Pause) => t!("desk.debug-paused"),
                Some(StopReason::Signal) => t!("desk.debug-signal"),
                _ => t!("desk.debug-stopped"),
            },
        };
        shell.set_debug_status(SharedString::from(status));
        shell.set_debug_error(SharedString::from(
            session.and_then(|s| s.error.clone()).unwrap_or_default(),
        ));
        let frames: Vec<DebugFrame> = session
            .map(|s| {
                s.stack
                    .iter()
                    .map(|frame| DebugFrame {
                        level: frame.level as i32,
                        function: frame.function.as_str().into(),
                        place: match (&frame.file, frame.line) {
                            (Some(file), Some(line)) => format!("{file}:{}", line + 1),
                            _ => frame.address.clone(),
                        }
                        .into(),
                        on: frame.level == s.frame,
                    })
                    .collect()
            })
            .unwrap_or_default();
        shell.set_debug_frames(ModelRc::new(VecModel::from(frames)));
        let variables: Vec<DebugVariable> = session
            .map(|s| {
                s.variables
                    .iter()
                    .map(|v| DebugVariable {
                        name: v.name.as_str().into(),
                        value: v.value.as_str().into(),
                        kind: v.kind.clone().unwrap_or_default().into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        shell.set_debug_variables(ModelRc::new(VecModel::from(variables)));
    }
}
