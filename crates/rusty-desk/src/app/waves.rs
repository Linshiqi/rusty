//! The Waves panel: one lane per pin from the run's trace, on the clock the
//! trace declares, and the window of time the lanes show — zoomed with the
//! wheel about the pointer, panned by dragging, following the newest edge
//! while the run goes on until the first pan, the way every logic analyser
//! behaves. Export writes a VCD beside the build, where PulseView and
//! GTKWave open it. The trace and its arithmetic are `rusty_embed::waves`.

use std::{rc::Rc, time::Duration};

use rusty_embed::{
    plot::TraceClock,
    waves::{format_us, tick_step, ticks_between},
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{Shell, WaveLane, WaveTick, run, work};

/// The lanes' own units across: the path is stretched over whatever room
/// the panel has.
const UNITS: f64 = 1000.0;

/// Where the panel looks.
pub(super) struct View {
    /// The first moment shown, in the trace's microseconds.
    start: f64,
    /// Microseconds a unit of the lanes' thousand.
    per_unit: f64,
    /// The newest edge stays in view.
    pub(super) follow: bool,
    /// How wide the lanes are on screen, for the ticks' spacing.
    width_px: f64,
    /// Where a drag began.
    grabbed: f64,
    /// A redraw is on its way.
    due: bool,
}

impl Default for View {
    fn default() -> View {
        View {
            start: 0.0,
            per_unit: 1_000.0,
            follow: true,
            width_px: 600.0,
            grabbed: 0.0,
            due: false,
        }
    }
}

impl View {
    fn span(&self) -> f64 {
        self.per_unit * UNITS
    }

    fn step(&self) -> f64 {
        tick_step(self.span() / self.width_px.max(50.0))
    }
}

impl App {
    /// The trace moved: the panel redraws a few times a second at most.
    pub(super) fn waves_moved(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.sim.waves.due {
                return;
            }
            state.sim.waves.due = true;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(100), move || {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().sim.waves.due = false;
                app.sync_waves();
            }
        });
    }

    pub(super) fn waves_resized(self: &Rc<Self>, width: f32) {
        self.state.borrow_mut().sim.waves.width_px = f64::from(width);
        self.sync_waves();
    }

    pub(super) fn waves_grab(&self) {
        let mut state = self.state.borrow_mut();
        let view = &mut state.sim.waves;
        view.follow = false;
        view.grabbed = view.start;
    }

    /// Dragged `by` of the lanes' width since the press.
    pub(super) fn waves_drag(self: &Rc<Self>, by: f32) {
        {
            let mut state = self.state.borrow_mut();
            let view = &mut state.sim.waves;
            view.start = view.grabbed - f64::from(by) * view.span();
        }
        self.sync_waves();
    }

    /// Zoomed by `factor` about `at` of the lanes' width.
    pub(super) fn waves_zoom(self: &Rc<Self>, at: f32, factor: f32) {
        {
            let mut state = self.state.borrow_mut();
            let view = &mut state.sim.waves;
            view.follow = false;
            let at = f64::from(at).clamp(0.0, 1.0);
            let anchor = view.start + at * view.span();
            view.per_unit = (view.per_unit / f64::from(factor)).clamp(0.01, 1.0e9);
            view.start = anchor - at * view.span();
        }
        self.sync_waves();
    }

    /// The time under the pointer, `at` of the lanes' width across.
    pub(super) fn waves_time_at(&self, at: f32) -> String {
        let state = self.state.borrow();
        let view = &state.sim.waves;
        format_us(view.step(), view.start + f64::from(at) * view.span())
    }

    pub(super) fn waves_follow(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.sim.waves.follow = !state.sim.waves.follow;
        }
        self.sync_waves();
    }

    /// The whole capture in view.
    pub(super) fn waves_fit(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let sim = &mut state.sim;
            let Some((first, last)) = sim.trace.span() else {
                return;
            };
            let span = (last - first).max(1) as f64 * 1.05;
            sim.waves.follow = false;
            sim.waves.per_unit = span / UNITS;
            sim.waves.start = first as f64 - span * 0.02;
        }
        self.sync_waves();
    }

    /// The trace as VCD, into the build's own directory.
    pub(super) fn waves_export(self: &Rc<Self>) {
        let text = {
            let state = self.state.borrow();
            if state.sim.trace.events.is_empty() {
                return;
            }
            rusty_embed::to_vcd(&state.sim.trace.events)
        };
        let Some(root) = self.root() else { return };
        work::spawn(
            move || {
                let firmware = rusty_embed::project::firmware_root(&root);
                let dir = firmware.join("target").join("rusty-sim");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join("trace.vcd");
                std::fs::write(&path, text).map_err(|e| e.to_string())?;
                Ok::<_, String>(path)
            },
            |app, written| match written {
                Ok(path) => {
                    app.runner.say(
                        &t!("misc.waveform-written", path = path.display().to_string()),
                        run::kind::DONE,
                    );
                    app.show_panel("bottom", "output");
                }
                Err(reason) => app.note(1, reason),
            },
        );
    }

    pub(super) fn sync_waves(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let mut state = self.state.borrow_mut();
        let sim = &mut state.sim;
        let trace = &sim.trace;
        let view = &mut sim.waves;
        if view.follow
            && let Some((_, last)) = trace.span()
        {
            view.start = last as f64 - view.span() * 0.85;
        }
        let (start, end) = (view.start, view.start + view.span());
        let lanes: Vec<WaveLane> = trace
            .pins()
            .into_iter()
            .map(|pin| WaveLane {
                name: rusty_embed::nets::pin_label(sim.ports, pin).into(),
                commands: trace.lane(pin, start, end, UNITS, 4.0, 26.0).into(),
            })
            .collect();
        let step = view.step();
        let ticks: Vec<WaveTick> = if lanes.is_empty() {
            Vec::new()
        } else {
            ticks_between(start, end, step)
                .into_iter()
                .map(|at| WaveTick {
                    at: ((at - start) / (end - start)) as f32,
                    text: format_us(step, at).into(),
                })
                .collect()
        };
        shell.set_waves_clock(SharedString::from(match trace.clock {
            Some(TraceClock::Firmware) => t!("waves.clock-firmware"),
            Some(TraceClock::Host) => t!("waves.clock-host"),
            None => t!("waves.no-trace"),
        }));
        shell.set_waves_edges(t!("waves.edges", count = trace.events.len().to_string()).into());
        shell.set_waves_capped(trace.truncated);
        shell.set_waves_follow(view.follow);
        shell.set_waves_lanes(ModelRc::new(VecModel::from(lanes)));
        shell.set_waves_ticks(ModelRc::new(VecModel::from(ticks)));
    }
}
