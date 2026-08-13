//! The parts Flash and Monitor share: pick a device, see the command, run it.
//!
//! Two panels rather than one because they are two moments in the loop — write
//! the image, then watch what it says — but one implementation, because the
//! difference between them is a single [`FlashAction`]. Written twice, the
//! device picker in one would gain a fix the other never got.
//!
//! The planned command is always shown before it runs. Embedded developers live
//! in a terminal; a tool that hides what it invokes is one they cannot check,
//! cannot paste into a bug report, and eventually route around.

use leptos::prelude::*;

use rusty_embed::{FlashAction, Transport};

use crate::{
    controller, format,
    state::AppState,
    view::components::{Button, ButtonKind, CommandLine, Dot, Empty, Pill, SectionLabel, Tone},
};

/// The whole working area for one action.
///
/// There is deliberately no explanatory paragraph at the top. Whichever tool
/// gets chosen and why is already stated next to the command it produces, where
/// it is about *this* board rather than about flashing in general — and a
/// standing lecture above the controls is read once and skipped forever after.
#[component]
pub fn Session(
    action: FlashAction,
    /// What the primary button says. The action alone does not name itself in
    /// the user's terms — `FlashAndMonitor` is not a verb anyone says.
    #[prop(into)]
    verb: String,
) -> impl IntoView {
    let state = AppState::expect();
    // The Output channel this session's lines belong to.
    let channel = match action {
        FlashAction::Monitor => "monitor",
        _ => "flash",
    };

    // Enumerate on open, and re-plan whenever the device or the binary changes.
    // A stale command line is worse than none: it is the one the user reads and
    // believes before pressing the button.
    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::scan_devices(state);
        }
        // Both are read so the effect re-runs when either changes.
        let _ = state.transport.get();
        let _ = state.current_firmware();
        controller::plan_session(state, action);
    });

    move || {
        if state.firmware.with(Vec::is_empty) {
            return view! {
                <Empty
                    title="Nothing built yet"
                    detail="There has to be a binary before there is anything to put on a board."
                >
                    <div class="mt-1">
                        <CommandLine command="cargo build --release" />
                    </div>
                </Empty>
            }
            .into_any();
        }

        view! {
            <div class="flex-1 overflow-y-auto">
                <Devices />
                <Plan verb=verb.clone() channel=channel />
            </div>
        }
        .into_any()
    }
}

/// Serial ports and probes, named against the board catalogue.
#[component]
pub fn Devices() -> impl IntoView {
    let state = AppState::expect();

    view! {
        <div class="flex items-center gap-3 px-4 pt-3">
            <span class="flex-1 text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                "Device"
            </span>
            <Button
                label="Re-scan"
                kind=ButtonKind::Quiet
                on_click=Callback::new(move |_| controller::scan_devices(state))
            />
        </div>

        {move || {
            let ports = state.ports.get();
            let probes = state.probes.get();

            if ports.is_empty() && probes.is_empty() {
                return view! {
                    <p class="px-4 py-3 text-callout leading-relaxed text-label-2">
                        "Nothing attached. On Windows a board with no driver enumerates as an \
                         unknown device rather than a COM port, so if the board is plugged in and \
                         not here, that is the thing to check first."
                    </p>
                }
                    .into_any();
            }

            let current = state.transport.get();

            view! {
                <div class="px-2 py-1.5">
                    {ports
                        .into_iter()
                        .map(|port| {
                            let chosen = matches!(
                                &current,
                                Some(Transport::Serial { port: p }) if p == &port.name
                            );
                            let name = port.name.clone();
                            // A port that matches a board in the catalogue is
                            // named as the board. "COM3 (CP210x)" tells you
                            // which bridge chip is fitted, not what is on it.
                            let described = if port.boards.is_empty() {
                                port.bridge.clone().unwrap_or_else(|| "serial port".to_string())
                            } else {
                                port.boards.join(" / ")
                            };
                            let tone = if port.likely_board { Tone::Patina } else { Tone::Neutral };

                            view! {
                                <DeviceRow
                                    chosen=chosen
                                    tone=tone
                                    name=port.name.clone()
                                    detail=described
                                    badge=(!port.likely_board).then(|| "probably not a board".to_string())
                                    on_pick=Callback::new(move |_| {
                                        state.transport.set(Some(Transport::Serial { port: name.clone() }))
                                    })
                                />
                            }
                        })
                        .collect_view()}

                    {probes
                        .into_iter()
                        .map(|probe| {
                            let chosen = matches!(
                                &current,
                                Some(Transport::Probe { identifier: Some(id) }) if id == &probe.identifier
                            );
                            let id = probe.identifier.clone();

                            view! {
                                <DeviceRow
                                    chosen=chosen
                                    tone=Tone::Rust
                                    name=probe.description.clone()
                                    detail=probe.identifier.clone()
                                    badge=Some("debug probe".to_string())
                                    on_pick=Callback::new(move |_| {
                                        state
                                            .transport
                                            .set(Some(Transport::Probe { identifier: Some(id.clone()) }))
                                    })
                                />
                            }
                        })
                        .collect_view()}
                </div>
            }
                .into_any()
        }}
    }
}

#[component]
fn DeviceRow(
    chosen: bool,
    tone: Tone,
    #[prop(into)] name: String,
    #[prop(into)] detail: String,
    badge: Option<String>,
    on_pick: Callback<()>,
) -> impl IntoView {
    let look = if chosen {
        "bg-selection text-rust"
    } else {
        "text-label-2 hover:bg-sunken hover:text-label"
    };

    view! {
        <button
            type="button"
            on:click=move |_| on_pick.run(())
            class=format!(
                "flex w-full items-center gap-2.5 rounded-[6px] px-2 py-1.5 text-left \
                 transition-colors {look}",
            )
        >
            <Dot tone=tone />
            <span class="font-mono text-footnote">{name}</span>
            <span class="min-w-0 flex-1 truncate text-callout text-label-3">{detail}</span>
            {badge.map(|b| view! { <Pill label=b /> })}
        </button>
    }
}

/// The command, and the button that runs it.
#[component]
fn Plan(#[prop(into)] verb: String, channel: &'static str) -> impl IntoView {
    let state = AppState::expect();

    view! {
        <SectionLabel label="Command" />
        {move || {
            let running = state.session_running.get();

            let Some(plan) = state.plan.get() else {
                // Say which half is missing. "Cannot flash" with no reason is
                // the failure mode this whole workbench exists to avoid.
                let reason = if state.transport.with(Option::is_none) {
                    "Choose a device above."
                } else {
                    "No build selected — the Memory panel lists what this project has built."
                };
                return view! {
                    <p class="px-4 pb-4 text-callout text-label-2">{reason}</p>
                }
                    .into_any();
            };

            let firmware = state.current_firmware();
            let rationale = plan.rationale.clone();
            let display = plan.display.clone();
            let verb = verb.clone();

            view! {
                <div class="px-4 pb-4">
                    <CommandLine command=display />
                    <p class="mt-2 max-w-[72ch] text-callout leading-relaxed text-label-2">
                        {rationale}
                    </p>

                    {firmware
                        .map(|f| {
                            let mismatch = !f.matches_configured_target;
                            view! {
                                <div class="mt-2 flex items-center gap-2">
                                    <span class="font-mono text-footnote text-label-3">
                                        {f.name}" "{f.profile}" · "{format::bytes(f.bytes)}
                                    </span>
                                    // The one warning worth interrupting for: an
                                    // image for another chip flashes cleanly and
                                    // then behaves like broken hardware.
                                    {mismatch
                                        .then(|| {
                                            view! {
                                                <Pill
                                                    label=format!("built for {}", f.target)
                                                    tone=Tone::Amber
                                                />
                                            }
                                        })}
                                </div>
                            }
                        })}

                    <div class="mt-3 flex items-center gap-2">
                        <Button
                            label=verb
                            kind=ButtonKind::Primary
                            disabled=Signal::derive(move || state.session_running.get())
                            on_click=Callback::new(move |_| {
                                if let Some(plan) = state.plan.get_untracked() {
                                    controller::run_session(state, plan, channel);
                                }
                            })
                        />
                        <Button
                            label="Stop"
                            disabled=Signal::derive(move || !state.session_running.get())
                            on_click=Callback::new(move |_| controller::stop_session(state))
                        />
                        {running
                            .then(|| {
                                view! {
                                    <span class="flex items-center gap-2 text-callout text-label-2">
                                        <Dot tone=Tone::Patina />
                                        "attached — output is in the dock below"
                                    </span>
                                }
                            })}
                    </div>
                </div>
            }
                .into_any()
        }}
    }
}
