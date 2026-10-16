//! Where Run goes, and getting the firmware onto a board.
//!
//! The target is one list — the simulator, a serial port, a probe, a board
//! waiting in its USB bootloader — chosen once from the title bar and kept.
//! With nothing chosen, the one board plugged in is the answer
//! (`rusty_embed::only_candidate`); with more than one, or none, the list
//! opens and the verb waits there to run on whatever is picked.
//!
//! Flash builds first and writes the image that build made, then stays
//! attached to what the board prints, as PlatformIO's Upload and Arduino's
//! do. Monitor attaches without writing. A monitor holding the port is let
//! go first, and the flash starts from its end — the one moment the port is
//! certainly free.

use std::{path::PathBuf, rc::Rc};

use rusty_embed::{
    BootDevice, BootKind, BuildSystem, CommandPlan, FlashAction, Probe, SerialPort, Transport,
    activity::Kind, catalog::Catalog,
};
use rusty_i18n::t;

use super::{App, part::Target};
use crate::{
    picker::{Mode, Pick},
    run::Ended,
    work,
};

/// What a board is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verb {
    /// Build, write the image, and watch what the board prints.
    Flash,
    /// Watch what the board prints, writing nothing.
    Monitor,
}

/// Where Run goes when somebody chose.
#[derive(Clone, Debug)]
pub(super) enum Chosen {
    Simulator,
    Board(Transport),
}

/// What is plugged in, as last scanned, and what was chosen.
#[derive(Default)]
pub(super) struct Devices {
    pub(super) chosen: Option<Chosen>,
    ports: Vec<SerialPort>,
    probes: Vec<Probe>,
    boot: Vec<BootDevice>,
    /// The rows of the list on screen, by index.
    listed: Vec<Chosen>,
    /// The verb waiting for a pick from the list.
    pub(super) pending: Option<Verb>,
    /// The verb waiting for the run it stopped to end.
    after_stop: Option<Verb>,
}

/// A device as a word: the port, the probe, the drive.
pub(super) fn label(transport: &Transport, probes: &[Probe]) -> String {
    match transport {
        Transport::Serial { port } => port.clone(),
        Transport::Probe {
            identifier: Some(id),
        } => probes
            .iter()
            .find(|probe| &probe.identifier == id)
            .map_or_else(|| id.clone(), |probe| probe.description.clone()),
        Transport::Probe { identifier: None } => t!("device.probe"),
        Transport::Uf2 { drive } => drive.clone(),
        Transport::Dfu { device, .. } => device.clone(),
    }
}

/// What is plugged in, asked of the machine: ports, probes and bootloaders.
/// Probes are asked of `probe-rs` and `wlink`, which takes seconds.
fn scan(root: PathBuf) -> (Vec<SerialPort>, Vec<Probe>, Vec<BootDevice>) {
    let catalog = Catalog::load(Some(&root));
    (
        rusty_embed::device::list_serial_ports(&catalog),
        rusty_embed::device::list_probes(),
        rusty_embed::device::list_boot_devices(),
    )
}

/// The command that does `action` on `transport`, planned against the
/// project as it is on disk now, with the image the last build made; and
/// where it runs.
fn plan_for(
    root: PathBuf,
    transport: Transport,
    action: FlashAction,
) -> Result<(CommandPlan, PathBuf), String> {
    let firmware_root = rusty_embed::project::firmware_root(&root);
    let detected = rusty_embed::project::detect(&firmware_root).map_err(|e| e.to_string())?;
    // PlatformIO flashes a PlatformIO project, reading the upload protocol
    // and the probe from the environment.
    if detected.build.system == BuildSystem::PlatformIo {
        let env = detected
            .build
            .environment
            .ok_or_else(|| t!("desk.no-pio-env"))?;
        let plan = rusty_embed::buildsys::platformio_flash(&env, &transport, action, None);
        return Ok((plan, firmware_root));
    }
    let chip_id = detected.chip.ok_or_else(|| t!("desk.no-chip-to-flash"))?;
    let catalog = Catalog::load(Some(&root));
    let ports = match &transport {
        Transport::Serial { .. } => rusty_embed::device::list_serial_ports(&catalog),
        _ => Vec::new(),
    };
    let warning = rusty_embed::flash::port_warning(&chip_id, &transport, &ports, &catalog);
    let firmware =
        rusty_embed::firmware::newest_in_project(&root).map(|image| PathBuf::from(image.path));
    let mut plan = rusty_embed::flash::plan(&rusty_embed::flash::FlashRequest {
        chip_id,
        probe_chip: detected.probe_chip,
        transport,
        action,
        firmware,
        defmt: detected.uses_defmt,
        baud: None,
    })
    .map_err(|e| e.to_string())?;
    plan.warning = warning;
    Ok((plan, firmware_root))
}

/// A question with Yes and No, answered before a board is written.
fn confirm(question: &str) -> bool {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(t!("toolbar.flash"))
        .set_description(question)
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

impl App {
    fn part(&self) -> Option<rusty_embed::Chip> {
        let state = self.state.borrow();
        state.project.as_ref().and_then(|p| p.chip.clone())
    }

    /// Where Run goes: what was chosen, or what the part makes of it.
    pub(super) fn run_target(&self) -> Option<Chosen> {
        let state = self.state.borrow();
        if let Some(chosen) = &state.devices.chosen {
            return Some(chosen.clone());
        }
        let project = state.project.as_ref()?;
        match super::part::target_of(Some(&project.embedded), project.chip.as_ref()) {
            Target::Simulator => Some(Chosen::Simulator),
            Target::Board(_) => None,
            _ => None,
        }
    }

    /// The title bar's target: the word in its tooltip and its icon's kind,
    /// when somebody chose one.
    pub(super) fn chosen_target(&self) -> Option<(String, &'static str)> {
        let state = self.state.borrow();
        match state.devices.chosen.as_ref()? {
            Chosen::Simulator => Some((t!("desk.simulator"), "simulator")),
            Chosen::Board(transport) => Some((label(transport, &state.devices.probes), "board")),
        }
    }

    /// Run, the title bar's one verb: the simulator, or the board.
    pub(super) fn run_verb(self: &Rc<Self>) {
        match self.run_target() {
            Some(Chosen::Simulator) => self.simulate(),
            _ => self.device_action(Verb::Flash),
        }
    }

    /// The list of places to run, asked of the machine first. `pending` is
    /// the verb that runs on the one picked.
    pub(super) fn choose_target(self: &Rc<Self>, pending: Option<Verb>) {
        let Some(root) = self.root() else { return };
        self.state.borrow_mut().devices.pending = pending;
        work::spawn(
            move || scan(root),
            |app, (ports, probes, boot)| {
                {
                    let mut state = app.state.borrow_mut();
                    state.devices.ports = ports;
                    state.devices.probes = probes;
                    state.devices.boot = boot;
                }
                app.show_targets();
            },
        );
    }

    fn show_targets(self: &Rc<Self>) {
        // A board verb waiting for its board is not offered the simulator:
        // there is nothing to flash there.
        let waiting = self.state.borrow().devices.pending.is_some();
        let simulates = !waiting
            && matches!(
                {
                    let state = self.state.borrow();
                    state.project.as_ref().map(|project| {
                        super::part::target_of(Some(&project.embedded), project.chip.as_ref())
                    })
                },
                Some(Target::Simulator)
            );
        let chip = self.part();
        let mut rows: Vec<(String, String, Pick)> = Vec::new();
        let mut listed: Vec<Chosen> = Vec::new();
        {
            let state = self.state.borrow();
            let devices = &state.devices;
            let mut row = |title: String, detail: String, chosen: Chosen| {
                rows.push((title, detail, Pick::Target(listed.len())));
                listed.push(chosen);
            };
            if simulates {
                row(t!("desk.simulator"), String::new(), Chosen::Simulator);
            }
            for port in &devices.ports {
                let mut detail = vec![t!("device.serial-port")];
                if !port.boards.is_empty() {
                    detail.push(port.boards.join(", "));
                } else if let Some(bridge) = &port.bridge {
                    detail.push(bridge.clone());
                }
                if !port.likely_board {
                    detail.push(t!("device.not-a-board"));
                }
                row(
                    port.name.clone(),
                    detail.join(" · "),
                    Chosen::Board(Transport::Serial {
                        port: port.name.clone(),
                    }),
                );
            }
            for probe in &devices.probes {
                row(
                    probe.description.clone(),
                    format!("{} · {}", t!("device.debug-probe"), probe.identifier),
                    Chosen::Board(Transport::Probe {
                        identifier: Some(probe.identifier.clone()),
                    }),
                );
            }
            for device in rusty_embed::usable_boot(chip.as_ref(), &devices.boot) {
                let kind = match device.kind {
                    BootKind::Uf2 => t!("device.uf2-drive"),
                    BootKind::Dfu => t!("device.dfu-device"),
                };
                row(
                    device.label.clone(),
                    format!("{kind} · {}", device.id),
                    Chosen::Board(rusty_embed::boot_transport(&device)),
                );
            }
        }
        let boards = listed.len() - usize::from(simulates);
        if boards == 0 {
            rows.push((
                t!("device.none-found"),
                t!("device.none-found-hint"),
                Pick::Nothing,
            ));
        }
        rows.push((
            t!("device.rescan"),
            String::new(),
            Pick::Action("run.rescan"),
        ));
        self.state.borrow_mut().devices.listed = listed;
        let heading = match self.state.borrow().devices.pending {
            Some(Verb::Flash) => t!("device.pick-to-flash"),
            Some(Verb::Monitor) => t!("device.pick-to-monitor"),
            None => t!("device.picker-hint"),
        };
        self.open_list(Mode::Targets, heading, rows);
    }

    /// A row of the list picked: the target from now on, and the verb that
    /// was waiting for it, run there.
    pub(super) fn pick_target(self: &Rc<Self>, index: usize) {
        let (chosen, pending) = {
            let mut state = self.state.borrow_mut();
            let chosen = state.devices.listed.get(index).cloned();
            (chosen, state.devices.pending.take())
        };
        let Some(chosen) = chosen else { return };
        self.state.borrow_mut().devices.chosen = Some(chosen.clone());
        self.sync_part();
        match (chosen, pending) {
            (Chosen::Board(transport), Some(verb)) => self.perform(verb, transport),
            (Chosen::Simulator, Some(_)) => self.run_verb(),
            _ => {}
        }
    }

    /// Flash or Monitor, on the chosen board — or the only one plugged in,
    /// or after asking which.
    pub(super) fn device_action(self: &Rc<Self>, verb: Verb) {
        if self.runner.is_running() {
            // A monitor holds the port a flash needs: let it go first, and
            // flash from its end.
            let holds_port = matches!(self.activity_kind(), Some(Kind::Monitor | Kind::Link));
            if verb == Verb::Flash && holds_port {
                self.state.borrow_mut().devices.after_stop = Some(verb);
                self.runner.stop();
            } else {
                self.note_transient(1, t!("desk.already-running"));
            }
            return;
        }
        let Some(root) = self.root() else { return };
        work::spawn(
            move || scan(root),
            move |app, (ports, probes, boot)| {
                let chip = app.part();
                let transport = {
                    let mut state = app.state.borrow_mut();
                    state.devices.ports = ports;
                    state.devices.probes = probes;
                    state.devices.boot = boot;
                    let devices = &state.devices;
                    // The board chosen, while it is still plugged in.
                    let chosen = match &devices.chosen {
                        Some(Chosen::Board(transport)) => {
                            let present = match transport {
                                Transport::Serial { port } => {
                                    devices.ports.iter().any(|p| &p.name == port)
                                }
                                Transport::Probe {
                                    identifier: Some(id),
                                } => devices.probes.iter().any(|p| &p.identifier == id),
                                Transport::Probe { identifier: None } => true,
                                Transport::Uf2 { drive } => {
                                    devices.boot.iter().any(|b| &b.id == drive)
                                }
                                Transport::Dfu { device, .. } => {
                                    devices.boot.iter().any(|b| &b.id == device)
                                }
                            };
                            present.then(|| transport.clone())
                        }
                        _ => None,
                    };
                    chosen.or_else(|| {
                        // Nothing runs in a bootloader to watch.
                        let boot = match verb {
                            Verb::Monitor => Vec::new(),
                            Verb::Flash => rusty_embed::usable_boot(chip.as_ref(), &devices.boot),
                        };
                        rusty_embed::only_candidate(
                            &devices.ports,
                            &devices.probes,
                            &boot,
                            rusty_embed::serial_capable(chip.as_ref()),
                        )
                    })
                };
                match transport {
                    Some(transport) => app.perform(verb, transport),
                    None => {
                        app.state.borrow_mut().devices.pending = Some(verb);
                        app.show_targets();
                    }
                }
            },
        );
    }

    fn perform(self: &Rc<Self>, verb: Verb, transport: Transport) {
        match verb {
            Verb::Monitor => {
                // A part espflash cannot talk to is watched over rusty's
                // own serial link: an nRF's USB-CDC, an STM32 on a
                // USB-UART.
                if let Transport::Serial { port } = &transport
                    && !rusty_embed::serial_capable(self.part().as_ref())
                {
                    self.link(port.clone());
                    return;
                }
                self.write(transport, FlashAction::Monitor);
            }
            Verb::Flash => {
                self.build(Box::new(move |app, built| {
                    if built {
                        app.write(transport, FlashAction::FlashAndMonitor);
                    }
                }));
            }
        }
    }

    /// The plan for `action` on `transport`, asked about first when it
    /// carries a warning, then run.
    fn write(self: &Rc<Self>, transport: Transport, action: FlashAction) {
        let Some(root) = self.root() else { return };
        let target = {
            let state = self.state.borrow();
            label(&transport, &state.devices.probes)
        };
        work::spawn(
            move || plan_for(root, transport, action),
            move |app, planned| {
                let (plan, dir) = match planned {
                    Ok(planned) => planned,
                    Err(reason) => {
                        app.note(2, reason);
                        return;
                    }
                };
                if let Some(warning) = &plan.warning
                    && !confirm(warning)
                {
                    return;
                }
                let kind = match action {
                    FlashAction::Monitor => Kind::Monitor,
                    _ => Kind::Flash,
                };
                // A monitor is a run of its own; a flash's write is the
                // second step of the run its build began, and the build's
                // lines stay above it.
                if kind == Kind::Monitor && !app.begin_run(kind) {
                    return;
                }
                app.begin(kind, Some(target));
                let started = app.run(&plan, Some(dir), |app, ended| app.board_ended(ended));
                if !started {
                    app.abandon();
                }
            },
        );
    }

    /// Watch a port over rusty's own link, typed into from the dock.
    fn link(self: &Rc<Self>, port: String) {
        const BAUD: u32 = 115_200;
        match rusty_embed::serial::open(&port, BAUD) {
            Ok(link) => {
                // A port that would not open is a note, and the dock is
                // left as it was; one that did is a run of its own.
                if !self.begin_run(Kind::Link) {
                    return;
                }
                let (tx, rx) = std::sync::mpsc::channel();
                let (input, stopper) = (link.input(), link.stopper());
                std::thread::Builder::new()
                    .name("link".into())
                    .spawn(move || {
                        while let Some(line) = link.recv() {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                    })
                    .expect("the link's thread starts");
                let session =
                    rusty_embed::process::Session::from_parts(rx, input, stopper, || None);
                self.begin(Kind::Link, Some(port.clone()));
                let started =
                    self.run_session(&format!("{port} @ {BAUD}"), session, |app, ended| {
                        app.board_ended(ended)
                    });
                if !started {
                    self.abandon();
                }
            }
            Err(error) => self.note(2, error.to_string()),
        }
    }

    /// A flash, a monitor or a link is over: its verdict, and the verb
    /// that was waiting for it to let the port go.
    fn board_ended(self: &Rc<Self>, ended: Ended) {
        match ended {
            Ended::Stopped => self.abandon(),
            Ended::Done => self.conclude(Some(0)),
            Ended::Failed(code) => self.conclude(code.or(Some(1))),
            Ended::Refused => self.conclude(Some(1)),
        }
        let waiting = self.state.borrow_mut().devices.after_stop.take();
        if let Some(verb) = waiting {
            self.device_action(verb);
        }
    }
}
