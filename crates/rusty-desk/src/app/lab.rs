//! The Signals tab: what a run's signals played and what the firmware made
//! of them, on the firmware's own clock (`docs/signals.md`). On the left the
//! sources — each generator on the sheet and each sensor reading a signal
//! moves, given a new signal while the run goes on, and the channels the
//! firmware declared on its console, fed at the host's pace. On the right
//! four instruments: the time view, the spectrum, a response swept or read
//! off a recording, and a filter designed against the signal with its code.
//!
//! The arithmetic is `rusty_embed::lab`, shared with the old window; this is
//! what the panel holds, the sweep that steps a source across a band, and
//! the drawing of each instrument as paths in its chart's own units.

use std::{collections::BTreeMap, rc::Rc, time::Duration, time::Instant};

use rusty_embed::{
    dsp::{self, Design, Pass, Window},
    generator,
    lab::{
        self, Played, Source, Switched,
        chart::{self, INK, PLAYED},
        design::{Kind, Param, params, with},
        record::{self, Of, Record},
        response::design_curve,
        spectrum::{frequency_ticks, place, tones_of},
        sweep::{self, Missed, Plan, Point},
        time::{conversion_steps, played_points},
    },
    plot::TraceClock,
    signal::Signal,
    wave::{WaveEvent, WaveReport, WaveTarget},
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{
    App,
    lab_words::{dsp_refusal_text, preset_name, signal_error_text},
};
use crate::{
    LabConsole, LabDot, LabLane, LabParam, LabPreset, LabRecord, LabSource, LabTick, Shell,
};

/// The Time view's windows, in seconds.
const WINDOWS: [f64; 5] = [0.05, 0.2, 1.0, 5.0, 20.0];
/// The spectrum's spans, in seconds.
const SPANS: [f64; 3] = [1.0, 5.0, 20.0];
/// Samples a spectrum reads at most.
const MOST: usize = 1 << 16;
/// How far below the peak the spectrum is drawn.
const DEPTH_DB: f64 = 100.0;
/// A chart's own units: a thousand across, a hundred down.
const W: f64 = 1000.0;
const H: f64 = 100.0;
/// How often a sweep looks for its step's switch and its records.
const LOOK_EVERY: Duration = Duration::from_millis(100);
/// How long a sweep waits for a step before saying what it waited for.
const PATIENCE: Duration = Duration::from_secs(30);
/// The rate a console channel's signal is fed at: the host's pace.
const CONSOLE_RATE: u32 = 50;

/// What the tab holds.
pub(super) struct Lab {
    /// Every stamped conversion on each pin this run, capped.
    conversions: BTreeMap<u8, Vec<(u64, u16)>>,
    /// The last switch of each table this run.
    switched: BTreeMap<WaveTarget, Switched>,
    /// What each source plays where the tab changed it this run.
    playing: BTreeMap<Source, String>,
    /// The source being studied.
    source: Option<Source>,
    /// A source's text being written, and why it would not read.
    errors: BTreeMap<Source, String>,
    view: i32,
    window: usize,
    // The spectrum's choices.
    chosen: Option<Of>,
    span: usize,
    window_kind: Window,
    log: bool,
    // The sweep.
    plan: Plan,
    input: Option<Of>,
    output: Option<String>,
    swept: Vec<Point>,
    sweeping: Option<u64>,
    sweep_number: u64,
    step: Option<(usize, usize, f64)>,
    missed: Option<String>,
    recorded: Option<String>,
    // The design.
    design: Design,
    design_rate: Option<f64>,
    code_name: String,
    // The console's channels, by name, a text per axis.
    console: BTreeMap<String, Vec<String>>,
    console_gen: u64,
    /// A redraw is on its way.
    due: bool,
}

impl Default for Lab {
    fn default() -> Lab {
        Lab {
            conversions: BTreeMap::new(),
            switched: BTreeMap::new(),
            playing: BTreeMap::new(),
            source: None,
            errors: BTreeMap::new(),
            view: 0,
            window: 2,
            chosen: None,
            span: 1,
            window_kind: Window::Hann,
            log: true,
            plan: Plan::default(),
            input: None,
            output: None,
            swept: Vec::new(),
            sweeping: None,
            sweep_number: 0,
            step: None,
            missed: None,
            recorded: None,
            // A second-order low-pass at 10 Hz: what the first preset's
            // slow reading wants, and a curve with something to see.
            design: Design::Butterworth {
                pass: Pass::Low,
                order: 2,
                cutoff: 10.0,
            },
            design_rate: None,
            code_name: "LowPass".to_string(),
            console: BTreeMap::new(),
            console_gen: 0,
            due: false,
        }
    }
}

impl Lab {
    /// A run starts: the last run's account goes, what is looked at stays.
    pub(super) fn clear_capture(&mut self) {
        self.conversions.clear();
        self.switched.clear();
        self.playing.clear();
        self.console.clear();
        self.console_gen += 1;
        self.sweeping = None;
        self.step = None;
    }

    pub(super) fn has_signals(&self) -> bool {
        !self.switched.is_empty()
    }
}

/// A sweep in progress.
#[derive(Clone)]
struct Run {
    number: u64,
    source: Source,
    plan: Plan,
    input: Of,
    output: String,
    freqs: Vec<f64>,
    index: usize,
    restore: Option<String>,
}

fn colour(hex: &str) -> slint::Color {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn window_of(index: i32) -> Window {
    match index {
        1 => Window::Blackman,
        2 => Window::Rectangular,
        _ => Window::Hann,
    }
}

fn window_index(window: Window) -> i32 {
    match window {
        Window::Hann => 0,
        Window::Blackman => 1,
        Window::Rectangular => 2,
    }
}

fn kind_label(kind: Kind) -> String {
    match kind {
        Kind::MovingAverage => t!("lab.kind-moving-average"),
        Kind::Exponential => t!("lab.kind-exponential"),
        Kind::ButterLow => t!("lab.kind-butterworth-low"),
        Kind::ButterHigh => t!("lab.kind-butterworth-high"),
        Kind::LowBiquad => t!("lab.kind-biquad-low"),
        Kind::HighBiquad => t!("lab.kind-biquad-high"),
        Kind::BandBiquad => t!("lab.kind-biquad-band"),
        Kind::Notch => t!("lab.kind-notch"),
        Kind::FirLow => t!("lab.kind-fir-low"),
        Kind::FirHigh => t!("lab.kind-fir-high"),
        Kind::FirBand => t!("lab.kind-fir-band"),
        Kind::Median => t!("lab.kind-median"),
    }
}

fn param_label(param: Param) -> String {
    match param {
        Param::Taps => t!("lab.param-taps"),
        Param::Alpha => t!("lab.param-alpha"),
        Param::Cutoff => t!("lab.param-cutoff"),
        Param::Q => t!("lab.param-q"),
        Param::Order => t!("lab.param-order"),
        Param::Low => t!("lab.param-low"),
        Param::High => t!("lab.param-high"),
    }
}

/// What the lab calls a record.
fn describe(of: &Of) -> String {
    match of {
        Of::Played => t!("lab.record-played"),
        Of::Converter(pin) => t!("lab.lane-converter", pin = *pin),
        Of::Channel(name) => name.clone(),
    }
}

fn record_label(of: &Of, played: Option<&Played>) -> String {
    match of {
        Of::Played => played.map_or_else(String::new, |p| {
            t!("lab.lane-played", source = p.source.label())
        }),
        Of::Converter(pin) => t!("lab.lane-converter", pin = *pin),
        Of::Channel(name) => name.clone(),
    }
}

/// A line across a chart at `x` of a thousand, as path commands.
fn verticals(xs: impl Iterator<Item = f64>) -> String {
    xs.map(|x| format!("M {x:.1} 0 V {H}"))
        .collect::<Vec<_>>()
        .join(" ")
}

impl App {
    /// The sheet the run was planned from.
    fn lab_sheet(&self) -> Option<rusty_embed::Sheet> {
        self.state.borrow().sim.plan.as_ref()?.board.clone()
    }

    /// What the studied source plays, or why it plays nothing.
    fn lab_played(&self) -> Option<Result<Played, String>> {
        let sheet = self.lab_sheet()?;
        let state = self.state.borrow();
        let source = state.lab.source.as_ref()?;
        Some(
            lab::played(&sheet, source, &state.lab.playing).map_err(|why| match why {
                lab::Unplayable::Gone(part) => t!("lab.gone", part = part),
                lab::Unplayable::Signal(error) => signal_error_text(&error),
                lab::Unplayable::Rate(text) => text,
            }),
        )
    }

    /// A conversion the emulator stamped, onto its pin's record.
    pub(super) fn lab_conversion(self: &Rc<Self>, pin: u8, at_us: u64, counts: u16) {
        {
            let mut state = self.state.borrow_mut();
            let record = state.lab.conversions.entry(pin).or_default();
            lab::record_conversion(record, at_us, counts);
        }
        self.lab_moved();
    }

    /// A table started or stopped on the emulator's clock: what a played
    /// signal is lined up with the conversions it made by.
    pub(super) fn lab_switched(self: &Rc<Self>, report: WaveReport, text: &str) {
        let start_us = match report.event {
            WaveEvent::On { start_us } => Some(start_us),
            WaveEvent::Off => None,
            // The emulator saying a table did not arrive whole, which
            // somebody has to be able to read.
            WaveEvent::Refused(_) => {
                self.runner.say(text, crate::run::kind::ERROR);
                return;
            }
        };
        let first = {
            let mut state = self.state.borrow_mut();
            let first = state.lab.switched.is_empty();
            state.lab.switched.insert(
                report.target,
                Switched {
                    at_us: report.at_us,
                    start_us,
                },
            );
            first
        };
        if first {
            self.sync_docks();
        }
        self.lab_moved();
    }

    /// Something the instruments read moved: a redraw four times a second.
    pub(super) fn lab_moved(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.lab.due {
                return;
            }
            state.lab.due = true;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(250), move || {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().lab.due = false;
                app.sync_lab();
            }
        });
    }

    // ---- the panel's choices -------------------------------------------

    pub(super) fn lab_pick_source(self: &Rc<Self>, index: i32) {
        let sources = self.lab_sources();
        if let Some((source, _)) = usize::try_from(index).ok().and_then(|i| sources.get(i)) {
            self.state.borrow_mut().lab.source = Some(source.clone());
        }
        self.sync_lab();
    }

    /// A source's text accepted: played from now on, if it reads.
    pub(super) fn lab_source_text(self: &Rc<Self>, index: i32, text: &str) {
        let sources = self.lab_sources();
        let Some((source, _)) = usize::try_from(index).ok().and_then(|i| sources.get(i)) else {
            return;
        };
        let source = source.clone();
        match Signal::parse(text) {
            Err(why) => {
                self.state
                    .borrow_mut()
                    .lab
                    .errors
                    .insert(source, signal_error_text(&why));
                self.sync_lab();
            }
            Ok(_) => {
                self.state.borrow_mut().lab.errors.remove(&source);
                self.lab_signal_set(source, Some(text.to_string()));
            }
        }
    }

    pub(super) fn lab_preset(self: &Rc<Self>, index: i32, preset: i32) {
        let Some(preset) = usize::try_from(preset)
            .ok()
            .and_then(|at| rusty_embed::signal::presets().into_iter().nth(at))
        else {
            return;
        };
        self.lab_source_text(index, &preset.signal.to_string());
    }

    /// Change what a source plays while the run goes on — rendered through
    /// the same circuit a run's start renders it through, off the window's
    /// thread — and say what the run says about it.
    fn lab_signal_set(self: &Rc<Self>, source: Source, text: Option<String>) {
        if !self.sim_live() {
            return;
        }
        let Some(pins) = self.state.borrow().sim.pins_channel() else {
            self.note_transient(1, t!("desk.sim-no-signal"));
            return;
        };
        let (part, key) = (source.part.clone(), source.key.clone());
        let asked = text.clone();
        crate::work::spawn(
            move || pins.set_signal(&part, &key, asked.as_deref()),
            move |app, said| match said {
                Ok(lines) => {
                    {
                        let mut state = app.state.borrow_mut();
                        match text {
                            Some(text) => {
                                state.lab.playing.insert(source, text);
                            }
                            None => {
                                state.lab.playing.remove(&source);
                            }
                        }
                    }
                    for line in lines {
                        app.runner.say(&line, crate::run::kind::PLAIN);
                    }
                    app.sync_lab();
                }
                Err(reason) => app.note_transient(1, reason),
            },
        );
    }

    /// A console channel's axis given a signal: every axis with one is fed
    /// from now on, at the host's pace.
    pub(super) fn lab_console_text(self: &Rc<Self>, name: &str, axis: i32, text: &str) {
        let Ok(axis) = usize::try_from(axis) else {
            return;
        };
        if !text.trim().is_empty()
            && let Err(why) = Signal::parse(text)
        {
            self.note_transient(1, signal_error_text(&why));
            return;
        }
        let axes = {
            let state = self.state.borrow();
            state
                .sim
                .sensors
                .iter()
                .find(|s| s.name == name)
                .map_or(1, |s| usize::from(s.components).max(1))
        };
        {
            let mut state = self.state.borrow_mut();
            let texts = state
                .lab
                .console
                .entry(name.to_string())
                .or_insert_with(|| vec![String::new(); axes]);
            texts.resize(axes, String::new());
            if let Some(slot) = texts.get_mut(axis) {
                *slot = text.to_string();
            }
        }
        self.console_signals();
    }

    /// Render every console axis's signal and feed them, a sample each
    /// 1/50 s, until a newer feed or the run's end.
    fn console_signals(self: &Rc<Self>) {
        let (generation, rendered) = {
            let mut state = self.state.borrow_mut();
            state.lab.console_gen += 1;
            let rendered: Vec<(String, usize, Vec<f64>)> = state
                .lab
                .console
                .iter()
                .flat_map(|(name, axes)| {
                    axes.iter().enumerate().filter_map(move |(axis, text)| {
                        if text.trim().is_empty() {
                            return None;
                        }
                        let signal = Signal::parse(text).ok()?;
                        let looped = generator::loop_of([&signal], CONSOLE_RATE);
                        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
                        for byte in name.bytes() {
                            hash ^= u64::from(byte);
                            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                        }
                        let samples = signal.render(
                            f64::from(CONSOLE_RATE),
                            looped.samples,
                            hash ^ axis as u64,
                        );
                        Some((name.clone(), axis, samples))
                    })
                })
                .collect();
            (state.lab.console_gen, rendered)
        };
        if rendered.is_empty() {
            return;
        }
        self.console_feed(generation, Rc::new(rendered), Instant::now());
    }

    fn console_feed(
        self: &Rc<Self>,
        generation: u64,
        rendered: Rc<Vec<(String, usize, Vec<f64>)>>,
        started: Instant,
    ) {
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(
            Duration::from_millis(u64::from(1000 / CONSOLE_RATE)),
            move || {
                let Some(app) = weak.upgrade() else { return };
                if app.state.borrow().lab.console_gen != generation || !app.sim_live() {
                    return;
                }
                let sample = (started.elapsed().as_secs_f64() * f64::from(CONSOLE_RATE)) as usize;
                let mut lines = Vec::new();
                {
                    let mut state = app.state.borrow_mut();
                    let declared = state.sim.sensors.clone();
                    let mut names: Vec<&String> = rendered.iter().map(|(n, _, _)| n).collect();
                    names.dedup();
                    for name in names {
                        let Some(def) = declared.iter().find(|d| &d.name == name) else {
                            continue;
                        };
                        let values = state
                            .sim
                            .sensor_values
                            .entry(name.clone())
                            .or_insert_with(|| vec![0.0; usize::from(def.components)]);
                        for (_, axis, samples) in rendered.iter().filter(|(n, _, _)| n == name) {
                            let Some(slot) = values.get_mut(*axis) else {
                                continue;
                            };
                            let mut value = samples[sample % samples.len()] as f32;
                            if let Some(low) = def.min {
                                value = value.max(low);
                            }
                            if let Some(high) = def.max {
                                value = value.min(high);
                            }
                            *slot = value;
                        }
                        lines.push(rusty_embed::sensor_line(name, values));
                    }
                }
                for line in lines {
                    app.sim_send_console(&line);
                }
                app.console_feed(generation, rendered, started);
            },
        );
    }

    pub(super) fn lab_set(self: &Rc<Self>, what: &str, index: i32) {
        {
            let mut state = self.state.borrow_mut();
            let lab = &mut state.lab;
            let at = usize::try_from(index).unwrap_or(0);
            match what {
                "view" => lab.view = index.clamp(0, 3),
                "window" => lab.window = at.min(WINDOWS.len() - 1),
                "span" => lab.span = at.min(SPANS.len() - 1),
                "window-kind" => lab.window_kind = window_of(index),
                "log" => lab.log = !lab.log,
                "kind" => {
                    if let Some(kind) = Kind::ALL.get(at) {
                        lab.design = kind.design(&lab.design);
                    }
                }
                "fir-window" => {
                    if let Design::Fir { window, .. } = &mut lab.design {
                        *window = window_of(index);
                    }
                }
                _ => {}
            }
        }
        self.sync_lab();
    }

    /// A record picked: the spectrum's, the sweep's input or its output.
    pub(super) fn lab_pick_record(self: &Rc<Self>, what: &str, index: i32) {
        let played = self.lab_played().and_then(Result::ok);
        let mut offered = self.records_offered(played.is_some());
        // The output's list is the channels alone.
        if what == "output" {
            offered.retain(|of| matches!(of, Of::Channel(_)));
        }
        let Some(of) = usize::try_from(index)
            .ok()
            .and_then(|i| offered.get(i))
            .cloned()
        else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            match what {
                "spectrum" => state.lab.chosen = Some(of),
                "input" => state.lab.input = Some(of),
                "output" => {
                    if let Of::Channel(name) = of {
                        state.lab.output = Some(name);
                    }
                }
                _ => {}
            }
        }
        self.sync_lab();
    }

    /// A number typed into a field: the sweep's plan, the design's
    /// parameters, its rate, its code's name.
    pub(super) fn lab_field(self: &Rc<Self>, what: &str, index: i32, text: &str) {
        let value = text.trim().parse::<f64>().ok();
        {
            let mut state = self.state.borrow_mut();
            let lab = &mut state.lab;
            match (what, value) {
                ("sweep", Some(value)) => match index {
                    0 => lab.plan.from = value,
                    1 => lab.plan.to = value,
                    2 => lab.plan.points = value.clamp(1.0, 200.0) as usize,
                    3 => lab.plan.amplitude = value,
                    _ => lab.plan.offset = value,
                },
                ("param", Some(value)) => {
                    if let Some((param, _)) = usize::try_from(index)
                        .ok()
                        .and_then(|i| params(&lab.design).get(i).copied())
                    {
                        lab.design = with(&lab.design, param, value);
                    }
                }
                ("rate", _) => lab.design_rate = value.filter(|rate| *rate > 0.0),
                ("code-name", _) => lab.code_name = text.trim().to_string(),
                _ => {}
            }
        }
        self.sync_lab();
    }

    // ---- the records -----------------------------------------------------

    /// Every source on the sheet, with what the sheet says it plays.
    fn lab_sources(&self) -> Vec<(Source, String)> {
        self.lab_sheet()
            .map(|sheet| lab::sources_of(&sheet))
            .unwrap_or_default()
    }

    /// What can be read: what is played, each converter, each channel on
    /// the firmware's clock.
    fn records_offered(&self, played: bool) -> Vec<Of> {
        let state = self.state.borrow();
        let mut offered = Vec::new();
        if played {
            offered.push(Of::Played);
        }
        offered.extend(state.lab.conversions.keys().map(|pin| Of::Converter(*pin)));
        if state.sim.plot.clock == Some(TraceClock::Firmware) {
            offered.extend(
                state
                    .sim
                    .plot
                    .channels
                    .iter()
                    .map(|(name, _)| Of::Channel(name.clone())),
            );
        }
        offered
    }

    /// A channel the firmware prints, as a record.
    fn channel_record(&self, name: &str) -> Option<Record> {
        let state = self.state.borrow();
        if state.sim.plot.clock != Some(TraceClock::Firmware) {
            return None;
        }
        let (_, points) = state
            .sim
            .plot
            .channels
            .iter()
            .find(|(known, _)| known == name)?;
        record::from_channel(points)
    }

    fn converter_record(&self, pin: u8) -> Option<Record> {
        let state = self.state.borrow();
        state
            .lab
            .conversions
            .get(&pin)
            .and_then(|r| record::from_conversions(r))
    }

    /// The newest moment the instruments have heard of.
    fn newest(&self) -> Option<u64> {
        let state = self.state.borrow();
        let converted = state
            .lab
            .conversions
            .values()
            .filter_map(|record| record.last().map(|(at, _)| *at))
            .max();
        let printed = (state.sim.plot.clock == Some(TraceClock::Firmware))
            .then(|| {
                state
                    .sim
                    .plot
                    .channels
                    .iter()
                    .filter_map(|(_, points)| points.last().map(|(at, _)| *at))
                    .max()
            })
            .flatten();
        converted.max(printed)
    }

    /// `seconds` of a record, the newest.
    fn take(&self, of: &Of, played: Option<&Played>, seconds: f64) -> Option<Record> {
        let whole = match of {
            Of::Played => {
                let played = played?;
                let start = {
                    let state = self.state.borrow();
                    lab::start_of(played, &state.lab.switched).unwrap_or(0)
                };
                let end = self.newest().unwrap_or(start + (seconds * 1e6) as u64);
                let from = end.saturating_sub((seconds * 1e6) as u64).max(start);
                record::from_played(played, start, from, f64::from(played.rate), seconds)
            }
            Of::Converter(pin) => self.converter_record(*pin)?,
            Of::Channel(name) => self.channel_record(name)?,
        };
        let keep = ((seconds * whole.rate) as usize).min(MOST);
        let skip = whole.samples.len().saturating_sub(keep);
        Some(Record {
            rate: whole.rate,
            start_us: whole.start_us + (skip as f64 * 1e6 / whole.rate) as u64,
            samples: whole.samples[skip..].to_vec(),
        })
    }

    /// The rate a design is for when nobody stated one: the firmware's own
    /// records'.
    fn estimated_rate(&self) -> Option<f64> {
        let first = {
            let state = self.state.borrow();
            (state.sim.plot.clock == Some(TraceClock::Firmware))
                .then(|| {
                    state
                        .sim
                        .plot
                        .channels
                        .first()
                        .map(|(name, _)| name.clone())
                })
                .flatten()
        };
        first
            .and_then(|name| self.channel_record(&name).map(|r| r.rate))
            .or_else(|| {
                let state = self.state.borrow();
                state
                    .lab
                    .conversions
                    .values()
                    .find_map(|r| record::from_conversions(r).map(|r| r.rate))
            })
    }

    // ---- the sweep -------------------------------------------------------

    pub(super) fn lab_sweep_start(self: &Rc<Self>) {
        if !self.sim_live() {
            return;
        }
        let run = {
            let mut state = self.state.borrow_mut();
            let (Some(source), Some(output)) = (state.lab.source.clone(), state.lab.output.clone())
            else {
                return;
            };
            let plan = state.lab.plan.clone();
            let freqs = sweep::frequencies(&plan);
            if freqs.is_empty() {
                return;
            }
            let restore = state.lab.playing.get(&source).cloned().or_else(|| {
                let sheet = state.sim.plan.as_ref()?.board.as_ref()?;
                let part = sheet.parts.iter().find(|p| p.reference == source.part)?;
                part.props.get(&source.key).cloned()
            });
            let lab = &mut state.lab;
            lab.sweep_number += 1;
            lab.sweeping = Some(lab.sweep_number);
            lab.swept.clear();
            lab.missed = None;
            lab.recorded = None;
            Run {
                number: lab.sweep_number,
                input: lab.input.clone().unwrap_or(Of::Played),
                source,
                plan,
                output,
                freqs,
                index: 0,
                restore,
            }
        };
        self.sweep_play(run);
    }

    pub(super) fn lab_sweep_stop(self: &Rc<Self>) {
        self.state.borrow_mut().lab.sweeping = None;
        self.sync_lab();
    }

    fn sweep_current(&self, number: u64) -> bool {
        self.state.borrow().lab.sweeping == Some(number) && self.sim_live()
    }

    fn last_switch(&self) -> Option<u64> {
        self.state
            .borrow()
            .lab
            .switched
            .values()
            .map(|s| s.at_us)
            .max()
    }

    fn sweep_play(self: &Rc<Self>, run: Run) {
        if !self.sweep_current(run.number) {
            return self.sweep_finish(run, None);
        }
        let Some(&freq) = run.freqs.get(run.index) else {
            return self.sweep_finish(run, None);
        };
        self.state.borrow_mut().lab.step = Some((run.index + 1, run.freqs.len(), freq));
        self.sync_lab();
        let before = self.last_switch();
        self.lab_signal_set(
            run.source.clone(),
            Some(sweep::step_signal(&run.plan, freq)),
        );
        self.sweep_listen(run, freq, before, Instant::now());
    }

    fn sweep_listen(self: &Rc<Self>, run: Run, freq: f64, before: Option<u64>, asked: Instant) {
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(LOOK_EVERY, move || {
            let Some(app) = weak.upgrade() else { return };
            if !app.sweep_current(run.number) {
                return app.sweep_finish(run, None);
            }
            if asked.elapsed() > PATIENCE {
                let why = t!("lab.sweep-waited", channel = run.output.clone());
                return app.sweep_finish(run, Some(why));
            }
            // The step has started when the emulator says its table switched.
            let switched = app
                .last_switch()
                .filter(|at| before.is_none_or(|b| *at > b));
            let Some(switched) = switched else {
                return app.sweep_listen(run, freq, before, asked);
            };
            let settle = (sweep::settle_seconds(freq) * 1e6) as u64;
            let heard = sweep::listen_seconds(freq);
            let from = switched + settle;
            if app
                .newest()
                .is_none_or(|now| now < from + (heard * 1e6) as u64)
            {
                return app.sweep_listen(run, freq, before, asked);
            }
            match app.sweep_measure(&run, freq, from, heard) {
                Ok(point) => {
                    app.state.borrow_mut().lab.swept.push(point);
                    let mut next = run;
                    next.index += 1;
                    app.sweep_play(next);
                }
                Err(why) => app.sweep_finish(run, Some(why)),
            }
        });
    }

    fn sweep_measure(&self, run: &Run, freq: f64, from: u64, heard: f64) -> Result<Point, String> {
        let missing = |of: &Of| t!("lab.sweep-no-record", record = describe(of));
        let output_of = Of::Channel(run.output.clone());
        let output = self
            .channel_record(&run.output)
            .map(|r| r.between(from, heard))
            .ok_or_else(|| missing(&output_of))?;
        let input = match &run.input {
            Of::Channel(name) => self.channel_record(name).map(|r| r.between(from, heard)),
            Of::Converter(pin) => self.converter_record(*pin).map(|r| r.between(from, heard)),
            Of::Played => self.lab_played().and_then(Result::ok).and_then(|played| {
                let start = {
                    let state = self.state.borrow();
                    lab::start_of(&played, &state.lab.switched)
                }?;
                Some(record::from_played(
                    &played,
                    start,
                    from,
                    output.rate,
                    heard,
                ))
            }),
        }
        .ok_or_else(|| missing(&run.input))?;
        sweep::measure(&input, &output, freq, from).map_err(|missed| match missed {
            Missed::Tone(refusal) => dsp_refusal_text(&refusal),
            Missed::Silent => t!("lab.sweep-silent", record = describe(&run.input)),
        })
    }

    fn sweep_finish(self: &Rc<Self>, run: Run, missed: Option<String>) {
        let superseded = self
            .state
            .borrow()
            .lab
            .sweeping
            .is_some_and(|number| number != run.number);
        if superseded {
            return;
        }
        // The source plays what it played before.
        if self.sim_live() {
            self.lab_signal_set(run.source.clone(), run.restore.clone());
        }
        {
            let mut state = self.state.borrow_mut();
            state.lab.sweeping = None;
            state.lab.step = None;
            if missed.is_some() {
                state.lab.missed = missed;
            }
        }
        self.sync_lab();
    }

    /// The sweep's measurement without the stepping: the input's tones
    /// found in its spectrum, each fitted on both records.
    pub(super) fn lab_measure_recorded(self: &Rc<Self>) {
        let (input, output) = {
            let mut state = self.state.borrow_mut();
            state.lab.step = None;
            (state.lab.input.clone(), state.lab.output.clone())
        };
        let said = |app: &Rc<App>, text: String| {
            let mut state = app.state.borrow_mut();
            state.lab.recorded = None;
            state.lab.missed = Some(text);
        };
        let outcome = (|| {
            let (Some(input), Some(output)) = (input, output) else {
                return Err(t!("lab.sweep-needs-output"));
            };
            if input == Of::Channel(output.clone()) {
                return Err(t!("lab.recorded-same"));
            }
            let going_in = match &input {
                Of::Channel(name) => self.channel_record(name),
                Of::Converter(pin) => self.converter_record(*pin),
                Of::Played => return Err(t!("lab.recorded-needs-record")),
            };
            let (Some(going_in), Some(coming_out)) = (going_in, self.channel_record(&output))
            else {
                return Err(t!("lab.recorded-no-record", record = describe(&input)));
            };
            match sweep::recorded(&going_in, &coming_out) {
                Ok(points) => {
                    let tones = points
                        .iter()
                        .map(|point| {
                            let db = dsp::db(point.gain);
                            let db = if db.abs() < 1.0 {
                                format!("{db:.2}")
                            } else {
                                format!("{db:.1}")
                            };
                            format!("{} Hz {db} dB {:.0}°", point.freq, point.phase.to_degrees())
                        })
                        .collect::<Vec<_>>()
                        .join(t!("lab.recorded-joiner").as_str());
                    Ok((
                        t!(
                            "lab.recorded-result",
                            input = describe(&input),
                            output = output,
                            tones = tones
                        ),
                        points,
                    ))
                }
                Err(sweep::Unread::Apart) => Err(t!(
                    "lab.recorded-apart",
                    input = describe(&input),
                    output = output
                )),
                Err(sweep::Unread::NoTones) => {
                    Err(t!("lab.recorded-no-tones", input = describe(&input)))
                }
                Err(sweep::Unread::Missed(Missed::Tone(refusal))) => {
                    Err(dsp_refusal_text(&refusal))
                }
                Err(sweep::Unread::Missed(Missed::Silent)) => {
                    Err(t!("lab.recorded-silent", record = describe(&input)))
                }
            }
        })();
        match outcome {
            Ok((text, points)) => {
                let mut state = self.state.borrow_mut();
                state.lab.missed = None;
                state.lab.recorded = Some(text);
                state.lab.swept = points;
            }
            Err(text) => said(self, text),
        }
        self.sync_lab();
    }

    // ---- drawing ---------------------------------------------------------

    pub(super) fn sync_lab(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let live = self.sim_live();
        let sources = self.lab_sources();
        // A source picked stays picked while it is on the sheet; otherwise
        // the first is.
        {
            let mut state = self.state.borrow_mut();
            let still = state
                .lab
                .source
                .as_ref()
                .is_some_and(|picked| sources.iter().any(|(s, _)| s == picked));
            if !still {
                state.lab.source = sources.first().map(|(s, _)| s.clone());
            }
        }
        let played_result = self.lab_played();
        let played = played_result.as_ref().and_then(|p| p.as_ref().ok());
        {
            let state = self.state.borrow();
            let lab = &state.lab;
            let rows: Vec<LabSource> = sources
                .iter()
                .map(|(source, sheet_text)| LabSource {
                    label: source.label().into(),
                    text: lab
                        .playing
                        .get(source)
                        .cloned()
                        .unwrap_or_else(|| sheet_text.clone())
                        .into(),
                    picked: lab.source.as_ref() == Some(source),
                    error: lab.errors.get(source).cloned().unwrap_or_default().into(),
                })
                .collect();
            shell.set_lab_sources(ModelRc::new(VecModel::from(rows)));
            let presets: Vec<LabPreset> = rusty_embed::signal::presets()
                .iter()
                .map(|p| LabPreset {
                    id: p.id.into(),
                    name: preset_name(p.id).into(),
                })
                .collect();
            let names: Vec<SharedString> = std::iter::once(t!("lab.presets").into())
                .chain(presets.iter().map(|p| p.name.clone()))
                .collect();
            shell.set_lab_preset_names(ModelRc::new(VecModel::from(names)));
            shell.set_lab_presets(ModelRc::new(VecModel::from(presets)));
            let mut console = Vec::new();
            for def in &state.sim.sensors {
                console.push(LabConsole {
                    name: def.name.as_str().into(),
                    unit: def.unit.clone().unwrap_or_default().into(),
                    axis: -1,
                    text: SharedString::new(),
                });
                for axis in 0..usize::from(def.components) {
                    console.push(LabConsole {
                        name: def.name.as_str().into(),
                        unit: SharedString::new(),
                        axis: axis as i32,
                        text: lab
                            .console
                            .get(&def.name)
                            .and_then(|texts| texts.get(axis).cloned())
                            .unwrap_or_default()
                            .into(),
                    });
                }
            }
            shell.set_lab_console(ModelRc::new(VecModel::from(console)));
            shell.set_lab_live(live);
            shell.set_lab_view(lab.view);
            shell.set_lab_refused(SharedString::from(
                played_result
                    .as_ref()
                    .and_then(|p| p.as_ref().err().cloned())
                    .unwrap_or_default(),
            ));
        }
        let view = self.state.borrow().lab.view;
        match view {
            0 => self.sync_lab_time(&shell, played),
            1 => self.sync_lab_spectrum(&shell, played),
            2 => self.sync_lab_response(&shell, played, live),
            _ => self.sync_lab_design(&shell, played),
        }
    }

    fn sync_lab_time(&self, shell: &Shell<'_>, played: Option<&Played>) {
        let window = self.state.borrow().lab.window;
        shell.set_lab_window(window as i32);
        let host_clock = {
            let state = self.state.borrow();
            state.sim.plot.clock == Some(TraceClock::Host) && !state.sim.plot.channels.is_empty()
        };
        shell.set_lab_host_clock(host_clock);
        let window_us = (WINDOWS[window] * 1e6) as u64;
        let mut lanes = Vec::new();
        if let Some(to) = self.newest() {
            let from = to.saturating_sub(window_us);
            let span = (to - from).max(1) as f64;
            let across = |points: Vec<(u64, f64)>| -> Vec<(f64, f64)> {
                let placed: Vec<(f64, f64)> = points
                    .into_iter()
                    .map(|(at, value)| ((at.saturating_sub(from)) as f64 / span, value))
                    .collect();
                chart::thin(&placed, 800)
            };
            let lane = |label: String, ink: &str, points: Vec<(f64, f64)>| {
                let range = chart::band(points.iter().map(|(_, y)| *y)).unwrap_or((0.0, 1.0));
                LabLane {
                    label: label.into(),
                    ink: colour(ink),
                    commands: chart::path(&points, range, W, H).into(),
                    scale: format!("{} … {}", chart::tick(range.0), chart::tick(range.1)).into(),
                }
            };
            let state = self.state.borrow();
            if let Some(played) = played
                && let Some(start) = lab::start_of(played, &state.lab.switched)
            {
                lanes.push(lane(
                    t!("lab.lane-played", source = played.source.label()),
                    PLAYED,
                    across(played_points(played, start, from, to)),
                ));
            }
            for (pin, record) in &state.lab.conversions {
                let steps = conversion_steps(record, from, to);
                if steps.is_empty() {
                    continue;
                }
                lanes.push(lane(
                    t!("lab.lane-converter", pin = *pin),
                    "#9aa2ae",
                    across(steps),
                ));
            }
            if state.sim.plot.clock == Some(TraceClock::Firmware) {
                for (index, (name, points)) in state.sim.plot.channels.iter().enumerate() {
                    let begin = points.partition_point(|(at, _)| *at < from);
                    let inside: Vec<(u64, f64)> = points[begin..]
                        .iter()
                        .take_while(|(at, _)| *at <= to)
                        .map(|(at, value)| (*at, f64::from(*value)))
                        .collect();
                    if inside.is_empty() {
                        continue;
                    }
                    lanes.push(lane(name.clone(), INK[index % INK.len()], across(inside)));
                }
            }
        }
        shell.set_lab_lanes(ModelRc::new(VecModel::from(lanes)));
    }

    fn records_model(
        &self,
        offered: &[Of],
        played: Option<&Played>,
        on: Option<&Of>,
    ) -> ModelRc<LabRecord> {
        let rows: Vec<LabRecord> = offered
            .iter()
            .enumerate()
            .map(|(index, of)| LabRecord {
                label: record_label(of, played).into(),
                ink: colour(if *of == Of::Played {
                    PLAYED
                } else {
                    INK[index % INK.len()]
                }),
                on: on == Some(of),
            })
            .collect();
        ModelRc::new(VecModel::from(rows))
    }

    fn sync_lab_spectrum(&self, shell: &Shell<'_>, played: Option<&Played>) {
        let offered = self.records_offered(played.is_some());
        let (chosen, span, window_kind, log) = {
            let state = self.state.borrow();
            let lab = &state.lab;
            (
                lab.chosen
                    .clone()
                    .filter(|of| offered.contains(of))
                    .or_else(|| offered.first().cloned()),
                lab.span,
                lab.window_kind,
                lab.log,
            )
        };
        shell.set_lab_records(self.records_model(&offered, played, chosen.as_ref()));
        shell.set_lab_span(span as i32);
        shell.set_lab_window_kind(window_index(window_kind));
        shell.set_lab_log(log);
        let computed = chosen.and_then(|of| {
            let record = self.take(&of, played, SPANS[span])?;
            let found = dsp::spectrum(&record.samples, record.rate, window_kind);
            (!found.freqs.is_empty()).then_some((found, record.rate, record.samples.len()))
        });
        let Some((found, rate, count)) = computed else {
            shell.set_lab_spectrum_empty(true);
            return;
        };
        shell.set_lab_spectrum_empty(false);
        let nyquist = rate / 2.0;
        let low = if log {
            found.freqs.get(1).copied().unwrap_or(1.0)
        } else {
            0.0
        };
        let decibels: Vec<f64> = found.amplitude.iter().map(|a| dsp::db(*a)).collect();
        let top = decibels
            .iter()
            .copied()
            .filter(|d| d.is_finite())
            .fold(f64::MIN, f64::max);
        let floor = top - DEPTH_DB;
        let points: Vec<(f64, f64)> = found
            .freqs
            .iter()
            .zip(&decibels)
            .filter(|(freq, _)| !log || **freq >= low)
            .map(|(freq, db)| (place(*freq, low, nyquist, log), db.max(floor)))
            .collect();
        let path = chart::path(&chart::thin(&points, 900), (floor, top), W, H);
        let peak =
            found
                .freqs
                .iter()
                .zip(&decibels)
                .skip(3)
                .fold(
                    (0.0, f64::MIN),
                    |best, (f, d)| if *d > best.1 { (*f, *d) } else { best },
                );
        let ticks = frequency_ticks(low, nyquist, log);
        shell.set_lab_spectrum_path(path.into());
        shell.set_lab_spectrum_grid(
            verticals(ticks.iter().map(|f| place(*f, low, nyquist, log) * W)).into(),
        );
        shell.set_lab_spectrum_tones(
            verticals(
                tones_of(played)
                    .into_iter()
                    .filter(|f| *f > low && *f < nyquist)
                    .map(|f| place(f, low, nyquist, log) * W),
            )
            .into(),
        );
        shell.set_lab_spectrum_ticks(ticks_model(&ticks, |f| place(f, low, nyquist, log)));
        shell.set_lab_spectrum_range(format!("{floor:.0} … {top:.0} dB").into());
        shell.set_lab_spectrum_caption(
            t!(
                "lab.spectrum-caption",
                count = count,
                rate = chart::tick(rate),
                peak = chart::tick(peak.0),
                level = format!("{:.1}", peak.1)
            )
            .into(),
        );
    }

    fn sync_lab_response(&self, shell: &Shell<'_>, played: Option<&Played>, live: bool) {
        let offered = self.records_offered(played.is_some());
        let channels: Vec<String> = offered
            .iter()
            .filter_map(|of| match of {
                Of::Channel(name) => Some(name.clone()),
                _ => None,
            })
            .collect();
        // What the sweep reads going in: a converter first, then a channel;
        // coming out, a channel that is not the input.
        {
            let mut state = self.state.borrow_mut();
            let lab = &mut state.lab;
            let first = offered
                .iter()
                .find(|of| matches!(of, Of::Converter(_)))
                .or_else(|| offered.iter().find(|of| matches!(of, Of::Channel(_))))
                .cloned();
            if lab.input.as_ref().is_none_or(|of| !offered.contains(of))
                && let Some(first) = first
            {
                lab.input = Some(first);
            }
            let input = lab.input.clone();
            if lab.output.as_ref().is_none_or(|name| {
                !channels.contains(name) || input == Some(Of::Channel(name.clone()))
            }) {
                lab.output = channels
                    .iter()
                    .find(|name| input.as_ref() != Some(&Of::Channel((*name).clone())))
                    .or(channels.first())
                    .cloned();
            }
        }
        let state = self.state.borrow();
        let lab = &state.lab;
        let plan = lab.plan.clone();
        let fields = [
            plan.from,
            plan.to,
            plan.points as f64,
            plan.amplitude,
            plan.offset,
        ];
        let labels = [
            t!("lab.sweep-from"),
            t!("lab.sweep-to"),
            t!("lab.sweep-points"),
            t!("lab.sweep-amplitude"),
            t!("lab.sweep-offset"),
        ];
        let fields: Vec<LabParam> = fields
            .iter()
            .zip(labels)
            .map(|(value, label)| LabParam {
                label: label.into(),
                text: chart::tick(*value).into(),
            })
            .collect();
        shell.set_lab_sweep_fields(ModelRc::new(VecModel::from(fields)));
        let input = lab.input.clone().unwrap_or(Of::Played);
        let names: Vec<SharedString> = offered
            .iter()
            .map(|of| record_label(of, played).into())
            .collect();
        shell.set_lab_input_names(ModelRc::new(VecModel::from(names)));
        shell.set_lab_input_index(
            offered
                .iter()
                .position(|of| *of == input)
                .map_or(-1, |at| at as i32),
        );
        let names: Vec<SharedString> = channels.iter().map(|name| name.as_str().into()).collect();
        shell.set_lab_output_names(ModelRc::new(VecModel::from(names)));
        shell.set_lab_output_index(
            lab.output
                .as_ref()
                .and_then(|name| channels.iter().position(|c| c == name))
                .map_or(-1, |at| at as i32),
        );
        let sweeping = lab.sweeping.is_some();
        shell.set_lab_sweeping(sweeping);
        shell.set_lab_sweep_ready(live && lab.output.is_some());
        shell.set_lab_measure_ready(!sweeping && lab.input.is_some() && lab.output.is_some());
        let status = if let Some((step, of, freq)) = lab.step {
            t!(
                "lab.sweep-step",
                step = step,
                of = of,
                freq = chart::tick(freq)
            )
        } else if let Some(why) = &lab.missed {
            why.clone()
        } else if let Some(found) = &lab.recorded {
            found.clone()
        } else if !live && channels.len() >= 2 {
            t!("lab.recorded-ready")
        } else if !live {
            t!("lab.sweep-needs-run")
        } else if channels.is_empty() {
            t!("lab.sweep-needs-output")
        } else {
            t!("lab.sweep-ready")
        };
        shell.set_lab_status(status.into());

        // The chart: what was measured, beside the design's own curve.
        let measured = &lab.swept;
        let lowest = measured.iter().map(|p| p.freq).fold(plan.from, f64::min);
        let highest = measured.iter().map(|p| p.freq).fold(plan.to, f64::max);
        let (from, to) = (lowest / 1.25, highest * 1.25);
        let rate = lab.design_rate.or_else(|| {
            lab.output.as_ref().and_then(|name| {
                (state.sim.plot.clock == Some(TraceClock::Firmware))
                    .then(|| {
                        let (_, points) = state
                            .sim
                            .plot
                            .channels
                            .iter()
                            .find(|(known, _)| known == name)?;
                        record::from_channel(points).map(|r| r.rate)
                    })
                    .flatten()
            })
        });
        let curve = rate
            .map(|rate| design_curve(&lab.design, rate, from, to))
            .unwrap_or_default();
        let joined = lab.recorded.is_none();
        let db = |p: &Point| dsp::db(p.gain).max(-120.0);
        let degrees = |p: &Point| p.phase.to_degrees();
        let gains = measured
            .iter()
            .chain(&curve)
            .map(db)
            .filter(|v| v.is_finite());
        let (low, high) = chart::band(gains).unwrap_or((-40.0, 5.0));
        let (low, high) = ((low - 3.0).max(-120.0), (high + 3.0).min(40.0));
        let at = |freq: f64| place(freq, from, to, true);
        let line = |points: &[Point], value: &dyn Fn(&Point) -> f64, range: (f64, f64)| {
            let placed: Vec<(f64, f64)> = points.iter().map(|p| (at(p.freq), value(p))).collect();
            chart::path(&placed, range, W, H)
        };
        let dots = |value: &dyn Fn(&Point) -> f64, range: (f64, f64)| -> ModelRc<LabDot> {
            let rows: Vec<LabDot> = measured
                .iter()
                .map(|p| LabDot {
                    x: (at(p.freq) * W) as f32,
                    y: (H - 2.0 - (value(p) - range.0) / (range.1 - range.0).max(1e-12) * (H - 4.0))
                        as f32,
                })
                .collect();
            ModelRc::new(VecModel::from(rows))
        };
        shell.set_lab_gain_curve(line(&curve, &db, (low, high)).into());
        shell.set_lab_phase_curve(line(&curve, &degrees, (-180.0, 180.0)).into());
        shell.set_lab_gain_measured(if joined {
            line(measured, &db, (low, high)).into()
        } else {
            SharedString::new()
        });
        shell.set_lab_phase_measured(if joined {
            line(measured, &degrees, (-180.0, 180.0)).into()
        } else {
            SharedString::new()
        });
        shell.set_lab_gain_dots(dots(&db, (low, high)));
        shell.set_lab_phase_dots(dots(&degrees, (-180.0, 180.0)));
        let ticks = frequency_ticks(from, to, true);
        shell.set_lab_response_grid(verticals(ticks.iter().map(|f| at(*f) * W)).into());
        shell.set_lab_response_ticks(ticks_model(&ticks, at));
        shell.set_lab_gain_range(format!("{low:.0} … {high:.0} dB").into());
        shell.set_lab_response_empty(measured.is_empty() && curve.is_empty());
    }

    fn sync_lab_design(&self, shell: &Shell<'_>, played: Option<&Played>) {
        let estimate = self.estimated_rate();
        let state = self.state.borrow();
        let lab = &state.lab;
        let now = Kind::of(&lab.design);
        let kinds: Vec<SharedString> = Kind::ALL
            .iter()
            .map(|kind| kind_label(*kind).into())
            .collect();
        shell.set_lab_kind_names(ModelRc::new(VecModel::from(kinds)));
        shell.set_lab_kind_index(
            Kind::ALL
                .iter()
                .position(|k| *k == now)
                .map_or(-1, |at| at as i32),
        );
        let shown: Vec<LabParam> = params(&lab.design)
            .into_iter()
            .map(|(param, value)| LabParam {
                label: param_label(param).into(),
                text: chart::tick(value).into(),
            })
            .collect();
        shell.set_lab_params(ModelRc::new(VecModel::from(shown)));
        shell.set_lab_fir_window(match &lab.design {
            Design::Fir { window, .. } => window_index(*window),
            _ => -1,
        });
        shell.set_lab_design_rate(lab.design_rate.map(chart::tick).unwrap_or_default().into());
        shell.set_lab_design_rate_hint(estimate.map(chart::tick).unwrap_or_default().into());
        shell.set_lab_code_name(lab.code_name.as_str().into());
        let rate = lab.design_rate.or(estimate);
        let Some(rate) = rate else {
            shell.set_lab_design_ok(false);
            shell.set_lab_design_note(t!("lab.design-needs-rate").into());
            return;
        };
        let filter = match lab.design.realize(rate) {
            Ok(filter) => filter,
            Err(refusal) => {
                shell.set_lab_design_ok(false);
                shell.set_lab_design_note(dsp_refusal_text(&refusal).into());
                return;
            }
        };
        shell.set_lab_design_ok(true);
        shell.set_lab_design_note(SharedString::new());
        let nyquist = rate / 2.0;
        let low = (nyquist / 2000.0).max(0.01);
        let freqs: Vec<f64> = (0..300)
            .map(|n| low * ((nyquist * 0.999 / low).ln() * f64::from(n) / 299.0).exp())
            .collect();
        let gain: Vec<(f64, f64)> = freqs
            .iter()
            .filter_map(|f| {
                let response = filter.response(*f)?;
                Some((
                    place(*f, low, nyquist, true),
                    dsp::db(response.gain).max(-100.0),
                ))
            })
            .collect();
        let input = played.map(|played| {
            let seconds = (played.samples.len() as f64 / f64::from(played.rate)).clamp(1.0, 20.0);
            record::from_played(played, 0, 0, rate, seconds)
        });
        let spectrum = input.as_ref().map(|input| {
            let found = dsp::spectrum(&input.samples, input.rate, Window::Hann);
            let top = found
                .amplitude
                .iter()
                .skip(1)
                .copied()
                .fold(0.0, f64::max)
                .max(1e-300);
            let points: Vec<(f64, f64)> = found
                .freqs
                .iter()
                .zip(&found.amplitude)
                .filter(|(f, _)| **f >= low)
                .map(|(f, a)| (place(*f, low, nyquist, true), dsp::db(a / top).max(-100.0)))
                .collect();
            chart::path(&chart::thin(&points, 900), (-100.0, 5.0), W, H)
        });
        let ticks = frequency_ticks(low, nyquist, true);
        shell.set_lab_design_response(chart::path(&gain, (-100.0, 5.0), W, H).into());
        shell.set_lab_design_spectrum(spectrum.unwrap_or_default().into());
        shell.set_lab_design_grid(
            verticals(ticks.iter().map(|f| place(*f, low, nyquist, true) * W)).into(),
        );
        shell.set_lab_design_ticks(ticks_model(&ticks, |f| place(f, low, nyquist, true)));
        match input {
            Some(input) => {
                let shown = input.samples.len().min((input.rate * 2.0) as usize).max(2);
                let before = &input.samples[..shown.min(input.samples.len())];
                let after = filter.apply(before);
                let range = chart::band(before.iter().chain(&after).copied()).unwrap_or((0.0, 1.0));
                let across = |values: &[f64]| {
                    let last = (values.len().max(2) - 1) as f64;
                    let placed: Vec<(f64, f64)> = values
                        .iter()
                        .enumerate()
                        .map(|(n, v)| (n as f64 / last, *v))
                        .collect();
                    chart::path(&chart::thin(&placed, 900), range, W, H)
                };
                shell.set_lab_design_before(across(before).into());
                shell.set_lab_design_after(across(&after).into());
                shell.set_lab_design_time(
                    t!(
                        "lab.design-time",
                        seconds = chart::tick(shown as f64 / input.rate)
                    )
                    .into(),
                );
            }
            None => {
                shell.set_lab_design_before(SharedString::new());
                shell.set_lab_design_after(SharedString::new());
                shell.set_lab_design_time(SharedString::new());
            }
        }
        match filter.code(&lab.code_name) {
            Ok(text) => {
                shell.set_lab_code(text.into());
                shell.set_lab_code_refusal(SharedString::new());
            }
            Err(refusal) => {
                shell.set_lab_code(SharedString::new());
                shell.set_lab_code_refusal(dsp_refusal_text(&refusal).into());
            }
        }
    }
}

fn ticks_model(ticks: &[f64], at: impl Fn(f64) -> f64) -> ModelRc<LabTick> {
    let rows: Vec<LabTick> = ticks
        .iter()
        .map(|f| LabTick {
            at: at(*f) as f32,
            text: chart::tick(*f).into(),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}
