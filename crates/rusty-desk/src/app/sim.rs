//! Running the firmware in the simulator, and reading what it reports.
//!
//! A run is the simulator's plan (`rusty_embed::simulate::plan`): build,
//! image, boot — rusty's QEMU or rusty's own emulator, whichever the
//! catalogue names for the part. The boot step gets a pin channel when the
//! emulator models the pins, and the sheet's parts are declared down it
//! before the firmware's first instruction. Everything the firmware and the
//! emulator say comes back through the one door every run's lines come
//! through (`App::heard`), where the protocol lines are read into what the
//! board shows rather than printed.
//!
//! Run while a simulation runs restarts it, Wokwi's loop: change the code,
//! run again.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    rc::Rc,
};

use rusty_embed::{
    CommandPlan, Duty, Param, PinSource, SensorDef, SimPlan,
    activity::Kind,
    simulate::{self, PinChannel, Start},
};
use rusty_i18n::t;

use super::{App, NoteAction};
use crate::{
    run::{self, Ended},
    work,
};

/// What the simulator knows while it runs, and since its last run.
#[derive(Default)]
pub(super) struct Sim {
    /// The plan the last run was made from: the board and its library.
    pub(super) plan: Option<SimPlan>,
    /// The emulator's pins, while a run has a channel to them.
    pins: Option<PinChannel>,
    /// Each pin's level, as last reported; a pin is here or in `duties`,
    /// never both — the newer report decides.
    pub(super) levels: BTreeMap<u8, bool>,
    pub(super) duties: BTreeMap<u8, Duty>,
    /// Whose word the levels are: the emulator's registers, or the
    /// firmware's own `[rusty:gpio]` lines.
    pub(super) source: Option<PinSource>,
    /// What the firmware put on its display line.
    pub(super) display: Option<String>,
    pub(super) params: Vec<Param>,
    pub(super) sensors: Vec<SensorDef>,
    /// The bytes a strip's wire last carried, by GPIO.
    pub(super) rmt: HashMap<u8, Vec<u8>>,
    /// What the firmware's converter last took off each pin — kept apart
    /// from `analog`, what the host drives, because the two agreeing is
    /// the interesting fact.
    pub(super) adc: HashMap<u8, u16>,
    pub(super) analog: HashMap<u8, u16>,
    /// Where each knob was turned, by part.
    pub(super) turns: HashMap<String, u8>,
    /// A register sensor's readings as slid, by part and reading.
    pub(super) readings: HashMap<(String, String), f64>,
    /// A declared sensor's last sample, whole, by channel.
    pub(super) sensor_values: HashMap<String, Vec<f32>>,
    /// The firmware's named channels over time.
    pub(super) plot: rusty_embed::plot::Plot,
    /// Every pin's edges, for the Waves panel, and where it looks.
    pub(super) trace: rusty_embed::waves::Trace,
    pub(super) waves: super::waves::View,
    /// How the part names its pins, for the lanes.
    pub(super) ports: Option<rusty_embed::Ports>,
    /// The channels the Plot panel draws; none named is all of them.
    pub(super) shown: Vec<String>,
    /// Run again once this run has stopped.
    restart: bool,
}

impl Sim {
    /// A new run starts from nothing reported.
    fn reset(&mut self) {
        self.levels.clear();
        self.duties.clear();
        self.source = None;
        self.display = None;
        self.params.clear();
        self.sensors.clear();
        self.plot = Default::default();
        self.trace = Default::default();
        self.waves.follow = true;
        self.rmt.clear();
        self.adc.clear();
        self.analog.clear();
        self.turns.clear();
        self.readings.clear();
        self.sensor_values.clear();
    }

    /// Whether the emulator's pins are connected.
    pub(super) fn has_pins(&self) -> bool {
        self.pins.is_some()
    }
}

/// What a run needs worked out before anything starts: the plan, what the
/// pin channel declares, and where the firmware is built.
struct Prepared {
    plan: SimPlan,
    start: Start,
    firmware: PathBuf,
    ports: Option<rusty_embed::Ports>,
}

fn prepare(root: PathBuf) -> Result<Prepared, String> {
    let firmware = rusty_embed::project::firmware_root(&root);
    let detected = rusty_embed::project::detect(&firmware).map_err(|e| e.to_string())?;
    let plan = simulate::plan(&detected, false);
    let mut start = Start::default();
    if plan.supported && plan.missing.is_empty() {
        // The sheet's declarations: button polarity, analog pins, the
        // buses, the signals — a generator's table is the circuit stepped a
        // sample at a time, so this is worth doing off the window's thread.
        if let Some(sheet) = &plan.board {
            start = simulate::start_of(
                sheet,
                &simulate::kit_rows_for(&firmware, &sheet.chip),
                &plan.parts,
            );
        }
        if !plan.emulator.as_ref().is_some_and(|e| e.waves) {
            start.without_signals();
        }
        simulate::prepare(&firmware).map_err(|e| e.to_string())?;
    }
    let ports = detected.chip.as_deref().and_then(|chip| {
        rusty_embed::catalog::Catalog::load(Some(&root))
            .chip(chip)
            .and_then(|part| part.ports)
    });
    Ok(Prepared {
        plan,
        start,
        firmware,
        ports,
    })
}

impl App {
    /// Read a protocol line into what the board shows. `true` when it was
    /// one, and is not printed; text a marker followed is printed alone.
    pub(super) fn absorb(self: &Rc<Self>, text: &str) -> bool {
        let mut state = self.state.borrow_mut();
        let sim = &mut state.sim;
        if let Some((before, telemetry)) = rusty_embed::protocol::split_telemetry(text) {
            let first = sim.plot.channels.is_empty() && sim.params.is_empty();
            let host_us = self.started.elapsed().as_micros() as u64;
            sim.plot.record(telemetry, host_us);
            drop(state);
            // The Plot tab arrives with the first sample, and does not take
            // the dock from whoever is reading Output.
            if first {
                self.sync_docks();
            }
            self.plot_moved();
            self.flight_moved();
            // What came before the marker is the firmware's own words — a
            // test harness's `test name ... `.
            if !before.trim().is_empty() {
                self.runner.say(before, run::kind_of(before));
            }
            return true;
        }
        if let Some(param) = rusty_embed::parse_param(text) {
            let first = sim.plot.channels.is_empty() && sim.params.is_empty();
            match sim.params.iter_mut().find(|p| p.name == param.name) {
                Some(known) => *known = param,
                None => sim.params.push(param),
            }
            drop(state);
            if first {
                self.sync_docks();
            }
            self.plot_moved();
            return true;
        }
        if let Some(sensor) = rusty_embed::parse_sensor_def(text) {
            // A sensor declaration brings the Flight tab.
            let first = sim.sensors.is_empty();
            match sim.sensors.iter_mut().find(|s| s.name == sensor.name) {
                Some(known) => *known = sensor,
                None => sim.sensors.push(sensor),
            }
            drop(state);
            if first {
                self.sync_docks();
            }
            self.flight_moved();
            return true;
        }
        if let Some(source) = rusty_embed::parse_pin_source(text) {
            // Said in the dock as well: it is what the board's levels are.
            sim.source = Some(source);
            return false;
        }
        if let Some(report) = rusty_embed::parse_gpio_report(text) {
            for (pin, level) in &report.pins {
                sim.duties.remove(pin);
                sim.levels.insert(*pin, *level);
            }
            // The Waves tab arrives with the first edge.
            let first = sim.trace.events.is_empty();
            let host_us = self.started.elapsed().as_micros() as u64;
            sim.trace.record(report, host_us);
            drop(state);
            if first {
                self.sync_docks();
            }
            self.board_moved();
            self.waves_moved();
            return true;
        }
        if let Some(report) = rusty_embed::parse_pwm_report(text) {
            for (pin, duty) in report.pins {
                sim.levels.remove(&pin);
                sim.duties.insert(pin, duty);
            }
            drop(state);
            self.board_moved();
            self.flight_moved();
            return true;
        }
        if let Some(shown) = rusty_embed::parse_display_report(text) {
            sim.display = Some(shown);
            drop(state);
            self.board_moved();
            return true;
        }
        if let Some(report) = rusty_embed::parse_rmt_report(text) {
            // One transmission sets every pixel of a strip, so it is kept
            // whole.
            sim.rmt.insert(report.pin, report.bytes);
            drop(state);
            self.board_moved();
            return true;
        }
        if let Some(report) = rusty_embed::parse_adc_report(text) {
            let moved = sim.adc.insert(report.pin, report.counts) != Some(report.counts);
            drop(state);
            if moved {
                self.board_moved();
            }
            return true;
        }
        if let Some(report) = rusty_embed::parse_i2c_report(text) {
            drop(state);
            if self.screen_heard(&report) {
                self.board_moved();
            }
            return true;
        }
        if rusty_embed::parse_spi_report(text).is_some()
            || rusty_embed::wave::parse_wave_report(text).is_some()
        {
            // What crossed a bus or a converter: the board's instruments
            // read these once they arrive.
            return true;
        }
        if let Some((before, drawn)) = rusty_embed::draw::split_draw(text) {
            drop(state);
            // Text before the marker is kept for the log: a harness running
            // one test at a time writes `test name ... ` on the line the
            // test's first print lands on.
            if !before.trim().is_empty() {
                self.runner.say(before, run::kind_of(before));
            }
            self.drawn(drawn);
            return true;
        }
        false
    }

    /// A switch on the board pressed or let go while a run goes on: two
    /// GPIOs joined, or one driven — on the console for firmware that reads
    /// rusty's text protocol, and on the pin channel for firmware that
    /// reads its input register.
    pub(super) fn sim_press(&self, tie: Option<(u8, u8)>, gpio: Option<u8>, down: bool) {
        if self.activity_kind() != Some(Kind::Simulate) {
            return;
        }
        let state = self.state.borrow();
        let pins = state.sim.pins.as_ref();
        match (tie, gpio) {
            (Some((a, b)), _) => {
                if let Some(pins) = pins {
                    pins.tie(a, b, down);
                }
            }
            (None, Some(gpio)) => {
                let line = rusty_embed::protocol::button_line(u32::from(gpio), down);
                self.runner.send(&line);
                if let Some(pins) = pins {
                    pins.follow(&line);
                }
            }
            (None, None) => {}
        }
    }

    /// A line for the simulation, on its console and — for the two console
    /// messages that are also about a pin, a press and a converter's counts
    /// — on the pin as well. Nothing while no simulation runs.
    pub(super) fn sim_send(&self, line: &str) {
        if !self.sim_live() {
            return;
        }
        self.runner.send(line);
        if let Some(pins) = self.state.borrow().sim.pins.as_ref() {
            pins.follow(line);
        }
    }

    /// A line for the firmware's console alone: rusty's text protocol,
    /// which no pin carries.
    pub(super) fn sim_send_console(&self, line: &str) {
        if self.sim_live() {
            self.runner.send(line);
        }
    }

    /// One reading of a sensor rusty answers for, written into its
    /// registers on the running bus; what refused it, when something did —
    /// a slider that silently did nothing would read as firmware ignoring
    /// the part.
    pub(super) fn sim_sensor_set(&self, part: &str, key: &str, value: f64) -> Option<String> {
        let state = self.state.borrow();
        let Some(pins) = state.sim.pins.as_ref() else {
            return Some(t!("desk.sim-no-bus"));
        };
        (!pins.set_sensor(part, key, value))
            .then(|| t!("desk.sim-no-reading", part = part, key = key))
    }

    /// Whether the firmware or the emulator has reported a pin or a sample
    /// since the run began.
    pub fn sim_heard(&self) -> bool {
        let state = self.state.borrow();
        let sim = &state.sim;
        !sim.levels.is_empty() || !sim.duties.is_empty() || !sim.plot.channels.is_empty()
    }

    /// What the simulator has heard, in a line: the probe's view of it.
    pub fn sim_report(&self) -> String {
        let state = self.state.borrow();
        let sim = &state.sim;
        format!(
            "source {:?}, levels {:?}, duties {:?}, display {:?}, channel {}",
            sim.source,
            sim.levels,
            sim.duties
                .iter()
                .map(|(pin, d)| (pin, d.duty))
                .collect::<Vec<_>>(),
            sim.display,
            sim.pins.is_some()
        )
    }

    /// Run in the simulator — or, while a simulation runs, run it again.
    pub(super) fn simulate(self: &Rc<Self>) {
        if self.runner.is_running() {
            if self.activity_kind() == Some(Kind::Simulate) {
                self.state.borrow_mut().sim.restart = true;
                self.runner.stop();
            } else {
                self.note_transient(1, t!("desk.already-running"));
            }
            return;
        }
        let Some(root) = self.root() else { return };
        self.write_all_then(move |_| {
            work::spawn(
                move || prepare(root),
                |app, prepared| match prepared {
                    Ok(prepared) => app.simulate_with(prepared),
                    Err(reason) => app.note(2, reason),
                },
            );
        });
    }

    fn simulate_with(self: &Rc<Self>, prepared: Prepared) {
        let Prepared {
            plan,
            start,
            firmware,
            ports,
        } = prepared;
        if !plan.supported {
            let reason = plan.reason.clone().unwrap_or_default();
            self.note(2, t!("desk.sim-refused", reason = reason));
            return;
        }
        if !plan.missing.is_empty() {
            let names: Vec<String> = plan.missing.iter().map(|tool| tool.name.clone()).collect();
            self.note_with(
                2,
                t!("desk.sim-missing", tools = names.join(", ")),
                Some(NoteAction::Install(names)),
            );
            return;
        }
        let steps: VecDeque<CommandPlan> = plan.steps.clone().into();
        // The board as this run's plan has it, beside the code.
        self.load_board(Some(plan.clone()));
        self.show_panel("right", "board");
        {
            let mut state = self.state.borrow_mut();
            state.sim.reset();
            state.sim.plan = Some(plan);
            state.sim.ports = ports;
        }
        // Nothing of the last run's left drawn.
        self.sync_plot();
        self.sync_waves();
        self.board_moved();
        self.runner.clear();
        self.begin(Kind::Simulate, None);
        self.sim_steps(steps, start, firmware);
    }

    /// The plan's steps in turn — build and image must exit zero — and the
    /// emulator last, with its monitor and its pin channel.
    fn sim_steps(self: &Rc<Self>, mut steps: VecDeque<CommandPlan>, start: Start, dir: PathBuf) {
        let Some(step) = steps.pop_front() else {
            return;
        };
        if !simulate::is_emulator(&step.program) {
            let next = dir.clone();
            let started = self.run(&step, Some(dir), move |app, ended| {
                // The build may have made `target/`: the tree is read again
                // once its `CACHEDIR.TAG` says what it is.
                app.reread_project();
                if ended == Ended::Done {
                    app.sim_steps(steps, start, next);
                } else {
                    app.sim_ended(ended);
                }
            });
            if !started {
                self.abandon();
            }
            return;
        }
        let program = PathBuf::from(&step.program);
        work::spawn(
            move || simulate::has_gpio_model(&program),
            move |app, models_pins| app.boot(step, models_pins, &start, dir),
        );
    }

    /// The emulator, booted: its monitor so a run can be paused, its pin
    /// channel when it models the pins, and the sheet declared down it.
    fn boot(
        self: &Rc<Self>,
        mut step: CommandPlan,
        models_pins: bool,
        start: &Start,
        dir: PathBuf,
    ) {
        let mut said = Vec::new();
        if let Some(port) = simulate::free_port() {
            step.extend_args(simulate::qmp_args_for(&step.program, port));
        }
        let mut pins_port = None;
        if models_pins && let Some(port) = simulate::free_port() {
            step.extend_args(simulate::pins_args_for(&step.program, port));
            pins_port = Some(port);
            said.push("[rusty:pins] emulator — pin state read from the GPIO registers".to_string());
            said.extend(start.said.iter().cloned());
        }
        if !models_pins {
            said.push(t!("desk.sim-stock-qemu"));
        }
        let session = match simulate::launch(&step, Some(&dir)) {
            Ok(session) => session,
            Err(error) => {
                self.runner.say(&error.to_string(), run::kind::ERROR);
                self.sim_ended(Ended::Refused);
                return;
            }
        };
        let started = self.run_session(&step.display, session, |app, ended| app.sim_ended(ended));
        if !started {
            self.abandon();
            return;
        }
        for line in said {
            self.heard(&line);
        }
        if let Some(port) = pins_port {
            // The sheet's own circuit, walked in step with the firmware;
            // without it a converter keeps what the sheet declares.
            let live = self.state.borrow().sim.plan.as_ref().and_then(|plan| {
                let sheet = plan.board.clone()?;
                let rows = simulate::kit_rows_for(&dir, &sheet.chip);
                rusty_embed::live::Live::at_rest(sheet, rows, BTreeMap::new(), Default::default())
                    .ok()
            });
            let channel = simulate::connect(port, start.clone(), live, |text| {
                work::deliver(text, |app, text| app.heard(&text));
            });
            self.state.borrow_mut().sim.pins = Some(channel);
        }
    }

    /// The run is over: the channel hung up, the verdict, and the run that
    /// was waiting for this one to stop.
    fn sim_ended(self: &Rc<Self>, ended: Ended) {
        let (pins, restart) = {
            let mut state = self.state.borrow_mut();
            (
                state.sim.pins.take(),
                std::mem::take(&mut state.sim.restart),
            )
        };
        if let Some(pins) = pins {
            pins.hang_up();
        }
        match ended {
            Ended::Stopped => self.abandon(),
            Ended::Done => self.conclude(Some(0)),
            Ended::Failed(code) => self.conclude(code.or(Some(1))),
            Ended::Refused => self.conclude(Some(1)),
        }
        // The sheet is a drawing again, and its controls reach nothing; a
        // plant left spinning would hand the next run a rate nobody
        // commanded.
        self.sync_board_live();
        if self.state.borrow().flight.closed {
            self.flight_close(false);
        }
        if restart {
            self.simulate();
        }
    }
}
