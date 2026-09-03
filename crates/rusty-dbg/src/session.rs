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

#[cfg(test)]
mod launch_tests {
    use super::*;

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
                let snapshot = {
                    let mut state = debugger.state.lock().expect("state");
                    state.attached = true;
                    state.clone()
                };
                let _ = initial.send(snapshot);
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

    /// Ask for the stack and the selected frame's variables. Called after
    /// every stop; the answers arrive as records and land in the state.
    pub fn refresh(&self, frame: u32) -> Result<()> {
        self.send(&format!("-stack-select-frame {frame}"))?;
        self.send("-stack-list-frames")?;
        // `--all-values` rather than names alone: a variables panel that
        // needs a round trip per row updates one row at a time on a target
        // that is already slow.
        self.send("-stack-list-variables --all-values")
    }

    /// Read a span of target memory — a peripheral's register block, for
    /// the register view. Only while stopped: gdb refuses otherwise, and
    /// a half-read block would decode into fiction.
    pub fn read_memory(&self, address: u64, bytes: u32) -> Result<()> {
        self.send(&format!("-data-read-memory-bytes 0x{address:x} {bytes}"))
    }

    /// End the session and the target with it.
    pub fn stop(&self) {
        let _ = self.send("-gdb-exit");
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }

    fn send(&self, command: &str) -> Result<()> {
        self.wire.send(command)
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
            if host && !line.trim().is_empty() && line.trim() != "(gdb)" {
                let snapshot = {
                    let mut state = state.lock().expect("state");
                    state.output.push(line);
                    state.clone()
                };
                let gone = sender.send(snapshot).is_err();
                state.lock().expect("state").output.clear();
                if gone {
                    break;
                }
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
        let (changed, exited) = {
            let mut state = state.lock().expect("state");
            let changed = apply(&mut state, &record, &root);
            (changed, state.exited.is_some())
        };
        if changed {
            let snapshot = state.lock().expect("state").clone();
            if sender.send(snapshot).is_err() {
                break;
            }
        }
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
    let mut final_state = state.lock().expect("state").clone();
    final_state.running = false;
    final_state.attached = false;
    if final_state.exited.is_none() && final_state.error.is_none() {
        final_state.error = Some("gdb ended the session without reporting an exit".to_string());
    }
    let _ = sender.send(final_state);
}

/// Fold one record into the state. Returns whether anything changed.
fn apply(state: &mut DebugState, record: &Record, root: &Path) -> bool {
    match record {
        Record::Exec { class, fields } => {
            let value = Value::Tuple(fields.clone());
            if class == "running" {
                state.running = true;
                state.attached = true;
                // A stack read while the target runs is a lie; drop it
                // rather than leave the panel showing a stale frame as if
                // it were current.
                state.stack.clear();
                state.variables.clear();
                state.reason = None;
                return true;
            }
            if class == "stopped" {
                state.running = false;
                state.attached = true;
                state.reason = Some(reason_of(value.field("reason").unwrap_or_default()));
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
    Some(StackFrame {
        level: frame
            .field("level")
            .and_then(|l| l.parse().ok())
            .unwrap_or(0),
        function: frame.field("func").unwrap_or("??").to_string(),
        file: frame
            .field("fullname")
            .map(|full| relative(full, root))
            .or_else(|| frame.field("file").map(normalise)),
        // gdb counts lines from one; everything this side of the boundary
        // counts from zero.
        line: frame
            .field("line")
            .and_then(|l| l.parse::<u32>().ok())
            .map(|line| line.saturating_sub(1)),
        address: frame.field("addr").unwrap_or_default().to_string(),
    })
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
    let file = bkpt
        .field("fullname")
        .map(|full| relative(full, root))
        .or_else(|| bkpt.field("file").map(normalise))
        .unwrap_or_default();
    let line = bkpt
        .field("line")
        .and_then(|l| l.parse::<u32>().ok())
        .map(|line| line.saturating_sub(1))
        .unwrap_or(0);
    // `original-location` is `path:line` as the request was written —
    // gdb's own record of what was asked for.
    let requested = bkpt
        .field("original-location")
        .and_then(|location| location.rsplit_once(':'))
        .and_then(|(_, line)| line.parse::<u32>().ok())
        .map(|line| line.saturating_sub(1));

    let entry = Breakpoint {
        number,
        file,
        line,
        requested,
        // gdb answering at all means it placed it — a refusal comes back
        // as `^error`, which lands in `state.error`.
        verified: true,
        reason: None,
        enabled: bkpt.field("enabled") != Some("n"),
    };
    match state
        .breakpoints
        .iter_mut()
        .find(|existing| existing.number == entry.number && entry.number.is_some())
    {
        Some(existing) => *existing = entry,
        None => state.breakpoints.push(entry),
    }
}

/// One spelling of a separator.
///
/// gdb mixes them within a single field — `src\bin/main.rs` came back from
/// a real session — and a path that differs from the editor's by one
/// backslash is a breakpoint that never lights up.
fn normalise(path: &str) -> String {
    path.replace('\\', "/")
}

/// gdb reports absolute paths; the editor and the gutter speak
/// project-relative, `/`-separated ones. One spelling per file, or a
/// breakpoint set in the editor never matches the one gdb reports back.
fn relative(full: &str, root: &Path) -> String {
    let path = Path::new(full);
    let relative = path.strip_prefix(root).unwrap_or(path);
    normalise(&relative.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(r"E:\embeded\blinky")
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
