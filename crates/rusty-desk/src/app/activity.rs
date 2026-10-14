//! The title bar's activity view, Xcode's: what is running, with the step
//! it is on and a clock, then the verdict until the next run — `Build
//! succeeded · 12.4 s · Flash 85.3 KB · RAM 20.1 KB / 320 KB (6%) · 2
//! warnings`. The reading is `rusty_embed::activity`, shared with the old
//! window; this is the wording and the clock.

use std::{rc::Rc, time::Duration};

use rusty_embed::activity::{Activity, Kind, Outcome, Size, clock};
use rusty_i18n::t;
use slint::{ComponentHandle, SharedString, TimerMode};

use super::App;
use crate::{Shell, format};

/// What the view knows: the run going on, or how the last one ended.
#[derive(Default)]
pub(super) struct Activities {
    now: Option<Activity>,
    outcome: Option<Outcome>,
}

/// "12.4 s", "1 min 05 s".
fn duration(ms: f64) -> String {
    match clock(ms) {
        (0, seconds) => t!("activity.seconds", seconds = format!("{seconds:.1}")),
        (minutes, seconds) => t!(
            "activity.minutes",
            minutes = minutes.to_string(),
            seconds = format!("{:02}", seconds as u64)
        ),
    }
}

/// Flash and RAM, and how full RAM is when the part's is known.
pub(super) fn size_line(size: Size) -> String {
    let flash = format::bytes(size.flash);
    let ram = format::bytes(size.ram);
    match size.ram_capacity.filter(|capacity| *capacity > 0) {
        Some(capacity) => t!(
            "activity.size-of",
            flash = flash,
            ram = ram,
            capacity = format::bytes(capacity as u64),
            percent = ((size.ram as f64 / capacity as f64) * 100.0)
                .round()
                .to_string()
        ),
        None => t!("activity.size", flash = flash, ram = ram),
    }
}

fn running(activity: &Activity) -> String {
    let target = activity.target.clone().unwrap_or_default();
    let head = match activity.kind {
        Kind::Build => t!("activity.building"),
        Kind::Test => t!("activity.testing"),
        Kind::Flash => t!("activity.flashing", target = target),
        Kind::Monitor => t!("activity.monitoring", target = target),
        Kind::Simulate => t!("activity.simulating"),
        Kind::Debug => t!("activity.debugging"),
        Kind::Install => t!("activity.installing", target = target),
        Kind::Link => t!("activity.linked", target = target),
        Kind::Command => t!("activity.running", target = target),
    };
    match &activity.step {
        Some(step) => format!("{head} · {step}"),
        None => head,
    }
}

fn verdict(outcome: &Outcome) -> String {
    let target = outcome.target.clone().unwrap_or_default();
    let code = outcome.code.unwrap_or(-1).to_string();
    let head = match (outcome.kind, outcome.ok) {
        (Kind::Build, true) => t!("activity.built"),
        (Kind::Build, false) => t!("activity.build-failed"),
        (Kind::Test, true) if outcome.passed > 0 => {
            t!("activity.tested", count = outcome.passed.to_string())
        }
        (Kind::Test, true) => t!("activity.tested-none"),
        (Kind::Test, false) if outcome.failed > 0 => t!(
            "activity.tests-failed",
            failed = outcome.failed.to_string(),
            total = (outcome.passed + outcome.failed).to_string()
        ),
        (Kind::Test, false) => t!("activity.test-failed"),
        (Kind::Flash, true) => t!("activity.flashed", target = target),
        (Kind::Flash, false) => t!("activity.flash-failed"),
        (Kind::Monitor, _) => t!("activity.monitor-ended", code = code),
        (Kind::Simulate | Kind::Debug, _) => t!("activity.sim-ended", code = code),
        (Kind::Install, true) => t!("activity.installed", target = target),
        (Kind::Install, false) => t!("activity.install-failed", target = target),
        (Kind::Link, _) => t!("activity.link-ended", code = code),
        (Kind::Command, _) => t!("activity.command-failed", target = target, code = code),
    };
    let mut parts = vec![head];
    if matches!(outcome.kind, Kind::Build | Kind::Test | Kind::Flash) {
        parts.push(duration(outcome.took_ms));
    }
    if outcome.errors > 0 {
        parts.push(t!("activity.errors", count = outcome.errors.to_string()));
    }
    if let Some(size) = outcome.size {
        parts.push(size_line(size));
    }
    if outcome.warnings > 0 {
        parts.push(t!(
            "activity.warnings",
            count = outcome.warnings.to_string()
        ));
    }
    parts.join(" · ")
}

impl App {
    /// The window's clock in milliseconds, which is all the reading needs.
    fn millis(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    /// Something began: the view says so, and the clock runs while it
    /// does — an idle title bar redrawn twice a second is work for nothing.
    pub(super) fn begin(self: &Rc<Self>, kind: Kind, target: Option<String>) {
        let mut activity = Activity::new(kind, self.millis());
        activity.target = target;
        {
            let mut state = self.state.borrow_mut();
            state.activities.now = Some(activity);
            state.activities.outcome = None;
        }
        let weak = Rc::downgrade(self);
        self.clock
            .start(TimerMode::Repeated, Duration::from_millis(500), move || {
                if let Some(app) = weak.upgrade() {
                    app.sync_activity();
                }
            });
        self.sync_activity();
    }

    /// A line the run printed: the crate being compiled, cargo's counts, a
    /// flash that has finished writing and become a monitor.
    pub(super) fn follow_activity(self: &Rc<Self>, line: &str) {
        if !rusty_embed::activity::relevant(line) {
            return;
        }
        let now = self.millis();
        let outcome = {
            let mut state = self.state.borrow_mut();
            let Some(activity) = state.activities.now.as_mut() else {
                return;
            };
            activity.observe(line, now)
        };
        if let Some(outcome) = outcome {
            self.state.borrow_mut().activities.outcome = Some(outcome);
        }
        self.sync_activity();
    }

    pub(super) fn activity_kind(&self) -> Option<Kind> {
        self.state.borrow().activities.now.as_ref().map(|a| a.kind)
    }

    /// It ended with `code`: the verdict, until the next run.
    pub(super) fn conclude(self: &Rc<Self>, code: Option<i32>) {
        let now = self.millis();
        {
            let mut state = self.state.borrow_mut();
            if let Some(activity) = state.activities.now.take()
                && let Some(outcome) = activity.finish(code, now)
            {
                state.activities.outcome = Some(outcome);
            }
        }
        self.clock.stop();
        self.sync_activity();
    }

    /// Somebody stopped it: no verdict, since nothing was decided.
    pub(super) fn abandon(self: &Rc<Self>) {
        self.state.borrow_mut().activities.now = None;
        self.clock.stop();
        self.sync_activity();
    }

    /// The image a build made, weighed after it: on the verdict it ends.
    pub(super) fn weighed(self: &Rc<Self>, size: Size) {
        if let Some(outcome) = self.state.borrow_mut().activities.outcome.as_mut()
            && outcome.kind == Kind::Build
            && outcome.ok
        {
            outcome.size = Some(size);
        }
        self.sync_activity();
    }

    pub(super) fn sync_activity(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let state = self.state.borrow();
        let activities = &state.activities;
        // 0 nothing yet, 1 running, 2 ended well, 3 ended badly.
        let (text, clock, state, kind) = match (&activities.now, &activities.outcome) {
            (Some(activity), _) => {
                let clock = duration(self.millis() - activity.started);
                (
                    format!("{} · {clock}", running(activity)),
                    clock,
                    1,
                    activity.kind,
                )
            }
            (None, Some(outcome)) => (
                verdict(outcome),
                String::new(),
                if outcome.ok { 2 } else { 3 },
                outcome.kind,
            ),
            (None, None) => (String::new(), String::new(), 0, Kind::Command),
        };
        shell.set_activity(SharedString::from(text));
        shell.set_activity_clock(SharedString::from(clock));
        shell.set_activity_state(state);
        shell.set_activity_kind(SharedString::from(match kind {
            Kind::Build => "build",
            Kind::Test => "test",
            Kind::Flash => "flash",
            Kind::Monitor => "monitor",
            Kind::Simulate => "simulate",
            Kind::Debug => "debug",
            Kind::Install => "install",
            Kind::Link => "link",
            Kind::Command => "command",
        }));
    }
}
