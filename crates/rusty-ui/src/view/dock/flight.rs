//! What a control loop is doing, in the shape the loop has.
//!
//! The Plot panel already draws any named channel over time, and for tuning a
//! single gain that is the right picture. It is the wrong picture for two
//! questions a multirotor raises constantly — *which way is it leaning* and
//! *is one motor doing all the work* — because both are about several
//! channels **at one instant**, and a time plot answers about one channel
//! over many.
//!
//! **Nothing here is invented.** The attitude comes from channels the
//! firmware named, matched by convention and overridable; the motor bars come
//! from `[rusty:pwm]`, which is the firmware's own word about what it
//! commanded. Where a channel is missing the panel says so rather than
//! drawing a level horizon, because a level horizon is a claim.

use std::collections::HashMap;

use rusty_i18n::t;

use super::*;
use crate::controller;

/// Channel names this panel will use for the three axes without being told.
///
/// A default, not a guess: every one of these is overridable in the row of
/// pickers, and the panel always names the channel it is actually showing.
/// Matching `gyro_r` before `roll` is deliberate — a rate loop's own word for
/// the axis is more likely to be the one being flown.
const ROLL: [&str; 4] = ["gyro_r", "roll", "gyro_x", "rate_roll"];
const PITCH: [&str; 4] = ["gyro_p", "pitch", "gyro_y", "rate_pitch"];
const YAW: [&str; 4] = ["gyro_y", "yaw", "gyro_z", "rate_yaw"];

/// The last value on a channel, if it has one.
fn latest(state: AppState, channel: &str) -> Option<f32> {
    state.sim.plot.with(|plot| latest_in(plot, channel))
}

fn latest_in(plot: &crate::state::Plot, channel: &str) -> Option<f32> {
    plot.channels
        .iter()
        .find(|(name, _)| name == channel)
        .and_then(|(_, samples)| samples.last())
        .map(|(_, value)| *value)
}

/// The first of `wanted` that the firmware is actually talking about.
fn preferred(state: AppState, wanted: &[&str]) -> Option<String> {
    state.sim.plot.with(|plot| preferred_in(plot, wanted))
}

fn preferred_in(plot: &crate::state::Plot, wanted: &[&str]) -> Option<String> {
    wanted
        .iter()
        .find(|name| plot.channels.iter().any(|(known, _)| known == *name))
        .map(|name| (*name).to_string())
}

#[cfg(test)]
mod channel_tests {
    use super::{PITCH, ROLL, YAW, latest_in, preferred_in};
    use crate::state::Plot;

    fn plot_with(channels: &[(&str, &[f32])]) -> Plot {
        let mut plot = Plot::default();
        for (name, samples) in channels {
            let bucket = plot.channel(name);
            for (at, value) in samples.iter().enumerate() {
                bucket.push((at as u64, *value));
            }
        }
        plot
    }

    /// The panel prefers the rate loop's own word for an axis over the
    /// generic one: with both `gyro_r` and `roll` present, `gyro_r` is shown,
    /// and a channel the firmware never printed is never picked.
    #[test]
    fn the_firmwares_own_axis_name_is_preferred() {
        let plot = plot_with(&[("roll", &[1.0]), ("gyro_r", &[2.0])]);
        assert_eq!(preferred_in(&plot, &ROLL).as_deref(), Some("gyro_r"));
        assert_eq!(preferred_in(&plot, &PITCH), None);
        assert_eq!(preferred_in(&plot, &YAW), None);
    }

    /// The value shown is the newest sample; a channel with none yields
    /// nothing rather than zero, because zero is a reading.
    #[test]
    fn the_latest_sample_is_the_one_shown() {
        let plot = plot_with(&[("gyro_r", &[0.5, -1.25])]);
        assert_eq!(latest_in(&plot, "gyro_r"), Some(-1.25));
        assert_eq!(latest_in(&plot, "gyro_p"), None);
        let empty = plot_with(&[("gyro_p", &[])]);
        assert_eq!(latest_in(&empty, "gyro_p"), None);
    }
}

#[component]
pub(super) fn FlightTab() -> impl IntoView {
    let state = AppState::expect();

    // Overrides, per session. Losing them costs one dropdown, which is the
    // storage rule's own test for what may live in memory.
    let roll_pick = RwSignal::new(String::new());
    let pitch_pick = RwSignal::new(String::new());
    let yaw_pick = RwSignal::new(String::new());

    let axis = move |pick: RwSignal<String>, fallback: &'static [&'static str; 4]| {
        let chosen = pick.get();
        let name = if chosen.is_empty() {
            preferred(state, fallback)
        } else {
            Some(chosen)
        };
        name.map(|name| {
            let value = latest(state, &name);
            (name, value)
        })
    };

    move || {
        let roll = axis(roll_pick, &ROLL);
        let pitch = axis(pitch_pick, &PITCH);
        let yaw = axis(yaw_pick, &YAW);

        // Every driven pin, lowest first, so four motors keep their order
        // between frames rather than reshuffling as a map iterates.
        let mut motors: Vec<(u8, f32)> = state
            .sim
            .pwm
            .with(|pwm| pwm.iter().map(|(p, d)| (*p, *d)).collect());
        motors.sort_by_key(|(pin, _)| *pin);

        let nothing = roll.is_none() && pitch.is_none() && yaw.is_none() && motors.is_empty();

        view! {
            <div class="flex min-h-0 flex-1 gap-4 overflow-y-auto px-4 py-3">
                <Show when=move || nothing>
                    <p class="text-caption leading-relaxed text-label-3">
                        {t!("dock.flight.empty-lead")}
                        <span class="font-mono">"[rusty:tel] gyro_r=…"</span>
                        {t!("dock.flight.empty-mid")}
                        <span class="font-mono">"[rusty:pwm] 0=0.62"</span>
                        {t!("dock.flight.empty-tail")}
                        <span class="font-mono">"examples/rate-loop"</span>
                        {t!("dock.flight.empty-end")}
                    </p>
                </Show>
                <Show when=move || !nothing>
                    <div class="flex flex-col gap-3">
                        <div class="flex gap-6">
                            <Craft />
                            <Horizon roll=roll.clone() pitch=pitch.clone() />
                            <div class="flex flex-col gap-2">
                                <Readout label=t!("dock.flight.roll") axis=roll.clone() />
                                <Readout label=t!("dock.flight.pitch") axis=pitch.clone() />
                                <Readout label=t!("dock.flight.yaw") axis=yaw.clone() />
                            </div>
                            <Motors motors=motors.clone() />
                        </div>
                        <ClosedLoop />
                    </div>
                </Show>
            </div>
        }
    }
}

/// A horizon that leans and pitches with the two channels behind it.
///
/// Deliberately not a 3D model of an aircraft: this is a *rate* reading in
/// most loops, and an aeroplane drawn banking would imply an integrated
/// angle nobody computed. A tilted line is as much as the number supports.
#[component]
fn Horizon(
    roll: Option<(String, Option<f32>)>,
    pitch: Option<(String, Option<f32>)>,
) -> impl IntoView {
    let value =
        |axis: &Option<(String, Option<f32>)>| axis.as_ref().and_then(|(_, v)| *v).unwrap_or(0.0);
    let known = roll.as_ref().and_then(|(_, v)| *v).is_some();
    let degrees = value(&roll) * 6.0;
    let shift = (value(&pitch) * -4.0).clamp(-40.0, 40.0);

    view! {
        <div class="relative size-[132px] shrink-0 overflow-hidden rounded-full border border-line-strong bg-sunken">
            <div
                class="absolute inset-[-40%] transition-transform duration-100"
                style=format!(
                    "transform: rotate({degrees:.1}deg) translateY({shift:.1}px)",
                )
            >
                <div class="absolute inset-x-0 top-0 h-1/2 bg-[#1d3350]" />
                <div class="absolute inset-x-0 bottom-0 h-1/2 bg-[#2a2016]" />
                <div class="absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-label-3" />
            </div>
            // The aircraft mark stays put; the world moves behind it, which is
            // what every attitude indicator ever built does.
            <div class="absolute top-1/2 left-1/2 h-px w-10 -translate-x-1/2 -translate-y-1/2 bg-rust" />
            <div class="absolute top-1/2 left-1/2 size-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-rust" />
            <Show when=move || !known>
                <div class="absolute inset-0 grid place-items-center bg-content/70 text-caption text-label-3">
                    {t!("misc.no-channel")}
                </div>
            </Show>
        </div>
    }
}

/// One axis, naming the channel it came from.
///
/// The name is not decoration. A panel showing "roll 2.4" while reading a
/// channel called `gyro_y` is worse than one showing nothing, and the only
/// way to notice is for it to say which channel it took.
#[component]
fn Readout(label: String, axis: Option<(String, Option<f32>)>) -> impl IntoView {
    match axis {
        Some((channel, value)) => view! {
            <div class="w-[13rem]">
                <div class="flex items-baseline justify-between gap-2">
                    <span class="text-caption text-label-3">{label}</span>
                    <span class="font-mono text-callout text-label">
                        {value.map_or_else(|| "—".to_string(), |v| format!("{v:+.2}"))}
                    </span>
                </div>
                <span class="font-mono text-caption text-label-3">{channel}</span>
            </div>
        }
        .into_any(),
        None => view! {
            <div class="w-[13rem]">
                <div class="flex items-baseline justify-between gap-2">
                    <span class="text-caption text-label-3">{label}</span>
                    <span class="font-mono text-callout text-label-3">"—"</span>
                </div>
                <span class="text-caption text-label-3">{t!("dock.flight.no-channel")}</span>
            </div>
        }
        .into_any(),
    }
}

/// One bar per driven pin.
///
/// Bars rather than numbers because the question is comparative — a hover
/// where one motor sits far above the other three is a frame that is not
/// straight, or a propeller on backwards, and that reads off four heights
/// instantly and off four decimals not at all.
#[component]
fn Motors(motors: Vec<(u8, f32)>) -> impl IntoView {
    view! {
        <div class="flex shrink-0 flex-col gap-1">
            <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                {t!("dock.flight.motors")}
            </span>
            <div class="flex items-end gap-2">
                {motors
                    .into_iter()
                    .map(|(pin, duty)| {
                        let height = (duty.clamp(0.0, 1.0) * 92.0).max(2.0);
                        view! {
                            <div class="flex w-9 flex-col items-center gap-1">
                                <span class="font-mono text-caption text-label-2">
                                    {format!("{:.0}%", duty * 100.0)}
                                </span>
                                <div class="flex h-[92px] w-4 items-end rounded-[3px] bg-sunken">
                                    <div
                                        class="w-full rounded-[3px] bg-rust transition-[height] duration-100"
                                        style=format!("height: {height:.0}px")
                                    />
                                </div>
                                <span class="font-mono text-caption text-label-3">
                                    {format!("{pin}")}
                                </span>
                            </div>
                        }
                    })
                    .collect::<Vec<_>>()}
            </div>
        </div>
    }
}

/// The switch that makes this a simulator rather than a readout.
///
/// Open, the panel shows what the firmware commanded and the gyro reads
/// whatever a person last dragged. Closed, a rigid body stands between them:
/// the motors turn it, and it answers with the rate a gyro bolted to it would
/// report. That is the difference between proving a loop *responds* and
/// watching whether it *settles*.
///
/// Off by default and never self-starting, for a reason worth stating: a
/// firmware reading a real IMU as well would see two sources disagreeing, and
/// the drift would be blamed on the sensor.
#[component]
fn ClosedLoop() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let closed = state.sim.plant_closed.get();
        let feeds = controller::plant_feeds(state);
        let motors = state.sim.pwm.with(HashMap::len);
        // Both are required and neither can be invented: without a declared
        // three-axis sensor there is nowhere to inject, and without four
        // driven pins there is no aircraft to model.
        let blocker = if feeds.is_empty() {
            Some(t!("dock.flight.no-sensor"))
        } else if motors < 4 {
            Some(t!("dock.flight.few-motors", motors = motors.to_string()))
        } else {
            None
        };
        let ready = blocker.is_none();

        view! {
            <div class="flex flex-col gap-1 border-t border-line pt-3">
                <label class="flex items-center gap-2">
                    <input
                        type="checkbox"
                        prop:checked=closed
                        disabled=!ready
                        on:change=move |event| {
                            controller::set_plant_closed(state, event_target_checked(&event))
                        }
                        class="accent-rust disabled:opacity-40"
                    />
                    <span class="text-callout text-label">{t!("dock.flight.close-loop")}</span>
                    <Show when=move || closed>
                        <span class="font-mono text-caption text-rust">
                            {feeds
                                .iter()
                                .map(|(name, feed)| format!("{name}←{}", feed.label()))
                                .collect::<Vec<_>>()
                                .join("  ")}
                        </span>
                    </Show>
                </label>
                <p class="max-w-[46rem] text-caption leading-relaxed text-label-3">
                    {blocker.clone().unwrap_or_else(|| t!("dock.flight.plant-help"))}
                </p>
            </div>
        }
    }
}

/// The aircraft, drawn where it is actually pointing.
///
/// **This is for axes and signs, not for looking at.** Tilt the board right
/// and watch the model go left and a reversed axis is obvious in one second;
/// the same mistake read off three changing numbers is nearly invisible, and
/// it is the mistake that ends a first flight. It also shows yaw, which an
/// artificial horizon structurally cannot.
///
/// Two sources, and it always says which. Closed-loop, the plant's own
/// orientation — unambiguous, because it *is* an attitude. Open, whatever
/// telemetry channels look like angles, **read as degrees** and shown beside
/// the drawing: firmware publishing radians makes a craft that barely moves
/// next to a number reading 0.31, which is its own diagnosis.
///
/// Rotation order is yaw, then pitch, then roll — the aerospace convention,
/// and stated because Euler angles have no meaning without it.
#[component]
fn Craft() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let closed = state.sim.plant_closed.get();
        let (angles, source) = if closed {
            let radians = state.sim.plant.with(rusty_embed::Plant::attitude);
            (
                Some([
                    radians[0].to_degrees(),
                    radians[1].to_degrees(),
                    radians[2].to_degrees(),
                ]),
                t!("dock.flight.from-plant"),
            )
        } else {
            let pick = |names: &[&str]| {
                preferred(state, names).and_then(|n| latest(state, &n).map(|v| (n, v)))
            };
            match (
                pick(&["att_roll", "roll"]),
                pick(&["att_pitch", "pitch", "pit"]),
                pick(&["att_yaw", "yaw"]),
            ) {
                (Some((rn, r)), Some((_, p)), Some((_, y))) => {
                    (Some([r, p, y]), t!("dock.flight.from-channels", name = rn))
                }
                _ => (None, String::new()),
            }
        };

        let Some([roll, pitch, yaw]) = angles else {
            return view! {
                <div class="grid size-[132px] shrink-0 place-items-center rounded-[8px] border border-line bg-sunken px-3 text-center text-caption leading-relaxed text-label-3">
                    {t!("dock.flight.no-attitude")}
                </div>
            }
                .into_any();
        };

        let arm = |degrees: f64, colour: &'static str| {
            view! {
                <div
                    class="absolute top-1/2 left-1/2 h-[3px] w-[56px] origin-left rounded-full"
                    style=format!(
                        "transform: rotate({degrees}deg) translateY(-1.5px); background: {colour}",
                    )
                >
                    <div
                        class="absolute top-1/2 right-0 size-[22px] -translate-y-1/2 translate-x-1/2 rounded-full opacity-70"
                        style=format!("background: {colour}")
                    />
                </div>
            }
        };

        view! {
            <div class="flex shrink-0 flex-col items-center gap-1">
                <div
                    class="grid size-[132px] place-items-center rounded-[8px] border border-line bg-sunken"
                    style="perspective: 420px"
                >
                    <div
                        class="relative size-[112px] transition-transform duration-75"
                        style=format!(
                            "transform-style: preserve-3d; transform: rotateX(58deg) \
                             rotateZ({:.1}deg) rotateY({:.1}deg) rotateX({:.1}deg)",
                            -yaw,
                            roll,
                            -pitch,
                        )
                    >
                        // Front pair rust, rear pair grey: without a nose the
                        // drawing is symmetric and yaw becomes unreadable.
                        {arm(-135.0, "var(--color-rust, #d97757)")}
                        {arm(-45.0, "var(--color-rust, #d97757)")}
                        {arm(45.0, "var(--color-label-3, #8a8a8a)")}
                        {arm(135.0, "var(--color-label-3, #8a8a8a)")}
                        <div class="absolute top-1/2 left-1/2 size-4 -translate-x-1/2 -translate-y-1/2 rounded-[3px] bg-label-2" />
                    </div>
                </div>
                <span class="font-mono text-caption text-label-2">
                    {format!("{roll:+.0}° {pitch:+.0}° {yaw:+.0}°")}
                </span>
                <span class="max-w-[132px] text-center text-caption leading-tight text-label-3">
                    {source}
                </span>
            </div>
        }
            .into_any()
    }
}
