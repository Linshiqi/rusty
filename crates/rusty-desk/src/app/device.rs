//! Where Run goes, and getting the firmware onto a board.
//!
//! The target is one list — the simulator, a serial port, a probe, a board
//! waiting in its USB bootloader — chosen once from the title bar and kept.
//! With nothing chosen, the one board plugged in is the answer
//! (`rusty_embed::only_candidate`); with more than one, or none, the list
//! opens and the verb waits there to run on whatever is picked.
//!
//! **The list opens in the frame of the click** (`crate::targets` is its
//! pure half and says how). It shows what the last scan found, a fresh one
//! starts as it opens — each source on a thread of its own, only those the
//! part can use — and each fills its rows in as it answers, without moving
//! a row already there. The window asks again in the background when it
//! comes back to the front, so the rows it opens on are seldom stale.
//!
//! Flash builds first and writes the image that build made, then stays
//! attached to what the board prints, as PlatformIO's Upload and Arduino's
//! do. Monitor attaches without writing. A monitor holding the port is let
//! go first, and the flash starts from its end — the one moment the port is
//! certainly free.

use std::{
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use rusty_embed::{
    BuildSystem, CommandPlan, FlashAction, Transport, activity::Kind, catalog::Catalog,
};
use rusty_i18n::t;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, VecModel};

use super::{App, part::Target};
pub(super) use crate::targets::Chosen;
use crate::{
    PickRow, Shell,
    picker::{Mode, Pick},
    run::Ended,
    targets::{self, Answer, Found, Line, Row, Source},
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

/// How long a source may take before the list stops saying it is still
/// asking: a `probe-rs list` that hangs on a confused USB device is a row
/// that never comes, not a list that never settles. Its answer is still
/// taken whenever it arrives.
const PATIENCE: Duration = Duration::from_secs(8);

/// How long Flash or Monitor with nothing chosen waits for every source
/// before it asks instead: deciding on what half the sources said would be
/// a guess.
const DECIDE_WITHIN: Duration = Duration::from_secs(3);

/// How long a scan is fresh enough that the window coming to the front
/// does not ask again.
const FRESH_FOR: Duration = Duration::from_secs(10);

/// What is plugged in, as last scanned, and what was chosen.
#[derive(Default)]
pub(super) struct Devices {
    pub(super) chosen: Option<Chosen>,
    /// What each source last answered.
    found: Found,
    /// The sources being asked: which, since when, and the ticket their
    /// answer carries — a source asked again after its patience ran out
    /// has two answers on the way, and only the newer one is waited for.
    asking: Vec<(Source, Instant, u64)>,
    next_ticket: u64,
    /// When a scan last began.
    last_scan: Option<Instant>,
    /// The list's rows, kept in place for as long as it is open.
    rows: Vec<Row>,
    /// The verb waiting for a pick from the list.
    pub(super) pending: Option<Verb>,
    /// Flash or Monitor, waiting for the sources to answer before it
    /// decides which board — and its ticket, for the timer that gives up.
    deciding: Option<(Verb, u64)>,
    /// The verb waiting for the run it stopped to end.
    after_stop: Option<Verb>,
}

impl Devices {
    /// Whether `source` is still being asked, within its patience.
    fn asking(&self, source: Source) -> bool {
        self.asking
            .iter()
            .any(|(asked, since, _)| *asked == source && since.elapsed() < PATIENCE)
    }

    /// The sources still being asked, of `sources`.
    fn still_asking(&self, sources: &[Source]) -> Vec<Source> {
        sources
            .iter()
            .copied()
            .filter(|source| self.asking(*source))
            .collect()
    }
}

/// A device as a word: the port, the probe, the drive.
fn label(transport: &Transport, found: &Found) -> String {
    targets::board_words(transport, found).0
}

/// One source asked on its own thread. A source that panics answers that
/// it found nothing rather than never answering: a list waiting for ever on
/// one source is the thing this is shaped to prevent.
fn ask(source: Source, root: PathBuf) -> Answer {
    let asked = std::panic::catch_unwind(move || match source {
        Source::Ports => Answer::Ports(rusty_embed::device::list_serial_ports(&Catalog::load(
            Some(&root),
        ))),
        Source::ProbeRs => Answer::Probes(rusty_embed::device::list_probe_rs_probes()),
        Source::Wlink => Answer::Probes(rusty_embed::device::list_wlink_probes()),
        Source::Uf2 => Answer::Boot(rusty_embed::device::list_uf2_drives()),
        Source::Dfu => Answer::Boot(rusty_embed::device::list_dfu_devices()),
    });
    asked.unwrap_or_else(|_| match source {
        Source::Ports => Answer::Ports(Vec::new()),
        Source::ProbeRs | Source::Wlink => Answer::Probes(Vec::new()),
        Source::Uf2 | Source::Dfu => Answer::Boot(Vec::new()),
    })
}

/// What a source still being asked is called at the list's foot.
fn source_name(source: Source) -> String {
    match source {
        Source::Ports => t!("device.serial-port"),
        Source::ProbeRs => "probe-rs".to_string(),
        Source::Wlink => "wlink".to_string(),
        Source::Uf2 => t!("device.uf2-drive"),
        Source::Dfu => "dfu-util".to_string(),
    }
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

    /// The sources worth asking for the open project's part.
    fn device_sources(&self) -> Vec<Source> {
        targets::sources_for(self.part().as_ref())
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

    /// What somebody chose, for the title bar's button: the simulator, or a
    /// board's label and tooltip.
    pub(super) fn chosen_target(&self) -> Option<super::part::Picked> {
        let state = self.state.borrow();
        Some(match state.devices.chosen.as_ref()? {
            Chosen::Simulator => super::part::Picked::Simulator,
            Chosen::Board(transport) => {
                let (label, tip) = targets::board_words(transport, &state.devices.found);
                super::part::Picked::Board { label, tip }
            }
        })
    }

    /// Run, the title bar's one verb: the simulator, or the board.
    pub(super) fn run_verb(self: &Rc<Self>) {
        match self.run_target() {
            Some(Chosen::Simulator) => self.simulate(),
            _ => self.device_action(Verb::Flash),
        }
    }

    /// Ask `sources` what is plugged in, each on a thread of its own; a
    /// source already being asked is not asked twice. Each answer is put in
    /// the list on screen as it comes.
    fn scan(self: &Rc<Self>, sources: &[Source]) {
        let Some(root) = self.root() else { return };
        let asked: Vec<(Source, u64)> = {
            let mut state = self.state.borrow_mut();
            let devices = &mut state.devices;
            devices
                .asking
                .retain(|(_, since, _)| since.elapsed() < PATIENCE);
            let now = Instant::now();
            devices.last_scan = Some(now);
            let mut asked = Vec::new();
            for source in sources {
                if devices.asking(*source) {
                    continue;
                }
                devices.next_ticket += 1;
                let ticket = devices.next_ticket;
                devices.asking.push((*source, now, ticket));
                asked.push((*source, ticket));
            }
            asked
        };
        if asked.is_empty() {
            return;
        }
        for (source, ticket) in asked {
            let root = root.clone();
            work::spawn(
                move || ask(source, root),
                move |app, answer| app.answered(source, ticket, answer),
            );
        }
        // A source still silent when its patience runs out stops holding
        // the list's foot at "looking".
        let weak = Rc::downgrade(self);
        Timer::single_shot(PATIENCE + Duration::from_millis(50), move || {
            if let Some(app) = weak.upgrade() {
                app.targets_changed(None);
                app.decide();
            }
        });
    }

    /// One source answered: known from now on, in the list if it is open,
    /// and for a verb waiting to decide.
    fn answered(self: &Rc<Self>, source: Source, ticket: u64, answer: Answer) {
        {
            let mut state = self.state.borrow_mut();
            let devices = &mut state.devices;
            devices
                .asking
                .retain(|(asked, _, held)| !(*asked == source && *held == ticket));
            devices.found.set(source, answer);
        }
        self.targets_changed(Some(source));
        // The button's tooltip knows more of the board now.
        self.sync_part();
        self.decide();
    }

    /// Ask again in the background: when the window comes back to the
    /// front, and as a project opens — not again within a few seconds, and
    /// not while something runs, since a flash or a monitor has the port
    /// and a listing beside a probe session is nobody's question then.
    pub(super) fn scan_in_background(self: &Rc<Self>) {
        if self.root().is_none() || self.runner.is_running() {
            return;
        }
        let fresh = self
            .state
            .borrow()
            .devices
            .last_scan
            .is_some_and(|at| at.elapsed() < FRESH_FOR);
        if !fresh {
            let sources = self.device_sources();
            self.scan(&sources);
        }
    }

    /// The window coming to the front asks what is plugged in, so the list
    /// opens on what is on the desk now — somebody plugging a board in has
    /// usually just been looking at something else. winit's own event, the
    /// one way Slint lets a window hear its focus.
    pub(super) fn watch_focus(self: &Rc<Self>, window: &crate::AppWindow) {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
        let weak = Rc::downgrade(self);
        window.window().on_winit_window_event(move |_, event| {
            if let WindowEvent::Focused(true) = event {
                let weak = weak.clone();
                // After the event, never inside it: the state may be borrowed.
                Timer::single_shot(Duration::ZERO, move || {
                    if let Some(app) = weak.upgrade() {
                        app.scan_in_background();
                    }
                });
            }
            EventResult::Propagate
        });
    }

    /// The list of places to run, open at once on what is known, and asked
    /// of the machine afresh while it shows. `pending` is the verb that runs
    /// on the one picked.
    pub(super) fn choose_target(self: &Rc<Self>, pending: Option<Verb>) {
        if self.root().is_none() {
            return;
        }
        self.state.borrow_mut().devices.pending = pending;
        // Before the list, so its foot says the scan is going.
        let sources = self.device_sources();
        self.scan(&sources);
        self.show_targets();
    }

    /// The list opened afresh: the simulator where it is offered, and every
    /// device the last scan found.
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
        let sources = targets::sources_for(chip.as_ref());
        {
            let mut state = self.state.borrow_mut();
            let devices = &mut state.devices;
            devices.rows = targets::rows(simulates, &devices.found, chip.as_ref(), &sources);
        }
        let rows = self.target_rows();
        let heading = match self.state.borrow().devices.pending {
            Some(Verb::Flash) => t!("device.pick-to-flash"),
            Some(Verb::Monitor) => t!("device.pick-to-monitor"),
            None => t!("device.picker-hint"),
        };
        self.open_list(Mode::Targets, heading, rows);
    }

    /// The list's rows as the picker shows them: each row, then its foot.
    fn target_rows(&self) -> Vec<(String, String, Pick)> {
        let sources = self.device_sources();
        let state = self.state.borrow();
        let devices = &state.devices;
        let asking = devices.still_asking(&sources);
        targets::lines(&devices.rows, !asking.is_empty())
            .into_iter()
            .map(|line| match line {
                Line::Row(at) => {
                    let row = &devices.rows[at];
                    if row.gone {
                        (row.title.clone(), t!("device.gone"), Pick::Nothing)
                    } else {
                        (row.title.clone(), row.detail.clone(), Pick::Target(at))
                    }
                }
                Line::Scanning => (
                    t!("device.scanning"),
                    asking
                        .iter()
                        .map(|source| source_name(*source))
                        .collect::<Vec<_>>()
                        .join(" · "),
                    Pick::Nothing,
                ),
                Line::NoneFound => (
                    t!("device.none-found"),
                    t!("device.none-found-hint"),
                    Pick::Nothing,
                ),
                Line::Rescan => (
                    t!("device.rescan"),
                    String::new(),
                    Pick::Action("run.rescan"),
                ),
            })
            .collect()
    }

    /// What `source` answered, in the list on screen if it is open — and
    /// with no source, only its foot again.
    fn targets_changed(self: &Rc<Self>, source: Option<Source>) {
        let open = matches!(self.state.borrow().picker, Some((Mode::Targets, _)));
        if !open {
            return;
        }
        let chip = self.part();
        let sources = targets::sources_for(chip.as_ref());
        if let Some(source) = source.filter(|source| sources.contains(source)) {
            let mut state = self.state.borrow_mut();
            let devices = &mut state.devices;
            let fresh = targets::rows_of(source, &devices.found, chip.as_ref());
            targets::merge(&mut devices.rows, source, fresh);
        }
        let rows = self.target_rows();
        self.refresh_list(rows);
    }

    /// The list on screen given its rows again while it stays open: each
    /// row changed in place and new ones added at the end, so the element
    /// under the pointer — and a press already on it — is the one it was,
    /// and the keyboard's row stays where it was.
    fn refresh_list(self: &Rc<Self>, rows: Vec<(String, String, Pick)>) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let query = shell.get_picker_text();
        let titles: Vec<String> = rows
            .iter()
            .map(|(title, detail, _)| format!("{title} {detail}"))
            .collect();
        let found: Vec<(String, String, Pick)> = crate::picker::rank(&query, &titles)
            .into_iter()
            .map(|at| rows[at].clone())
            .collect();
        {
            let mut state = self.state.borrow_mut();
            state.listed = rows;
            state.picker = Some((
                Mode::Targets,
                found.iter().map(|(.., pick)| pick.clone()).collect(),
            ));
        }
        let shown: Vec<PickRow> = found
            .iter()
            .map(|(title, detail, _)| PickRow {
                title: title.into(),
                detail: detail.into(),
                chord: SharedString::new(),
                windowed: false,
                glyph: 0,
                tint: 0,
                group: SharedString::new(),
            })
            .collect();
        let model = shell.get_picker_rows();
        match model.as_any().downcast_ref::<VecModel<PickRow>>() {
            Some(on_screen) => {
                for (at, row) in shown.iter().enumerate() {
                    if at >= on_screen.row_count() {
                        on_screen.push(row.clone());
                    } else if on_screen.row_data(at).as_ref() != Some(row) {
                        on_screen.set_row_data(at, row.clone());
                    }
                }
                while on_screen.row_count() > shown.len() {
                    on_screen.remove(on_screen.row_count() - 1);
                }
            }
            None => shell.set_picker_rows(ModelRc::new(VecModel::from(shown.clone()))),
        }
        let last = i32::try_from(shown.len()).unwrap_or(i32::MAX) - 1;
        shell.set_picker_selected(shell.get_picker_selected().clamp(0, last.max(0)));
    }

    /// A row of the list picked: the target from now on, and the verb that
    /// was waiting for it, run there.
    pub(super) fn pick_target(self: &Rc<Self>, index: usize) {
        let (chosen, pending) = {
            let mut state = self.state.borrow_mut();
            let chosen = state
                .devices
                .rows
                .get(index)
                .filter(|row| !row.gone)
                .map(|row| row.chosen.clone());
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
    /// or after asking which. Every source the part can use is asked again
    /// first, all at once, and the board decided as soon as what is known
    /// is enough (`decide`).
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
        if self.root().is_none() {
            return;
        }
        let ticket = {
            let mut state = self.state.borrow_mut();
            let devices = &mut state.devices;
            devices.next_ticket += 1;
            devices.deciding = Some((verb, devices.next_ticket));
            devices.next_ticket
        };
        let sources = self.device_sources();
        self.scan(&sources);
        // A source that will not answer is not waited for: the list opens
        // with the verb waiting on it rather than a board being guessed.
        let weak = Rc::downgrade(self);
        Timer::single_shot(DECIDE_WITHIN, move || {
            let Some(app) = weak.upgrade() else { return };
            let gave_up = {
                let mut state = app.state.borrow_mut();
                let devices = &mut state.devices;
                match devices.deciding {
                    Some((verb, held)) if held == ticket => {
                        devices.deciding = None;
                        devices.pending = Some(verb);
                        true
                    }
                    _ => false,
                }
            };
            if gave_up {
                app.show_targets();
            }
        });
        self.decide();
    }

    /// The verb waiting to decide which board, decided once what is known
    /// is enough: the board chosen, as soon as its own source has said it is
    /// still plugged in; otherwise the one board plugged in, once every
    /// source has answered; otherwise the list, with the verb waiting.
    fn decide(self: &Rc<Self>) {
        let Some((verb, _)) = self.state.borrow().devices.deciding else {
            return;
        };
        let chip = self.part();
        let sources = targets::sources_for(chip.as_ref());
        let decided: Option<Option<Transport>> = {
            let state = self.state.borrow();
            let devices = &state.devices;
            let present = match &devices.chosen {
                Some(Chosen::Board(transport))
                    if !devices.asking(Source::of(transport)) && devices.found.has(transport) =>
                {
                    Some(transport.clone())
                }
                _ => None,
            };
            if present.is_some() {
                Some(present)
            } else if !devices.still_asking(&sources).is_empty() {
                None
            } else {
                // Nothing runs in a bootloader to watch.
                let boot = match verb {
                    Verb::Monitor => Vec::new(),
                    Verb::Flash => rusty_embed::usable_boot(chip.as_ref(), &devices.found.boot()),
                };
                Some(rusty_embed::only_candidate(
                    &devices.found.ports,
                    &devices.found.probes(),
                    &boot,
                    rusty_embed::serial_capable(chip.as_ref()),
                ))
            }
        };
        let Some(transport) = decided else { return };
        self.state.borrow_mut().devices.deciding = None;
        match transport {
            Some(transport) => self.perform(verb, transport),
            None => {
                self.state.borrow_mut().devices.pending = Some(verb);
                self.show_targets();
            }
        }
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
            label(&transport, &state.devices.found)
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
