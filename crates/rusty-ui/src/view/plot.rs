//! Named channels over time, and the tunables behind them.
//!
//! The panel a control loop is actually developed against. A debugger is the
//! wrong instrument here — stopping a flight controller to read a variable
//! means the craft falls — so what the firmware knows travels as text on the
//! serial line it is already using, and this draws it.
//!
//! Two halves, deliberately together: the curve says what the loop is doing,
//! and the sliders change it *without a reflash*. Watching a gain take effect
//! in the same second it is typed is the difference between tuning and
//! guessing, and it is the whole reason both live in one tab.
//!
//! Drawn as an SVG polyline per channel rather than a canvas: a few thousand
//! points is well within what the browser draws at 60fps, and it keeps the
//! panel inspectable by the same DOM-reading tests everything else here uses.

use leptos::prelude::*;

use crate::{
    controller,
    state::{AppState, TraceClock},
};

/// How much of the tail is drawn. The window a tuning change is judged in;
/// anything older is scrollback nobody reads while flying.
const WINDOW: usize = 600;

/// One colour per channel, cycled. Distinct hues rather than a gradient —
/// two curves that differ by a shade are two curves nobody can tell apart.
const INK: [&str; 8] = [
    "#e0a838", "#5aa9e6", "#7bc47f", "#e06c75", "#b48ead", "#56b6c2", "#d19a66", "#98c379",
];

/// One channel ready to draw: its index into [`INK`], its name, and the tail
/// of its samples. The index travels with it so a soloed channel keeps the
/// colour it had in the legend.
type Drawn = (usize, String, Vec<(u64, f32)>);

/// A channel's smallest drawn swing, as a fraction of its own magnitude.
///
/// Pure autoscale is wrong here and the real board proved it: a loop settled
/// at 88.0 ± 0.5 has a 1% ripple, and scaling that to the full height drew a
/// converged controller as maximum-amplitude static. "Has it settled?" is the
/// single question this panel exists to answer, so a small wobble must *look*
/// small. Five percent: a ripple under a twentieth of the signal is drawn
/// within a twentieth of the height, and anything larger scales normally.
const FLOOR: f32 = 0.05;

/// The value range a channel is drawn against, and its midpoint behaviour.
///
/// Returns a band that always has width, so the caller never divides by zero
/// and a constant lands in the middle of the plot rather than on its floor —
/// a setpoint pinned to the bottom edge reads as "at minimum", which is a
/// different claim from "not changing".
fn band(points: &[(u64, f32)]) -> (f32, f32) {
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for (_, value) in points {
        low = low.min(*value);
        high = high.max(*value);
    }
    let middle = (low + high) / 2.0;
    let swing = high - low;
    let floor = (middle.abs().max(swing) * FLOOR).max(f32::EPSILON);
    let span = if swing < floor { floor } else { swing };
    (middle - span / 2.0, middle + span / 2.0)
}

#[component]
pub fn Plot() -> impl IntoView {
    // Two columns, always. The right one carries Connect, so folding the
    // panel down to an explanation while empty would hide the only control
    // that makes it non-empty — you could never start the link that starts
    // the data.
    view! {
        <div class="flex min-h-0 flex-1">
            <div class="flex min-w-0 flex-1 flex-col">
                <Signals />
            </div>
            <div class="w-px bg-line" />
            <div class="flex w-[17rem] shrink-0 flex-col overflow-y-auto">
                <Tunables />
                <Sensors />
            </div>
        </div>
    }
}

/// The curve half: the channels, or what to print to get some.
#[component]
fn Signals() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let plot = state.sim.plot.get();
        if plot.channels.is_empty() {
            return view! {
                <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
                    <p class="max-w-[70ch] text-callout leading-relaxed text-label-2">
                        "Nothing is being plotted. Firmware prints a line per loop and this \
                         draws it:"
                    </p>
                    <pre class="mt-2 rounded-[6px] bg-sunken px-3 py-2 font-mono text-caption text-label-3 select-text">
"println!(\"[rusty:tel@{}] gyro_x={},pid_p={}\", now_us, gyro_x, p);
println!(\"[rusty:param] pid_roll_p={} 0..50\", gain);   // announce a tunable"
                    </pre>
                    <p class="mt-2 max-w-[70ch] text-caption leading-relaxed text-label-3">
                        "The stamp is optional and is the firmware's own clock in \
                         microseconds. Without it the panel times by arrival and says so — \
                         a plot that mixed the two silently would lie about when, which is \
                         the only thing a control loop is read for. \
                         examples/pid-tune is a working end of both lines."
                    </p>
                </div>
            }
            .into_any();
        }

        let clock = plot.clock;
        let shown = state.sim.plot_shown.get();
        let drawn: Vec<Drawn> = plot
            .channels
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| shown.is_empty() || shown.contains(name))
            .map(|(index, (name, points))| {
                let tail = points[points.len().saturating_sub(WINDOW)..].to_vec();
                (index, name.clone(), tail)
            })
            .collect();

        view! {
            <Curves drawn=drawn clock=clock truncated=plot.truncated />
            <Legend channels=plot.channels.iter().map(|(n, _)| n.clone()).collect() />
        }
        .into_any()
    }
}

/// The curves themselves, each autoscaled to its own range.
///
/// Per-channel scaling rather than one shared axis: a gyro rate in deg/s and
/// a throttle in 0..1 on one axis makes the second one a flat line at zero.
/// The cost is that heights are not comparable between channels, which the
/// readout compensates for by naming each one's range.
#[component]
fn Curves(drawn: Vec<Drawn>, clock: Option<TraceClock>, truncated: bool) -> impl IntoView {
    const W: f64 = 1000.0;
    const H: f64 = 220.0;

    let note = match clock {
        Some(TraceClock::Firmware) => "firmware clock",
        Some(TraceClock::Host) => "arrival time — the firmware sent no stamp",
        None => "",
    };

    view! {
        <div class="min-h-0 flex-1 overflow-hidden px-3 pt-2">
            <svg
                viewBox=format!("0 0 {W} {H}")
                preserveAspectRatio="none"
                class="h-full w-full"
            >
                {drawn
                    .into_iter()
                    .map(|(index, name, points)| {
                        if points.len() < 2 {
                            return view! { <g /> }.into_any();
                        }
                        let (low, high) = band(&points);
                        let span = high - low;
                        let last = points.len() - 1;
                        let path: String = points
                            .iter()
                            .enumerate()
                            .map(|(at, (_, value))| {
                                let x = at as f64 / last as f64 * W;
                                let y = H - ((value - low) / span) as f64 * (H - 8.0) - 4.0;
                                format!("{x:.1},{y:.1}")
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        let ink = INK[index % INK.len()];
                        view! {
                            <polyline
                                points=path
                                fill="none"
                                stroke=ink
                                stroke-width="1.5"
                                vector-effect="non-scaling-stroke"
                            >
                                <title>{format!("{name}: {low} … {high}")}</title>
                            </polyline>
                        }
                            .into_any()
                    })
                    .collect_view()}
            </svg>
            {(!note.is_empty())
                .then(|| {
                    let text = if truncated {
                        format!("{note} · oldest samples dropped")
                    } else {
                        note.to_string()
                    };
                    view! {
                        <p class="pt-1 text-caption text-label-4">{text}</p>
                    }
                })}
        </div>
    }
}

/// Which channels are drawn, and what each is doing right now.
///
/// Clicking one shows only it; clicking it again brings the rest back. A
/// firmware with forty channels is normal and forty curves at once is not
/// readable, so the filter is the panel's main control.
#[component]
fn Legend(channels: Vec<String>) -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="flex flex-wrap gap-1 border-t border-line px-3 py-1.5">
            {channels
                .into_iter()
                .enumerate()
                .map(|(index, name)| {
                    let ink = INK[index % INK.len()];
                    let pick = name.clone();
                    let label = name.clone();
                    // Value, then how far it moved across the drawn window.
                    // The curve's *height* cannot say that — every channel is
                    // scaled to fit — so without this number a 1% ripple and
                    // a 50% swing look identical, which is the whole question
                    // when you are asking whether a loop has settled.
                    let latest = move || {
                        state.sim.plot.with(|plot| {
                            let Some((_, points)) = plot
                                .channels
                                .iter()
                                .find(|(known, _)| *known == name)
                            else {
                                return String::new();
                            };
                            let tail = &points[points.len().saturating_sub(WINDOW)..];
                            let Some((_, last)) = tail.last() else {
                                return String::new();
                            };
                            let (mut low, mut high) = (f32::MAX, f32::MIN);
                            for (_, value) in tail {
                                low = low.min(*value);
                                high = high.max(*value);
                            }
                            format!("{last:.3}  ±{:.3}", (high - low) / 2.0)
                        })
                    };
                    let dim = {
                        let name = pick.clone();
                        move || {
                            state
                                .sim.plot_shown
                                .with(|shown| !shown.is_empty() && !shown.contains(&name))
                        }
                    };
                    view! {
                        <button
                            type="button"
                            on:click=move |_| {
                                state
                                    .sim.plot_shown
                                    .update(|shown| {
                                        if shown.as_slice() == [pick.clone()] {
                                            shown.clear();
                                        } else {
                                            *shown = vec![pick.clone()];
                                        }
                                    })
                            }
                            class=move || {
                                let base = "flex items-baseline gap-1.5 rounded-[5px] px-1.5 py-0.5 \
                                            font-mono text-caption transition-opacity";
                                if dim() { format!("{base} opacity-40") } else { base.to_string() }
                            }
                        >
                            <span style=format!("color: {ink}")>"—"</span>
                            <span class="text-label-2">{label}</span>
                            <span class="text-label-4">{latest}</span>
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// Opening and closing the two-way link to a board.
///
/// Deliberately not on the device page: that page is "pick a device, see the
/// command, run it", and this runs no command — rusty holds the port itself.
/// It lives beside the sliders it exists to make work.
#[component]
fn Link() -> impl IntoView {
    let state = AppState::expect();
    let picked = RwSignal::new(String::new());

    // The list is the device page's; ask for it here too, because a user who
    // came straight to this tab has never opened that page. Once, untracked:
    // reading `ports` here and scanning when it is empty re-triggers itself
    // for ever on a machine that genuinely has no serial ports.
    Effect::new(move |ran: Option<()>| {
        if ran.is_none() && state.device.ports.with_untracked(Vec::is_empty) {
            controller::scan_devices(state);
        }
    });

    move || {
        if let Some(port) = state.sim.link_port.get() {
            return view! {
                <div class="flex items-center justify-between gap-2 px-3 pb-2">
                    <span class="min-w-0 truncate font-mono text-caption text-label-2">{port}</span>
                    <button
                        type="button"
                        title="Close the port"
                        on:click=move |_| controller::close_link(state)
                        class="shrink-0 rounded-[5px] px-2 py-0.5 text-caption text-crimson transition-colors hover:bg-sunken"
                    >
                        "Disconnect"
                    </button>
                </div>
            }
            .into_any();
        }

        let ports = state.device.ports.get();
        let busy = state.app.session_running.get();
        // What Connect would open: the explicit choice, or the first port that
        // looks like a board. Derived rather than written into `picked` on the
        // way past — writing a signal while rendering the view that reads it is
        // how a panel starts re-rendering itself.
        let chosen = {
            let ports = ports.clone();
            move || {
                let explicit = picked.get();
                if !explicit.is_empty() {
                    return explicit;
                }
                ports
                    .iter()
                    .find(|p| p.likely_board)
                    .or(ports.first())
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            }
        };
        let connect = chosen.clone();

        view! {
            <div class="px-3 pb-2">
                <div class="flex items-center gap-1.5">
                    <select
                        prop:value=chosen
                        on:change=move |event| picked.set(event_target_value(&event))
                        class="min-w-0 flex-1 rounded-[4px] border border-line bg-sunken px-1 py-0.5 font-mono text-caption text-label"
                    >
                        {ports
                            .into_iter()
                            .map(|port| {
                                let label = match &port.bridge {
                                    Some(bridge) => format!("{} · {bridge}", port.name),
                                    None => port.name.clone(),
                                };
                                view! { <option value=port.name>{label}</option> }
                            })
                            .collect_view()}
                    </select>
                    <button
                        type="button"
                        disabled=busy
                        title="Open this port for reading and writing"
                        on:click=move |_| {
                            let port = connect();
                            if !port.is_empty() {
                                controller::open_link(state, port, 115_200);
                            }
                        }
                        class="shrink-0 rounded-[5px] px-2 py-0.5 text-caption text-rust transition-colors hover:bg-sunken disabled:opacity-40"
                    >
                        "Connect"
                    </button>
                </div>
                <p class="pt-1 text-caption leading-relaxed text-label-4">
                    {move || {
                        if busy {
                            "Something else is running. Its telemetry is plotted, but only a \
                             port rusty opened itself can be written to."
                        } else {
                            "rusty opens the port itself, which is what makes a tunable \
                             writable. defmt decoding is espflash's — this mode is plain text."
                        }
                    }}
                </p>
            </div>
        }
        .into_any()
    }
}

/// The tunables the firmware announced, and the sliders that change them.
/// Feeding the firmware a sensor it asked for.
///
/// The input mirror of [`Tunables`], and it follows the same rule for the
/// same reason: **only what the firmware declared**. A panel that offered
/// `gyro` because a drone usually has one, with a range it chose itself,
/// would one day inject 2000°/s into a loop written for 250 — the invented
/// range, in the other direction.
///
/// This is what makes a control loop testable without hardware. QEMU models
/// no I2C or SPI slave, so a firmware that reads its IMU over a bus reads
/// nothing; a firmware that also accepts `Igyro=…` can be flown at a desk.
#[component]
fn Sensors() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let sensors = state.sim.sensors.get();
        if sensors.is_empty() {
            return view! {
                <div class="shrink-0 border-t border-line">
                    <div class="px-3 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                        "Sensors"
                    </div>
                    <p class="px-3 pb-2 text-caption leading-relaxed text-label-3">
                        "None announced. Firmware prints "
                        <span class="font-mono">"[rusty:sensor] gyro=3 rad/s -35..35"</span>
                        " and a card appears here; reading "
                        <span class="font-mono">"Igyro=1.25,-0.5,0.02"</span>
                        " back off its serial input is what lets a loop run with no IMU \
                         attached. Announce on a timer, not only at boot."
                    </p>
                </div>
            }
                .into_any();
        }
        // The same gate the tunables use, and for the same reason: a spawned
        // `espflash monitor` is a running session whose stdin the board never
        // sees, so sliders that silently went nowhere would read as firmware
        // ignoring them.
        let writable = state.sim.link_port.with(Option::is_some) || state.app.session_running.get();
        view! {
            <div class="shrink-0 border-t border-line">
                <div class="px-3 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    "Sensors"
                </div>
                {sensors
                    .into_iter()
                    .map(|def| {
                        // `StoredValue` so every closure below stays `Copy`:
                        // a captured `String` would move into the first one
                        // and leave the rest without a name.
                        let name = StoredValue::new(def.name.clone());
                        let count = def.components as usize;
                        // Bounds from the firmware or none at all.
                        let (low, high) = (def.min.unwrap_or(-1.0), def.max.unwrap_or(1.0));
                        let unit = def.unit.clone().unwrap_or_default();
                        let label = def.name.clone();
                        let held = move || {
                            state
                                .sim
                                .sensor_values
                                .with(|all| {
                                    name.with_value(|n| all.get(n).cloned())
                                        .unwrap_or_else(|| vec![0.0; count])
                                })
                        };
                        // One line carries the whole sample: moving any
                        // component sends every component. A torn sample is
                        // worse than a late one for anything that fuses them.
                        let send = move |axis: usize, raw: String| {
                            let Ok(parsed) = raw.trim().parse::<f32>() else {
                                return;
                            };
                            let mut values = held();
                            if let Some(slot) = values.get_mut(axis) {
                                *slot = parsed;
                            }
                            name.with_value(|n| controller::sim_sensor(state, n.clone(), values));
                        };
                        view! {
                            <div class="px-3 py-1.5">
                                <div class="flex items-baseline justify-between gap-2">
                                    <span class="min-w-0 truncate font-mono text-caption text-label-2">
                                        {label}
                                    </span>
                                    <span class="shrink-0 text-caption text-label-3">{unit}</span>
                                </div>
                                {(0..count)
                                    .map(|axis| {
                                        let reading = move || {
                                            held().get(axis).copied().unwrap_or(0.0)
                                        };
                                        view! {
                                            <div class="flex items-center gap-2 pt-0.5">
                                                <input
                                                    type="range"
                                                    min=low.to_string()
                                                    max=high.to_string()
                                                    step="any"
                                                    disabled=!writable
                                                    prop:value=move || reading().to_string()
                                                    on:input=move |event| {
                                                        send(axis, event_target_value(&event))
                                                    }
                                                    class="min-w-0 flex-1 accent-rust disabled:opacity-40"
                                                />
                                                <span class="w-[5ch] shrink-0 text-right font-mono text-caption text-label-3">
                                                    {move || format!("{:.2}", reading())}
                                                </span>
                                            </div>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </div>
                        }
                    })
                    .collect::<Vec<_>>()}
            </div>
        }
            .into_any()
    }
}

#[component]
fn Tunables() -> impl IntoView {
    let state = AppState::expect();

    move || {
        let params = state.sim.params.get();
        // Writable only over a port rusty holds. A spawned `espflash monitor`
        // is a running session whose stdin the board never sees, so gating on
        // "something is running" would offer a slider that silently does
        // nothing — read as firmware ignoring the change.
        let running = state.sim.link_port.with(Option::is_some);
        view! {
            <div class="shrink-0">
                <div class="px-3 py-1.5 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                    "Tunables"
                </div>
                <Link />
                {params
                    .is_empty()
                    .then(|| {
                        view! {
                            <p class="px-3 text-caption leading-relaxed text-label-3">
                                "None announced. Firmware prints "
                                <span class="font-mono">"[rusty:param] name=value min..max"</span>
                                " and it appears here — no config file, so the panel cannot \
                                 drift from the binary that is running. Print it on a timer \
                                 rather than only at boot: this panel usually connects to a \
                                 board that has been flying for a while, and would otherwise \
                                 never hear the announcement."
                            </p>
                        }
                    })}
                {params
                    .into_iter()
                    .map(|param| {
                        // Bounds come from the firmware or not at all. A
                        // slider whose range the panel invented is how
                        // somebody sends a gain of 500 to a motor loop.
                        let (low, high) = (param.min, param.max);
                        let name = param.name.clone();
                        let value = param.value;
                        let send = move |raw: String| {
                            if let Ok(parsed) = raw.trim().parse::<f32>() {
                                controller::set_param(state, name.clone(), parsed);
                            }
                        };
                        let slide = send.clone();
                        view! {
                            <div class="px-3 py-1.5">
                                <div class="flex items-baseline justify-between gap-2">
                                    <span class="min-w-0 truncate font-mono text-caption text-label-2">
                                        {param.name.clone()}
                                    </span>
                                    <input
                                        type="number"
                                        step="any"
                                        disabled=!running
                                        prop:value=value.to_string()
                                        on:change=move |event| send(event_target_value(&event))
                                        class="w-20 shrink-0 rounded-[4px] border border-line bg-sunken px-1 py-0.5 text-right font-mono text-caption text-label disabled:opacity-40"
                                    />
                                </div>
                                {(low.is_some() && high.is_some())
                                    .then(|| {
                                        view! {
                                            <input
                                                type="range"
                                                min=low.unwrap_or(0.0)
                                                max=high.unwrap_or(1.0)
                                                step="any"
                                                disabled=!running
                                                prop:value=value.to_string()
                                                on:input=move |event| slide(event_target_value(&event))
                                                class="mt-1 w-full accent-rust disabled:opacity-40"
                                            />
                                        }
                                    })}
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug the real board found: a loop settled at 88.0 ± 0.5 was scaled
    /// to the full height and drew a converged controller as maximum-amplitude
    /// static. A ripple that small must occupy a small share of the plot.
    #[test]
    fn a_small_ripple_on_a_large_signal_is_drawn_small() {
        let settled: Vec<(u64, f32)> = (0..20)
            .map(|at| (at, if at % 2 == 0 { 87.5 } else { 88.5 }))
            .collect();
        let (low, high) = band(&settled);
        let share = 1.0 / (high - low);
        assert!(
            share < 0.3,
            "a 1.0 ripple on an 88.0 signal filled {:.0}% of the height",
            share * 100.0
        );
        // Still centred on the data, not shoved to an edge.
        assert!(low < 87.5 && high > 88.5);
    }

    /// A swing worth looking at keeps the whole plot. The floor is a floor,
    /// not a ceiling — clamping a real oscillation would hide the thing the
    /// panel exists to show.
    #[test]
    fn a_real_swing_still_uses_the_full_height() {
        let ringing: Vec<(u64, f32)> = (0..20)
            .map(|at| (at, if at % 2 == 0 { 20.0 } else { 100.0 }))
            .collect();
        let (low, high) = band(&ringing);
        assert_eq!((low, high), (20.0, 100.0));
    }

    /// A constant lands in the middle. On the floor it reads as "at minimum",
    /// which is a different claim from "not changing" — and a setpoint nobody
    /// has touched is the most common constant there is.
    #[test]
    fn a_constant_is_centred_rather_than_on_the_floor() {
        let flat: Vec<(u64, f32)> = (0..10).map(|at| (at, 90.0)).collect();
        let (low, high) = band(&flat);
        assert!(high > low, "a constant must still have a band to divide by");
        let middle = (low + high) / 2.0;
        assert!((middle - 90.0).abs() < 1e-3, "centred on {middle}, not 90");
    }

    /// A channel sitting at zero has no magnitude to take a fraction of, and
    /// `0.0 / 0.0` would put every point at NaN — which SVG renders as a
    /// silently missing curve rather than an error.
    #[test]
    fn a_constant_zero_channel_still_divides() {
        let zeros: Vec<(u64, f32)> = (0..10).map(|at| (at, 0.0)).collect();
        let (low, high) = band(&zeros);
        assert!(high > low);
        let y = (0.0 - low) / (high - low);
        assert!(y.is_finite(), "a zero channel produced {y}");
    }
}
