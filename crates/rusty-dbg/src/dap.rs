//! A debug adapter, driven — the second debugger backend.
//!
//! gdb cannot read every platform's debug information. On Windows, Rust's
//! default `-msvc` target emits a PDB, and gdb reads DWARF: pointed at one it
//! loads, sets breakpoints that never hit, and shows addresses where source
//! lines should be. LLDB reads PDB correctly, and LLDB's machine interface is
//! not MI — it is the Debug Adapter Protocol, spoken by `lldb-dap` and by
//! CodeLLDB's `codelldb`. So this module is to [`crate::session`] what DAP is
//! to MI: the same [`DebugState`] out, a different protocol in.
//!
//! **Over a socket, never over the adapter's stdin.** Both adapters hand the
//! debuggee their own stdout, so a test's `running 1 test` lands in the middle
//! of a DAP frame and every frame after it is garbage — measured, not feared.
//! Driven over TCP the two streams separate: the socket carries the protocol
//! and the adapter's stdout carries only the program's own output, which is
//! exactly what the dock wants to show.
//!
//! **Every request is fire-and-forget and every answer is dispatched by its
//! `command`.** A reader that blocked waiting for its own reply would
//! deadlock, because the reply arrives on the thread doing the blocking. The
//! stop sequence is therefore a small chain in the reader: `stopped` asks for
//! the stack, the stack's answer asks for the frame's scopes, and the scopes'
//! answer asks for its variables.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::model::{Breakpoint, DebugState, StackFrame, StopReason, Variable};
use crate::session::{Error, Events, Result, relative};

/// How long the adapter has to answer `initialized`. Generous: the first
/// launch loads the target's symbols, and a Rust test binary with its whole
/// dependency graph is not small.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// How long it has to start *listening*, which is a different question and a
/// much shorter one. An adapter that works opens its port at once; one that
/// is going to do nothing at all — LLVM ships builds that answer neither
/// stdio nor a socket — does nothing quickly. Short, because the caller has
/// another candidate to try and waiting out a broken one is time the person
/// spends looking at a frozen button.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(8);

/// What to debug, and with which adapter.
#[derive(Debug, Clone)]
pub struct DapLaunch {
    /// `lldb-dap` or `codelldb`.
    pub adapter: PathBuf,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Where the program runs, and what source paths are relative to.
    pub root: PathBuf,
    /// The breakpoints standing when the session starts, project-relative and
    /// zero-based.
    ///
    /// Passed in rather than placed afterwards, because DAP only accepts
    /// breakpoints between `initialized` and `configurationDone` — after that
    /// the program is already running, and a breakpoint placed then is one the
    /// test may have run past. The frontend holds the list, so it sends it.
    pub breakpoints: Vec<(String, u32)>,
}

/// `Content-Length: N` out of a frame's header block, ignoring any other
/// header and tolerating the casing adapters actually use.
fn content_length(header: &str) -> Option<usize> {
    header
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
}

/// DAP counts lines from one and this side counts from zero, as everywhere
/// that crosses this boundary.
fn zero_based(line: i64) -> u32 {
    u32::try_from(line.max(1) - 1).unwrap_or(0)
}

/// One frame of a `stackTrace` body, in this workbench's terms.
fn frame_of(level: u32, frame: &Value, root: &Path) -> StackFrame {
    let file = frame
        .get("source")
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)
        .map(|path| relative(path, root));
    StackFrame {
        level,
        function: frame
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("??")
            .to_string(),
        file,
        line: frame.get("line").and_then(Value::as_i64).map(zero_based),
        address: frame
            .get("instructionPointerReference")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// One entry of a `variables` body.
///
/// `variablesReference` is non-zero when the value can be expanded; the panel
/// only needs to know *that* it can, which is what `children` says.
fn variable_of(item: &Value) -> Option<Variable> {
    Some(Variable {
        name: item.get("name").and_then(Value::as_str)?.to_string(),
        value: item
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("<unreadable>")
            .to_string(),
        kind: item.get("type").and_then(Value::as_str).map(str::to_string),
        handle: None,
        children: u32::from(
            item.get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                != 0,
        ),
    })
}

/// DAP's stop reasons, in the model's terms. The strings are the protocol's
/// own; anything else a particular adapter invents lands on `Other` rather
/// than being guessed at.
fn reason_of(reason: &str) -> StopReason {
    match reason {
        "breakpoint" | "function breakpoint" | "data breakpoint" => StopReason::Breakpoint,
        "step" => StopReason::Step,
        "pause" => StopReason::Pause,
        "exception" | "signal" => StopReason::Signal,
        "exited" => StopReason::Exited,
        _ => StopReason::Other,
    }
}

/// The write half, shared with the reader thread so a stop can ask its own
/// follow-up questions.
struct Wire {
    socket: Mutex<Option<TcpStream>>,
    seq: AtomicI64,
}

impl Wire {
    fn send(&self, command: &str, arguments: Value) -> Result<i64> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let message = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        let body = serde_json::to_vec(&message).map_err(|_| Error::Closed)?;
        let mut slot = self.socket.lock().expect("dap socket");
        let socket = slot.as_mut().ok_or(Error::Closed)?;
        socket
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .map_err(|_| Error::Closed)?;
        socket.write_all(&body).map_err(|_| Error::Closed)?;
        socket.flush().map_err(|_| Error::Closed)?;
        Ok(seq)
    }
}

/// A live debug adapter.
pub struct DapSession {
    child: Mutex<Child>,
    wire: Arc<Wire>,
    state: Arc<Mutex<DebugState>>,
    /// The whole list per file: `setBreakpoints` replaces a source's
    /// breakpoints rather than adding one, so the client owns the set.
    placed: Mutex<HashMap<String, Vec<u32>>>,
    /// The thread the last stop named — every control request needs it.
    thread: Arc<Mutex<i64>>,
    /// Frame ids by level, from the last `stackTrace`: the panel selects a
    /// level and DAP wants the adapter's own id.
    frames: Arc<Mutex<Vec<i64>>>,
    root: PathBuf,
    running: Arc<AtomicBool>,
}

impl DapSession {
    /// Start the adapter, launch the program, and place the standing
    /// breakpoints before it runs.
    pub fn start(launch: &DapLaunch) -> Result<(Self, Events)> {
        // A port of our own. Bound and released so the adapter can take it:
        // a race with another process is possible in principle and has never
        // been the failure worth engineering against, while "ask the adapter
        // which port it chose" is not something both adapters agree on.
        let port = {
            let probe = TcpListener::bind(("127.0.0.1", 0)).map_err(|source| Error::Spawn {
                gdb: "a local port".to_string(),
                source,
            })?;
            probe
                .local_addr()
                .map_err(|source| Error::Spawn {
                    gdb: "a local port".to_string(),
                    source,
                })?
                .port()
        };

        let mut command = Command::new(&launch.adapter);
        command
            .arg("--port")
            .arg(port.to_string())
            .current_dir(&launch.root)
            .stdin(Stdio::null())
            // The debuggee's own console. Read on its own thread and forwarded
            // as `output`, which is the whole reason for driving the protocol
            // over a socket instead of this pipe.
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command.env_remove("RUSTUP_TOOLCHAIN");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|source| Error::Spawn {
            gdb: launch.adapter.display().to_string(),
            source,
        })?;

        let socket = connect(port, &mut child, &launch.adapter)?;
        let reader_half = socket.try_clone().map_err(|source| Error::Spawn {
            gdb: launch.adapter.display().to_string(),
            source,
        })?;

        let state = Arc::new(Mutex::new(DebugState::default()));
        let (sender, receiver) = channel();
        let wire = Arc::new(Wire {
            socket: Mutex::new(Some(socket)),
            seq: AtomicI64::new(1),
        });
        let thread = Arc::new(Mutex::new(0));
        let frames = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = channel();

        {
            let state = Arc::clone(&state);
            let wire = Arc::clone(&wire);
            let sender = sender.clone();
            let root = launch.root.clone();
            let thread = Arc::clone(&thread);
            let frames = Arc::clone(&frames);
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                pump(
                    reader_half,
                    state,
                    wire,
                    sender,
                    root,
                    thread,
                    frames,
                    running,
                    ready_tx,
                )
            });
        }

        // The program's console, on its own thread. Lines rather than frames:
        // this pipe is plain text.
        if let Some(stdout) = child.stdout.take() {
            let state = Arc::clone(&state);
            let sender = sender.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    let snapshot = {
                        let mut state = state.lock().expect("dap state");
                        state.output.push(line);
                        state.clone()
                    };
                    let gone = sender.send(snapshot).is_err();
                    state.lock().expect("dap state").output.clear();
                    if gone {
                        return;
                    }
                }
            });
        }

        let session = Self {
            child: Mutex::new(child),
            wire,
            state,
            placed: Mutex::new(HashMap::new()),
            thread,
            frames,
            root: launch.root.clone(),
            running,
        };

        session.wire.send(
            "initialize",
            json!({
                "adapterID": "lldb",
                "clientID": "rusty",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                // Not declared, deliberately: with it the adapter asks the
                // client to open a terminal for the debuggee, and there is no
                // terminal here to open. Without it the adapter runs the
                // program itself, which is what the console thread reads.
                "supportsRunInTerminalRequest": false,
            }),
        )?;
        session.wire.send(
            "launch",
            json!({
                "program": launch.program.to_string_lossy(),
                "args": launch.args,
                "cwd": launch.root.to_string_lossy(),
                "stopOnEntry": false,
            }),
        )?;

        // `initialized` is the adapter saying it will take configuration now.
        // Its `launch` reply does not come until after `configurationDone`, so
        // waiting on that instead would deadlock the handshake.
        if ready_rx.recv_timeout(READY_TIMEOUT).is_err() {
            session.stop();
            return Err(Error::Spawn {
                gdb: launch.adapter.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the debug adapter never reported itself ready",
                ),
            });
        }

        session.place_all(&launch.breakpoints)?;
        session.wire.send("configurationDone", json!({}))?;

        // Attached, and running: the program starts the moment configuration
        // is done. The panel's first resume is therefore a no-op rather than
        // an error about a program that is already going.
        session.running.store(true, Ordering::SeqCst);
        let snapshot = {
            let mut state = session.state.lock().expect("dap state");
            state.attached = true;
            state.running = true;
            state.clone()
        };
        let _ = sender.send(snapshot);

        Ok((session, Events::new(receiver)))
    }

    /// The state as it stands, for a caller that missed the last push.
    pub fn state(&self) -> DebugState {
        self.state.lock().expect("dap state").clone()
    }

    /// Send every file's list. DAP replaces a source's breakpoints wholesale.
    fn place_all(&self, breakpoints: &[(String, u32)]) -> Result<()> {
        {
            let mut placed = self.placed.lock().expect("dap breakpoints");
            placed.clear();
            for (file, line) in breakpoints {
                placed.entry(file.clone()).or_default().push(*line);
            }
        }
        let files: Vec<String> = self
            .placed
            .lock()
            .expect("dap breakpoints")
            .keys()
            .cloned()
            .collect();
        for file in files {
            self.send_source(&file)?;
        }
        Ok(())
    }

    /// One source's whole breakpoint list.
    fn send_source(&self, file: &str) -> Result<()> {
        let lines = self
            .placed
            .lock()
            .expect("dap breakpoints")
            .get(file)
            .cloned()
            .unwrap_or_default();
        let absolute = self.root.join(file);
        self.wire.send(
            "setBreakpoints",
            json!({
                "source": { "path": absolute.to_string_lossy() },
                "breakpoints": lines.iter().map(|l| json!({"line": l + 1})).collect::<Vec<_>>(),
            }),
        )?;
        Ok(())
    }

    pub fn add_breakpoint(&self, file: &str, line: u32) -> Result<()> {
        {
            let mut placed = self.placed.lock().expect("dap breakpoints");
            let lines = placed.entry(file.to_string()).or_default();
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        self.send_source(file)
    }

    /// DAP has no "delete by number", so the number is resolved back to its
    /// file and line and that source is sent again without it.
    pub fn remove_breakpoint(&self, number: u32) -> Result<()> {
        let found = self
            .state
            .lock()
            .expect("dap state")
            .breakpoints
            .iter()
            .find(|b| b.number == Some(number))
            .map(|b| (b.file.clone(), b.requested.unwrap_or(b.line)));
        let Some((file, line)) = found else {
            return Ok(());
        };
        {
            let mut placed = self.placed.lock().expect("dap breakpoints");
            if let Some(lines) = placed.get_mut(&file) {
                lines.retain(|l| *l != line);
            }
        }
        self.state
            .lock()
            .expect("dap state")
            .breakpoints
            .retain(|b| b.number != Some(number));
        self.send_source(&file)
    }

    /// Continue — or nothing, when the program is already going.
    ///
    /// The panel resumes once on attach, as it must for a target frozen at
    /// reset. A launched program is not frozen, and `continue` against a
    /// running one is an adapter error on the one button that has to work.
    pub fn resume(&self) -> Result<()> {
        if self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.control("continue")
    }

    pub fn pause(&self) -> Result<()> {
        self.control("pause")
    }

    pub fn step_over(&self) -> Result<()> {
        self.control("next")
    }

    pub fn step_into(&self) -> Result<()> {
        self.control("stepIn")
    }

    pub fn step_out(&self) -> Result<()> {
        self.control("stepOut")
    }

    fn control(&self, command: &str) -> Result<()> {
        let thread = *self.thread.lock().expect("dap thread");
        self.wire.send(command, json!({ "threadId": thread }))?;
        Ok(())
    }

    /// Select a frame and read its variables. The panel counts levels; the
    /// adapter counts its own frame ids.
    pub fn refresh(&self, frame: u32) -> Result<()> {
        self.state.lock().expect("dap state").frame = frame;
        let id = self
            .frames
            .lock()
            .expect("dap frames")
            .get(frame as usize)
            .copied();
        let Some(id) = id else {
            return Ok(());
        };
        self.wire.send("scopes", json!({ "frameId": id }))?;
        Ok(())
    }

    /// Not offered here, and saying so beats answering with zeroes.
    ///
    /// The register view reads a peripheral block from a chip through its
    /// SVD; a host process has no such thing, and an adapter's `readMemory`
    /// would answer about this machine's address space instead.
    pub fn read_memory(&self, _address: u64, _bytes: u32) -> Result<()> {
        Ok(())
    }

    /// End the session and the program with it.
    pub fn stop(&self) {
        let _ = self
            .wire
            .send("disconnect", json!({ "terminateDebuggee": true }));
        // The adapter is given a moment to take the program down cleanly;
        // killing it first orphans the debuggee, which then holds the test
        // binary open and the next `cargo test` fails to link.
        std::thread::sleep(Duration::from_millis(120));
        *self.wire.socket.lock().expect("dap socket") = None;
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Connect to the adapter, retrying while it starts listening.
fn connect(port: u16, child: &mut Child, adapter: &Path) -> Result<TcpStream> {
    let deadline = Instant::now() + LISTEN_TIMEOUT;
    let mut last = std::io::Error::new(std::io::ErrorKind::TimedOut, "never listened");
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(Error::Spawn {
                gdb: adapter.display().to_string(),
                source: std::io::Error::other(format!(
                    "the debug adapter exited before listening ({status})"
                )),
            });
        }
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(socket) => return Ok(socket),
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    Err(Error::Spawn {
        gdb: adapter.display().to_string(),
        source: last,
    })
}

/// Read the adapter for the life of the session.
#[allow(clippy::too_many_arguments)]
fn pump(
    socket: TcpStream,
    state: Arc<Mutex<DebugState>>,
    wire: Arc<Wire>,
    sender: Sender<DebugState>,
    root: PathBuf,
    thread: Arc<Mutex<i64>>,
    frames: Arc<Mutex<Vec<i64>>>,
    running: Arc<AtomicBool>,
    ready: Sender<()>,
) {
    let mut reader = BufReader::new(socket);
    while let Some(message) = read_frame(&mut reader) {
        let changed = apply(
            &message, &state, &wire, &root, &thread, &frames, &running, &ready,
        );
        if changed {
            let snapshot = state.lock().expect("dap state").clone();
            if sender.send(snapshot).is_err() {
                break;
            }
        }
        if state.lock().expect("dap state").exited.is_some() {
            break;
        }
    }
    let mut final_state = state.lock().expect("dap state").clone();
    final_state.running = false;
    final_state.attached = false;
    if final_state.exited.is_none() && final_state.error.is_none() {
        final_state.exited = Some(0);
    }
    let _ = sender.send(final_state);
}

/// One `Content-Length` framed JSON message, or `None` at the end.
fn read_frame(reader: &mut BufReader<TcpStream>) -> Option<Value> {
    let mut header = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        header.push_str(&line);
    }
    let length = content_length(&header)?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// Fold one message into the state. Returns whether anything changed.
#[allow(clippy::too_many_arguments)]
fn apply(
    message: &Value,
    state: &Arc<Mutex<DebugState>>,
    wire: &Arc<Wire>,
    root: &Path,
    thread: &Arc<Mutex<i64>>,
    frames: &Arc<Mutex<Vec<i64>>>,
    running: &Arc<AtomicBool>,
    ready: &Sender<()>,
) -> bool {
    match message.get("type").and_then(Value::as_str) {
        Some("event") => {
            let body = message.get("body").cloned().unwrap_or(Value::Null);
            match message.get("event").and_then(Value::as_str).unwrap_or("") {
                "initialized" => {
                    let _ = ready.send(());
                    false
                }
                "stopped" => {
                    if let Some(id) = body.get("threadId").and_then(Value::as_i64) {
                        *thread.lock().expect("dap thread") = id;
                    }
                    running.store(false, Ordering::SeqCst);
                    {
                        let mut state = state.lock().expect("dap state");
                        state.running = false;
                        state.attached = true;
                        state.reason = Some(reason_of(
                            body.get("reason").and_then(Value::as_str).unwrap_or(""),
                        ));
                    }
                    // The stop names one thread and nothing else; the stack is
                    // a separate question, and asking it here is what makes
                    // the panel complete the moment it appears.
                    let id = *thread.lock().expect("dap thread");
                    let _ = wire.send("stackTrace", json!({ "threadId": id, "levels": 64 }));
                    true
                }
                "continued" => {
                    running.store(true, Ordering::SeqCst);
                    let mut state = state.lock().expect("dap state");
                    state.running = true;
                    // A stack read while the program runs is a lie.
                    state.stack.clear();
                    state.variables.clear();
                    state.reason = None;
                    true
                }
                "exited" => {
                    let code = body.get("exitCode").and_then(Value::as_i64).unwrap_or(0);
                    let mut state = state.lock().expect("dap state");
                    state.exited = Some(i32::try_from(code).unwrap_or(-1));
                    state.running = false;
                    state.reason = Some(StopReason::Exited);
                    true
                }
                "terminated" => {
                    let mut state = state.lock().expect("dap state");
                    if state.exited.is_none() {
                        state.exited = Some(0);
                    }
                    state.running = false;
                    true
                }
                "output" => {
                    let Some(text) = body.get("output").and_then(Value::as_str) else {
                        return false;
                    };
                    // The adapter's own narration — "Launched process 1234" —
                    // is not the program's output and would read as the test
                    // printing it. Only the debuggee's streams are forwarded.
                    let category = body
                        .get("category")
                        .and_then(Value::as_str)
                        .unwrap_or("console");
                    if category != "stdout" && category != "stderr" {
                        return false;
                    }
                    let mut state = state.lock().expect("dap state");
                    state.output.extend(text.lines().map(str::to_string));
                    true
                }
                "breakpoint" => {
                    let Some(bkpt) = body.get("breakpoint") else {
                        return false;
                    };
                    let mut state = state.lock().expect("dap state");
                    upsert_breakpoint(&mut state, bkpt, root);
                    true
                }
                _ => false,
            }
        }
        Some("response") => {
            let command = message.get("command").and_then(Value::as_str).unwrap_or("");
            let body = message.get("body").cloned().unwrap_or(Value::Null);
            if message.get("success").and_then(Value::as_bool) == Some(false) {
                // `pause` on an already-stopped program and `continue` on a
                // running one are races the panel can lose harmlessly; the
                // rest is worth showing.
                if matches!(
                    command,
                    "pause" | "continue" | "next" | "stepIn" | "stepOut"
                ) {
                    return false;
                }
                let detail = message
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the debug adapter refused the request");
                state.lock().expect("dap state").error = Some(detail.to_string());
                return true;
            }
            match command {
                "stackTrace" => {
                    let listed = body
                        .get("stackFrames")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut ids = Vec::with_capacity(listed.len());
                    let mut stack = Vec::with_capacity(listed.len());
                    for (level, frame) in listed.iter().enumerate() {
                        ids.push(frame.get("id").and_then(Value::as_i64).unwrap_or(0));
                        stack.push(frame_of(level as u32, frame, root));
                    }
                    *frames.lock().expect("dap frames") = ids.clone();
                    state.lock().expect("dap state").stack = stack;
                    if let Some(first) = ids.first() {
                        let _ = wire.send("scopes", json!({ "frameId": first }));
                    }
                    true
                }
                "scopes" => {
                    // The first scope is the frame's locals in both adapters;
                    // the rest are registers and globals, which this panel
                    // does not show.
                    let reference = body
                        .get("scopes")
                        .and_then(Value::as_array)
                        .and_then(|scopes| scopes.first())
                        .and_then(|scope| scope.get("variablesReference"))
                        .and_then(Value::as_i64);
                    if let Some(reference) = reference {
                        let _ = wire.send("variables", json!({ "variablesReference": reference }));
                    }
                    false
                }
                "variables" => {
                    let listed = body
                        .get("variables")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    state.lock().expect("dap state").variables =
                        listed.iter().filter_map(variable_of).collect();
                    true
                }
                "setBreakpoints" => {
                    let listed = body
                        .get("breakpoints")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let mut state = state.lock().expect("dap state");
                    for bkpt in &listed {
                        upsert_breakpoint(&mut state, bkpt, root);
                    }
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// Add or update one breakpoint, keyed by the adapter's id.
fn upsert_breakpoint(state: &mut DebugState, bkpt: &Value, root: &Path) {
    let number = bkpt
        .get("id")
        .and_then(Value::as_i64)
        .and_then(|id| u32::try_from(id).ok());
    let line = bkpt.get("line").and_then(Value::as_i64).map(zero_based);
    let file = bkpt
        .get("source")
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)
        .map(|path| relative(path, root));

    let existing = state
        .breakpoints
        .iter()
        .position(|b| b.number.is_some() && b.number == number);
    let previous = existing.map(|at| state.breakpoints[at].clone());
    let entry = Breakpoint {
        number,
        file: file
            .or_else(|| previous.as_ref().map(|b| b.file.clone()))
            .unwrap_or_default(),
        line: line
            .or_else(|| previous.as_ref().map(|b| b.line))
            .unwrap_or(0),
        // Where it was asked for, kept across updates so the margin can move
        // its dot to where the adapter actually put it.
        requested: previous
            .as_ref()
            .and_then(|b| b.requested)
            .or_else(|| previous.as_ref().map(|b| b.line))
            .or(line),
        verified: bkpt
            .get("verified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        reason: bkpt
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
        enabled: true,
    };
    match existing {
        Some(at) => state.breakpoints[at] = entry,
        None => state.breakpoints.push(entry),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from(r"E:\CodeBase\proj")
    }

    #[test]
    fn a_header_block_yields_its_length_whatever_its_casing() {
        assert_eq!(content_length("Content-Length: 42\r\n"), Some(42));
        assert_eq!(content_length("content-length:7\r\n"), Some(7));
        assert_eq!(
            content_length("Content-Type: application/json\r\nContent-Length: 9\r\n"),
            Some(9),
            "another header before it is not a reason to miss it",
        );
        assert_eq!(content_length("X-Nothing: 1\r\n"), None);
    }

    /// The adapter counts lines from one. Off by one here is a breakpoint on
    /// the wrong line, every time.
    #[test]
    fn lines_arrive_one_based_and_leave_zero_based() {
        assert_eq!(zero_based(1), 0);
        assert_eq!(zero_based(244), 243);
        assert_eq!(zero_based(0), 0, "a bad line does not underflow");
    }

    /// A real `stackTrace` frame, as CodeLLDB answers it for a Rust test.
    #[test]
    fn a_frame_becomes_a_project_relative_position() {
        let frame = json!({
            "id": 1001,
            "name": "cf_core::math::quaternion::tests::rotations_do_not_commute",
            "source": { "path": r"E:\CodeBase\proj\core\src\math\quaternion.rs" },
            "line": 244,
            "instructionPointerReference": "0x7ff7b5c2d9b0",
        });
        let got = frame_of(0, &frame, &root());
        assert_eq!(got.file.as_deref(), Some("core/src/math/quaternion.rs"));
        assert_eq!(got.line, Some(243));
        assert_eq!(got.level, 0);
        assert_eq!(got.address, "0x7ff7b5c2d9b0");
    }

    /// A frame with no source — inside the standard library, or a thunk —
    /// keeps its name and simply has nowhere to point.
    #[test]
    fn a_frame_without_a_source_still_names_its_function() {
        let frame = json!({ "id": 7, "name": "ntdll!RtlUserThreadStart", "line": 0 });
        let got = frame_of(3, &frame, &root());
        assert_eq!(got.function, "ntdll!RtlUserThreadStart");
        assert_eq!(got.file, None);
    }

    #[test]
    fn a_variable_carries_its_type_and_whether_it_opens() {
        let listed = json!({
            "name": "roll",
            "value": "{w:0.707106769, x:0.707106769}",
            "type": "cf_core::math::quaternion::Quaternion",
            "variablesReference": 12,
        });
        let got = variable_of(&listed).expect("a variable");
        assert_eq!(got.name, "roll");
        assert_eq!(
            got.kind.as_deref(),
            Some("cf_core::math::quaternion::Quaternion")
        );
        assert_eq!(got.children, 1, "a non-zero reference is something to open");

        let leaf = json!({ "name": "n", "value": "3", "variablesReference": 0 });
        assert_eq!(variable_of(&leaf).expect("a variable").children, 0);
    }

    #[test]
    fn stop_reasons_map_to_the_model_and_strangers_land_on_other() {
        assert_eq!(reason_of("breakpoint"), StopReason::Breakpoint);
        assert_eq!(reason_of("step"), StopReason::Step);
        assert_eq!(reason_of("pause"), StopReason::Pause);
        assert_eq!(reason_of("exception"), StopReason::Signal);
        assert_eq!(reason_of("goodness knows"), StopReason::Other);
    }

    /// The adapter re-reports a breakpoint once it resolves — verified first
    /// with no location, then with one. The same id must update in place
    /// rather than pile up, and the line it was asked for must survive.
    #[test]
    fn a_resolved_breakpoint_updates_in_place_and_keeps_what_was_asked() {
        let mut state = DebugState::default();
        upsert_breakpoint(
            &mut state,
            &json!({
                "id": 1, "verified": true, "line": 244,
                "message": "Resolved locations: 0",
                "source": { "path": r"E:\CodeBase\proj\core\src\math\quaternion.rs" },
            }),
            &root(),
        );
        assert_eq!(state.breakpoints.len(), 1);
        assert_eq!(state.breakpoints[0].line, 243);
        assert_eq!(state.breakpoints[0].file, "core/src/math/quaternion.rs");

        upsert_breakpoint(
            &mut state,
            &json!({ "id": 1, "verified": true, "line": 246, "message": "Resolved locations: 1" }),
            &root(),
        );
        assert_eq!(state.breakpoints.len(), 1, "one id is one breakpoint");
        assert_eq!(state.breakpoints[0].line, 245, "it moved to where code is");
        assert_eq!(
            state.breakpoints[0].requested,
            Some(243),
            "and the margin still knows which line was clicked",
        );
        assert_eq!(
            state.breakpoints[0].file, "core/src/math/quaternion.rs",
            "an update without a source keeps the one it had",
        );
    }
}
