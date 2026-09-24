//! A live gdb, driven.
//!
//! One process and one reader thread. Commands carry a token and answers
//! quote it back — MI's own mechanism, so nothing here has to guess which
//! `^done` belongs to which request, which is what makes stepping reliable
//! while output is still arriving from the last continue.
//!
//! The session owns *interpretation* as well as transport: what the panel
//! receives is a [`DebugState`], not a pile of records. Somebody has to turn
//! `*stopped,reason="breakpoint-hit",frame={…}` into "line 68 of main.rs",
//! and doing it here means the frontend cannot get it subtly wrong in its
//! own way.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::mi::{self, Record, Value};
use crate::model::{Breakpoint, DebugState, MemoryRead, StackFrame, StopReason, Variable};

/// What gdb connects to — or, for a program built for this machine, starts.
#[derive(Debug, Clone)]
pub enum Target {
    /// Espressif QEMU's gdbstub, frozen at reset by `-s -S`.
    Qemu { port: u16 },
    /// probe-rs serving gdb for real hardware.
    Probe { port: u16 },
    /// A program for this machine — a test binary — that gdb runs itself,
    /// with these arguments. Nothing is listening: the first resume is
    /// `-exec-run` rather than `-exec-continue`, the program's own stdout
    /// arrives in the pipe beside gdb's records, and its exit ends the
    /// session, since there is no target left to talk to.
    Host { args: Vec<String> },
}

impl Target {
    fn is_host(&self) -> bool {
        matches!(self, Target::Host { .. })
    }
}

/// `-exec-arguments` takes the rest of the line, split the way a shell
/// would, so an argument with a space or a quote in it is quoted.
fn exec_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(|c| c.is_whitespace() || c == '"') {
                format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What "resume" means: a host program that has not started yet is started,
/// and everything else — a remote target frozen at reset, a program already
/// running — is continued. `-exec-continue` against a program that was never
/// run is gdb's "The program is not being run.", which the panel would show
/// as an error on the one button that had to work.
fn resume_command(first_host_run: bool) -> &'static str {
    if first_host_run {
        "-exec-run"
    } else {
        "-exec-continue"
    }
}

/// A session's inputs: which gdb, which ELF, what to connect to.
#[derive(Debug, Clone)]
pub struct Launch {
    pub gdb: PathBuf,
    pub elf: PathBuf,
    pub target: Target,
    /// Where source paths are relative to — the project root.
    pub root: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not start {gdb}: {source}")]
    Spawn {
        gdb: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the debugger's input is closed")]
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;

/// Everything the panel needs, pushed as it changes.
pub struct Events(Receiver<DebugState>);

impl Events {
    /// Block until the next state. `None` once the session is over.
    pub fn next(&self) -> Option<DebugState> {
        self.0.recv().ok()
    }

    /// For the other backend: both sessions push the same states, so both
    /// hand the caller the same stream.
    pub(crate) fn new(receiver: Receiver<DebugState>) -> Self {
        Self(receiver)
    }
}

/// The write half of the session, shared with the reader.
///
/// A stop is a question the session answers for itself — "what is the
/// stack, what do the locals hold" — so the thread that notices the stop
/// has to be able to ask. Leaving that to the UI meant one round trip per
/// stop and, until something asked, a call stack one frame deep.
struct Wire {
    stdin: Mutex<Option<ChildStdin>>,
    token: AtomicU32,
}

impl Wire {
    fn send(&self, command: &str) -> Result<()> {
        let token = self.token.fetch_add(1, Ordering::Relaxed);
        let mut slot = self.stdin.lock().expect("stdin");
        let stdin = slot.as_mut().ok_or(Error::Closed)?;
        writeln!(stdin, "{token}{command}").map_err(|_| Error::Closed)?;
        stdin.flush().map_err(|_| Error::Closed)
    }
}

pub struct Debugger {
    child: Mutex<Child>,
    wire: Arc<Wire>,
    state: Arc<Mutex<DebugState>>,
    /// A program gdb runs itself, as opposed to a target it attached to.
    host: bool,
    /// Whether that program has been started — the first resume is the run.
    launched: AtomicBool,
    /// Whether `stop` has run. It runs once, whether asked for or on drop.
    stopped: AtomicBool,
}

impl Debugger {
    /// Start gdb, load the ELF, attach to the target and stop at `main`.
    ///
    /// `--interpreter=mi3` from the first byte: gdb's human interface and
    /// its machine interface are different languages, and asking for MI
    /// after the fact means parsing a banner in the first one.
    pub fn start(launch: &Launch) -> Result<(Self, Events)> {
        let mut command = Command::new(&launch.gdb);
        command
            .arg("--interpreter=mi3")
            .arg("--quiet")
            .arg(&launch.elf)
            .current_dir(&launch.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Not piped: nothing read it, so a gdb with something to say on
            // stderr — a Python warning per startup, a remote-protocol
            // complaint per step — filled the pipe and then blocked on it,
            // and the session hung with every MI answer still to come.
            .stderr(Stdio::null());
        // The rustup shim exports this for rusty's own build, and gdb has
        // no business inheriting it — the same leak that made a spawned
        // cargo compile an esp project with stable.
        command.env_remove("RUSTUP_TOOLCHAIN");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let mut child = command.spawn().map_err(|source| Error::Spawn {
            gdb: launch.gdb.display().to_string(),
            source,
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped");

        let state = Arc::new(Mutex::new(DebugState::default()));
        let (sender, receiver) = channel();

        let wire = Arc::new(Wire {
            stdin: Mutex::new(stdin),
            token: AtomicU32::new(1),
        });

        let host = launch.target.is_host();
        let initial = sender.clone();
        {
            let state = Arc::clone(&state);
            let wire = Arc::clone(&wire);
            let root = launch.root.clone();
            std::thread::spawn(move || pump(stdout, state, wire, sender, root, host));
        }

        let debugger = Self {
            child: Mutex::new(child),
            wire,
            state,
            host,
            launched: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        };

        // Connect, and stop before anything runs. `set` lines first: a
        // pagination prompt from gdb inside MI is a session that hangs
        // waiting for a keypress nobody can send.
        debugger.send("-gdb-set mi-async on")?;
        debugger.send("-gdb-set pagination off")?;
        debugger.send("-gdb-set confirm off")?;
        match &launch.target {
            Target::Qemu { port } | Target::Probe { port } => {
                debugger.send(&format!("-target-select extended-remote localhost:{port}"))?;
            }
            Target::Host { args } => {
                // Nothing to connect to: gdb has the binary from its command
                // line and starts it on the first resume. So the session is
                // attached from the start, and says so now rather than after
                // the first record, because the frontend places its
                // breakpoints on that word and then asks for the run — and
                // a breakpoint placed after `-exec-run` is one the program
                // may already have run past.
                debugger.send(&format!("-exec-arguments {}", exec_arguments(args)))?;
                let mut state = debugger.state.lock().expect("state");
                state.attached = true;
                publish(&mut state, &initial);
            }
        }
        Ok((debugger, Events(receiver)))
    }

    /// The state as it stands, for a caller that missed the last push.
    pub fn state(&self) -> DebugState {
        self.state.lock().expect("state").clone()
    }

    /// Place a breakpoint. Lines cross this boundary zero-based; gdb counts
    /// from one, and the conversion belongs at exactly one edge.
    pub fn add_breakpoint(&self, file: &str, line: u32) -> Result<()> {
        self.send(&format!("-break-insert {file}:{}", line + 1))
    }

    pub fn remove_breakpoint(&self, number: u32) -> Result<()> {
        self.send(&format!("-break-delete {number}"))
    }

    pub fn resume(&self) -> Result<()> {
        let first_host_run = self.host && !self.launched.swap(true, Ordering::SeqCst);
        self.send(resume_command(first_host_run))
    }

    pub fn pause(&self) -> Result<()> {
        self.send("-exec-interrupt")
    }

    /// Over the next source line, staying in this frame.
    pub fn step_over(&self) -> Result<()> {
        self.send("-exec-next")
    }

    /// Into the call on this line, if it has source.
    pub fn step_into(&self) -> Result<()> {
        self.send("-exec-step")
    }

    /// Out of this frame, stopping where it returns.
    pub fn step_out(&self) -> Result<()> {
        self.send("-exec-finish")
    }

    /// Select a frame and ask for the stack and that frame's variables —
    /// what choosing a row of the stack does; a stop asks for its own. The
    /// answers arrive as records and land in the state.
    ///
    /// The panel marks the row `frame` names beside the variables shown, so
    /// the two have to be about the same frame, and no answer from gdb says
    /// which frame it is about. So the marker moves here, once every request
    /// has gone. Never set, it stayed on the innermost frame while the
    /// variables beside it were another's; a refresh gdb could not be sent
    /// leaves it where it was.
    pub fn refresh(&self, frame: u32) -> Result<()> {
        self.send(&format!("-stack-select-frame {frame}"))?;
        self.send("-stack-list-frames")?;
        // `--all-values` rather than names alone: a variables panel that
        // needs a round trip per row updates one row at a time on a target
        // that is already slow.
        self.send("-stack-list-variables --all-values")?;
        self.state.lock().expect("state").frame = frame;
        Ok(())
    }

    /// Read a span of target memory — a peripheral's register block, for
    /// the register view. Only while stopped: gdb refuses otherwise, and
    /// a half-read block would decode into fiction.
    pub fn read_memory(&self, address: u64, bytes: u32) -> Result<()> {
        self.send(&format!("-data-read-memory-bytes 0x{address:x} {bytes}"))
    }

    /// End the session and the target with it — once: a session stopped and
    /// then dropped is not stopped again.
    ///
    /// Killed is not gone. A killed gdb nobody waits for stays a zombie on
    /// Unix for as long as rusty runs, so it is waited for, and `stop`
    /// returns with gdb's end in hand.
    pub fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.send("-gdb-exit");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn send(&self, command: &str) -> Result<()> {
        self.wire.send(command)
    }
}

/// A session let go of without `stop` takes gdb with it. The app lets go
/// of one when its stream ends — after a host program's exit, say, which
/// told gdb to quit and then never waited for it — and a start that fails
/// lets go of the one it was building.
impl Drop for Debugger {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Read gdb's stdout for the life of the session, folding records into the
/// shared state and pushing a copy after each one that changed something.
fn pump(
    stdout: std::process::ChildStdout,
    state: Arc<Mutex<DebugState>>,
    wire: Arc<Wire>,
    sender: Sender<DebugState>,
    root: PathBuf,
    host: bool,
) {
    let reader = BufReader::new(stdout);
    for line in reader.lines().map_while(std::result::Result::ok) {
        let Some(record) = mi::parse(&line) else {
            // Not a record. On a host run that is the program's own stdout:
            // it inherits gdb's, which is this pipe, so a test's `println!`
            // arrives here between the records. Forwarded as `output`, one
            // push per line and cleared once sent, because the reason to
            // run a test under a debugger rather than read its assertion is
            // usually what it prints on the way there.
            let printed = host && !line.trim().is_empty() && line.trim() != "(gdb)";
            if printed && !push_line(&state, &sender, line) {
                break;
            }
            continue;
        };
        // A stop is only half an answer: it names one frame. The rest of
        // the stack and the frame's variables are separate questions, and
        // asking them here means a panel that is complete the moment it
        // appears rather than one round trip later.
        if let Record::Exec { class, fields } = &record
            && class == "stopped"
            && Value::Tuple(fields.clone()).field("reason") != Some("exited-normally")
        {
            let _ = wire.send("-stack-list-frames");
            let _ = wire.send("-stack-list-variables --all-values");
        }
        let mut current = state.lock().expect("state");
        if apply(&mut current, &record, &root) && !publish(&mut current, &sender) {
            break;
        }
        let exited = current.exited.is_some();
        drop(current);
        // A host program that has exited leaves gdb with nothing to debug;
        // a remote target that stopped with an exit code is QEMU's or the
        // probe's to end. Quitting here is what closes this pipe and lets
        // the loop below report the session over.
        if host && exited {
            let _ = wire.send("-gdb-exit");
        }
    }
    // gdb is gone; say so once so the panel stops offering to step. A target
    // that exited said so itself, code and all; a gdb that went away without
    // that is *not* a clean exit, and inventing `Some(0)` here read a crashed
    // debugger as a program that finished normally.
    let mut state = state.lock().expect("state");
    state.running = false;
    state.attached = false;
    if state.exited.is_none() && state.error.is_none() {
        state.error = Some("gdb ended the session without reporting an exit".to_string());
    }
    publish(&mut state, &sender);
}

/// Send the state as it stands, and with it every line printed since the
/// last state went out.
///
/// Every state either debugger sends goes this way. `output` in the shared
/// state is what has been printed and not yet sent, and the state that goes
/// out takes it — so a line travels in exactly one state, whichever thread
/// sends it: gdb's reader, or the DAP session's reader and console thread.
/// A line left behind went out again with every later state, and the dock
/// printed an adapter's output once more at every step. The caller holds
/// the lock across the send, so states leave in the order they were taken.
pub(crate) fn publish(state: &mut DebugState, sender: &Sender<DebugState>) -> bool {
    let output = std::mem::take(&mut state.output);
    sender
        .send(DebugState {
            output,
            ..state.clone()
        })
        .is_ok()
}

/// One line the program printed, sent at once. False once nobody is
/// listening.
pub(crate) fn push_line(
    state: &Mutex<DebugState>,
    sender: &Sender<DebugState>,
    line: String,
) -> bool {
    let mut state = state.lock().expect("state");
    state.output.push(line);
    publish(&mut state, sender)
}

/// Fold one record into the state. Returns whether anything changed.
fn apply(state: &mut DebugState, record: &Record, root: &Path) -> bool {
    match record {
        Record::Exec { class, fields } => {
            let value = Value::Tuple(fields.clone());
            if class == "running" {
                state.attached = true;
                state.resumed();
                return true;
            }
            if class == "stopped" {
                state.halted(reason_of(value.field("reason").unwrap_or_default()));
                // gdb prints `exit-code` in *octal* — `exit-code="012"` is
                // ten — as the MI manual says and as nothing about the field
                // suggests. Read as decimal, exit 10 was reported as 12.
                if let Some(code) = value.field("exit-code") {
                    state.exited = i32::from_str_radix(code, 8).ok();
                }
                if value.field("reason") == Some("exited-normally") {
                    state.exited = Some(0);
                }
                // The stop's own frame, so the caret can move before the
                // full stack arrives.
                if let Some(frame) = value.get("frame")
                    && let Some(frame) = frame_of(frame, root)
                {
                    state.stack = vec![frame];
                }
                return true;
            }
            false
        }
        Record::Result { class, fields, .. } => {
            let value = Value::Tuple(fields.clone());
            match class.as_str() {
                "error" => {
                    state.error = value.field("msg").map(str::to_string);
                    true
                }
                "done" | "connected" => {
                    let mut changed = false;
                    if let Some(bkpt) = value.get("bkpt") {
                        upsert_breakpoint(state, bkpt, root);
                        changed = true;
                    }
                    if let Some(stack) = value.get("stack") {
                        state.stack = stack
                            .items()
                            .iter()
                            .filter_map(|item| item.get("frame"))
                            .filter_map(|frame| frame_of(frame, root))
                            .collect();
                        changed = true;
                    }
                    if let Some(memory) = value.get("memory") {
                        state.memory = memory.items().iter().filter_map(memory_of).collect();
                        changed = true;
                    }
                    if let Some(variables) = value.get("variables") {
                        state.variables =
                            variables.items().iter().filter_map(variable_of).collect();
                        changed = true;
                    }
                    if class == "connected" {
                        state.attached = true;
                        changed = true;
                    }
                    changed
                }
                "running" => {
                    state.running = true;
                    true
                }
                _ => false,
            }
        }
        Record::Notify { class, fields } => {
            // `-break-delete` answers a bare `^done`; the deletion itself
            // arrives as this notification. Without handling it the list kept
            // every breakpoint ever placed, and the panel only survived
            // because the frontend keeps its own copy.
            if class == "breakpoint-deleted" {
                let value = Value::Tuple(fields.clone());
                if let Some(id) = value.field("id").and_then(|id| id.parse::<u32>().ok()) {
                    let before = state.breakpoints.len();
                    state.breakpoints.retain(|b| b.number != Some(id));
                    return state.breakpoints.len() != before;
                }
                return false;
            }
            if class == "breakpoint-modified" {
                let value = Value::Tuple(fields.clone());
                if let Some(bkpt) = value.get("bkpt") {
                    upsert_breakpoint(state, bkpt, root);
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn reason_of(reason: &str) -> StopReason {
    match reason {
        "breakpoint-hit" => StopReason::Breakpoint,
        "end-stepping-range" | "function-finished" => StopReason::Step,
        "signal-received" => StopReason::Signal,
        "exited" | "exited-normally" | "exited-signalled" => StopReason::Exited,
        "interrupted" | "" => StopReason::Pause,
        _ => StopReason::Other,
    }
}

/// gdb's frame tuple, in this workbench's terms.
fn frame_of(frame: &Value, root: &Path) -> Option<StackFrame> {
    let (file, line) = location_of(frame, root);
    Some(StackFrame {
        level: frame
            .field("level")
            .and_then(|l| l.parse().ok())
            .unwrap_or(0),
        function: frame.field("func").unwrap_or("??").to_string(),
        file,
        line,
        address: frame.field("addr").unwrap_or_default().to_string(),
    })
}

/// Where a frame or a breakpoint is: the file project-relative — from
/// `fullname` when gdb gave one, from `file` in one spelling when it did
/// not — and the line zero-based. gdb counts lines from one; everything
/// this side of the boundary counts from zero.
fn location_of(record: &Value, root: &Path) -> (Option<String>, Option<u32>) {
    let file = record
        .field("fullname")
        .map(|full| relative(full, root))
        .or_else(|| record.field("file").map(normalise));
    let line = record
        .field("line")
        .and_then(|l| l.parse::<u32>().ok())
        .map(|line| line.saturating_sub(1));
    (file, line)
}

/// `{begin="0x3ff44004",contents="0400000f"}` — hex pairs, little-endian
/// as the target holds them.
fn memory_of(item: &Value) -> Option<MemoryRead> {
    // The element may be the tuple itself or a `memory={…}` field wearing
    // list brackets, depending on gdb's mood about `-data-read-memory-bytes`.
    let tuple = item.get("memory").unwrap_or(item);
    let begin = tuple.field("begin")?;
    let begin = u64::from_str_radix(begin.trim_start_matches("0x"), 16).ok()?;
    let contents = tuple.field("contents")?;
    let data = contents
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            let text = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(text, 16).ok()
        })
        .collect();
    Some(MemoryRead { begin, data })
}

fn variable_of(item: &Value) -> Option<Variable> {
    let name = item.field("name")?;
    Some(Variable {
        name: name.to_string(),
        value: item.field("value").unwrap_or("<unreadable>").to_string(),
        kind: item.field("type").map(str::to_string),
        handle: None,
        children: 0,
    })
}

fn upsert_breakpoint(state: &mut DebugState, bkpt: &Value, root: &Path) {
    let number = bkpt.field("number").and_then(|n| n.parse().ok());
    let (file, line) = location_of(bkpt, root);
    // `original-location` is `path:line` as the request was written —
    // gdb's own record of what was asked for.
    let requested = bkpt
        .field("original-location")
        .and_then(|location| location.rsplit_once(':'))
        .and_then(|(_, line)| line.parse::<u32>().ok())
        .map(|line| line.saturating_sub(1));

    state.record_breakpoint(Breakpoint {
        number,
        file: file.unwrap_or_default(),
        line: line.unwrap_or(0),
        requested,
        // gdb answering at all means it placed it — a refusal comes back
        // as `^error`, which lands in `state.error`.
        verified: true,
        reason: None,
        enabled: bkpt.field("enabled") != Some("n"),
    });
}

/// One spelling of a separator.
///
/// gdb mixes them within a single field — `src\bin/main.rs` came back from
/// a real session — and a path that differs from the editor's by one
/// backslash is a breakpoint that never lights up.
pub(crate) fn normalise(path: &str) -> String {
    path.replace('\\', "/")
}

/// gdb reports absolute paths; the editor and the gutter speak
/// project-relative, `/`-separated ones. One spelling per file, or a
/// breakpoint set in the editor never matches the one gdb reports back.
///
/// Textual, not `Path::strip_prefix`: on Linux a backslash is not a
/// separator, so a Windows path — from a Windows gdb, or from the tests
/// below, which feed real Windows sessions' records on every OS — is one
/// component that no root is a prefix of, and the Linux runner failed two
/// tests this machine could not. Both sides are brought to one spelling
/// first. The drive letter's case is folded, as `same_file_uri` folds it
/// for rust-analyzer, and nothing else is: the rest of a path is
/// case-sensitive on every other filesystem.
pub(crate) fn relative(full: &str, root: &Path) -> String {
    let full = normalise(full);
    let root = normalise(&root.to_string_lossy());
    let root = root.trim_end_matches('/');
    let inside = full.len() > root.len()
        && full.is_char_boundary(root.len())
        && full.as_bytes()[root.len()] == b'/'
        && fold_drive(&full[..root.len()]) == fold_drive(root);
    if inside {
        full[root.len() + 1..].to_string()
    } else {
        full
    }
}

/// `E:/x` and `e:/x` are one path on Windows.
fn fold_drive(path: &str) -> String {
    let mut chars = path.chars();
    match (chars.next(), chars.next()) {
        (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => {
            let mut folded = letter.to_ascii_lowercase().to_string();
            folded.push(':');
            folded.push_str(chars.as_str());
            folded
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::process::ChildStderr;
    use std::time::Duration;

    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(r"E:\embeded\blinky")
    }

    /// What makes this test binary, run again, stand in for a debugger.
    const STAND_IN: &str = "RUSTY_DBG_STAND_IN";

    /// A process to stop: this test binary run again as nothing but the
    /// ignored test below, which waits. What becomes of a debugger's process
    /// is the one thing here only a real child can show, and whether it was
    /// stopped is whether its stderr ends. Returned once it has said it is
    /// waiting, so a test that then sees it end has seen it stopped — not a
    /// filter that matched no test and let it exit on its own.
    pub(crate) fn stand_in() -> (Child, BufReader<ChildStderr>) {
        let mut child = Command::new(std::env::current_exe().expect("the test binary"))
            .args([
                "session::tests::standing_in",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(STAND_IN, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the test binary runs again");
        let mut stderr = BufReader::new(child.stderr.take().expect("piped"));
        let mut said = String::new();
        let _ = stderr.read_line(&mut said);
        assert_eq!(said.trim(), "standing in", "the stand-in never started");
        (child, stderr)
    }

    /// Whether the stand-in's stderr ends — its process gone — within a
    /// bound no loaded runner comes near.
    pub(crate) fn ended(mut stderr: BufReader<ChildStderr>) -> bool {
        let (closed, gone) = channel();
        std::thread::spawn(move || {
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            let _ = closed.send(());
        });
        gone.recv_timeout(Duration::from_secs(20)).is_ok()
    }

    #[test]
    #[ignore = "a process for the tests to stop, not a test"]
    fn standing_in() {
        if std::env::var_os(STAND_IN).is_some() {
            eprintln!("standing in");
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    /// A session around a process that is not gdb: enough for what the
    /// session does to the process itself, and for what it writes to it.
    fn around(mut gdb: Child) -> Debugger {
        let stdin = gdb.stdin.take();
        Debugger {
            child: Mutex::new(gdb),
            wire: Arc::new(Wire {
                stdin: Mutex::new(stdin),
                token: AtomicU32::new(1),
            }),
            state: Arc::new(Mutex::new(DebugState::default())),
            host: false,
            launched: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }

    /// Killed is not gone: a gdb nobody waits for is a zombie on Unix until
    /// rusty exits. `stop` returns once gdb has ended, with its status in
    /// hand — a kill alone returns while the process is still going down.
    #[test]
    fn stopping_gdb_waits_until_it_is_gone() {
        let (gdb, _stderr) = stand_in();
        let debugger = around(gdb);
        debugger.stop();
        let status = debugger
            .child
            .lock()
            .expect("gdb")
            .try_wait()
            .expect("a status");
        assert!(status.is_some(), "gdb has ended by the time stop returns");
    }

    /// A session let go of without `stop` takes gdb with it, as the app lets
    /// go of one when its stream ends and a failed start of the one it was
    /// building.
    #[test]
    fn a_debugger_let_go_of_takes_gdb_with_it() {
        let (gdb, stderr) = stand_in();
        drop(around(gdb));
        assert!(ended(stderr), "gdb was stopped, not left running");
    }

    /// The panel marks the stack row `frame` names and shows the variables
    /// beside it, so both have to be about one frame. gdb's refresh never set
    /// it, and the marker stayed on the innermost frame while the variables
    /// were another's. It moves once gdb has been asked, stays when gdb could
    /// not be, and a stop takes it back to the innermost frame — the one
    /// whose variables a stop asks for.
    #[test]
    fn the_selected_frame_is_the_one_whose_variables_are_shown() {
        let (gdb, _stderr) = stand_in();
        let debugger = around(gdb);
        let stop = mi::parse(
            r#"*stopped,reason="breakpoint-hit",bkptno="1",frame={addr="0x400d1a2c",func="blinky::main",file="src/bin/main.rs",line="68"},thread-id="1""#,
        )
        .unwrap();
        apply(&mut debugger.state.lock().unwrap(), &stop, &root());

        debugger.refresh(2).expect("gdb is asked");
        assert_eq!(debugger.state().frame, 2, "the frame asked about");

        *debugger.wire.stdin.lock().unwrap() = None;
        assert!(debugger.refresh(1).is_err(), "gdb's input is closed");
        assert_eq!(
            debugger.state().frame,
            2,
            "a refresh that never went moves nothing",
        );

        apply(&mut debugger.state.lock().unwrap(), &stop, &root());
        assert_eq!(debugger.state().frame, 0, "a stop is read at frame 0");
    }

    #[test]
    fn arguments_with_spaces_or_quotes_are_quoted_and_plain_ones_are_not() {
        let args = vec![
            "tests::it_works".to_string(),
            "--nocapture".to_string(),
            "a b".to_string(),
            "say \"hi\"".to_string(),
        ];
        assert_eq!(
            exec_arguments(&args),
            r#"tests::it_works --nocapture "a b" "say \"hi\"""#
        );
    }

    #[test]
    fn a_host_program_is_run_once_and_continued_after() {
        assert_eq!(resume_command(true), "-exec-run");
        assert_eq!(resume_command(false), "-exec-continue");
    }

    /// The records the tests below feed are from Windows sessions, and the
    /// runner they must pass on is Linux, where a backslash is not a
    /// separator. Relativising is textual for exactly that reason.
    #[test]
    fn a_windows_fullname_is_made_project_relative_on_every_host() {
        assert_eq!(
            relative(r"E:\embeded\blinky\src\bin\main.rs", &root()),
            "src/bin/main.rs"
        );
        assert_eq!(
            relative("e:/embeded/blinky/src/bin/main.rs", &root()),
            "src/bin/main.rs",
            "the drive letter's case is folded, nothing else",
        );
        assert_eq!(
            relative(r"E:\elsewhere\lib.rs", &root()),
            "E:/elsewhere/lib.rs",
            "outside the project the path stays absolute, in one spelling",
        );
    }

    #[test]
    fn a_posix_fullname_is_made_project_relative_too() {
        let root = PathBuf::from("/home/u/blinky");
        assert_eq!(relative("/home/u/blinky/src/main.rs", &root), "src/main.rs");
        assert_eq!(
            relative("/home/u/blinky-other/src/main.rs", &root),
            "/home/u/blinky-other/src/main.rs",
            "a sibling that merely starts with the root's text is not inside it",
        );
    }

    /// The whole point of the session layer: records in, a state the panel
    /// can draw out. Real MI lines, in the order a stop produces them.
    #[test]
    fn a_breakpoint_hit_becomes_a_stopped_state_at_a_project_path() {
        let mut state = DebugState::default();
        let root = root();

        assert!(apply(&mut state, &mi::parse("^running").unwrap(), &root,));
        assert!(state.running, "running is what ^running means");

        let stop = mi::parse(
            r#"*stopped,reason="breakpoint-hit",bkptno="1",frame={addr="0x400d1a2c",func="blinky::main",file="src/bin/main.rs",fullname="E:\\embeded\\blinky\\src\\bin\\main.rs",line="68"},thread-id="1""#,
        )
        .unwrap();
        assert!(apply(&mut state, &stop, &root));

        assert!(!state.running);
        assert_eq!(state.reason, Some(StopReason::Breakpoint));
        let frame = &state.stack[0];
        assert_eq!(
            frame.file.as_deref(),
            Some("src/bin/main.rs"),
            "the absolute path gdb reports is made project-relative, so the \
             gutter can match it to the open file",
        );
        assert_eq!(
            frame.line,
            Some(67),
            "gdb's one-based line 68 is line 67 on this side of the boundary",
        );
    }

    /// Real gdb output, captured from a live esp32 session: one field,
    /// both separators. A path that differs from the editor's by a single
    /// backslash is a breakpoint that never lights up.
    #[test]
    fn mixed_separators_come_back_as_one_spelling() {
        let mut state = DebugState::default();
        let stop = mi::parse(
            r#"*stopped,reason="breakpoint-hit",bkptno="1",frame={addr="0x400d121f",func="blinky::__xtensa_lx_rt_main",file="src\\bin/main.rs",line="75"},thread-id="1""#,
        )
        .unwrap();
        apply(&mut state, &stop, &root());
        assert_eq!(
            state.stack[0].file.as_deref(),
            Some("src/bin/main.rs"),
            "without a fullname to strip, the file field is still normalised",
        );
    }

    #[test]
    fn a_placed_breakpoint_is_remembered_once() {
        let mut state = DebugState::default();
        let root = root();
        let placed = mi::parse(
            r#"^done,bkpt={number="1",type="breakpoint",enabled="y",file="src/bin/main.rs",fullname="E:\\embeded\\blinky\\src\\bin\\main.rs",line="68"}"#,
        )
        .unwrap();
        apply(&mut state, &placed, &root);
        assert_eq!(state.breakpoints.len(), 1);
        assert_eq!(state.breakpoints[0].line, 68 - 1);
        assert_eq!(state.breakpoints[0].file, "src/bin/main.rs");

        // gdb re-reports the same breakpoint whenever it moves or is hit.
        let moved = mi::parse(
            r#"=breakpoint-modified,bkpt={number="1",enabled="y",file="src/bin/main.rs",line="70",times="3"}"#,
        )
        .unwrap();
        apply(&mut state, &moved, &root);
        assert_eq!(
            state.breakpoints.len(),
            1,
            "the same number updates in place rather than piling up",
        );
        assert_eq!(state.breakpoints[0].line, 69);
    }

    /// `-break-delete 1` is answered with a bare `^done` and the deletion
    /// arrives as `=breakpoint-deleted,id="1"`. A list that only ever heard
    /// `bkpt=` records kept every breakpoint for the life of the session.
    #[test]
    fn a_deleted_breakpoint_leaves_the_list() {
        let mut state = DebugState::default();
        let root = root();
        for number in ["1", "2"] {
            let placed = mi::parse(&format!(
                r#"^done,bkpt={{number="{number}",type="breakpoint",enabled="y",file="src/bin/main.rs",line="68"}}"#,
            ))
            .unwrap();
            apply(&mut state, &placed, &root);
        }
        assert_eq!(state.breakpoints.len(), 2);

        let deleted = mi::parse(r#"=breakpoint-deleted,id="1""#).unwrap();
        assert!(apply(&mut state, &deleted, &root), "a deletion is a change");
        assert_eq!(state.breakpoints.len(), 1);
        assert_eq!(state.breakpoints[0].number, Some(2), "the other one stays");

        let again = mi::parse(r#"=breakpoint-deleted,id="1""#).unwrap();
        assert!(
            !apply(&mut state, &again, &root),
            "deleting what is gone changes nothing"
        );
    }

    /// gdb writes `exit-code` in octal. `"012"` is ten, not twelve, and an
    /// exit code the panel shows wrong is worse than none.
    #[test]
    fn a_target_exit_code_is_read_as_octal() {
        let mut state = DebugState::default();
        let stop = mi::parse(r#"*stopped,reason="exited",exit-code="012""#).unwrap();
        apply(&mut state, &stop, &root());
        assert_eq!(state.exited, Some(10));
        assert_eq!(state.reason, Some(StopReason::Exited));

        let mut state = DebugState::default();
        let stop = mi::parse(r#"*stopped,reason="exited-normally""#).unwrap();
        apply(&mut state, &stop, &root());
        assert_eq!(state.exited, Some(0));
    }

    /// Real gdb output from an optimised esp32 build: line 69 was asked
    /// for, line 75 is where code exists. The margin needs both — the dot
    /// belongs where execution will stop, and knowing which request it
    /// answers is what lets the old dot move rather than multiply.
    #[test]
    fn a_moved_breakpoint_remembers_what_was_asked_for() {
        let mut state = DebugState::default();
        let placed = mi::parse(
            r#"^done,bkpt={number="1",type="breakpoint",enabled="y",file="src/bin/main.rs",line="75",original-location="src/bin/main.rs:69"}"#,
        )
        .unwrap();
        apply(&mut state, &placed, &root());

        let breakpoint = &state.breakpoints[0];
        assert_eq!(breakpoint.line, 74, "where it landed, zero-based");
        assert_eq!(
            breakpoint.requested,
            Some(68),
            "and where it was asked for, so the margin can move its dot",
        );
    }

    #[test]
    fn running_clears_the_stack_it_would_otherwise_lie_about() {
        let mut state = DebugState {
            stack: vec![StackFrame {
                level: 0,
                function: "old".into(),
                file: None,
                line: None,
                address: "0x0".into(),
            }],
            variables: vec![Variable {
                name: "tick".into(),
                value: "3".into(),
                kind: None,
                handle: None,
                children: 0,
            }],
            ..DebugState::default()
        };
        apply(
            &mut state,
            &mi::parse("*running,thread-id=\"all\"").unwrap(),
            &root(),
        );
        assert!(state.stack.is_empty(), "a stack read mid-flight is a lie");
        assert!(state.variables.is_empty());
    }

    #[test]
    fn a_memory_read_decodes_to_the_bytes_the_target_holds() {
        let mut state = DebugState::default();
        let record = mi::parse(
            r#"^done,memory=[{begin="0x3ff44004",offset="0x00000000",end="0x3ff44008",contents="0400000f"}]"#,
        )
        .unwrap();
        assert!(apply(&mut state, &record, &root()));

        let read = &state.memory[0];
        assert_eq!(read.begin, 0x3FF4_4004);
        assert_eq!(
            read.data,
            vec![0x04, 0x00, 0x00, 0x0F],
            "hex pairs decode in order; the little-endian assembly is the panel's job, \
             not the transport's",
        );
    }

    /// A line the program printed goes out as a state of its own and is gone
    /// from the next one — both debuggers forward the program's console this
    /// way. Once nobody is listening, the reader is told to stop.
    #[test]
    fn a_printed_line_is_sent_once_and_not_kept() {
        let state = Mutex::new(DebugState::default());
        let (sender, receiver) = channel();
        assert!(push_line(&state, &sender, "running 1 test".to_string()));
        assert_eq!(receiver.recv().unwrap().output, ["running 1 test"]);
        assert!(
            state.lock().unwrap().output.is_empty(),
            "sent, then cleared"
        );

        drop(receiver);
        assert!(!push_line(&state, &sender, "test it ... ok".to_string()));
        assert!(state.lock().unwrap().output.is_empty());
    }

    #[test]
    fn gdbs_error_is_carried_verbatim() {
        let mut state = DebugState::default();
        apply(
            &mut state,
            &mi::parse(r#"^error,msg="No symbol \"nope\" in current context.""#).unwrap(),
            &root(),
        );
        assert_eq!(
            state.error.as_deref(),
            Some(r#"No symbol "nope" in current context."#),
            "gdb's own words reach the user — a paraphrase loses the symbol",
        );
    }
}
