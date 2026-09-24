//! A simulation run without a window: build the firmware, image it, boot it,
//! and watch the serial line and the pins until what the run was asked to
//! see has appeared, something it was told to fail on has, or time runs out.
//!
//! `rusty-cli sim` and the assistant's `simulate` tool are this and nothing
//! more. The window runs the same pieces — the plan, the start the sheet
//! declares, the pin channel — through its own dock, because it streams
//! every step there and a button can stop it.
//!
//! Time is the host's: a delay is wall-clock seconds, and so is the timeout,
//! counted from the emulator starting so a cold build does not eat it.
//! QEMU runs unthrottled rather than cycle-exact, so a scenario that needs a
//! precise interval should wait for what the firmware prints rather than
//! for a number of seconds.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::{PinChannel, free_port, kit_rows_for, pins_args};
use crate::live::Live;
use crate::model::{CommandPlan, Sheet, SimLimit, SimPlan};
use crate::nets::Row;
use crate::process;
use crate::protocol;
use crate::sensor::Spec;

/// How long a run may go when nobody said: long enough for a board to boot
/// and say something, short enough that a hung firmware ends a CI job.
pub const DEFAULT_TIMEOUT: f64 = 10.0;

/// What a run is asked to do — a scenario file, or the assistant's
/// arguments, which are the same shape in JSON.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Scenario {
    /// Seconds the firmware may run, from the emulator starting.
    #[serde(default)]
    pub timeout: Option<f64>,
    /// Text the firmware must print. The run passes once every one has
    /// appeared and every step is done.
    #[serde(default)]
    pub expect: Vec<String>,
    /// Text that fails the run the moment it appears: a panic, an error
    /// the firmware reports.
    #[serde(default)]
    pub fail: Vec<String>,
    /// Steps taken in order while the firmware runs: `[[step]]` in a file.
    #[serde(default, rename = "step", alias = "steps")]
    pub steps: Vec<Step>,
}

/// One step: a table with exactly one of these set.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Step {
    /// Wait until a line the firmware prints contains this.
    #[serde(default)]
    pub wait_serial: Option<String>,
    /// Send a line to the firmware's console. `B9=1` and `A3=2048` reach
    /// the pins as well, as they do from the panel.
    #[serde(default)]
    pub write_serial: Option<String>,
    /// Press a switch on the sheet (`"SW1"`), or drive a GPIO as a press
    /// would (`9`).
    #[serde(default)]
    pub press: Option<Target>,
    #[serde(default)]
    pub release: Option<Target>,
    /// Seconds to let the firmware run before the next step.
    #[serde(default)]
    pub delay: Option<f64>,
    /// The level a GPIO must be at, now.
    #[serde(default)]
    pub expect_pin: Option<PinLevel>,
    /// A sensor's readings: `{ part = "U2", ax = 0.5 }`.
    #[serde(default)]
    pub set: Option<SetReadings>,
}

/// A switch by its reference on the sheet, or a GPIO by number.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Gpio(u8),
    Part(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinLevel {
    pub gpio: u8,
    /// 1 or 0.
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SetReadings {
    pub part: String,
    #[serde(flatten)]
    pub values: BTreeMap<String, f64>,
}

/// A step, once it is known to say exactly one thing.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    WaitSerial(String),
    WriteSerial(String),
    Press(Target, bool),
    Delay(f64),
    ExpectPin(u8, bool),
    Set(String, BTreeMap<String, f64>),
}

impl Step {
    fn action(&self) -> Result<Action, String> {
        let mut found = Vec::new();
        if let Some(text) = &self.wait_serial {
            found.push(Action::WaitSerial(text.clone()));
        }
        if let Some(text) = &self.write_serial {
            found.push(Action::WriteSerial(text.clone()));
        }
        if let Some(target) = &self.press {
            found.push(Action::Press(target.clone(), true));
        }
        if let Some(target) = &self.release {
            found.push(Action::Press(target.clone(), false));
        }
        if let Some(seconds) = self.delay {
            if !seconds.is_finite() || seconds < 0.0 {
                return Err(format!(
                    "a delay of {seconds} seconds is not a length of time"
                ));
            }
            found.push(Action::Delay(seconds));
        }
        if let Some(pin) = &self.expect_pin {
            if pin.level > 1 {
                return Err(format!("GPIO{} cannot be at level {}", pin.gpio, pin.level));
            }
            found.push(Action::ExpectPin(pin.gpio, pin.level == 1));
        }
        if let Some(set) = &self.set {
            if set.values.is_empty() {
                return Err(format!("setting {} names no reading to set", set.part));
            }
            found.push(Action::Set(set.part.clone(), set.values.clone()));
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err("a step says nothing to do".to_string()),
            _ => Err("a step says more than one thing to do; give each its own step".to_string()),
        }
    }
}

impl Scenario {
    /// A scenario file, which is TOML.
    pub fn from_toml(text: &str) -> Result<Scenario, String> {
        let scenario: Scenario = toml::from_str(text).map_err(|e| e.to_string())?;
        scenario.check()?;
        Ok(scenario)
    }

    /// Every step says exactly one thing, and the timeout is a length of
    /// time — refused before anything is built rather than halfway through
    /// a run.
    pub fn check(&self) -> Result<(), String> {
        if let Some(seconds) = self.timeout
            && !(seconds.is_finite() && seconds > 0.0)
        {
            return Err(format!(
                "a timeout of {seconds} seconds is not a length of time"
            ));
        }
        for (index, step) in self.steps.iter().enumerate() {
            step.action()
                .map_err(|reason| format!("step {}: {reason}", index + 1))?;
        }
        Ok(())
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Everything expected appeared and every step is done — or, with
    /// nothing expected, the firmware ran for the whole timeout without
    /// printing anything it was told to fail on.
    Passed,
    /// Something it was told to fail on appeared, a step's check did not
    /// hold, or the firmware stopped before it was done.
    Failed(String),
    /// Time ran out with something still expected.
    TimedOut(String),
    /// It could not be run at all: no chip, a missing tool, a failed build.
    Unrunnable(String),
}

/// What a run saw.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub verdict: Verdict,
    /// Every line the firmware printed, in order.
    pub serial: Vec<String>,
    /// Every level reported for each GPIO, as `(microseconds, pin, level)`:
    /// the emulator's own stamp when there is one, the host's since boot
    /// when there is not.
    pub events: Vec<(u64, u8, bool)>,
    /// Every bus transaction the emulator reported, as it reported it —
    /// and what crossed the other peripherals it answers for: a duty with
    /// its frequency, a strip's bytes. One list, because what a headless
    /// run is read for is *what the board did*, and splitting it by
    /// peripheral would hide the order the four of them happened in.
    pub bus: Vec<String>,
    /// Whether the levels came from the GPIO registers (rusty's QEMU) or
    /// only from what the firmware printed about them.
    pub pins_from_emulator: bool,
    /// What the emulator is known not to do on this chip.
    pub limits: Vec<SimLimit>,
    /// Things the run wants read that are not verdicts.
    pub notes: Vec<String>,
}

impl Outcome {
    fn unrunnable(reason: impl Into<String>) -> Outcome {
        Outcome {
            verdict: Verdict::Unrunnable(reason.into()),
            serial: Vec::new(),
            events: Vec::new(),
            bus: Vec::new(),
            pins_from_emulator: false,
            limits: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// A run about to start, with the plan's limits and notes said, each
    /// once. The notes are what the plan found on the way — a symbol
    /// library or a part's declaration that would not read, a sheet drawn
    /// for another chip.
    fn opening(plan: &SimPlan, on: &mut dyn FnMut(Event<'_>)) -> Outcome {
        let mut outcome = Outcome::unrunnable("");
        outcome.limits = plan.limits.clone();
        for limit in &plan.limits {
            on(Event::Note(&limit.text));
        }
        for note in &plan.notes {
            outcome.note(note.clone(), on);
        }
        outcome
    }

    /// Said as the run goes, and kept with what it saw.
    fn note(&mut self, text: String, on: &mut dyn FnMut(Event<'_>)) {
        on(Event::Note(&text));
        self.notes.push(text);
    }

    /// The last level reported for each GPIO.
    pub fn levels(&self) -> BTreeMap<u8, bool> {
        let mut levels = BTreeMap::new();
        for (_, pin, level) in &self.events {
            levels.insert(*pin, *level);
        }
        levels
    }
}

/// What a run says as it goes, for a caller that shows it live.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    /// A command about to run: `cargo build --release`.
    Command(&'a str),
    /// A line a build or image step printed.
    Output(&'a str),
    /// A line the firmware printed.
    Serial(&'a str),
    /// Something rusty wants said about the run.
    Note(&'a str),
}

enum Heard {
    Serial(String),
    Pin(String),
    Exited(Option<i32>),
}

/// The reports that carry what crossed a bus or a peripheral rather than a
/// level: kept whole in [`Outcome::bus`].
const BUS_REPORTS: [&str; 4] = ["[rusty:i2c", "[rusty:spi", "[rusty:pwm", "[rusty:rmt"];

/// Build, image and boot the project at `root` (its firmware directory),
/// and run `scenario` against it.
pub fn run(root: &Path, scenario: &Scenario, on: &mut dyn FnMut(Event<'_>)) -> Outcome {
    let (chip, plan) = match ready(root, scenario) {
        Ok(ready) => ready,
        Err(reason) => return Outcome::unrunnable(reason),
    };
    let mut outcome = Outcome::opening(&plan, on);
    if plan
        .emulator
        .as_ref()
        .is_some_and(|emulator| emulator.gpio_model && !emulator.peripherals)
    {
        let note = "this is an early build of rusty's QEMU: it models the pins but not the \
                    ADC, the I2C bus or SPI, so read_oneshot() and bus transactions wait for \
                    ever. The Simulate panel's Upgrade installs the current build.";
        outcome.note(note.to_string(), on);
    }
    if let Err(error) = super::prepare(root) {
        outcome.verdict =
            Verdict::Unrunnable(format!("could not create target/rusty-sim: {error}"));
        return outcome;
    }

    let mut steps = plan.steps;
    let Some(mut boot) = steps.pop() else {
        outcome.verdict = Verdict::Unrunnable("the plan has no emulator step".to_string());
        return outcome;
    };
    if let Err(reason) = build(&steps, root, on) {
        outcome.verdict = Verdict::Unrunnable(reason);
        return outcome;
    }

    let pins_port = super::has_gpio_model(Path::new(&boot.program))
        .then(free_port)
        .flatten();
    if let Some(port) = pins_port {
        boot.extend_args(pins_args(port));
    }
    outcome.pins_from_emulator = pins_port.is_some();
    if pins_port.is_none() {
        let note = "the emulator keeps no pin state (Espressif's stock QEMU), so levels are only \
                    what the firmware prints about them and presses reach it only as B<pin>= \
                    console lines";
        outcome.note(note.to_string(), on);
    }
    on(Event::Command(&boot.display));
    let session = match process::spawn(&boot, Some(root)) {
        Ok(session) => session,
        Err(error) => {
            outcome.verdict = Verdict::Unrunnable(error.to_string());
            return outcome;
        }
    };
    let input = session.input();
    let stopper = session.stopper();
    let (tx, heard) = mpsc::channel::<Heard>();

    let sheet = plan.board;
    let rows = sheet
        .as_ref()
        .map(|sheet| kit_rows_for(root, &sheet.chip))
        .unwrap_or_default();
    // The parts the sheet's `model` props name, read from the same three
    // layers the symbols are. A declaration that would not read is already
    // among the plan's notes.
    let parts = crate::partfile::load(Some(root));
    let pins = pins_port.map(|port| {
        pin_channel(
            port,
            sheet.as_ref(),
            &rows,
            &parts.specs,
            tx.clone(),
            &mut outcome,
            on,
        )
    });
    std::thread::spawn({
        let tx = tx.clone();
        move || {
            while let Some(line) = session.recv() {
                if tx.send(Heard::Serial(line.text)).is_err() {
                    break;
                }
            }
            let _ = tx.send(Heard::Exited(session.wait()));
        }
    });
    drop(tx);

    let board = Board {
        input,
        pins,
        sheet,
        rows,
    };
    let mut play = Play::new(scenario);
    let verdict = loop {
        if let Err(verdict) = play.take_steps(&board, outcome.pins_from_emulator) {
            break verdict;
        }
        if play.finished() {
            break Verdict::Passed;
        }

        let now = Instant::now();
        if now >= play.deadline {
            break if play.nothing_asked() {
                Verdict::Passed
            } else {
                Verdict::TimedOut(play.waiting_for())
            };
        }
        let wake = play
            .resume_at
            .map_or(play.deadline, |at| at.min(play.deadline));
        let heard_now = match heard.recv_timeout(
            wake.saturating_duration_since(now)
                .min(Duration::from_millis(50)),
        ) {
            Ok(heard) => heard,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Verdict::Failed("the emulator stopped".to_string());
            }
        };
        match heard_now {
            Heard::Serial(text) => {
                if let Some(verdict) = play.serial(text, &chip, &mut outcome, on) {
                    break verdict;
                }
            }
            Heard::Pin(text) => play.pin(text, &mut outcome),
            Heard::Exited(code) => break play.exited(code),
        }
    };

    stopper.stop();
    if let Some(pins) = &board.pins {
        pins.hang_up();
    }
    drop(board);
    // What is still in flight, so the log ends where the firmware did.
    while let Ok(heard) = heard.recv_timeout(Duration::from_millis(100)) {
        if let Heard::Serial(text) = heard {
            on(Event::Serial(&text));
            outcome.serial.push(text);
        }
    }
    outcome.events.sort_by_key(|(at, _, _)| *at);
    outcome.verdict = verdict;
    outcome
}

/// The project's chip and its plan, or why it cannot be run at all: a
/// scenario that does not say one thing per step, a directory that is no
/// project, a part the emulator does not model, a tool not installed.
fn ready(root: &Path, scenario: &Scenario) -> Result<(String, SimPlan), String> {
    scenario.check()?;
    let project = crate::project::detect(root).map_err(|error| {
        format!(
            "{} is not a project rusty can read: {error}",
            root.display()
        )
    })?;
    let plan = super::plan(&project, false);
    if !plan.supported {
        return Err(plan
            .reason
            .unwrap_or_else(|| "this project cannot be simulated".to_string()));
    }
    if !plan.missing.is_empty() {
        let tools: Vec<String> = plan
            .missing
            .iter()
            .map(|tool| format!("{} ({})", tool.name, tool.install))
            .collect();
        return Err(format!(
            "the simulator needs tools that are not installed: {}",
            tools.join("; ")
        ));
    }
    Ok((project.chip.unwrap_or_default(), plan))
}

/// The steps before the boot — the build, the image — each said as it
/// starts and its output as it comes. Why the run cannot go on, when one
/// fails.
fn build(steps: &[CommandPlan], root: &Path, on: &mut dyn FnMut(Event<'_>)) -> Result<(), String> {
    for step in steps {
        on(Event::Command(&step.display));
        let session = process::spawn(step, Some(root)).map_err(|error| error.to_string())?;
        while let Some(line) = session.recv() {
            on(Event::Output(&line.text));
        }
        let code = session.wait();
        if code != Some(0) {
            let code = code.map_or_else(|| "a signal".to_string(), |c| c.to_string());
            return Err(format!("`{}` failed (exit {code})", step.display));
        }
    }
    Ok(())
}

/// The pin channel to the emulator listening on `port`, started as the
/// sheet declares — its switches' polarities, its sensors, and the
/// voltages it solves to, which a sheet that cannot be solved goes without.
fn pin_channel(
    port: u16,
    sheet: Option<&Sheet>,
    rows: &[Row],
    specs: &[Spec],
    tx: mpsc::Sender<Heard>,
    outcome: &mut Outcome,
    on: &mut dyn FnMut(Event<'_>),
) -> PinChannel {
    let start = sheet
        .map(|sheet| super::start_of(sheet, rows, specs))
        .unwrap_or_default();
    let live = sheet.and_then(|sheet| {
        match Live::at_rest(
            sheet.clone(),
            rows.to_vec(),
            Default::default(),
            Default::default(),
        ) {
            Ok(live) => Some(live),
            Err(unstated) => {
                let note = format!(
                    "the sheet is not solved: {unstated}. Analog pins keep whatever the sheet \
                     declares."
                );
                outcome.note(note, on);
                None
            }
        }
    });
    super::connect(port, start, live, move |line| {
        let _ = tx.send(Heard::Pin(line));
    })
}

/// What a scenario's steps act on: the console, the pin channel when the
/// emulator has one, and the sheet a switch is looked up on.
struct Board {
    input: process::Input,
    pins: Option<PinChannel>,
    sheet: Option<Sheet>,
    rows: Vec<Row>,
}

impl Board {
    /// A line to the firmware's console, and to the pin channel, which
    /// moves a pin for the lines that say one moved.
    fn send(&self, text: &str) {
        self.input.send_line(text);
        if let Some(pins) = &self.pins {
            pins.follow(text);
        }
    }

    /// The GPIO pressing or releasing `target` drives, or `None` when it
    /// is a key between two GPIOs, which joins them rather than driving
    /// either and so goes as a switch: the text protocol has no way to say
    /// "these two pads are connected", so the console hears nothing.
    fn press(&self, target: &Target, down: bool) -> Result<Option<u8>, Verdict> {
        let part = match target {
            Target::Gpio(gpio) => return Ok(Some(*gpio)),
            Target::Part(part) => part,
        };
        let Some(sheet) = &self.sheet else {
            return Err(Verdict::Failed(format!(
                "there is no board to find {part} on"
            )));
        };
        if let Some((a, b)) = crate::nets::switch_tie(sheet, &self.rows, part) {
            let Some(pins) = &self.pins else {
                return Err(Verdict::Failed(format!(
                    "{part} joins GPIO{a} and GPIO{b}, which only rusty's emulator can do"
                )));
            };
            pins.tie(a, b, down);
            return Ok(None);
        }
        match crate::nets::button_drives(sheet, &self.rows, part) {
            Some((gpio, _)) => Ok(Some(gpio)),
            // The sheet's own finding, in its words: which half is missing
            // is the fix.
            None => Err(Verdict::Failed(
                crate::nets::Warning::SwitchDrivesNothing {
                    part: part.to_string(),
                }
                .to_string(),
            )),
        }
    }

    /// A sensor's readings, set on the bus.
    fn set(&self, part: &str, values: &BTreeMap<String, f64>) -> Result<(), Verdict> {
        let Some(pins) = &self.pins else {
            return Err(Verdict::Failed(
                "sensor readings need rusty's emulator, which puts sensors on the bus".to_string(),
            ));
        };
        for (key, value) in values {
            if !pins.set_sensor(part, key, *value) {
                return Err(Verdict::Failed(format!(
                    "{part} is not a sensor on the bus with a reading called {key}"
                )));
            }
        }
        Ok(())
    }
}

/// A scenario being played against a running firmware: where it has got
/// to, and what it has seen on the way.
struct Play<'s> {
    scenario: &'s Scenario,
    actions: Vec<Action>,
    /// The step to take next.
    next: usize,
    /// When the delay being waited out ends.
    resume_at: Option<Instant>,
    /// Which of the scenario's `expect` have appeared.
    seen: Vec<bool>,
    /// The last level heard for each GPIO.
    levels: BTreeMap<u8, bool>,
    started: Instant,
    deadline: Instant,
}

impl<'s> Play<'s> {
    fn new(scenario: &'s Scenario) -> Self {
        let started = Instant::now();
        Play {
            scenario,
            actions: scenario
                .steps
                .iter()
                .filter_map(|step| step.action().ok())
                .collect(),
            next: 0,
            resume_at: None,
            seen: vec![false; scenario.expect.len()],
            levels: BTreeMap::new(),
            started,
            deadline: started
                + Duration::from_secs_f64(scenario.timeout.unwrap_or(DEFAULT_TIMEOUT)),
        }
    }

    /// Every step that can be taken now, in order, up to one that has to
    /// wait. A verdict when a step's check does not hold or it cannot be
    /// taken at all.
    fn take_steps(&mut self, board: &Board, pins_from_emulator: bool) -> Result<(), Verdict> {
        while self.next < self.actions.len() {
            match &self.actions[self.next] {
                Action::WaitSerial(_) => break,
                Action::Delay(seconds) => match self.resume_at {
                    None => {
                        self.resume_at = Some(Instant::now() + Duration::from_secs_f64(*seconds));
                        break;
                    }
                    Some(at) if Instant::now() < at => break,
                    Some(_) => self.resume_at = None,
                },
                Action::WriteSerial(text) => board.send(text),
                Action::Press(target, down) => {
                    // A key between two GPIOs has already gone down the pin
                    // channel as a switch, and the console has no words for
                    // it; the step is taken all the same.
                    if let Some(gpio) = board.press(target, *down)? {
                        board.send(&crate::protocol::button_line(u32::from(gpio), *down));
                    }
                }
                Action::ExpectPin(gpio, level) => {
                    self.expect_pin(*gpio, *level, pins_from_emulator)?;
                }
                Action::Set(part, values) => board.set(part, values)?,
            }
            self.next += 1;
        }
        Ok(())
    }

    fn expect_pin(&self, gpio: u8, level: bool, pins_from_emulator: bool) -> Result<(), Verdict> {
        let actual = match self.levels.get(&gpio) {
            Some(level) => *level,
            // With the emulator's registers, a pin that never moved is at
            // its reset level. Without them, nothing is known about a pin
            // the firmware never mentioned.
            None if pins_from_emulator => false,
            None => {
                return Err(Verdict::Failed(format!(
                    "nothing has said what level GPIO{gpio} is at"
                )));
            }
        };
        if actual != level {
            return Err(Verdict::Failed(format!(
                "GPIO{gpio} is {}, expected {}",
                u8::from(actual),
                u8::from(level)
            )));
        }
        Ok(())
    }

    /// Nothing to wait for and nothing to do: the run is the firmware
    /// running for the whole timeout.
    fn nothing_asked(&self) -> bool {
        self.actions.is_empty() && self.seen.is_empty()
    }

    /// Every step taken and everything expected seen.
    fn finished(&self) -> bool {
        self.next == self.actions.len() && self.seen.iter().all(|s| *s) && !self.nothing_asked()
    }

    fn waiting_for(&self) -> String {
        waiting_for(&self.actions, self.next, self.scenario, &self.seen)
    }

    /// A line the firmware printed: kept, read for pins when nothing
    /// better reports them, and checked against what the run fails on and
    /// waits for. A verdict when it ends the run.
    fn serial(
        &mut self,
        text: String,
        chip: &str,
        outcome: &mut Outcome,
        on: &mut dyn FnMut(Event<'_>),
    ) -> Option<Verdict> {
        on(Event::Serial(&text));
        // What the firmware says about its pins counts only when there is
        // nothing better: with the emulator's registers on the channel, the
        // narration is the same edge twice, a few microseconds apart, on a
        // clock that is the firmware's.
        if !outcome.pins_from_emulator
            && let Some(report) = protocol::parse_gpio_report(&text)
        {
            self.record(&report, outcome);
        }
        if let Some(limit) = SimLimit::explaining(chip, &text) {
            on(Event::Note(&limit.text));
        }
        outcome.serial.push(text.clone());
        if let Some(bad) = self
            .scenario
            .fail
            .iter()
            .find(|bad| text.contains(bad.as_str()))
        {
            return Some(Verdict::Failed(format!("the firmware printed {bad:?}")));
        }
        for (index, want) in self.scenario.expect.iter().enumerate() {
            if text.contains(want.as_str()) {
                self.seen[index] = true;
            }
        }
        if let Some(Action::WaitSerial(want)) = self.actions.get(self.next)
            && text.contains(want.as_str())
        {
            self.next += 1;
        }
        None
    }

    /// A line from the pin channel: levels, or what crossed a bus.
    fn pin(&mut self, text: String, outcome: &mut Outcome) {
        if let Some(report) = protocol::parse_gpio_report(&text) {
            self.record(&report, outcome);
        } else if BUS_REPORTS.iter().any(|prefix| text.starts_with(prefix)) {
            outcome.bus.push(text);
        }
    }

    /// Levels reported, remembered and put on the timeline — at the
    /// emulator's own stamp when there is one, the host's since boot when
    /// there is not.
    fn record(&mut self, report: &protocol::GpioReport, outcome: &mut Outcome) {
        for (pin, level) in &report.pins {
            self.levels.insert(*pin, *level);
            let at = report
                .at_us
                .unwrap_or_else(|| self.started.elapsed().as_micros() as u64);
            outcome.events.push((at, *pin, *level));
        }
    }

    /// The verdict on an emulator that ended by itself.
    fn exited(&self, code: Option<i32>) -> Verdict {
        let how = code.map_or_else(|| "was stopped".to_string(), |c| format!("exited ({c})"));
        if self.nothing_asked() {
            Verdict::Failed(format!("the emulator {how} before the time was up"))
        } else {
            Verdict::Failed(format!("the emulator {how} while {}", self.waiting_for()))
        }
    }
}

/// What the run was still waiting for, said the way a person would ask.
fn waiting_for(actions: &[Action], next: usize, scenario: &Scenario, seen: &[bool]) -> String {
    if let Some(Action::WaitSerial(want)) = actions.get(next) {
        return format!("step {} was waiting for {want:?}", next + 1);
    }
    let missing: Vec<&str> = scenario
        .expect
        .iter()
        .zip(seen)
        .filter(|(_, seen)| !**seen)
        .map(|(want, _)| want.as_str())
        .collect();
    if missing.is_empty() {
        format!("step {} of {} had not finished", next + 1, actions.len())
    } else {
        format!(
            "never saw {}",
            missing
                .iter()
                .map(|m| format!("{m:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file format: `[[step]]` tables with one action each, a target
    /// that is a part or a GPIO, and a set that names its readings.
    #[test]
    fn a_scenario_file_reads_into_steps() {
        let scenario = Scenario::from_toml(
            r#"
timeout = 4
expect = ["ready"]
fail = ["panicked"]

[[step]]
wait-serial = "boot"

[[step]]
press = "SW1"

[[step]]
delay = 0.25

[[step]]
release = 9

[[step]]
expect-pin = { gpio = 2, level = 1 }

[[step]]
set = { part = "U2", ax = 0.5, gz = -30 }

[[step]]
write-serial = "Skp=2.5"
"#,
        )
        .unwrap();
        assert_eq!(scenario.timeout, Some(4.0));
        let actions: Vec<Action> = scenario.steps.iter().map(|s| s.action().unwrap()).collect();
        assert_eq!(actions[0], Action::WaitSerial("boot".into()));
        assert_eq!(actions[1], Action::Press(Target::Part("SW1".into()), true));
        assert_eq!(actions[2], Action::Delay(0.25));
        assert_eq!(actions[3], Action::Press(Target::Gpio(9), false));
        assert_eq!(actions[4], Action::ExpectPin(2, true));
        let Action::Set(part, values) = &actions[5] else {
            panic!("{:?}", actions[5]);
        };
        assert_eq!(part, "U2");
        assert_eq!(values.get("ax"), Some(&0.5));
        assert_eq!(values.get("gz"), Some(&-30.0));
        assert_eq!(actions[6], Action::WriteSerial("Skp=2.5".into()));
    }

    /// A step that says two things, or nothing, is refused before anything
    /// is built — halfway through a run is the wrong time to find out.
    #[test]
    fn a_step_says_exactly_one_thing() {
        let two = Scenario::from_toml("[[step]]\npress = \"SW1\"\ndelay = 1\n");
        assert!(two.unwrap_err().contains("step 1"));
        let none = Scenario::from_toml("[[step]]\n");
        assert!(none.unwrap_err().contains("nothing to do"));
        let typo = Scenario::from_toml("[[step]]\nwait_serial = \"x\"\n");
        assert!(
            typo.is_err(),
            "kebab-case keys, and an unknown one is refused"
        );
        let level = Scenario::from_toml("[[step]]\nexpect-pin = { gpio = 2, level = 7 }\n");
        assert!(level.unwrap_err().contains("level 7"));
        assert!(Scenario::from_toml("timeout = -1\n").is_err());
    }

    /// The assistant sends the same shape as JSON, with `steps` for the list.
    #[test]
    fn the_same_scenario_reads_from_json() {
        let scenario: Scenario = serde_json::from_value(serde_json::json!({
            "expect": ["tilted"],
            "steps": [{ "set": { "part": "U2", "ax": 0.7 } }, { "delay": 0.5 }],
        }))
        .unwrap();
        scenario.check().unwrap();
        assert_eq!(scenario.steps.len(), 2);
    }

    /// Said the way a person would ask what a run was still waiting for.
    #[test]
    fn a_timeout_names_what_never_came() {
        let scenario = Scenario {
            expect: vec!["ready".into(), "done".into()],
            ..Scenario::default()
        };
        assert_eq!(
            waiting_for(&[], 0, &scenario, &[true, false]),
            "never saw \"done\""
        );
        let actions = [Action::WaitSerial("boot".into())];
        assert_eq!(
            waiting_for(&actions, 0, &scenario, &[false, false]),
            "step 1 was waiting for \"boot\""
        );
    }

    /// Everything the plan noted is said once and kept once. A part
    /// declaration that would not read used to be noted by the plan and
    /// again by the run, which read the declarations for itself, so the
    /// assistant's `simulate` was told every such warning twice — and
    /// `rusty-cli sim` heard none of the plan's other notes at all.
    #[test]
    fn a_plans_notes_are_said_and_kept_once() {
        let mut plan = SimPlan::refused("");
        plan.notes = vec![
            "parts/imu.toml: a reading has no register".to_string(),
            "the sheet is drawn for esp32; the project builds for esp32c3".to_string(),
        ];
        let mut said = Vec::new();
        let outcome = Outcome::opening(&plan, &mut |event| {
            if let Event::Note(text) = event {
                said.push(text.to_string());
            }
        });
        assert_eq!(outcome.notes, plan.notes);
        assert_eq!(said, plan.notes);
    }

    /// What a board's console was sent, kept where the test can read it.
    #[derive(Clone, Default)]
    struct Console(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Console {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A key between two GPIOs is a step like any other: it goes down the
    /// pin channel as a switch, the console hears nothing, and the play
    /// moves on. It used to take the same press again and again, so a
    /// scenario that pressed a keypad key never ended — not even at its
    /// timeout, which is checked between steps.
    #[test]
    fn a_key_between_two_gpios_is_taken_as_a_step() {
        use crate::model::{Instance, Pin, PinKind, PinRef, Symbol, Wire};

        let pin = |number: &str, x: f64| Pin {
            number: number.into(),
            name: number.into(),
            kind: PinKind::Passive,
            at: (x, 0.0),
            length: 2.54,
            angle: if x < 0.0 { 0 } else { 180 },
            hidden: false,
        };
        let mut sheet = Sheet::empty("esp32c3");
        sheet.symbols = vec![Symbol {
            library: "Device".into(),
            name: "SW_Push".into(),
            reference: "SW".into(),
            value: "SW_Push".into(),
            description: None,
            pins: vec![pin("1", -5.08), pin("2", 5.08)],
            graphics: Vec::new(),
        }];
        sheet.parts.push(Instance {
            reference: "SW1".into(),
            symbol: "Device:SW_Push".into(),
            value: String::new(),
            x: 0.0,
            y: 0.0,
            rot: 0,
            mirror: false,
            props: BTreeMap::new(),
        });
        for (from, to) in [("U1.GPIO9", "SW1.1"), ("SW1.2", "U1.GPIO4")] {
            sheet.wires.push(Wire {
                from: PinRef::parse(from).unwrap(),
                to: PinRef::parse(to).unwrap(),
                bends: Vec::new(),
            });
        }

        // Nobody listens on the port: what the channel would say is lost,
        // which is the case where a step must still be taken.
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let pins = super::super::connect(port, Default::default(), None, |_| {});
        let console = Console::default();
        let board = Board {
            input: process::Input::new(Some(Box::new(console.clone()))),
            pins: Some(pins.clone()),
            sheet: Some(sheet),
            rows: crate::nets::kit_rows("esp32c3", &[4, 9]),
        };
        let scenario: &'static Scenario = Box::leak(Box::new(
            Scenario::from_toml("[[step]]\npress = \"SW1\"\n\n[[step]]\nrelease = \"SW1\"\n")
                .unwrap(),
        ));

        // On a thread of its own, so a play that never moves on fails this
        // test rather than hanging it.
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let mut play = Play::new(scenario);
            let taken = play.take_steps(&board, true).map(|()| play.next);
            let _ = done.send(taken);
        });
        let taken = finished.recv_timeout(Duration::from_secs(10));
        pins.hang_up();
        assert_eq!(
            taken,
            Ok(Ok(2)),
            "both steps taken, the press and the release"
        );
        assert!(
            console.0.lock().unwrap().is_empty(),
            "a key is a switch, which the console has no line for"
        );
    }
}
