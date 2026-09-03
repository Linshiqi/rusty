//! The debugger's commands.
//!
//! One session at a time, like the terminal, and for the same reason: the
//! panel shows one, and a stranded gdb holds the ELF open so the next build
//! fails with a link error nobody connects to a debugger.
//!
//! State is pushed rather than polled. A stopped target changes nothing for
//! minutes at a time and then changes everything in one step; asking sixty
//! times a second would burn a core to learn nothing.

use std::sync::Arc;

use rusty_dbg::{AnySession, DapLaunch, DapSession, DebugState, Debugger, Events, Launch, Target};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
};

/// Start gdb against a target that is already listening, and stream the
/// session's state until it ends.
///
/// The target — QEMU frozen at reset, or probe-rs serving hardware — is
/// started by whoever asked for the debug run; this attaches to it. Two
/// things starting QEMU would be two QEMUs.
///
/// Which ELF and which port come from that run, not from here and not from
/// the frontend: it built the image, so it is the only thing that knows what
/// is executing.
#[tauri::command]
pub async fn debug_start(
    hardware: bool,
    on_state: Channel<DebugState>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    // Refusing beats attaching to whatever happens to be built. gdb reading a
    // different compilation from the one executing is the worst kind of wrong:
    // it answers every question, fluently, about another binary.
    let attach = state.attach().await.ok_or_else(|| {
        CommandError::new(
            "Nothing is waiting for a debugger. Start the run with Debug rather \
             than Run — it builds unoptimised, freezes the target at reset, and \
             hands the debugger the exact binary it booted.",
        )
    })?;
    let project = tokio::task::spawn_blocking({
        let root = root.clone();
        move || rusty_embed::project::detect(&root)
    })
    .await
    .map_err(|e| CommandError::new(format!("project detection panicked: {e}")))??;

    // Which gdb: the chip's architecture decides, and refusing beats
    // guessing — an Xtensa gdb cannot debug a RISC-V image, and the
    // failure it produces names neither.
    let gdb = rusty_embed::simulate::gdb_for(&project).ok_or_else(|| {
        CommandError::new(
            "No gdb for this chip is installed. The Simulate panel's tools card \
             installs the matching one.",
        )
    })?;

    let launch = Launch {
        gdb,
        elf: std::path::PathBuf::from(attach.elf),
        target: if hardware {
            Target::Probe { port: attach.port }
        } else {
            Target::Qemu { port: attach.port }
        },
        root,
    };

    let (debugger, events) = blocking("the debugger", move || Debugger::start(&launch)).await??;
    stream_session(AnySession::Gdb(debugger), events, on_state, &state).await
}

/// Debug one test built for this machine: build the test binaries, find the
/// one holding the test, run it under gdb with the standing breakpoints
/// placed. What `Debug` beside a test in the editor does.
///
/// Refusals come before the build — a five-minute compile in front of "gdb
/// cannot read this" is the wrong order — and the build itself streams into
/// the same channel the session will, as `output` lines: one channel carries
/// the whole story from `Compiling` to the exit code. Which binary holds the
/// test is asked of the binaries rather than inferred from the file's path;
/// see `rusty_embed::host_debug` for why.
#[tauri::command]
pub async fn debug_test(
    filter: String,
    breakpoints: Vec<(String, u32)>,
    on_state: Channel<DebugState>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    use rusty_embed::{host_debug, process};

    // The opened project, not `firmware_root`: a host test is built and run
    // for this machine, and the firmware crate has no test harness to link.
    let root = state.root().await.ok_or_else(CommandError::no_project)?;

    let host = blocking("rustc", {
        let root = root.clone();
        move || host_debug::host_triple(&root)
    })
    .await??;
    // Which debugger can read what this toolchain builds. gdb where the debug
    // information is DWARF; a debug adapter — LLDB — where it is a PDB, which
    // is Windows' default and the case gdb cannot serve at all.
    enum Backend {
        Gdb(std::path::PathBuf),
        Adapters(Vec<std::path::PathBuf>),
    }
    let backend = match host_debug::gdb_reads(&host) {
        Ok(()) => Backend::Gdb(host_debug::host_gdb().ok_or_else(|| {
            CommandError::new(
                "No `gdb` for this machine is on PATH. The chips' debuggers — \
                 riscv32-esp-elf-gdb, xtensa-esp32-elf-gdb — cannot debug a process built \
                 for this machine; install the platform's own gdb.",
            )
        })?),
        Err(why) => {
            let adapters = host_debug::host_adapters();
            if adapters.is_empty() {
                return Err(CommandError::from(why));
            }
            Backend::Adapters(adapters)
        }
    };

    // The build, visibly, and stoppable like any other command.
    let plan = host_debug::build_plan();
    let session = process::spawn(&plan, Some(&root))?;
    let ours = state.start_session(session.stopper()).await;
    let progress = on_state.clone();
    let code = blocking("the test build", move || {
        while let Some(line) = session.recv() {
            let _ = progress.send(DebugState {
                output: vec![line.text],
                ..DebugState::default()
            });
        }
        session.wait()
    })
    .await?;
    state.release_session(&ours).await;
    if code != Some(0) {
        return Err(CommandError::new(format!(
            "`{}` did not succeed{}; its output is in the dock.",
            plan.display,
            code.map(|c| format!(" (exit code {c})"))
                .unwrap_or_default(),
        )));
    }

    let exe = blocking("finding the test binary", {
        let root = root.clone();
        let filter = filter.clone();
        move || -> rusty_embed::Result<std::path::PathBuf> {
            let json = host_debug::built_json(&root)?;
            let built = host_debug::test_executables(&json);
            host_debug::binary_holding(&built, &filter)
        }
    })
    .await??;

    // One thread, so a step is a step and not a switch to whichever test the
    // scheduler picked; `--nocapture` because what the test prints is usually
    // why it is being run this way.
    let args = vec![
        filter,
        "--nocapture".to_string(),
        "--test-threads=1".to_string(),
    ];

    let (session, events) = match backend {
        Backend::Gdb(gdb) => {
            let launch = Launch {
                gdb,
                elf: exe,
                target: Target::Host { args },
                root,
            };
            let (session, events) =
                blocking("the debugger", move || Debugger::start(&launch)).await??;
            (AnySession::Gdb(session), events)
        }
        // Each in turn: "the adapter exists" and "the adapter answers" are
        // different facts, and LLVM ships a Windows `lldb-dap` that starts and
        // then does nothing at all. The first one that talks is the one used.
        Backend::Adapters(adapters) => {
            let mut refused: Vec<String> = Vec::new();
            let mut started = None;
            for adapter in adapters {
                let launch = DapLaunch {
                    adapter: adapter.clone(),
                    program: exe.clone(),
                    args: args.clone(),
                    root: root.clone(),
                    breakpoints: breakpoints.clone(),
                };
                match blocking("the debug adapter", move || DapSession::start(&launch)).await? {
                    Ok(pair) => {
                        started = Some(pair);
                        break;
                    }
                    Err(e) => refused.push(format!("{}: {e}", adapter.display())),
                }
            }
            let Some((session, events)) = started else {
                return Err(CommandError::new(format!(
                    "No debug adapter on this machine would start, so there is nothing to \
                     debug with. Tried:\n{}",
                    refused.join("\n"),
                )));
            };
            (AnySession::Dap(session), events)
        }
    };
    stream_session(session, events, on_state, &state).await
}

/// Own the session for its life: register it, push every state to the
/// panel, and let go when it ends. Shared by the two ways a session starts.
async fn stream_session(
    debugger: AnySession,
    events: Events,
    on_state: Channel<DebugState>,
    state: &AppState,
) -> Result<(), CommandError> {
    let debugger = Arc::new(debugger);
    let ours = Arc::clone(&debugger);
    state.set_debugger(Some(Arc::clone(&debugger))).await;

    // Blocking by nature — it sits on a channel — so it belongs on a
    // blocking thread rather than starving an async worker for the life of
    // a debug session.
    blocking("the debug reader", move || {
        while let Some(update) = events.next() {
            let ended = update.exited.is_some();
            if on_state.send(update).is_err() {
                // The WebView itself is gone — the only failure a send
                // reports. A debugger with no window is a process holding the
                // ELF hostage. (A closed panel is not this: the slot in
                // `AppState` is what ends a session the user walked away from.)
                debugger.stop();
                break;
            }
            if ended {
                break;
            }
        }
    })
    .await?;

    state.release_debugger(&ours).await;
    Ok(())
}

/// Place or clear a breakpoint. Lines are zero-based on this side, as
/// everywhere else that crosses this boundary.
#[tauri::command]
pub async fn debug_breakpoint(
    file: String,
    line: u32,
    remove: Option<u32>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    Ok(match remove {
        Some(number) => debugger.remove_breakpoint(number),
        None => debugger.add_breakpoint(&file, line),
    }?)
}

/// Resume, pause, or step. One command with a verb rather than five
/// commands: they differ by one MI line and share every guard.
#[tauri::command]
pub async fn debug_control(action: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    let result = match action.as_str() {
        "resume" => debugger.resume(),
        "pause" => debugger.pause(),
        "over" => debugger.step_over(),
        "into" => debugger.step_into(),
        "out" => debugger.step_out(),
        other => {
            return Err(CommandError::new(format!(
                "{other} is not something the debugger can do",
            )));
        }
    };
    Ok(result?)
}

/// Select a stack frame and read its variables.
#[tauri::command]
pub async fn debug_frame(level: u32, state: State<'_, AppState>) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    Ok(debugger.refresh(level)?)
}

/// End the session — and the run it was attached to.
///
/// The Debug button is what booted QEMU, frozen, with the gdbstub listening.
/// Killing only gdb left that QEMU executing with no client and no owner: the
/// board kept updating, the panel already said "nothing is being debugged",
/// and the one control that could still have stopped it was on a different
/// panel. One action started it, so one action ends it.
///
/// The recorded attach point is what says the run belongs to a debug session;
/// a plain Run never sets it, and stopping a debugger it has none of does
/// nothing to it.
#[tauri::command]
pub async fn debug_stop(state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(debugger) = state.debugger().await {
        debugger.stop();
    }
    if state.attach().await.is_some() {
        state.stop_session().await;
    }
    Ok(())
}

/// Read a span of target memory — a peripheral's register block.
#[tauri::command]
pub async fn debug_read_memory(
    address: u64,
    bytes: u32,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let debugger = state
        .debugger()
        .await
        .ok_or_else(|| CommandError::new("No debug session is running."))?;
    Ok(debugger.read_memory(address, bytes)?)
}

/// The chip's peripherals, from whichever SVD this machine has.
///
/// Absent is not an error: a chip with no SVD is a register view that
/// cannot open, and the panel says which file it wants and offers to fetch
/// it. Guessing register addresses would be the worst possible answer.
#[tauri::command]
pub async fn register_map(
    state: State<'_, AppState>,
) -> Result<Option<rusty_embed::RegisterMap>, CommandError> {
    let root = state.firmware_root().await;
    let Some(chip) = state.chip().await else {
        return Ok(None);
    };
    tokio::task::spawn_blocking(move || {
        let path = rusty_embed::svd::find(&chip, root.as_deref())?;
        let xml = std::fs::read_to_string(path).ok()?;
        Some(rusty_embed::svd::parse(&xml))
    })
    .await
    .map_err(|e| CommandError::new(format!("reading the SVD panicked: {e}")))
}

/// Fetch the chip's SVD, streaming progress like every other download.
#[tauri::command]
pub async fn fetch_svd(
    on_line: Channel<rusty_embed::LogLine>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let chip = state
        .chip()
        .await
        .ok_or_else(|| CommandError::new("The chip is unknown, so rusty cannot pick an SVD."))?;
    tokio::task::spawn_blocking(move || {
        rusty_embed::svd::fetch(&chip, |line| {
            let _ = on_line.send(rusty_embed::LogLine {
                stream: rusty_embed::LogStream::Stdout,
                text: line,
                level: None,
            });
        })
    })
    .await
    .map_err(|e| CommandError::new(format!("the SVD download panicked: {e}")))?
    .map(|_| ())
    .map_err(CommandError::from)
}
