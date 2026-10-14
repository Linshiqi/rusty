//! The Plot panel: the firmware's named channels over time, and the
//! tunables it announced — what a control loop is developed against rather
//! than a debugger, since stopping a flight controller to read a variable
//! means the craft falls.
//!
//! The store and the scaling are `rusty_embed::plot`'s, shared with the old
//! window: each channel against its own range, with a floor so a settled
//! loop's ripple is drawn small. A tunable gets a slider only over a range
//! the firmware gave — a range the tool invented is how somebody sends a
//! gain of 500 to a motor loop — and what the firmware says it took is what
//! the panel shows, so a clamp reads as a clamp.

use std::{rc::Rc, time::Duration};

use rusty_embed::plot::{TraceClock, band};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{PlotCurve, PlotTunable, Shell};

/// How much of each channel's tail is drawn: the window a tuning change is
/// judged in.
const WINDOW: usize = 600;

/// The chart's own units; the path is stretched over whatever room it has.
const W: f64 = 1000.0;
const H: f64 = 220.0;

/// One colour per channel, cycled — distinct hues, since two curves that
/// differ by a shade are two curves nobody can tell apart.
const INK: [u32; 8] = [
    0xe0a838, 0x5aa9e6, 0x7bc47f, 0xe06c75, 0xb48ead, 0x56b6c2, 0xd19a66, 0x98c379,
];

/// A number as a reading: three significant figures, no exponent for the
/// ordinary range.
fn reading(value: f32) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    // A band's floor is a hair above nothing; it reads as nothing.
    if value.abs() < 1e-6 {
        return "0".to_string();
    }
    let digits = (3 - 1 - value.abs().log10().floor() as i32).clamp(0, 6) as usize;
    format!("{value:.digits$}")
}

impl App {
    /// The channels or the tunables moved: the panel redraws a few times a
    /// second at most — a 1 kHz loop prints a thousand samples.
    pub(super) fn plot_moved(self: &Rc<Self>) {
        if self.plot_due.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(80), move || {
            if let Some(app) = weak.upgrade() {
                app.plot_due.set(false);
                app.sync_plot();
            }
        });
    }

    /// A channel in the legend clicked: it alone is drawn, or, clicked
    /// again, every channel is.
    pub(super) fn plot_solo(self: &Rc<Self>, name: &str) {
        {
            let mut state = self.state.borrow_mut();
            let shown = &mut state.sim.shown;
            if shown.len() == 1 && shown[0] == name {
                shown.clear();
            } else {
                *shown = vec![name.to_string()];
            }
        }
        self.sync_plot();
    }

    /// A tunable set from the panel: sent on the line the firmware reads,
    /// and shown as what the firmware answers it took.
    pub(super) fn plot_set(self: &Rc<Self>, name: &str, value: f32) {
        let line = rusty_embed::set_param_line(name, value);
        self.runner.send(&line);
    }

    pub(super) fn sync_plot(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let sim = &state.sim;
        let plot = &sim.plot;
        let mut curves = Vec::new();
        for (index, (name, points)) in plot.channels.iter().enumerate() {
            let ink = slint::Color::from_argb_encoded(0xff00_0000 | INK[index % INK.len()]);
            let shown = sim.shown.is_empty() || sim.shown.contains(name);
            let tail = &points[points.len().saturating_sub(WINDOW)..];
            let (low, high) = if tail.is_empty() {
                (0.0, 1.0)
            } else {
                band(tail)
            };
            let span = (high - low) as f64;
            let mut commands = String::new();
            if shown && tail.len() >= 2 {
                let last = (tail.len() - 1) as f64;
                for (at, (_, value)) in tail.iter().enumerate() {
                    let x = at as f64 / last * W;
                    let y = H - ((*value - low) as f64 / span) * (H - 8.0) - 4.0;
                    commands.push_str(if at == 0 { "M " } else { " L " });
                    commands.push_str(&format!("{x:.1} {y:.1}"));
                }
            }
            let latest = tail.last().map(|(_, v)| *v).unwrap_or_default();
            curves.push(PlotCurve {
                name: name.as_str().into(),
                ink,
                commands: commands.into(),
                value: reading(latest).into(),
                swing: format!("±{}", reading((high - low) / 2.0)).into(),
                shown,
            });
        }
        let tunables: Vec<PlotTunable> = sim
            .params
            .iter()
            .map(|param| PlotTunable {
                name: param.name.as_str().into(),
                value: param.value,
                text: reading(param.value).into(),
                min: param.min.unwrap_or_default(),
                max: param.max.unwrap_or_default(),
                ranged: param.min.zip(param.max).is_some_and(|(lo, hi)| hi > lo),
            })
            .collect();
        let note = match plot.clock {
            Some(TraceClock::Firmware) => t!("plot.clock-firmware"),
            Some(TraceClock::Host) => t!("plot.clock-host"),
            None => String::new(),
        };
        let note = if plot.truncated && !note.is_empty() {
            t!("plot.dropped", note = note)
        } else {
            note
        };
        shell.set_plot_curves(ModelRc::new(VecModel::from(curves)));
        shell.set_plot_tunables(ModelRc::new(VecModel::from(tunables)));
        shell.set_plot_note(SharedString::from(note));
        // Writable only while something runs that reads what is typed.
        shell.set_plot_writable(self.runner.takes_input());
    }
}
