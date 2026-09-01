//! Simulation commands: plan the three steps, then run them end to end.

use rusty_embed::{LogLine, LogStream, SimBoard, SimPlan, install, process, project, simulate};
use tauri::{State, ipc::Channel};

use crate::{error::CommandError, state::AppState};

/// Persist the board editor's layout into the project's `.rusty/sim.toml`.
#[tauri::command]
pub async fn save_sim_board(
    board: SimBoard,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    simulate::save_board(&root, &board).map_err(CommandError::new)
}

/// How this project would be simulated, or exactly why it cannot be.
#[tauri::command]
pub async fn plan_simulation(state: State<'_, AppState>) -> Result<SimPlan, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let detected = project::detect(&root)?;
    Ok(simulate::plan(&detected, false))
}

/// Install one missing tool, streaming every line — the panel's one-click.
#[tauri::command]
pub async fn install_sim_tool(
    name: String,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    if name.starts_with("qemu-system-") || name.ends_with("-gdb") || name.ends_with("-gcc") {
        return install_archive(&name, on_line, state).await;
    }
    let steps = install::install_steps(&name).map_err(CommandError::new)?;

    let mut last_code = None;
    for step in steps {
        let _ = on_line.send(LogLine {
            stream: LogStream::Stdout,
            text: format!("$ {}", step.display),
            level: None,
        });
        let session = process::spawn(&step, None)?;
        state.start_session(session.stopper()).await;

        let feed = on_line.clone();
        let code = tokio::task::spawn_blocking(move || {
            while let Some(line) = session.recv() {
                if feed.send(line).is_err() {
                    break;
                }
            }
            session.wait()
        })
        .await
        .map_err(|e| CommandError::new(format!("install step panicked: {e}")))?;

        last_code = code;
        if code != Some(0) {
            break;
        }
    }

    state.stop_session().await;
    Ok(last_code)
}

/// The captured waveform, written where the build artefacts already live.
/// Returns the absolute path so the dock can name it.
#[tauri::command]
pub async fn save_sim_trace(
    text: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let dir = root.join("target/rusty-sim");
    std::fs::create_dir_all(&dir)
        .map_err(|e| CommandError::new(format!("could not create {}: {e}", dir.display())))?;
    let path = dir.join("trace.vcd");
    std::fs::write(&path, text)
        .map_err(|e| CommandError::new(format!("could not write {}: {e}", path.display())))?;
    Ok(path.to_string_lossy().into_owned())
}

/// The emulator's pin channel: its own account of every pin, and the way to
/// drive one back.
///
/// Only rusty's build of QEMU has it. Espressif's discards every GPIO write,
/// so there is nothing on the other end and this is simply absent.
#[derive(Clone)]
pub struct PinChannel {
    out: std::sync::Arc<std::sync::Mutex<Option<std::net::TcpStream>>>,
}

impl PinChannel {
    /// Drive a pin from the host — a button press, reaching the firmware
    /// through `GPIO_IN` rather than through a message it had to be written
    /// to expect.
    pub fn drive(&self, pin: u32, level: u8) {
        use std::io::Write;
        if let Ok(mut socket) = self.out.lock()
            && let Some(stream) = socket.as_mut()
        {
            let _ = writeln!(stream, "{pin}={level}");
            let _ = stream.flush();
        }
    }
}

/// A port nothing else is on, learned by binding and letting go.
///
/// QEMU listens and rusty connects — the arrangement the CI gate boots. The
/// gap between releasing this and QEMU claiming it is a race in theory; in
/// practice the alternative is a fixed port, and a fixed port is a second
/// simulation failing to start for a reason the panel cannot explain.
fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Connect to the emulator's pin channel and feed every line into the same
/// stream the serial console uses.
///
/// The same channel deliberately: `[rusty:gpio@…] 0=1` is the line the
/// frontend already parses, and it is parsed in exactly one place. Reading
/// this stream anywhere else would be the second reader that made telemetry
/// work in the simulator and vanish on hardware.
fn open_pin_channel(port: u16, feed: Channel<LogLine>) -> PinChannel {
    use std::io::{BufRead, BufReader};

    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let handle = PinChannel { out: out.clone() };

    std::thread::spawn(move || {
        // QEMU has to get as far as opening its listening socket, which is
        // after argument parsing and machine creation. Retry rather than
        // assume, and give up quietly: a missing pin channel is a board that
        // falls back to the firmware's own narration, not an error.
        let mut stream = None;
        for _ in 0..100 {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => {
                    stream = Some(socket);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
        let Some(socket) = stream else {
            return;
        };
        let Ok(reader) = socket.try_clone() else {
            return;
        };
        if let Ok(mut slot) = out.lock() {
            *slot = Some(socket);
        }

        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if line.is_empty() {
                continue;
            }
            if feed
                .send(LogLine {
                    stream: LogStream::Stdout,
                    text: line,
                    level: None,
                })
                .is_err()
            {
                break;
            }
        }
    });

    handle
}

/// A line into the running simulation — how a button press on the board view
/// reaches the firmware.
///
/// Both ways, when both exist. `B14=1` goes to the console for firmware that
/// reads rusty's text protocol, which is what the bundled examples do; with
/// the GPIO model there is also a real pin to drive, and unmodified firmware
/// reading `Input::is_high()` only sees that one. Sending only the second
/// would break every example; sending only the first is the limitation this
/// whole path exists to remove.
#[tauri::command]
pub async fn sim_send(text: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(input) = state.session_input().await {
        input.send_line(&text);
    }
    if let (Some(pins), Some((pin, level))) = (state.pins().await, button_press(&text)) {
        pins.drive(pin, level);
    }
    Ok(())
}

/// `B<pin>=<level>` — the board's button message, and nothing else.
///
/// Deliberately not the potentiometer's `P34=128`: a GPIO carries one bit,
/// and squeezing an analog value into it would put a pin somewhere between
/// the two levels it can have. That needs the ADC modelled, which it is not.
fn button_press(text: &str) -> Option<(u32, u8)> {
    let (pin, level) = text.trim().strip_prefix('B')?.split_once('=')?;
    let level: u8 = level.trim().parse().ok()?;
    Some((pin.trim().parse().ok()?, u8::from(level != 0)))
}

/// QEMU: in-process download with a mirror fallback, then tar extraction.
///
/// The download runs on rustls rather than through curl — the user's curl
/// died with a schannel abort and then could not reach github.com at all;
/// the Espressif mirror is the second URL for exactly that network.
async fn install_archive(
    name: &str,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let plan = if name.ends_with("-gcc") {
        install::gcc_download(name).map_err(CommandError::new)?
    } else if name.ends_with("-gdb") {
        install::gdb_download(name).map_err(CommandError::new)?
    } else {
        install::qemu_download(name).map_err(CommandError::new)?
    };

    let feed = on_line.clone();
    let archive = plan.archive.clone();
    let urls = plan.urls.clone();
    let downloaded = tokio::task::spawn_blocking(move || {
        install::download(&urls, &archive, |line| {
            let _ = feed.send(LogLine {
                stream: LogStream::Stdout,
                text: line,
                level: None,
            });
        })
    })
    .await
    .map_err(|e| CommandError::new(format!("download panicked: {e}")))?;
    if let Err(error) = downloaded {
        return Err(CommandError::new(error));
    }

    let _ = on_line.send(LogLine {
        stream: LogStream::Stdout,
        text: format!("$ {}", plan.extract.display),
        level: None,
    });
    let session = process::spawn(&plan.extract, None)?;
    state.start_session(session.stopper()).await;
    let feed = on_line.clone();
    let code = tokio::task::spawn_blocking(move || {
        while let Some(line) = session.recv() {
            if feed.send(line).is_err() {
                break;
            }
        }
        session.wait()
    })
    .await
    .map_err(|e| CommandError::new(format!("extraction panicked: {e}")))?;
    state.stop_session().await;

    if code == Some(0) {
        let _ = std::fs::remove_file(&plan.archive);
    }
    Ok(code)
}

/// Build, image, boot — streaming every line, stoppable at any step.
///
/// The first two steps must exit zero before the next runs; QEMU itself runs
/// until the user stops it (the same session Stop every panel shares).
#[tauri::command]
pub async fn run_simulation(
    debug: bool,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let root = state.root().await.ok_or_else(CommandError::no_project)?;
    let detected = project::detect(&root)?;
    let plan = simulate::plan(&detected, debug);

    if !plan.supported {
        return Err(CommandError::new(
            plan.reason
                .unwrap_or_else(|| "this project cannot be simulated".to_string()),
        ));
    }
    if !plan.missing.is_empty() {
        let mut lines = vec!["simulation needs tools that are not installed:".to_string()];
        for tool in &plan.missing {
            lines.push(format!("  {} — {}", tool.name, tool.install));
        }
        return Err(CommandError::new(lines.join("\n")));
    }

    // A debug run freezes the CPU at reset so breakpoints can be placed before
    // the first instruction. With no gdb to place them, that freeze is
    // permanent: a blank board, a live QEMU, and nothing anywhere saying why.
    // Refuse while there is still something useful to say.
    if debug && plan.debug.is_none() {
        let how = plan.debug_tool.as_ref().map_or_else(
            || "none is installed".to_string(),
            |tool| format!("{} is not installed — {}", tool.name, tool.install),
        );
        return Err(CommandError::new(format!(
            "Debugging needs a gdb that matches this chip, and {how}. Run without \
             the debugger, or install it from the Simulate panel's tools card.",
        )));
    }

    simulate::prepare(&root)
        .map_err(|e| CommandError::new(format!("could not create target/rusty-sim: {e}")))?;

    // Where a debugger may attach — and, crucially, *which ELF it must read*.
    // Only a debug run arms a gdbstub, and only this run knows whether it
    // built the optimised binary or the unoptimised one, so recording it
    // anywhere else is a second copy of the decision waiting to disagree.
    state
        .set_attach(match (debug, plan.debug.as_ref()) {
            (true, Some(target)) => Some(crate::state::Attach {
                elf: target.elf.clone(),
                port: target.port,
            }),
            // A plain Run arms no gdbstub, and must not leave a stale target
            // behind for a later attach to find.
            _ => None,
        })
        .await;

    let mut last_code = None;
    let total = plan.steps.len();
    for (index, step) in plan.steps.into_iter().enumerate() {
        let _ = on_line.send(LogLine {
            stream: LogStream::Stdout,
            text: format!("$ {}", step.display),
            level: None,
        });

        let mut step = if debug && step.program.contains("qemu-system") {
            // Frozen at the first instruction with the gdbstub listening —
            // and a deterministic virtual clock, because a debugger that
            // perturbs timing hides the bugs people came to see.
            let mut armed = step.clone();
            for flag in ["-s", "-S", "-icount", "shift=auto,sleep=on"] {
                armed.args.push(flag.to_string());
            }
            armed.display = format!("{} -s -S -icount shift=auto,sleep=on", armed.display);
            let _ = on_line.send(LogLine {
                stream: LogStream::Stdout,
                text: "[rusty:debug] gdbstub on :1234, cpu frozen — attaching, then \
                       running to your breakpoints"
                    .to_string(),
                level: None,
            });
            armed
        } else {
            step.clone()
        };
        // Ask the emulator that is about to run, not the one that was
        // installed last week: a user who replaced the binary by hand gets an
        // answer about the binary they replaced it with. `has_gpio_model`
        // caches on path, size and mtime, so this costs one scan per install.
        let mut pins_port = None;
        if step.program.contains("qemu-system")
            && simulate::has_gpio_model(std::path::Path::new(&step.program))
            && let Some(port) = free_port()
        {
            let extra = simulate::pins_args(port);
            step.display = format!("{} {}", step.display, extra.join(" "));
            step.args.extend(extra);
            pins_port = Some(port);
            // Said in the dock and read by the board, so the panel can stop
            // claiming these levels came from the firmware. One line per run —
            // the pin reports themselves never reach the log.
            let _ = on_line.send(LogLine {
                stream: LogStream::Stdout,
                text: "[rusty:pins] emulator — pin state read from the GPIO registers".to_string(),
                level: None,
            });
        }

        let session = process::spawn(&step, Some(root.as_path()))?;
        state.start_session(session.stopper()).await;
        // The boot step is QEMU; its stdin is the board's input path.
        state.set_session_input(Some(session.input())).await;
        // After the spawn, because there is nothing to connect to until QEMU
        // has opened its listening socket. The reader retries and gives up
        // quietly: a pin channel that never answers leaves the board on the
        // firmware's own narration, which is where it has always been.
        if let Some(port) = pins_port {
            state
                .set_pins(Some(open_pin_channel(port, on_line.clone())))
                .await;
        }

        let feed = on_line.clone();
        let code = tokio::task::spawn_blocking(move || {
            while let Some(line) = session.recv() {
                if feed.send(line).is_err() {
                    break;
                }
            }
            session.wait()
        })
        .await
        .map_err(|e| CommandError::new(format!("simulation step panicked: {e}")))?;

        last_code = code;
        let is_boot = index + 1 == total;
        if !is_boot && code != Some(0) {
            // A failed build or image stops the pipeline; the lines that
            // explain it are already in the dock.
            break;
        }
    }

    state.stop_session().await;
    // QEMU has exited; there is no longer anything to attach to.
    state.set_attach(None).await;
    Ok(last_code)
}
