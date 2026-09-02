//! Simulation commands: plan the three steps, then run them end to end.

use std::path::Path;

use rusty_embed::{LogLine, LogStream, SimBoard, SimPlan, install, process, project, simulate};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
    stream,
};

/// A line of rusty's own into the dock, beside the tools' output.
fn note(on_line: &Channel<LogLine>, text: impl Into<String>) {
    let _ = on_line.send(LogLine {
        stream: LogStream::Stdout,
        text: text.into(),
        level: None,
    });
}

/// Persist the board editor's layout into the project's `.rusty/sim.toml`.
#[tauri::command]
pub async fn save_sim_board(
    board: SimBoard,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    blocking("saving the board", move || {
        simulate::save_board(&root, &board)
    })
    .await?
    .map_err(CommandError::from)
}

/// How this project would be simulated, or exactly why it cannot be.
#[tauri::command]
pub async fn plan_simulation(state: State<'_, AppState>) -> Result<SimPlan, CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    // Detection reads the project; the plan probes PATH and the data directory
    // for every tool it names.
    Ok(blocking("planning the simulation", move || {
        project::detect(&root).map(|detected| simulate::plan(&detected, false))
    })
    .await??)
}

/// How a named tool gets installed.
///
/// Knowledge that belongs to `rusty_embed::install` — it is the module that
/// knows which names it fetches as archives and which it hands to `cargo
/// install` — and is here only until it grows an `install::method(name)`. The
/// test pins the rule so the move is mechanical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallMethod {
    /// A prebuilt archive rusty downloads and unpacks itself.
    Archive(Archive),
    /// Commands the shared session runner streams into the dock.
    Steps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Archive {
    Qemu,
    Gdb,
    Gcc,
}

fn install_method(name: &str) -> InstallMethod {
    if name.starts_with("qemu-system-") {
        InstallMethod::Archive(Archive::Qemu)
    } else if name.ends_with("-gdb") {
        InstallMethod::Archive(Archive::Gdb)
    } else if name.ends_with("-gcc") {
        InstallMethod::Archive(Archive::Gcc)
    } else {
        InstallMethod::Steps
    }
}

/// Install one missing tool, streaming every line — the panel's one-click.
#[tauri::command]
pub async fn install_sim_tool(
    name: String,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let archive = match install_method(&name) {
        InstallMethod::Archive(archive) => archive,
        InstallMethod::Steps => return install_steps(&name, on_line, state).await,
    };
    install_archive(archive, &name, on_line, state).await
}

/// `cargo install` and friends, one step after another, stopping at the first
/// failure — two `cargo install`s at once fight over the package-cache lock.
async fn install_steps(
    name: &str,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let steps = install::install_steps(name)?;

    let mut last_code = None;
    let mut current = None;
    for step in steps {
        note(&on_line, format!("$ {}", step.display));
        let session = process::spawn(&step, None)?;
        current = Some(state.start_session(session.stopper()).await);

        let feed = on_line.clone();
        let code = blocking("the install step", move || {
            stream::forward(|| session.recv(), &feed);
            session.wait()
        })
        .await?;

        last_code = code;
        if code != Some(0) {
            break;
        }
    }

    if let Some(ours) = current {
        state.release_session(&ours).await;
    }
    Ok(last_code)
}

/// The captured waveform, written where the build artefacts already live.
/// Returns the absolute path so the dock can name it.
#[tauri::command]
pub async fn save_sim_trace(
    text: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    blocking("saving the trace", move || {
        let dir = root.join("target/rusty-sim");
        std::fs::create_dir_all(&dir)
            .map_err(|e| CommandError::new(format!("could not create {}: {e}", dir.display())))?;
        let path = dir.join("trace.vcd");
        std::fs::write(&path, text)
            .map_err(|e| CommandError::new(format!("could not write {}: {e}", path.display())))?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await?
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
            let _ = stream.write_all(pin_line(pin, level).as_bytes());
            let _ = stream.flush();
        }
    }
}

/// The pin channel's inbound line, `<pin>=<level>\n` — what rusty's GPIO
/// model reads on the socket it was told to listen on.
///
/// One of the two inbound wire formats this file writes; the other is
/// [`button_press`]'s `B<pin>=<level>` on the console. Both are the serial
/// protocol's and belong in `rusty_embed::protocol` beside its parsers, where
/// the outbound half already is; they are here, tested, until that move.
fn pin_line(pin: u32, level: u8) -> String {
    format!("{pin}={level}\n")
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

        let mut lines = BufReader::new(reader)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.is_empty());
        stream::forward(
            || {
                lines.next().map(|text| LogLine {
                    stream: LogStream::Stdout,
                    text,
                    level: None,
                })
            },
            &feed,
        );
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
/// Any non-zero level is high, because the message is a level and not a
/// count.
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
    archive: Archive,
    name: &str,
    on_line: Channel<LogLine>,
    state: State<'_, AppState>,
) -> Result<Option<i32>, CommandError> {
    let plan = {
        let name = name.to_string();
        blocking("planning the download", move || match archive {
            Archive::Gcc => install::gcc_download(&name),
            Archive::Gdb => install::gdb_download(&name),
            Archive::Qemu => install::qemu_download(&name),
        })
        .await?
        .map_err(CommandError::from)?
    };

    let feed = on_line.clone();
    let target = plan.archive.clone();
    let urls = plan.urls.clone();
    blocking("the download", move || {
        install::download(&urls, &target, |line| note(&feed, line))
    })
    .await?
    .map_err(CommandError::from)?;

    note(&on_line, format!("$ {}", plan.extract.display));
    let session = process::spawn(&plan.extract, None)?;
    let ours = state.start_session(session.stopper()).await;
    let feed = on_line.clone();
    let code = blocking("the extraction", move || {
        stream::forward(|| session.recv(), &feed);
        session.wait()
    })
    .await?;
    state.release_session(&ours).await;

    if code == Some(0) {
        let _ = std::fs::remove_file(&plan.archive);
    }
    Ok(code)
}

/// QEMU's flags for a debug run: frozen at the first instruction with the
/// gdbstub listening on the plan's port, and a deterministic virtual clock,
/// because a debugger that perturbs timing hides the bugs people came to see.
///
/// `-gdb tcp::<port>` rather than `-s`. `-s` means `-gdb tcp::1234` by QEMU's
/// convention, and `rusty_embed::simulate` writes `1234` into the plan by its
/// own — two places agreeing by luck. The port is the plan's, the debugger
/// attaches to the plan's, and only the plan says what it is.
fn debug_args(port: u16) -> Vec<String> {
    vec![
        "-gdb".to_string(),
        format!("tcp::{port}"),
        "-S".to_string(),
        "-icount".to_string(),
        "shift=auto,sleep=on".to_string(),
    ]
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
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    let plan = {
        let root = root.clone();
        blocking("planning the simulation", move || {
            project::detect(&root).map(|detected| simulate::plan(&detected, debug))
        })
        .await??
    };

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
    let gdb_port = plan.debug.as_ref().map(|target| target.port);
    if debug && gdb_port.is_none() {
        let how = plan.debug_tool.as_ref().map_or_else(
            || "none is installed".to_string(),
            |tool| format!("{} is not installed — {}", tool.name, tool.install),
        );
        return Err(CommandError::new(format!(
            "Debugging needs a gdb that matches this chip, and {how}. Run without \
             the debugger, or install it from the Simulate panel's tools card.",
        )));
    }

    {
        let root = root.clone();
        blocking("preparing target/rusty-sim", move || {
            simulate::prepare(&root)
        })
        .await?
        .map_err(|e| CommandError::new(format!("could not create target/rusty-sim: {e}")))?;
    }

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
    let mut current = None;
    let total = plan.steps.len();
    for (index, mut step) in plan.steps.into_iter().enumerate() {
        note(&on_line, format!("$ {}", step.display));

        let is_emulator = step.program.contains("qemu-system");
        if let (true, Some(port)) = (debug && is_emulator, gdb_port) {
            let extra = debug_args(port);
            step.display = format!("{} {}", step.display, extra.join(" "));
            step.args.extend(extra);
            note(
                &on_line,
                format!(
                    "[rusty:debug] gdbstub on :{port}, cpu frozen — attaching, then running to \
                     your breakpoints"
                ),
            );
        }

        // Ask the emulator that is about to run, not the one that was
        // installed last week: a user who replaced the binary by hand gets an
        // answer about the binary they replaced it with. `has_gpio_model`
        // caches on path, size and mtime, so this costs one scan per install
        // — a scan, so off the async thread.
        let mut pins_port = None;
        if is_emulator {
            let program = step.program.clone();
            let has_model = blocking("inspecting the emulator", move || {
                simulate::has_gpio_model(Path::new(&program))
            })
            .await?;
            if has_model && let Some(port) = free_port() {
                let extra = simulate::pins_args(port);
                step.display = format!("{} {}", step.display, extra.join(" "));
                step.args.extend(extra);
                pins_port = Some(port);
                // Said in the dock and read by the board, so the panel can stop
                // claiming these levels came from the firmware. One line per
                // run — the pin reports themselves never reach the log.
                note(
                    &on_line,
                    "[rusty:pins] emulator — pin state read from the GPIO registers",
                );
            }
        }

        let session = process::spawn(&step, Some(root.as_path()))?;
        current = Some(state.start_session(session.stopper()).await);
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
        let code = blocking("the simulation step", move || {
            stream::forward(|| session.recv(), &feed);
            session.wait()
        })
        .await?;

        last_code = code;
        let is_boot = index + 1 == total;
        if !is_boot && code != Some(0) {
            // A failed build or image stops the pipeline; the lines that
            // explain it are already in the dock.
            break;
        }
    }

    if let Some(ours) = current {
        state.release_session(&ours).await;
    }
    // QEMU has exited; there is no longer anything to attach to.
    state.set_attach(None).await;
    Ok(last_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archives_are_told_from_cargo_installs_by_name() {
        assert_eq!(
            install_method("qemu-system-riscv32"),
            InstallMethod::Archive(Archive::Qemu)
        );
        assert_eq!(
            install_method("qemu-system-xtensa"),
            InstallMethod::Archive(Archive::Qemu)
        );
        assert_eq!(
            install_method("riscv32-esp-elf-gdb"),
            InstallMethod::Archive(Archive::Gdb)
        );
        assert_eq!(
            install_method("xtensa-esp-elf-gcc"),
            InstallMethod::Archive(Archive::Gcc)
        );
        assert_eq!(install_method("espflash"), InstallMethod::Steps);
        assert_eq!(install_method("probe-rs"), InstallMethod::Steps);
    }

    /// The port comes from the plan and nowhere else — no `-s`, whose meaning
    /// is a convention the plan happened to agree with.
    #[test]
    fn a_debug_run_arms_the_gdbstub_on_the_plans_port() {
        let args = debug_args(1234);
        assert_eq!(
            args,
            vec!["-gdb", "tcp::1234", "-S", "-icount", "shift=auto,sleep=on"]
        );
        assert!(
            !args.iter().any(|a| a == "-s"),
            "no shorthand with a hidden port"
        );
        assert!(debug_args(4321).contains(&"tcp::4321".to_string()));
    }

    /// The two inbound wire formats this file writes.
    #[test]
    fn a_button_press_is_read_off_the_console_and_written_to_the_pin_channel() {
        assert_eq!(button_press("B14=1"), Some((14, 1)));
        assert_eq!(button_press("B14=0"), Some((14, 0)));
        assert_eq!(
            button_press(" B2 = 7 "),
            Some((2, 1)),
            "a level is a level: non-zero is high, whitespace is noise",
        );
        assert_eq!(
            button_press("P34=128"),
            None,
            "the potentiometer is analog, and a GPIO carries one bit"
        );
        assert_eq!(button_press("14=1"), None, "the prefix is the message");
        assert_eq!(button_press("B=1"), None);
        assert_eq!(button_press("Bx=1"), None);
        assert_eq!(button_press("Skp=8.5"), None, "a tunable is not a pin");

        assert_eq!(pin_line(14, 1), "14=1\n");
        assert_eq!(
            button_press("B14=1").map(|(pin, level)| pin_line(pin, level)),
            Some("14=1\n".to_string()),
            "the console message and the pin line name the same pin at the same level",
        );
    }
}
