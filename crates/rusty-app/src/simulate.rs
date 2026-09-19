//! Simulation commands: plan the three steps, then run them end to end.

use std::path::Path;

use rusty_embed::{
    LogLine, LogStream, Sheet, SimPlan, Symbol, install, process, project, simulate,
};
use tauri::{State, ipc::Channel};

use crate::{
    error::CommandError,
    state::{AppState, blocking},
    stream,
};

/// A line of rusty's own into the dock, beside the tools' output.
pub(crate) fn note(on_line: &Channel<LogLine>, text: impl Into<String>) {
    let _ = on_line.send(LogLine {
        stream: LogStream::Stdout,
        text: text.into(),
        level: None,
    });
}

/// Persist the board editor's sheet into the project's `.rusty/sim.toml`.
#[tauri::command]
pub async fn save_sim_board(board: Sheet, state: State<'_, AppState>) -> Result<(), CommandError> {
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

/// Read a `.kicad_sch` onto the sheet.
///
/// Answers with the sheet *and* what it could not bring across, because the
/// second is the half a user has to see: a schematic drawn in KiCad has a
/// microcontroller of its own where rusty's has a devkit, so an imported
/// board draws and checks and does not simulate until something is wired to
/// `U1`. Said here rather than discovered when Run does nothing.
#[tauri::command]
pub async fn sim_import_kicad(
    path: String,
    state: State<'_, AppState>,
) -> Result<Sheet, CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    let chip = state.chip().await.unwrap_or_else(|| "esp32c3".to_string());
    blocking("importing the schematic", move || {
        rusty_embed::schematic::import(&root, Path::new(&path), &chip)
    })
    .await?
    .map_err(CommandError::from)
}

/// Read a Wokwi `diagram.json` onto the sheet — the parts rusty has a
/// counterpart for, wired to this chip's devkit, and in the sheet's notes
/// everything that did not come across and why.
#[tauri::command]
pub async fn sim_import_wokwi(
    path: String,
    state: State<'_, AppState>,
) -> Result<Sheet, CommandError> {
    let root = state
        .firmware_root()
        .await
        .ok_or_else(CommandError::no_project)?;
    let chip = state.chip().await.unwrap_or_else(|| "esp32c3".to_string());
    blocking("importing the diagram", move || {
        rusty_embed::schematic::import_wokwi(&root, Path::new(&path), &chip)
    })
    .await?
    .map_err(CommandError::from)
}

/// Write the sheet out as `.kicad_sch`, patching whatever is at `path`.
#[tauri::command]
pub async fn sim_export_kicad(path: String, board: Sheet) -> Result<Vec<String>, CommandError> {
    blocking("exporting the schematic", move || {
        rusty_embed::schematic::export(Path::new(&path), &board)
    })
    .await?
    .map_err(CommandError::from)
}

/// An LCSC part as a schematic symbol, fetched from EasyEDA's component
/// service and kept in the data directory's `symbols/lcsc.kicad_sym`, so
/// the next plan offers it in the library. The symbol comes back for the
/// sheet to place at once.
#[tauri::command]
pub async fn sim_import_symbol(number: String) -> Result<Symbol, CommandError> {
    let imported = blocking("importing the symbol", move || {
        rusty_embed::schematic::easyeda::import(&number)
    })
    .await?
    .map_err(CommandError::from)?;
    for warning in &imported.warnings {
        eprintln!("symbol import: {warning}");
    }
    Ok(imported.symbol)
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
    CodeLldb,
}

fn install_method(name: &str) -> InstallMethod {
    if name.starts_with("qemu-system-") {
        InstallMethod::Archive(Archive::Qemu)
    } else if name == "codelldb" {
        InstallMethod::Archive(Archive::CodeLldb)
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

/// Stop the emulator's clock, or start it again.
///
/// QEMU's own machine protocol, on the socket the run opened: the monitor
/// is otherwise multiplexed onto the console the firmware reads, and a
/// `stop` typed there is a line the firmware might have wanted. Each call
/// is its own connection — QMP wants a capabilities handshake before it
/// takes a command, so a held socket saves nothing.
///
/// Refused, with the reason, when no emulator is running or the build has
/// no monitor: a Pause button that quietly did nothing would read as a
/// simulation that ignores you.
#[tauri::command]
pub async fn sim_pause(pause: bool, state: State<'_, AppState>) -> Result<(), CommandError> {
    let Some(port) = state.qmp().await else {
        return Err(CommandError::new(
            "nothing is running that can be paused — the emulator opens its monitor when a \
             simulation starts",
        ));
    };
    let verb = if pause { "stop" } else { "cont" };
    blocking("pausing the simulation", move || simulate::qmp(port, verb))
        .await?
        .map(|_| ())
        .map_err(CommandError::new)
}

/// A line into the running simulation — how a button press on the board view
/// reaches the firmware.
///
/// Both ways, when both exist. `B14=1` goes to the console for firmware that
/// reads rusty's text protocol, which is what the bundled examples do; with
/// the GPIO model there is also a real pin to drive, and unmodified firmware
/// reading `Input::is_high()` only sees that one. Sending only the second
/// would break every example; sending only the first is the limitation this
/// whole path exists to remove. The message says *pressed*; the level the
/// pin is driven to follows the button's wiring ([`pin_level`]).
#[tauri::command]
pub async fn sim_send(text: String, state: State<'_, AppState>) -> Result<(), CommandError> {
    if let Some(input) = state.session_input().await {
        input.send_line(&text);
    }
    if let Some(pins) = state.pins().await {
        pins.follow(&text);
    }
    Ok(())
}

/// Move one reading of a sensor on the sheet while the simulation runs: the
/// part, the channel (`ax`, `temp`), and the value in the channel's unit.
///
/// Refused when nothing is running that has the sensor, rather than taken
/// and dropped: a slider that silently did nothing would read as firmware
/// ignoring the part.
#[tauri::command]
pub async fn sim_sensor_set(
    part: String,
    key: String,
    value: f64,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let Some(pins) = state.pins().await else {
        return Err(CommandError::new(
            "nothing is running that can take a sensor reading — only rusty's emulator puts \
             sensors on the bus",
        ));
    };
    if pins.set_sensor(&part, &key, value) {
        Ok(())
    } else {
        Err(CommandError::new(format!(
            "{part} is not a sensor on this run's bus with a reading called {key}"
        )))
    }
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
            Archive::CodeLldb => install::codelldb_download(),
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
    // What the run declares down the pin channel before the firmware's first
    // instruction: button polarity, analog pins, and everything on the buses.
    let start = plan
        .board
        .as_ref()
        .map(|sheet| simulate::start_of(sheet, &simulate::kit_rows_for(&root, &sheet.chip)))
        .unwrap_or_default();

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
        // The monitor, so the run can be stopped and started again. Opened
        // for the emulator only: there is nothing to pause about a build.
        if is_emulator && let Some(port) = free_port() {
            let extra = simulate::qmp_args(port);
            step.display = format!("{} {}", step.display, extra.join(" "));
            step.args.extend(extra);
            state.set_qmp(Some(port)).await;
        }
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
            // The stock build, and what that costs, said where the run is
            // read. A blinky whose `toggle()` printed `false` for ever was
            // the report: `is_set_high()` reads a register this emulator
            // never stores, and nothing on screen said so.
            if !has_model {
                note(
                    &on_line,
                    "[rusty:pins] firmware — Espressif's stock QEMU: its GPIO write handler \
                     is empty, so is_set_high()/is_high() read 0 in the emulator (real \
                     hardware is fine) and the board shows only what the firmware prints. \
                     The Simulate panel's Upgrade installs rusty's build, which models the \
                     pins.",
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
            // The sheet's own circuit, walked in step with the firmware.
            // Absent when there is no board, and absent *with a reason in
            // the dock* when the sheet does not say enough to put numbers
            // on: a run that quietly stopped answering would read as a
            // converter that had gone dead.
            let live = match plan.board.as_ref() {
                Some(board) => {
                    let rows = simulate::kit_rows_for(&root, &board.chip);
                    match rusty_embed::live::Live::at_rest(
                        board.clone(),
                        rows,
                        Default::default(),
                        Default::default(),
                    ) {
                        Ok(live) => Some(live),
                        Err(unstated) => {
                            note(
                                &on_line,
                                format!(
                                    "[rusty:volts] the sheet is not solved: {unstated}. Analog \
                                     pins keep whatever the sheet declares."
                                ),
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            // Every line the emulator reports goes into the same stream the
            // serial console uses: `[rusty:gpio@…] 0=1` is parsed in exactly
            // one place, and a second reader is what once made telemetry
            // work in the simulator and vanish on hardware.
            let feed = on_line.clone();
            let mut open = true;
            let channel = simulate::connect(port, start.clone(), live, move |text| {
                // A failed send means the WebView is gone; the session's own
                // slot ends the run.
                open = open
                    && feed
                        .send(LogLine {
                            stream: LogStream::Stdout,
                            text,
                            level: None,
                        })
                        .is_ok();
            });
            state.set_pins(Some(channel)).await;
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
    // QEMU has exited; there is no longer anything to attach to, nothing to
    // pause, and no pin channel to keep trying to reach.
    if let Some(pins) = state.pins().await {
        pins.hang_up();
    }
    state.set_pins(None).await;
    state.set_attach(None).await;
    state.set_qmp(None).await;
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
}
