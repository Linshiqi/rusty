//! The status bar's first item: what is running, and how the last run went.
//!
//! The reading is `crate::activity`; this is only the wording. One line,
//! clipped with the whole of it in the tooltip, like every item in the bar —
//! a verdict with a size, a time and a warning count is the longest thing
//! the bar says, and it gives way before the chip does.

use std::time::Duration;

use leptos::prelude::*;

use rusty_i18n::t;

use crate::{
    activity::{Activity, Kind, Outcome, clock},
    controller,
    state::{AppState, DockTab},
    view::components::{Dot, Spinner, Tone},
    view::icon::{Icon, IconView},
};

/// "12.4 s", "1 min 05 s" — worded for the reader.
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
    let mut parts = Vec::new();
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
    parts.push(head);
    if matches!(outcome.kind, Kind::Build | Kind::Test | Kind::Flash) {
        parts.push(duration(outcome.took_ms));
    }
    if outcome.errors > 0 {
        parts.push(t!("activity.errors", count = outcome.errors.to_string()));
    }
    if let Some(size) = outcome.size {
        parts.push(controller::size_line(size));
    }
    if outcome.warnings > 0 {
        parts.push(t!(
            "activity.warnings",
            count = outcome.warnings.to_string()
        ));
    }
    parts.join(" · ")
}

#[component]
pub fn ActivityStatus() -> impl IntoView {
    let state = AppState::expect();

    // A clock that moves only while something runs: an idle bar redrawn
    // twice a second is work for nothing.
    let now = RwSignal::new(js_sys::Date::now());
    let ticker = set_interval_with_handle(
        move || {
            if state.app.activity.with_untracked(Option::is_some) {
                now.set(js_sys::Date::now());
            }
        },
        Duration::from_millis(500),
    )
    .ok();
    on_cleanup(move || {
        if let Some(ticker) = ticker {
            ticker.clear();
        }
    });

    let base = "flex h-full min-w-0 max-w-[34rem] shrink items-center gap-1.5 \
                whitespace-nowrap border-r border-line px-3";
    let show_output = move |_| state.show_dock(DockTab::Output);

    move || {
        if let Some(activity) = state.app.activity.get() {
            let text = running(&activity);
            let took = duration(now.get() - activity.started);
            let title = format!("{text} · {took}");
            return view! {
                <button
                    type="button"
                    title=title
                    on:click=show_output
                    class=format!("{base} transition-colors hover:bg-sunken hover:text-label")
                >
                    <span class="text-amber">
                        <Spinner size=11 />
                    </span>
                    <span class="min-w-0 truncate">{text}</span>
                    <span class="shrink-0 text-label-3">{took}</span>
                </button>
            }
            .into_any();
        }
        if let Some(outcome) = state.app.outcome.get() {
            let text = verdict(&outcome);
            let title = text.clone();
            let (icon, colour) = if outcome.ok {
                (Icon::Check, "text-patina")
            } else {
                (Icon::Warn, "text-crimson")
            };
            return view! {
                <button
                    type="button"
                    title=title
                    on:click=show_output
                    class=format!("{base} transition-colors hover:bg-sunken hover:text-label")
                >
                    <span class=colour>
                        <IconView icon=icon size=12 />
                    </span>
                    <span class="min-w-0 truncate">{text}</span>
                </button>
            }
            .into_any();
        }
        let busy = state.is_busy();
        view! {
            <span class=format!("{base} shrink-0")>
                <Dot tone=if busy { Tone::Amber } else { Tone::Patina } />
                {if busy { t!("status.working") } else { t!("status.ready") }}
            </span>
        }
        .into_any()
    }
}
