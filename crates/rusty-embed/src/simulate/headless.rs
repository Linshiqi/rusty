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

use super::{kit_rows_for, pins_args};
use crate::model::SimLimit;
use crate::process;
use crate::protocol;

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

/// Build, image and boot the project at `root` (its firmware directory),
/// and run `scenario` against it.
pub fn run(root: &Path, scenario: &Scenario, on: &mut dyn FnMut(Event<'_>)) -> Outcome {
    if let Err(reason) = scenario.check() {
        return Outcome::unrunnable(reason);
    }
    let project = match crate::project::detect(root) {
        Ok(project) => project,
        Err(error) => {
            return Outcome::unrunnable(format!(
                "{} is not a project rusty can read: {error}",
                root.display()
            ));
        }
    };
    let chip = project.chip.clone().unwrap_or_default();
    let plan = super::plan(&project, false);
    if !plan.supported {
        return Outcome::unrunnable(
            plan.reason
                .unwrap_or_else(|| "this project cannot be simulated".to_string()),
        );
    }
    if !plan.missing.is_empty() {
        let tools: Vec<String> = plan
            .missing
            .iter()
            .map(|tool| format!("{} ({})", tool.name, tool.install))
            .collect();
        return Outcome::unrunnable(format!(
            "the simulator needs tools that are not installed: {}",
            tools.join("; ")
        ));
    }
    let mut outcome = Outcome::unrunnable("");
    outcome.limits = plan.limits.clone();
    outcome.notes = plan.notes.clone();
    for limit in &plan.limits {
        on(Event::Note(&limit.text));
    }
    if plan
        .emulator
        .as_ref()
        .is_some_and(|emulator| emulator.gpio_model && !emulator.peripherals)
    {
        let note = "this is an early build of rusty's QEMU: it models the pins but not the \
                    ADC, the I2C bus or SPI, so read_oneshot() and bus transactions wait for \
                    ever. The Simulate panel's Upgrade installs the current build.";
        on(Event::Note(note));
        outcome.notes.push(note.to_string());
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
    for step in &steps {
        on(Event::Command(&step.display));
        let session = match process::spawn(step, Some(root)) {
            Ok(session) => session,
            Err(error) => {
                outcome.verdict = Verdict::Unrunnable(error.to_string());
                return outcome;
            }
        };
        while let Some(line) = session.recv() {
            on(Event::Output(&line.text));
        }
        let code = session.wait();
        if code != Some(0) {
            let code = code.map_or_else(|| "a signal".to_string(), |c| c.to_string());
            outcome.verdict =
                Verdict::Unrunnable(format!("`{}` failed (exit {code})", step.display));
            return outcome;
        }
    }

    let pins_port = super::has_gpio_model(Path::new(&boot.program))
        .then(free_port)
        .flatten();
    if let Some(port) = pins_port {
        boot.args.extend(pins_args(port));
        boot.display = format!("{} {}", boot.display, pins_args(port).join(" "));
    }
    outcome.pins_from_emulator = pins_port.is_some();
    if pins_port.is_none() {
        let note = "the emulator keeps no pin state (Espressif's stock QEMU), so levels are only \
                    what the firmware prints about them and presses reach it only as B<pin>= \
                    console lines";
        on(Event::Note(note));
        outcome.notes.push(note.to_string());
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

    let sheet = plan.board.clone();
    let rows = sheet
        .as_ref()
        .map(|sheet| kit_rows_for(root, &sheet.chip))
        .unwrap_or_default();
    // The parts the sheet's `model` props name, read from the same three
    // layers the symbols are.
    let parts = crate::partfile::load(Some(root));
    for warning in &parts.warnings {
        on(Event::Note(warning));
        outcome.notes.push(warning.clone());
    }
    let specs = parts.specs;
    let pins = pins_port.map(|port| {
        let start = sheet
            .as_ref()
            .map(|sheet| super::start_of(sheet, &rows, &specs))
            .unwrap_or_default();
        let live = sheet.as_ref().and_then(|sheet| {
            match crate::live::Live::at_rest(sheet.clone(), rows.clone(), Default::default(), Default::default()) {
                Ok(live) => Some(live),
                Err(unstated) => {
                    let note = format!("the sheet is not solved: {unstated}. Analog pins keep whatever the sheet declares.");
                    on(Event::Note(&note));
                    outcome.notes.push(note);
                    None
                }
            }
        });
        let tx = tx.clone();
        super::connect(port, start, live, move |line| {
            let _ = tx.send(Heard::Pin(line));
        })
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

    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(scenario.timeout.unwrap_or(DEFAULT_TIMEOUT));
    let actions: Vec<Action> = scenario
        .steps
        .iter()
        .filter_map(|step| step.action().ok())
        .collect();
    let mut next = 0usize;
    let mut resume_at: Option<Instant> = None;
    let mut seen = vec![false; scenario.expect.len()];
    let mut levels: BTreeMap<u8, bool> = BTreeMap::new();
    let micros = |at: Option<u64>| at.unwrap_or_else(|| started.elapsed().as_micros() as u64);

    let verdict = 'run: loop {
        // Every step that can be taken now, in order.
        while next < actions.len() {
            match &actions[next] {
                Action::WaitSerial(_) => break,
                Action::Delay(seconds) => match resume_at {
                    None => {
                        resume_at = Some(Instant::now() + Duration::from_secs_f64(*seconds));
                        break;
                    }
                    Some(at) if Instant::now() < at => break,
                    Some(_) => resume_at = None,
                },
                Action::WriteSerial(text) => {
                    input.send_line(text);
                    if let Some(pins) = &pins {
                        pins.follow(text);
                    }
                }
                Action::Press(target, down) => {
                    let gpio = match target {
                        Target::Gpio(gpio) => *gpio,
                        Target::Part(part) => {
                            let Some(sheet) = &sheet else {
                                break 'run Verdict::Failed(format!(
                                    "there is no board to find {part} on"
                                ));
                            };
                            // A key between two GPIOs joins them rather than
                            // driving either, so it goes as a switch and the
                            // console hears nothing: the text protocol has no
                            // way to say "these two pads are connected".
                            if let Some((a, b)) = crate::nets::switch_tie(sheet, &rows, part) {
                                match &pins {
                                    Some(pins) => {
                                        pins.tie(a, b, *down);
                                        continue;
                                    }
                                    None => {
                                        break 'run Verdict::Failed(format!(
                                            "{part} joins GPIO{a} and GPIO{b}, which only \
                                             rusty's emulator can do"
                                        ));
                                    }
                                }
                            }
                            match crate::nets::button_drives(sheet, &rows, part) {
                                Some((gpio, _)) => gpio,
                                None => {
                                    break 'run Verdict::Failed(format!(
                                        "{part} is not a switch that reaches a GPIO on the board"
                                    ));
                                }
                            }
                        }
                    };
                    let text = format!("B{gpio}={}", u8::from(*down));
                    input.send_line(&text);
                    if let Some(pins) = &pins {
                        pins.follow(&text);
                    }
                }
                Action::ExpectPin(gpio, level) => {
                    let actual = match levels.get(gpio) {
                        Some(level) => *level,
                        // With the emulator's registers, a pin that never
                        // moved is at its reset level. Without them, nothing
                        // is known about a pin the firmware never mentioned.
                        None if outcome.pins_from_emulator => false,
                        None => {
                            break 'run Verdict::Failed(format!(
                                "nothing has said what level GPIO{gpio} is at"
                            ));
                        }
                    };
                    if actual != *level {
                        break 'run Verdict::Failed(format!(
                            "GPIO{gpio} is {}, expected {}",
                            u8::from(actual),
                            u8::from(*level)
                        ));
                    }
                }
                Action::Set(part, values) => {
                    let Some(pins) = &pins else {
                        break 'run Verdict::Failed(
                            "sensor readings need rusty's emulator, which puts sensors on the bus"
                                .to_string(),
                        );
                    };
                    for (key, value) in values {
                        if !pins.set_sensor(part, key, *value) {
                            break 'run Verdict::Failed(format!(
                                "{part} is not a sensor on the bus with a reading called {key}"
                            ));
                        }
                    }
                }
            }
            next += 1;
        }
        if next == actions.len()
            && seen.iter().all(|s| *s)
            && !(actions.is_empty() && seen.is_empty())
        {
            break Verdict::Passed;
        }

        let now = Instant::now();
        if now >= deadline {
            break if actions.is_empty() && seen.is_empty() {
                Verdict::Passed
            } else {
                Verdict::TimedOut(waiting_for(&actions, next, scenario, &seen))
            };
        }
        let wake = resume_at.map_or(deadline, |at| at.min(deadline));
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
                on(Event::Serial(&text));
                // What the firmware says about its pins counts only when
                // there is nothing better: with the emulator's registers on
                // the channel, the narration is the same edge twice, a few
                // microseconds apart, on a clock that is the firmware's.
                if !outcome.pins_from_emulator
                    && let Some(report) = protocol::parse_gpio_report(&text)
                {
                    for (pin, level) in &report.pins {
                        levels.insert(*pin, *level);
                        outcome.events.push((micros(report.at_us), *pin, *level));
                    }
                }
                if let Some(limit) = SimLimit::explaining(&chip, &text) {
                    on(Event::Note(&limit.text));
                }
                outcome.serial.push(text.clone());
                if let Some(bad) = scenario.fail.iter().find(|bad| text.contains(bad.as_str())) {
                    break Verdict::Failed(format!("the firmware printed {bad:?}"));
                }
                for (index, want) in scenario.expect.iter().enumerate() {
                    if text.contains(want.as_str()) {
                        seen[index] = true;
                    }
                }
                if let Some(Action::WaitSerial(want)) = actions.get(next)
                    && text.contains(want.as_str())
                {
                    next += 1;
                }
            }
            Heard::Pin(text) => {
                if let Some(report) = protocol::parse_gpio_report(&text) {
                    for (pin, level) in &report.pins {
                        levels.insert(*pin, *level);
                        outcome.events.push((micros(report.at_us), *pin, *level));
                    }
                } else if ["[rusty:i2c", "[rusty:spi", "[rusty:pwm", "[rusty:rmt"]
                    .iter()
                    .any(|prefix| text.starts_with(prefix))
                {
                    outcome.bus.push(text);
                }
            }
            Heard::Exited(code) => {
                let how =
                    code.map_or_else(|| "was stopped".to_string(), |c| format!("exited ({c})"));
                break if actions.is_empty() && seen.is_empty() {
                    Verdict::Failed(format!("the emulator {how} before the time was up"))
                } else {
                    Verdict::Failed(format!(
                        "the emulator {how} while {}",
                        waiting_for(&actions, next, scenario, &seen)
                    ))
                };
            }
        }
    };

    stopper.stop();
    if let Some(pins) = &pins {
        pins.hang_up();
    }
    drop(pins);
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

/// A port nothing else is on, learned by binding and letting go. QEMU
/// listens there and the channel connects.
fn free_port() -> Option<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
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
}
