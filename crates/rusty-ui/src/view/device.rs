//! The device Flash and Monitor go to, in the title bar beside them — the
//! board-and-port box of Arduino IDE 2, the destination picker of Xcode.
//!
//! Chosen once and kept for the session; with one board plugged in there is
//! nothing to choose, because Flash takes it (`controller::device_action`).
//! Opened by a verb that found several, it says which verb is waiting, and
//! picking a row does it. The command the device would get is at the foot,
//! copyable, before anything runs — the rule the old Devices tab kept, kept
//! here without the tab.

use leptos::prelude::*;

use rusty_embed::{FlashAction, SerialPort, Transport};
use rusty_i18n::t;

use crate::{
    controller,
    state::{AppState, DeviceAction},
    view::components::{CommandLine, Dot, Pill, Spinner, Tone},
    view::icon::{Icon, IconView},
};

/// The chips the boards on a port carry, by id — what is plausibly on the
/// other end of the wire. Empty when the port names no board, which is not
/// evidence of anything.
fn chips_on(state: AppState, port: &SerialPort) -> Vec<String> {
    state.project.boards.with_untracked(|boards| {
        let mut chips: Vec<String> = boards
            .iter()
            .filter(|board| port.boards.iter().any(|name| name == &board.name))
            .map(|board| board.chip.clone())
            .collect();
        chips.sort();
        chips.dedup();
        chips
    })
}

/// A chip's name as the catalogue writes it, or its id.
fn chip_name(state: AppState, id: &str) -> String {
    state.project.chips.with_untracked(|chips| {
        chips
            .iter()
            .find(|chip| chip.id == id)
            .map_or_else(|| id.to_string(), |chip| chip.name.clone())
    })
}

/// "Looks like an ESP32", when the port's boards cannot carry the project's
/// chip — said beside the row, before a flash fails on a chip magic
/// mismatch that names neither.
fn mismatch(state: AppState, port: &SerialPort) -> Option<String> {
    let project = state
        .project
        .detected
        .with_untracked(|p| p.as_ref().and_then(|p| p.chip.clone()))?;
    let chips = chips_on(state, port);
    if chips.is_empty() || chips.iter().any(|chip| chip == &project) {
        return None;
    }
    let seen = chips
        .iter()
        .map(|chip| chip_name(state, chip))
        .collect::<Vec<_>>()
        .join(" / ");
    Some(t!("device.looks-like", chip = seen))
}

/// The pill in the title bar, and the list under it.
#[component]
pub fn DevicePicker() -> impl IntoView {
    let state = AppState::expect();
    let open = state.device.picker;

    // What the chosen device would get, re-planned while the list is open
    // and whenever the device or the build under it changes. Never while
    // closed: nothing reads it then.
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let _ = state.device.transport.get();
        let _ = state.project.firmware.get();
        let action = match state.device.pending.get() {
            Some(DeviceAction::Monitor) => FlashAction::Monitor,
            Some(DeviceAction::FlashOnly) => FlashAction::Flash,
            _ => FlashAction::FlashAndMonitor,
        };
        controller::plan_session(state, action);
    });

    // The pill's words: the port and the board on it, or the probe.
    let label = move || match state.device.transport.get() {
        Some(Transport::Serial { port }) => {
            let board = state.device.ports.with(|ports| {
                ports
                    .iter()
                    .find(|p| p.name == port)
                    .and_then(|p| p.boards.first().cloned().or_else(|| p.bridge.clone()))
            });
            match board {
                Some(board) => format!("{port} · {board}"),
                None => port,
            }
        }
        Some(Transport::Probe { identifier }) => state.device.probes.with(|probes| {
            probes
                .iter()
                .find(|p| Some(&p.identifier) == identifier.as_ref())
                .map(|p| p.description.clone())
                .or(identifier)
                .unwrap_or_else(|| t!("device.probe"))
        }),
        None => t!("device.choose"),
    };
    // The chosen port's board cannot be the project's chip.
    let warned = move || match state.device.transport.get() {
        Some(Transport::Serial { port }) => state.device.ports.with(|ports| {
            ports
                .iter()
                .find(|p| p.name == port)
                .and_then(|p| mismatch(state, p))
        }),
        _ => None,
    };

    view! {
        <div class="relative">
            <button
                type="button"
                title=move || warned().unwrap_or_else(|| t!("device.picker-hint"))
                on:click=move |_| {
                    if open.get_untracked() {
                        controller::close_picker(state);
                    } else {
                        controller::scan_devices(state);
                        open.set(true);
                    }
                }
                class=move || {
                    let base = "flex h-7 max-w-[15rem] items-center gap-1.5 rounded-[6px] px-2 \
                                text-footnote transition-colors hover:bg-sunken";
                    let tone = if state.device.transport.with(Option::is_some) {
                        "text-label"
                    } else {
                        "text-label-3 hover:text-label"
                    };
                    format!("{base} {tone}")
                }
            >
                <IconView icon=Icon::Plug size=13 />
                {move || warned().map(|_| view! { <Dot tone=Tone::Amber /> })}
                <span class="min-w-0 truncate">{label}</span>
                <span class="text-label-4">"▾"</span>
            </button>
            {move || open.get().then(|| view! { <PickerList /> })}
        </div>
    }
}

#[component]
fn PickerList() -> impl IntoView {
    let state = AppState::expect();

    // What the list is for: the verb waiting on it, or just the choice.
    let heading = move || match state.device.pending.get() {
        Some(DeviceAction::Flash | DeviceAction::FlashOnly) => t!("device.pick-to-flash"),
        Some(DeviceAction::Monitor) => t!("device.pick-to-monitor"),
        None => t!("device.heading"),
    };

    let rows = move || {
        let chosen = state.device.transport.get();
        let mut ports = state.device.ports.get();
        // Boards first; a modem or a Bluetooth port after them, still there
        // for the board that enumerates as something unexpected.
        ports.sort_by_key(|port| !port.likely_board);
        let probes = state.device.probes.get();
        if ports.is_empty() && probes.is_empty() {
            return view! {
                <div class="px-3 py-3">
                    <p class="text-callout text-label-2">{t!("device.none-found")}</p>
                    <p class="mt-1 text-footnote leading-relaxed text-label-3">
                        {t!("device.none-found-hint")}
                    </p>
                </div>
            }
            .into_any();
        }
        let serial_rows = ports
            .into_iter()
            .map(|port| {
                let picked = matches!(
                    &chosen,
                    Some(Transport::Serial { port: p }) if p == &port.name
                );
                let detail = if port.boards.is_empty() {
                    port.bridge
                        .clone()
                        .unwrap_or_else(|| t!("device.serial-port"))
                } else {
                    port.boards.join(" / ")
                };
                let note = mismatch(state, &port);
                let badge = (!port.likely_board).then(|| t!("device.not-a-board"));
                let tone = if port.likely_board {
                    Tone::Patina
                } else {
                    Tone::Neutral
                };
                let transport = Transport::Serial {
                    port: port.name.clone(),
                };
                view! {
                    <Row
                        picked=picked
                        tone=tone
                        name=port.name
                        detail=detail
                        badge=badge
                        note=note
                        on_pick=Callback::new(move |_| {
                            controller::choose_device(state, transport.clone())
                        })
                    />
                }
            })
            .collect_view();
        let probe_rows = probes
            .into_iter()
            .map(|probe| {
                let picked = matches!(
                    &chosen,
                    Some(Transport::Probe { identifier: Some(id) }) if id == &probe.identifier
                );
                let transport = Transport::Probe {
                    identifier: Some(probe.identifier.clone()),
                };
                view! {
                    <Row
                        picked=picked
                        tone=Tone::Rust
                        name=probe.description
                        detail=probe.identifier
                        badge=Some(t!("device.debug-probe"))
                        note=None
                        on_pick=Callback::new(move |_| {
                            controller::choose_device(state, transport.clone())
                        })
                    />
                }
            })
            .collect_view();
        view! { <div class="max-h-[18rem] overflow-y-auto py-1">{serial_rows}{probe_rows}</div> }
            .into_any()
    };

    // The foot: what the chosen device would be sent.
    let foot = move || {
        state.device.transport.get()?;
        let built = state.project.firmware.with(|f| !f.is_empty());
        let plan = state.device.plan.get();
        let waiting_on_monitor = state.device.pending.get() == Some(DeviceAction::Monitor);
        Some(view! {
            <div class="border-t border-line px-3 py-2.5">
                <p class="mb-1.5 text-caption text-label-3">
                    {if !built && !waiting_on_monitor {
                        t!("device.will-build")
                    } else {
                        t!("device.will-run")
                    }}
                </p>
                {match plan {
                    Some(plan) => view! { <CommandLine command=plan.display /> }.into_any(),
                    None if !built && !waiting_on_monitor => {
                        view! { <CommandLine command="cargo build --release" /> }.into_any()
                    }
                    None => view! {
                        <span class="flex items-center gap-1.5 text-footnote text-label-3">
                            <Spinner size=11 />
                            {t!("device.planning")}
                        </span>
                    }
                        .into_any(),
                }}
            </div>
        })
    };

    view! {
        // The catcher: a click anywhere else closes the list, as every menu
        // here does — and drops the verb that was waiting on it.
        <div class="fixed inset-0 z-40" on:click=move |_| controller::close_picker(state) />
        <div class="absolute top-full right-0 z-50 mt-1 w-[24rem] overflow-hidden rounded-[8px] border border-line bg-raised shadow-lg">
            <div class="flex items-center gap-2 border-b border-line px-3 py-2">
                <span class="min-w-0 flex-1 truncate text-footnote font-medium text-label">
                    {heading}
                </span>
                <button
                    type="button"
                    title=t!("device.rescan")
                    on:click=move |_| controller::scan_devices(state)
                    class="grid size-6 place-items-center rounded-[5px] text-label-3 hover:bg-sunken hover:text-label"
                >
                    {move || {
                        if state.app.in_flight.get() > 0 {
                            view! { <Spinner size=12 /> }.into_any()
                        } else {
                            view! { <IconView icon=Icon::Refresh size=13 /> }.into_any()
                        }
                    }}
                </button>
            </div>
            {rows}
            {foot}
        </div>
    }
}

#[component]
fn Row(
    picked: bool,
    tone: Tone,
    #[prop(into)] name: String,
    #[prop(into)] detail: String,
    badge: Option<String>,
    note: Option<String>,
    on_pick: Callback<()>,
) -> impl IntoView {
    let look = if picked {
        "bg-selection"
    } else {
        "hover:bg-sunken"
    };
    view! {
        <button
            type="button"
            on:click=move |_| on_pick.run(())
            class=format!(
                "flex w-full items-start gap-2.5 px-3 py-1.5 text-left transition-colors {look}",
            )
        >
            <div class="mt-[7px]">
                <Dot tone=tone />
            </div>
            <div class="min-w-0 flex-1">
                <div class="flex items-center gap-2">
                    <span class="font-mono text-footnote text-label">{name}</span>
                    <span class="min-w-0 flex-1 truncate text-footnote text-label-3">{detail}</span>
                    {badge.map(|b| view! { <Pill label=b /> })}
                </div>
                {note.map(|note| view! { <p class="mt-0.5 text-caption text-amber">{note}</p> })}
            </div>
            <span class=if picked { "mt-0.5 text-rust" } else { "mt-0.5 invisible" }>
                <IconView icon=Icon::Check size=13 />
            </span>
        </button>
    }
}
