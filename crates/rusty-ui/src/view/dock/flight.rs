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

use leptos::prelude::*;

use super::*;

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
    state.sim.plot.with(|plot| {
        plot.channels
            .iter()
            .find(|(name, _)| name == channel)
            .and_then(|(_, samples)| samples.last())
            .map(|(_, value)| *value)
    })
}

/// The first of `wanted` that the firmware is actually talking about.
fn preferred(state: AppState, wanted: &[&str]) -> Option<String> {
    state.sim.plot.with(|plot| {
        wanted
            .iter()
            .find(|name| plot.channels.iter().any(|(known, _)| known == *name))
            .map(|name| (*name).to_string())
    })
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
                        "Nothing to show yet. This panel reads two things the firmware says: "
                        <span class="font-mono">"[rusty:tel] gyro_r=…"</span>
                        " for the attitude and "
                        <span class="font-mono">"[rusty:pwm] 0=0.62"</span>
                        " for the motors. "
                        <span class="font-mono">"examples/rate-loop"</span>
                        " prints both."
                    </p>
                </Show>
                <Show when=move || !nothing>
                    <div class="flex gap-6">
                        <Horizon roll=roll.clone() pitch=pitch.clone() />
                        <div class="flex flex-col gap-2">
                            <Readout label="roll" axis=roll.clone() />
                            <Readout label="pitch" axis=pitch.clone() />
                            <Readout label="yaw" axis=yaw.clone() />
                        </div>
                        <Motors motors=motors.clone() />
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
                    "no channel"
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
fn Readout(label: &'static str, axis: Option<(String, Option<f32>)>) -> impl IntoView {
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
                <span class="text-caption text-label-3">"no channel named for it"</span>
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
                "Motors"
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
