//! Running firmware with no hardware on the desk — and wiring the desk up.
//!
//! The page is a schematic editor in the Wokwi shape: a parts library on
//! the left, the sheet with the devkit on the right, the sheet's own
//! controls in its corner and a properties panel beside it. Parts are
//! KiCad symbols with real pins (`rusty_embed::Symbol`), wires join pins to
//! pins, and what lights, conducts or drives what is read off the wires by
//! `rusty_embed::nets` — the same rules the backend reads a button's GPIO
//! by. Saved into the project's `.rusty/sim.toml`, a file diffed and
//! reviewed like any other. `docs/schematic.md` is the design.
//!
//! At run time each lamp lights from the pin levels the firmware reports
//! or the emulator holds, and the caption says which: the stock QEMU
//! exposes no GPIO state, and the board says so rather than pretending.
//!
//! The same editor stands beside the code, narrower (`compact`): Wokwi's
//! shape, and the playground's. There the sheet is the whole pane, and the
//! parts library and the inspector float over it when they are wanted — the
//! library from the corner's `+`, the inspector while something is selected
//! — rather than taking two columns out of a pane that is only a column.

use std::collections::{BTreeSet, HashSet};

use leptos::{ev, prelude::*};

mod art;
mod edit;
mod geometry;
mod glow;
mod layout;
mod library;
mod readout;

use geometry::*;
use library::Library;
use rusty_embed::circuit;
use rusty_embed::nets::{self, Behaviour, Row, Warning, behaviour_of};
use rusty_embed::period::{self, Period};
use rusty_embed::{PinRef, Sheet, Symbol, Wire};

use rusty_i18n::t;

use crate::{
    controller,
    state::AppState,
    view::components::{ContextMenu, Empty, MenuItem, MenuSeparator},
    view::icon::{Icon, IconView},
};

/// How far the sheet zooms, as a scale factor. Fit-to-view and the wheel
/// both clamp to it; spelled once so the two cannot disagree about what
/// "as far as it goes" means.
const CANVAS_ZOOM_RANGE: (f64, f64) = (0.35, 2.5);

/// One control in the sheet's corner cluster.
const SHEET_BUTTON: &str = "grid size-7 place-items-center rounded-[6px] text-label-2 \
                            hover:bg-sunken hover:text-label disabled:pointer-events-none \
                            disabled:opacity-35";

/// How close, in sheet units, a pulled wire has to come to a pin to land
/// on it. Generous, and in sheet units so it does not shrink as the view
/// zooms out: a pin is a 7px dot and the hand has a wire to mind.
const REACH: f64 = 12.0;

#[component]
pub fn Simulate(
    /// Drawn in a pane beside the editor rather than as a panel of its own.
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let state = AppState::expect();

    Effect::new(move |first: Option<()>| {
        if first.is_none() {
            controller::load_sim_plan(state);
        }
    });

    move || {
        if !state.has_project() {
            return view! {
                <Empty
                    title=t!("simulate.no-project-title")
                    detail=t!("simulate.no-project-detail")
                />
            }
            .into_any();
        }
        let Some(plan) = state.sim.plan.get() else {
            return view! {
                <p class="px-5 py-4 text-callout text-label-3">{t!("simulate.planning")}</p>
            }
            .into_any();
        };

        if !plan.supported {
            let reason = plan.reason.unwrap_or_default();
            return view! {
                <div class="px-5 py-4">
                    <p class="max-w-[64ch] text-callout leading-relaxed text-label-2 select-text">
                        {reason}
                    </p>
                </div>
            }
            .into_any();
        }

        let mut missing = plan.missing.clone();
        // The gdb gates only Debug, so it joins the card without blocking Run.
        if let Some(tool) = plan.debug_tool.clone() {
            missing.push(tool);
        }
        let running = state.app.session_running;
        let limits = plan.limits.clone();
        let notes = plan.notes.clone();

        view! {
            <div class="flex min-h-0 flex-1 flex-col">
                // A stock QEMU, named where Run is: its GPIO write handler is
                // empty, so a pin read back is always 0, and a blinky that
                // prints `false` for ever is the emulator's doing. rusty's
                // build models the pins; the same download that installs a
                // missing emulator installs it over the copy that is there.
                {plan
                    .emulator
                    .clone()
                    .filter(|emulator| !emulator.gpio_model)
                    .map(|emulator| {
                        let name = emulator.name.clone();
                        view! {
                            <div class="flex flex-col gap-1.5 border-b border-line bg-amber-fill px-4 py-3">
                                <p class="max-w-[80ch] text-callout">{t!("simulate.stock-qemu")}</p>
                                <div class="flex items-center gap-2.5">
                                    <span class="min-w-0 truncate font-mono text-caption text-label-3 select-text">
                                        {emulator.path.clone()}
                                    </span>
                                    <button
                                        type="button"
                                        disabled=move || running.get()
                                        on:click=move |_| {
                                            controller::install_sim_tool(state, name.clone())
                                        }
                                        class="shrink-0 rounded-[6px] bg-rust px-2.5 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                    >
                                        {t!("simulate.upgrade-qemu")}
                                    </button>
                                </div>
                            </div>
                        }
                    })}
                // An early build of rusty's own: the pins, and none of the
                // converter or the buses. Firmware reading either waits for
                // ever inside its own `read`, which looks like its bug.
                {plan
                    .emulator
                    .clone()
                    .filter(|emulator| emulator.gpio_model && !emulator.peripherals)
                    .map(|emulator| {
                        let name = emulator.name.clone();
                        view! {
                            <div class="flex flex-col gap-1.5 border-b border-line bg-amber-fill px-4 py-3">
                                <p class="max-w-[80ch] text-callout">{t!("simulate.early-qemu")}</p>
                                <div class="flex items-center gap-2.5">
                                    <span class="min-w-0 truncate font-mono text-caption text-label-3 select-text">
                                        {emulator.path.clone()}
                                    </span>
                                    <button
                                        type="button"
                                        disabled=move || running.get()
                                        on:click=move |_| {
                                            controller::install_sim_tool(state, name.clone())
                                        }
                                        class="shrink-0 rounded-[6px] bg-rust px-2.5 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                    >
                                        {t!("simulate.upgrade-qemu")}
                                    </button>
                                </div>
                            </div>
                        }
                    })}
                // What the emulator cannot do on this chip, said before the
                // run, so a run that ends mid-boot or a read that never
                // returns is not blamed on the firmware.
                {(!limits.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex flex-col gap-1 border-b border-line bg-amber-fill px-4 py-3">
                                {limits
                                    .iter()
                                    .map(|limit| {
                                        view! {
                                            <p class="max-w-[80ch] text-callout">{limit_text(limit)}</p>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })}
                // What the plan wants read that is not a refusal: a board
                // file for another chip, a symbol library that would not
                // read, a migration from the first format.
                {(!notes.is_empty())
                    .then(|| {
                        view! {
                            <ul class="flex flex-col gap-0.5 border-b border-line px-4 py-2">
                                {notes
                                    .into_iter()
                                    .map(|note| {
                                        view! {
                                            <li class="text-caption leading-snug text-label-3 select-text">
                                                {note}
                                            </li>
                                        }
                                    })
                                    .collect_view()}
                            </ul>
                        }
                    })}
                {(!missing.is_empty())
                    .then(|| {
                        view! {
                            <div class="flex flex-col gap-2.5 border-b border-line bg-amber-fill px-4 py-3">
                                <p class="text-callout font-medium">
                                    {t!("simulate.tools-missing")}
                                </p>
                                {missing
                                    .iter()
                                    .map(|tool| {
                                        let name = tool.name.clone();
                                        let install_name = name.clone();
                                        let manual = tool.install.clone();
                                        let failed = {
                                            let name = name.clone();
                                            Signal::derive(move || {
                                                state
                                                    .sim.install_failed
                                                    .with(|f| f.contains(&name))
                                            })
                                        };
                                        view! {
                                            <div class="flex flex-col gap-1.5">
                                                <div class="flex items-center gap-2.5">
                                                    <span class="font-mono text-footnote">
                                                        {name.clone()}
                                                    </span>
                                                    <button
                                                        type="button"
                                                        disabled=move || running.get()
                                                        on:click=move |_| {
                                                            controller::install_sim_tool(
                                                                state,
                                                                install_name.clone(),
                                                            )
                                                        }
                                                        class="rounded-[6px] bg-rust px-2.5 py-0.5 text-footnote font-medium text-white hover:opacity-90 disabled:pointer-events-none disabled:opacity-40"
                                                    >
                                                        {t!("simulate.install")}
                                                    </button>
                                                </div>
                                                {move || {
                                                    let manual = manual.clone();
                                                    failed
                                                        .get()
                                                        .then(|| {
                                                            view! {
                                                                <div class="flex flex-col gap-1">
                                                                    <span class="text-footnote text-label-2">
                                                                        {t!("simulate.install-failed")}
                                                                    </span>
                                                                    <code class="rounded-[6px] bg-sunken px-2 py-1 font-mono text-footnote select-text">
                                                                        {manual}
                                                                    </code>
                                                                </div>
                                                            }
                                                        })
                                                }}
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })}

                <BoardEditor
                    // No board file yet: an empty sheet for the *project's*
                    // chip. No chip draws rails only, which is the honest
                    // picture of a project rusty could not identify.
                    board=plan.board.clone().unwrap_or_else(|| {
                        let chip = state
                            .project
                            .detected
                            .with_untracked(|p| p.as_ref().and_then(|p| p.chip.clone()))
                            .unwrap_or_default();
                        empty_sheet(&chip)
                    })
                    library=plan.library.clone()
                    sensors=plan.parts.clone()
                    compact=compact
                />
            </div>
        }
        .into_any()
    }
}

/// A finding of the rules, in the interface's language.
fn warning_text(warning: &Warning) -> String {
    match warning {
        Warning::LedWithoutResistor { part } => t!("simulate.warning-led-resistor", part = part),
        Warning::DanglingWire { from, to } => t!("simulate.warning-dangling", from = from, to = to),
        Warning::Short { pins } => t!("simulate.warning-short", pins = pins.join(", ")),
        Warning::Conflict { pins } => t!("simulate.warning-conflict", pins = pins.join(", ")),
        Warning::SwitchDrivesNothing { part } => t!("simulate.warning-switch", part = part),
        Warning::BusAddressUnreadable { part, value } => {
            t!("simulate.warning-bus-address", part = part, value = value)
        }
        Warning::BusRegistersUnreadable { part, value } => {
            t!("simulate.warning-bus-registers", part = part, value = value)
        }
        Warning::BusNotWired { part } => t!("simulate.warning-bus-wiring", part = part),
        Warning::SensorModelUnknown { part, value } => {
            t!("simulate.warning-sensor-model", part = part, value = value)
        }
        Warning::WireSelectUnreadable { part, value } => {
            t!("simulate.warning-wire-select", part = part, value = value)
        }
        Warning::WireNotWired { part } => t!("simulate.warning-wire-wiring", part = part),
        Warning::PinReachesNothing { part, pin } => {
            t!("simulate.warning-loose-pin", part = part, pin = pin)
        }
        Warning::OutputsFighting { pins } => {
            t!("simulate.warning-outputs", pins = pins.join(", "))
        }
    }
}

/// A register sensor's reading, named in the window's language.
fn reading_label(key: &str) -> String {
    match key {
        "ax" => t!("simulate.reading-ax"),
        "ay" => t!("simulate.reading-ay"),
        "az" => t!("simulate.reading-az"),
        "gx" => t!("simulate.reading-gx"),
        "gy" => t!("simulate.reading-gy"),
        "gz" => t!("simulate.reading-gz"),
        "temp" => t!("simulate.reading-temp"),
        "pressure" => t!("simulate.reading-pressure"),
        "humidity" => t!("simulate.reading-humidity"),
        other => other.to_string(),
    }
}

/// A reading as the sheet stores it and the slider shows it: no more
/// digits than a slider two hundred steps long can set.
fn reading_text(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// What the emulator cannot do on this chip, in the window's language: by
/// the limit's stable name, and in the backend's English for one this
/// frontend has no words for yet.
fn limit_text(limit: &rusty_embed::SimLimit) -> String {
    match limit.kind.as_str() {
        "esp32-outdated" => t!("simulate.limit-esp32-outdated"),
        "cpu-fpu-off" => t!("simulate.limit-cpu-fpu-off"),
        "s3-unproven" => t!("simulate.limit-s3-unproven"),
        _ => limit.text.clone(),
    }
}

/// A net's colour while the board runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tone {
    High,
    Low,
    /// Under PWM: high for part of every period, drawn as the pulses it is.
    Switching,
}

/// What a net is doing, in words: its level, or how much of every period it
/// is high while a PWM pin drives it.
fn level_word(level: Option<period::Level>) -> String {
    match level {
        Some(period::Level::High) => t!("simulate.net-high"),
        Some(period::Level::Low) => t!("simulate.net-low"),
        Some(period::Level::Switching(high)) => {
            t!("simulate.net-pwm", share = format!("{:.0}", high * 100.0))
        }
        None => t!("simulate.net-floating"),
    }
}

/// Why the sheet has no numbers on it, in the window's own language.
///
/// The same rule `warning_text` follows: the variant's *name* is the stable
/// half and the values travel beside it, so a refusal reads as a sentence
/// here and prints as English in the CLI. `Display` on these types is the
/// English one and is what the headless tools use; a panel calling it would
/// be the one place in the window that answers in the wrong language.
fn unsolved_text(why: &circuit::Unsolved) -> String {
    use circuit::{Unsolved, Unstated};
    use rusty_embed::solve::Trouble;
    match why {
        Unsolved::Unstated(Unstated::Resistance { part, value }) => {
            t!("simulate.unstated-resistance", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::Supply { part, value }) => {
            t!("simulate.unstated-supply", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::ForwardVoltage { part }) => {
            t!("simulate.unstated-vf", part = part)
        }
        Unsolved::Unstated(Unstated::Capacitance { part, value }) => {
            t!("simulate.unstated-capacitance", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::NoGround) => t!("simulate.unstated-ground"),
        Unsolved::Floating { pins } => t!(
            "simulate.unsolved-floating",
            pins = pins
                .iter()
                .map(PinRef::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Unsolved::Trouble(Trouble::Contradiction) => t!("simulate.unsolved-contradiction"),
        Unsolved::Trouble(Trouble::BadResistance { ohms }) => {
            t!("simulate.unsolved-resistance", ohms = ohms.to_string())
        }
        Unsolved::Trouble(Trouble::DidNotConverge { .. }) => t!("simulate.unsolved-converge"),
        // Neither can reach a panel: `operating_point` catches `Floating`
        // and turns it into the pins it is about, and a step length belongs
        // to a transient, which this memo never runs. They fall back to the
        // English the type itself writes rather than borrowing a key about
        // something else — no entry, no claim, which is the same rule the
        // backend's own text follows. Spelled out rather than left to a
        // catch-all so that a variant added later is a compile error here.
        Unsolved::Trouble(Trouble::Floating { .. } | Trouble::BadStep { .. }) => why.to_string(),
    }
}

/// The editor: library, sheet, corner controls, properties. Local state
/// until Save writes it into `.rusty/sim.toml` and the plan reloads.
#[component]
fn BoardEditor(
    board: Sheet,
    library: Vec<Symbol>,
    /// The parts a sensor's `model` prop may name — rusty's own and the
    /// project's, as the plan read them. A sheet naming one nobody declared
    /// gets no sliders, which is the tunables' rule: a range this panel
    /// invented is how somebody feeds 2000 °/s to a loop written for 250.
    sensors: Vec<rusty_embed::sensor::Spec>,
    /// Beside the editor: the library and the inspector float over the
    /// sheet when wanted instead of standing either side of it.
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let state = AppState::expect();
    let running = state.app.session_running;
    // While the firmware runs, the sheet is the board on the desk and not a
    // drawing: nothing moves, nothing is rewired, nothing is selected, and
    // no inspector opens over it. A switch is pressed, a knob turned, a
    // slider slid; a drag on the sheet moves the view; hovering says what a
    // part or a wire is at. Wokwi's rule, and the user's words for the
    // alternative — still an editor, wires that drag, a panel over the
    // board — were exactly this. From the moment Run is pressed, build
    // included, until the run ends.
    let live = Signal::derive(move || {
        running.get()
            && state.app.activity.with(|activity| {
                activity.as_ref().is_some_and(|a| {
                    matches!(
                        a.kind,
                        crate::activity::Kind::Simulate | crate::activity::Kind::Debug
                    )
                })
            })
    });
    // The part under the pointer while it runs, for the reading line.
    let hover_part = RwSignal::new(None::<usize>);
    // The floating library, beside the editor. Closed until asked for, and
    // closed again by the part it adds, as Wokwi's picker is.
    let library_open = RwSignal::new(false);
    let chip = board.chip.clone();
    // Copy handles to the chip's name, so the closures the parts' views
    // share can be `Copy` themselves — a `String` captured by move is what
    // stops a closure being used twice.
    // A `Copy` handle, so every closure that needs to look a sensor up can
    // hold one rather than a clone of the list.
    let sensors = StoredValue::new(sensors);
    let chip_id = StoredValue::new(chip.clone());
    let chip_label = StoredValue::new(board.chip.to_uppercase());
    // The board around the pins — module, buttons, connector — by family.
    let kit_look = kit_style(&board.chip);

    // The pin rows this part actually has. From the catalogue, so a chip
    // added tomorrow draws its own pins rather than the ESP32 devkit's —
    // which is what every board used to show, C3 boards included, labelled
    // with GPIO36/39/34/35 that the part does not have.
    let rows: Memo<Vec<Row>> = {
        let chip = chip.clone();
        Memo::new(move |_| {
            let gpio = state
                .project
                .chips
                .get()
                .into_iter()
                .find(|c| c.id == chip)
                .map(|c| c.gpio)
                .unwrap_or_default();
            nets::kit_rows(&chip, &gpio)
        })
    };

    let parts = RwSignal::new(parts_of(&board, &rows.get_untracked()));
    let wires = RwSignal::new(board.wires.clone());
    // The catalogue can arrive after the sheet: the devkit's symbol follows
    // its rows, or a wire to GPIO2 would have nowhere to land until the
    // panel was reopened.
    {
        let chip = chip.clone();
        Effect::new(move |_| {
            let symbol = kit_symbol(&chip, &rows.get());
            parts.update(|list| {
                if let Some(kit) = list.first_mut().filter(|p| p.is_kit()) {
                    kit.symbol = Some(symbol);
                }
            });
        });
    }
    // The screens the sheet declares, kept in step with it.
    //
    // `absorb` draws on a screen and never creates one: which controller is
    // behind the glass decides how the bytes read, and that is the sheet's
    // to say. Here is where it says it — an entry per display part that
    // names its panel, rebuilt when the choice changes and dropped when the
    // part or its address goes. A part merely moved keeps its picture,
    // which is why this compares before it writes.
    Effect::new(move |_| {
        let declared: Vec<(u8, rusty_embed::screen::Panel)> = parts
            .get()
            .iter()
            .filter(|part| part.symbol.as_ref().map(behaviour_of) == Some(Behaviour::Display))
            .filter_map(|part| {
                let panel = rusty_embed::screen::Panel::from_id(part.inst.props.get("panel")?)?;
                Some((display_address(part)?, panel))
            })
            .collect();
        // Asked before it is written: this runs on every change to the
        // parts, which during a drag is every frame, and `update` wakes
        // what reads the map whether or not anything changed. What is read
        // is a screen's pixels, so waking it costs a walk of eight thousand
        // of them per frame for a part nobody has touched.
        let changed = state.sim.screens.with_untracked(|screens| {
            screens.len() != declared.len()
                || declared.iter().any(|(address, panel)| {
                    screens.get(address).map(|screen| screen.panel()) != Some(*panel)
                })
        });
        if changed {
            state.sim.screens.update(|screens| {
                screens.retain(|address, screen| {
                    declared
                        .iter()
                        .any(|(at, panel)| at == address && *panel == screen.panel())
                });
                for (address, panel) in declared {
                    screens
                        .entry(address)
                        .or_insert_with(|| rusty_embed::screen::Screen::of(panel));
                }
            });
        }
    });

    // Symbols imported during this session join the plan's library at once,
    // so the part can be placed without waiting for a re-plan.
    let extra: RwSignal<Vec<Symbol>> = RwSignal::new(Vec::new());
    let symbols: Signal<Vec<Symbol>> = {
        let library = library.clone();
        Signal::derive(move || {
            let mut all = library.clone();
            for symbol in extra.get() {
                match all.iter_mut().find(|s| s.id() == symbol.id()) {
                    Some(slot) => *slot = symbol,
                    None => all.push(symbol),
                }
            }
            all
        })
    };
    let importing = RwSignal::new(false);

    let dirty = RwSignal::new(false);
    let selected = RwSignal::new(None::<usize>);
    let selected_wire = RwSignal::new(None::<usize>);
    // The wire under the pointer, for the brightening that says "this one".
    let hover_wire = RwSignal::new(None::<usize>);
    // (client x, client y, what was clicked)
    let menu = RwSignal::new(None::<(f64, f64, MenuTarget)>);
    // Alignment guides shown while a part is being dragged into line with
    // another one — the quiet confirmation every drawing tool gives.
    let guides = RwSignal::new((None::<f64>, None::<f64>));
    // The active grid step. Coarse grids place, fine grids nudge — and the
    // dial exists because no single step suits both.
    let grid = RwSignal::new(SNAP);
    let drag = RwSignal::new(None::<Drag>);
    // While pulling a wire: current cursor in world coords, and the pin the
    // cursor is within reach of — the dot that lights up to take it.
    let ghost = RwSignal::new(None::<(f64, f64)>);
    let hover_pin = RwSignal::new(None::<(usize, String)>);
    // A wire being drawn click by click — started by a click on a pin, a
    // corner per click on the sheet, finished by a click on a pin or a
    // wire. Beside `drag` rather than in it, because it outlives every
    // press: a pan in the middle of a wire must not end the wire.
    let drawing = RwSignal::new(None::<Drawing>);
    // Every part in the selection: the one under the ring plus whatever a
    // rubber band or a Shift-click added. `selected` stays the one the
    // inspector describes and the keys act on when the group is one part.
    let marked = RwSignal::new(Vec::<usize>::new());
    // Where each other marked part stood when a group drag began, so every
    // frame is start plus one displacement rather than an accumulation of
    // snapped deltas.
    let group_start = RwSignal::new(Vec::<GroupStart>::new());
    // The rubber band's moving corner while a box drag is in flight.
    let box_to = RwSignal::new(None::<(f64, f64)>);
    let view = RwSignal::new((0.0f64, 0.0f64, 1.0f64));
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();
    // The switches held down right now, by reference — an input to the
    // rules, so a lamp behind a pressed button lights on the sheet.
    let pressed: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());

    // The pins the author has answered for: "yes, this one reaches nothing,
    // on purpose". The rules read it, and it is what keeps the loose-pin
    // finding from being a thing people scroll past.
    let no_connect: RwSignal<Vec<PinRef>> = RwSignal::new(board.no_connect.clone());

    let history = RwSignal::new(Vec::<Snapshot>::new());
    let future = RwSignal::new(Vec::<Snapshot>::new());
    let checkpoint = move || {
        history.update(|h| {
            edit::remember(
                h,
                (
                    parts.get_untracked(),
                    wires.get_untracked(),
                    no_connect.get_untracked(),
                ),
            )
        });
        future.set(Vec::new());
    };
    let undo = move || {
        let Some((p, w, n)) = history.try_update(|h| h.pop()).flatten() else {
            return;
        };
        future.update(|f| {
            f.push((
                parts.get_untracked(),
                wires.get_untracked(),
                no_connect.get_untracked(),
            ))
        });
        parts.set(p);
        wires.set(w);
        no_connect.set(n);
        dirty.set(true);
    };
    let redo = move || {
        let Some((p, w, n)) = future.try_update(|f| f.pop()).flatten() else {
            return;
        };
        history.update(|h| {
            h.push((
                parts.get_untracked(),
                wires.get_untracked(),
                no_connect.get_untracked(),
            ))
        });
        parts.set(p);
        wires.set(w);
        no_connect.set(n);
        dirty.set(true);
    };

    // The sheet as the rules read it: parts, wires, and the symbols the
    // parts carry. Built untracked for a command, tracked for the reading.
    let sheet_with_symbols =
        |chip: &str, list: &[EditPart], wires: &[Wire], marks: &[PinRef]| -> Sheet {
            let mut sheet = sheet_of(chip, list, wires, marks);
            for part in list {
                if let Some(symbol) = &part.symbol
                    && !sheet.symbols.iter().any(|s| s.id() == symbol.id())
                {
                    sheet.symbols.push(symbol.clone());
                }
            }
            sheet
        };
    // "Yes, that pin reaches nothing, on purpose." The one answer the
    // loose-pin finding can be given, and the reason it is allowed to exist.
    let toggle_no_connect = move |part: usize, pin: usize| {
        let Some(named) = parts.with_untracked(|list| {
            let part = list.get(part)?;
            let symbol = part.symbol.as_ref()?;
            let found = symbol.pins.get(pin)?;
            Some(PinRef::new(&part.inst.reference, symbol.wire_key(found)))
        }) else {
            return;
        };
        checkpoint();
        no_connect.update(|marks| match marks.iter().position(|p| *p == named) {
            Some(at) => {
                marks.remove(at);
            }
            None => marks.push(named),
        });
        dirty.set(true);
    };

    let sheet_now = move || {
        sheet_with_symbols(
            &chip_id.get_value(),
            &parts.get_untracked(),
            &wires.get_untracked(),
            &no_connect.get_untracked(),
        )
    };
    // Everything the rules and the solver say about the sheet — which lamps
    // are lit, what level every pin sits at, what is wrong, and what every
    // part is at in volts — read over one period of whatever the firmware
    // drives with PWM (`rusty_embed::period`). With nothing on PWM that is
    // the sheet as it stands, read once. The same parts, the same wires, the
    // same held switches and the same levels the firmware has reported, so
    // on-and-off and the numbers are two readings of one drawing rather than
    // two drawings.
    //
    // Four memos rather than one, and the split is the cost. Which pins are
    // on PWM, and the order their duties fall in, decide what has to be read
    // (`period`); the duties themselves only decide how long each reading
    // lasts (`weights`). A breathing lamp moves its duty a hundred times a
    // second, and that has to be a hundred re-weightings, not a hundred
    // readings of the whole sheet.
    //
    // The solver's half is the operating point and not a transient: a sheet
    // nobody is running has no instant to be at, and where it settles is
    // what a probe on a schematic is asking. It is an `Err` far more often
    // than it is an answer, and that is the design rather than a
    // shortcoming: a lamp with no `vf` and a rail called `VCC` are ordinary
    // states of a sheet somebody is still drawing, and each names the
    // property that would answer it.
    let wired: Memo<BTreeSet<u8>> =
        Memo::new(move |_| wires.with(|all| period::wired_gpios(all, &rows.get())));
    let order: Memo<Vec<u8>> = Memo::new(move |_| {
        state
            .sim
            .pwm
            .with(|pwm| wired.with(|wired| period::ranking(pwm, wired)))
    });
    let period: Memo<Period> = {
        let chip = chip.clone();
        Memo::new(move |_| {
            let sheet = sheet_with_symbols(&chip, &parts.get(), &wires.get(), &no_connect.get());
            Period::read(
                &sheet,
                &rows.get(),
                &pressed.get(),
                &state.sim.gpio.get(),
                order.get(),
            )
        })
    };
    let weights: Memo<Vec<f64>> =
        Memo::new(move |_| state.sim.pwm.with(|pwm| period.with(|p| p.weights(pwm))));
    // The period, asked in the shapes the board's readers want.
    let lit_share = move |reference: &str, pin: &str| {
        period.with(|p| weights.with(|w| p.pin_lit(w, reference, pin)))
    };
    let level_of = move |pin: &PinRef| period.with(|p| weights.with(|w| p.level(w, pin)));
    let measured = move |reference: &str| {
        period.with(|p| weights.with(|w| p.reading(w, reference).ok().flatten()))
    };
    let volts_of =
        move |pin: &PinRef| period.with(|p| weights.with(|w| p.volts_at(w, pin).ok().flatten()));
    // What the rules found at any moment of the period — a memo of its own,
    // so the list redraws when a finding comes or goes and not with every
    // change of duty.
    let findings: Memo<Vec<Warning>> =
        Memo::new(move |_| period.with(|p| weights.with(|w| p.warnings(w))));
    // What each wire's net does, as its colour says it. A memo of its own
    // for the same reason: the colour changes when a net starts or stops
    // switching, and a breathing lamp must not redraw every wire on the
    // sheet a hundred times a second.
    let tones: Memo<Vec<Option<Tone>>> = Memo::new(move |_| {
        wires.with(|all| {
            all.iter()
                .map(|wire| {
                    level_of(&wire.from).map(|level| match level {
                        period::Level::High => Tone::High,
                        period::Level::Low => Tone::Low,
                        period::Level::Switching(_) => Tone::Switching,
                    })
                })
                .collect()
        })
    });
    // The GPIO a part's pin reaches through the wires — what a knob, a
    // source or a motor is *on*, in the firmware's terms.
    let gpio_for = move |reference: &str, pin: &str| -> Option<u8> {
        nets::gpio_of(&sheet_now(), &rows.get_untracked(), reference, pin)
    };
    // Where a pot's track runs between the rails, when the sheet says. The
    // knob then reads as ADC counts through `adc.read_oneshot()` and not
    // only as the text protocol's `P<pin>=`; `None` is a pot whose ends the
    // sheet has not committed to, which gets the text line alone as before.
    let pot_span_for = move |reference: &str| -> Option<nets::PotSpan> {
        nets::pot_span(&sheet_now(), &rows.get_untracked(), reference)
    };

    // A new part arrives unwired: connecting it is the user's move, made by
    // pulling a pin to another pin. KiCad's placement: picking a part arms
    // it to the cursor — a ghost follows the mouse, a click plants it there,
    // Escape puts it back.
    let placing = RwSignal::new(None::<Symbol>);
    let place_at = RwSignal::new(None::<(f64, f64)>);

    let drop_part = move |symbol: Symbol, x: f64, y: f64| {
        checkpoint();
        parts.update(|list| selected.set(Some(edit::add(list, &symbol, x, y))));
        marked.set(selected.get_untracked().into_iter().collect());
        dirty.set(true);
    };
    let add_part = move |symbol: Symbol| {
        if live.get_untracked() {
            return;
        }
        placing.set(Some(symbol));
        place_at.set(None);
    };
    let import = move |number: String| {
        importing.set(true);
        controller::import_symbol(
            state,
            number,
            Callback::new(move |symbol: Option<Symbol>| {
                importing.set(false);
                if let Some(symbol) = symbol {
                    extra.update(|list| list.push(symbol.clone()));
                    add_part(symbol);
                }
            }),
        );
    };

    let save = {
        let chip = chip.clone();
        Callback::new(move |_: ()| {
            let sheet = sheet_of(
                &chip,
                &parts.get_untracked(),
                &wires.get_untracked(),
                &no_connect.get_untracked(),
            );
            controller::save_sim_board(state, sheet, dirty);
        })
    };
    // Run writes this sheet first when it has changes the file does not, so
    // the emulator is wired as the screen shows.
    {
        let chip = chip.clone();
        let unsaved = Callback::new(move |_: ()| {
            if !dirty.try_get_untracked()? {
                return None;
            }
            Some(sheet_of(
                &chip,
                &parts.try_get_untracked()?,
                &wires.try_get_untracked()?,
                &no_connect.try_get_untracked()?,
            ))
        });
        let number = controller::offer_unsaved_sheet(state, unsaved);
        on_cleanup(move || controller::withdraw_unsaved_sheet(state, number));
    }

    let to_world = move |client_x: f64, client_y: f64| -> (f64, f64) {
        let Some(element) = canvas.get_untracked() else {
            return (client_x, client_y);
        };
        let rect = element.get_bounding_client_rect();
        let (tx, ty, k) = view.get_untracked();
        (
            (client_x - rect.left() - tx) / k,
            (client_y - rect.top() - ty) / k,
        )
    };

    let rotate_part = move |index: usize| {
        checkpoint();
        parts.update(|list| edit::rotate(list, index));
        dirty.set(true);
    };
    let mirror_part = move |index: usize| {
        checkpoint();
        parts.update(|list| edit::mirror(list, index));
        dirty.set(true);
    };
    let nudge = move |dx: f64, dy: f64| {
        let Some(index) = selected.get_untracked() else {
            return;
        };
        checkpoint();
        parts.update(|list| edit::nudge(list, index, dx, dy));
        dirty.set(true);
    };
    // Frame everything the sheet holds, the way every canvas tool's F does.
    // Everything on screen, no larger than `cap`, centred in whichever
    // direction has room to spare.
    //
    // Every read is a `try_`: the fit on open runs a frame after mount, and
    // by then this editor may already be gone — replaced when the plan
    // loads a second time — and reading a disposed handle panics, which in
    // wasm takes the whole window with it.
    let fit_within = move |cap: f64| {
        let Some(Some(element)) = canvas.try_get_untracked() else {
            return;
        };
        let Some((min, max)) = parts.try_with_untracked(|list| bounds(list)) else {
            return;
        };
        let rect = element.get_bounding_client_rect();
        let (w, h) = (max.0 - min.0 + 80.0, max.1 - min.1 + 80.0);
        if w <= 0.0 || h <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let k = (rect.width() / w)
            .min(rect.height() / h)
            .clamp(CANVAS_ZOOM_RANGE.0, cap.max(CANVAS_ZOOM_RANGE.0));
        let _ = view.try_set((
            (rect.width() - w * k) / 2.0 - (min.0 - 40.0) * k,
            (rect.height() - h * k) / 2.0 - (min.1 - 40.0) * k,
            k,
        ));
    };
    let fit_view = move || fit_within(CANVAS_ZOOM_RANGE.1);
    // Beside the editor the pane is a column of whatever width it was
    // dragged to, so the board opens fitted to it, no larger than life —
    // left at the origin, a devkit placed for the panel's width stood half
    // off the pane's edge.
    if compact {
        Effect::new(move |fitted: Option<bool>| {
            if fitted == Some(true) || canvas.get().is_none() {
                return fitted == Some(true);
            }
            request_animation_frame(move || fit_within(1.0));
            true
        });
    }

    let straighten_wire = move |index: usize| {
        checkpoint();
        let list = parts.get_untracked();
        wires.update(|all| edit::straighten(&list, all, index));
        dirty.set(true);
    };
    let remove_wire = move |index: usize| {
        checkpoint();
        wires.update(|list| edit::remove_wire(list, index));
        selected_wire.set(None);
        hover_wire.set(None);
        dirty.set(true);
    };
    let disconnect_pin = move |index: usize, number: String| {
        checkpoint();
        let list = parts.get_untracked();
        wires.update(|w| edit::disconnect_pin(&list, w, index, &number));
        selected_wire.set(None);
        dirty.set(true);
    };
    let disconnect_all = move |index: usize| {
        checkpoint();
        let list = parts.get_untracked();
        wires.update(|w| edit::disconnect_all(&list, w, index));
        selected_wire.set(None);
        dirty.set(true);
    };
    let remove_part = move |index: usize| {
        checkpoint();
        let removed = parts
            .try_update(|list| {
                wires
                    .try_update(|w| edit::remove(list, w, index))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !removed {
            return;
        }
        selected.set(None);
        selected_wire.set(None);
        // Every index above the removed part has shifted; the cheapest
        // correct answer is that nothing is marked any more.
        marked.set(Vec::new());
        dirty.set(true);
    };
    // Every marked part at once — Delete on a rubber-band selection.
    let remove_marked = move || {
        let group = marked.get_untracked();
        if group.is_empty() {
            return;
        }
        checkpoint();
        parts.update(|list| wires.update(|w| edit::remove_many(list, w, &group)));
        selected.set(None);
        selected_wire.set(None);
        marked.set(Vec::new());
        dirty.set(true);
    };
    let duplicate_part = move |index: usize| {
        checkpoint();
        parts.update(|list| {
            if let Some(copy) = edit::duplicate(list, index) {
                selected.set(Some(copy));
                marked.set(vec![copy]);
            }
        });
        dirty.set(true);
    };

    // Remove the selected wire, or the selected part — through the same
    // commands the menu uses, rather than a third copy of each.
    let delete_selection = move || {
        if let Some(index) = selected_wire.get_untracked() {
            remove_wire(index);
        } else if marked.with_untracked(|m| m.len() > 1) {
            remove_marked();
        } else if let Some(index) = selected.get_untracked() {
            remove_part(index);
        }
    };

    // A wire being drawn, abandoned: Escape, a right-click, or a click back
    // on the pin it started from.
    let cancel_drawing = move || {
        drawing.set(None);
        ghost.set(None);
        hover_pin.set(None);
    };

    // One click while a wire is being drawn, at `world` on the sheet. A pin
    // in reach finishes it there — its own start pin takes it back — a wire
    // in reach finishes it as a branch, and anywhere else is a corner. The
    // preview under the pointer is drawn from the same `Drawing`, so what
    // was on screen before the click is what the click makes.
    let drawing_click = move |world: (f64, f64)| {
        let Some(mut draft) = drawing.get_untracked() else {
            return;
        };
        let list = parts.get_untracked();
        let start = list.get(draft.from.0).and_then(|part| {
            let pin = part.pin(&draft.from.1)?;
            Some((pin_point(part, pin), pin_out(part, pin)))
        });
        let Some((pin, out)) = start else {
            // The part it started from is gone — undone under it.
            cancel_drawing();
            return;
        };
        let from = (draft.from.0, draft.from.1.as_str());
        let made = if let Some(hit) = pin_under(&list, world, REACH) {
            if hit == draft.from {
                cancel_drawing();
                return;
            }
            let to = (hit.0, hit.1.as_str());
            if draft.placed.is_empty() {
                // Pin to pin with nothing laid between: the same routed
                // wire a drag makes, which is what the preview showed.
                checkpoint();
                wires
                    .try_update(|all| edit::connect(&list, all, from, to))
                    .flatten()
            } else {
                let end = list.get(hit.0).and_then(|part| {
                    let pin = part.pin(&hit.1)?;
                    Some((pin_point(part, pin), pin_out(part, pin)))
                });
                end.and_then(|(end, end_out)| {
                    let route = draft.route_into(pin, out, end, end_out);
                    checkpoint();
                    wires
                        .try_update(|all| edit::connect_drawn(&list, all, from, to, route))
                        .flatten()
                })
            }
        } else {
            let step = grid.get_untracked();
            let at = (snap_to(world.0, step), snap_to(world.1, step));
            match wires.with_untracked(|all| wire_under(&list, all, at, REACH)) {
                Some(trunk) => {
                    let route = draft.route_to(pin, out, at);
                    checkpoint();
                    wires
                        .try_update(|all| edit::branch_drawn(&list, all, from, trunk, route))
                        .flatten()
                }
                None => {
                    draft.place(pin, out, at);
                    drawing.set(Some(draft));
                    return;
                }
            }
        };
        if made.is_some() {
            selected.set(None);
            selected_wire.set(made);
            dirty.set(true);
        }
        // Finished either way: a pair already joined is refused, and there is
        // nothing more that drawing could become.
        cancel_drawing();
    };

    // A switch pressed on the sheet: the rules see it as conducting, and
    // while a session runs the GPIO it reaches is driven to the level its
    // other side holds — through the same message the old buttons sent, so
    // firmware written for `B<pin>=1` hears it too.
    // Which key of which keypad the pointer is holding: one at a time,
    // because a pointer is one finger. `(part index, row, column)`.
    let key_down = RwSignal::new(None::<(usize, usize, usize)>);

    // A key of a matrix keypad, pressed or released. It **joins** its row to
    // its column rather than driving either — see `nets::keypad_tie` — so it
    // goes as a switch and the console hears nothing.
    let press_key = move |index: usize, row: usize, column: usize, down: bool| {
        if down {
            key_down.set(Some((index, row, column)));
        } else if key_down.get_untracked() != Some((index, row, column)) {
            return;
        } else {
            key_down.set(None);
        }
        if !running.get_untracked() {
            return;
        }
        let Some(reference) =
            parts.with_untracked(|l| l.get(index).map(|p| p.inst.reference.clone()))
        else {
            return;
        };
        if let Some((a, b)) =
            nets::keypad_tie(&sheet_now(), &rows.get_untracked(), &reference, row, column)
        {
            controller::sim_switch(state, a, b, down);
        }
    };

    let press = move |index: usize, down: bool| {
        let Some(reference) =
            parts.with_untracked(|l| l.get(index).map(|p| p.inst.reference.clone()))
        else {
            return;
        };
        pressed.update(|held| {
            if down {
                held.insert(reference.clone());
            } else {
                held.remove(&reference);
            }
        });
        if !running.get_untracked() {
            return;
        }
        // A key between two GPIOs joins them; a switch to a rail drives
        // one. The sheet says which, and the two travel differently: a tie
        // is the emulator's `sw a-b=`, a drive is the level every example's
        // text protocol already reads.
        let sheet = sheet_now();
        let rows = rows.get_untracked();
        if let Some((a, b)) = nets::switch_tie(&sheet, &rows, &reference) {
            controller::sim_switch(state, a, b, down);
        } else if let Some((gpio, _)) = nets::button_drives(&sheet, &rows, &reference) {
            controller::sim_press(state, gpio, down);
        }
    };

    // Going live puts down whatever was in hand: a selection, a part being
    // placed, a wire half drawn, a menu, the floating library.
    Effect::new(move |_| {
        if live.get() {
            selected.set(None);
            selected_wire.set(None);
            marked.set(Vec::new());
            drawing.set(None);
            placing.set(None);
            place_at.set(None);
            drag.set(None);
            ghost.set(None);
            hover_pin.set(None);
            menu.set(None);
            library_open.set(false);
        }
    });

    // Whether the inspector has anything to say — beside the editor it is
    // drawn only then.
    let inspecting = move || {
        selected.get().is_some() || selected_wire.get().is_some() || marked.with(|m| m.len() > 1)
    };
    // Beside the editor it floats on the side of the pane away from the
    // part it describes, so what is being edited is never under the panel
    // editing it — on the right, the ESP32's parts all were.
    let inspector_left = move || {
        let Some(x) = selected
            .get()
            .and_then(|index| parts.with(|list| list.get(index).map(|p| p.inst.x)))
        else {
            return false;
        };
        let (tx, _, k) = view.get();
        let width = canvas
            .get()
            .map_or(0.0, |element| element.get_bounding_client_rect().width());
        tx + x * k > width / 2.0
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="relative flex min-h-0 flex-1">
                <div class=move || {
                    if !compact {
                        if live.get() {
                            "pointer-events-none flex flex-none opacity-40"
                        } else {
                            "flex flex-none"
                        }
                    } else if library_open.get() {
                        // Below the corner's controls, as the inspector is:
                        // in a narrow pane the two meet, and the `+` that
                        // closes the library must not be under it.
                        "absolute top-12 bottom-2 left-2 z-30 flex overflow-hidden rounded-[8px] shadow-2xl ring-1 ring-line-strong"
                    } else {
                        "hidden"
                    }
                }>
                    <Library
                        symbols=symbols
                        on_add=Callback::new(move |symbol: Symbol| {
                            add_part(symbol);
                            library_open.set(false);
                        })
                        on_import=Callback::new(move |number: String| import(number))
                        importing=Signal::derive(move || importing.get())
                    />
                </div>

                <div
                    node_ref=canvas
                    tabindex="0"
                    on:keydown=move |event: ev::KeyboardEvent| {
                        // A running board takes no edits; F still fits.
                        if live.get_untracked() {
                            if matches!(event.key().as_str(), "f" | "F") && !event.ctrl_key() {
                                event.prevent_default();
                                fit_view();
                            }
                            return;
                        }
                        // While a wire is being drawn the keys are about the
                        // wire: Backspace takes back the last click (and the
                        // drawing itself once there is none), Space turns
                        // the live leg the other way round — KiCad's
                        // posture — and Escape abandons it.
                        if drawing.with_untracked(Option::is_some) {
                            match event.key().as_str() {
                                "Delete" | "Backspace" => {
                                    event.prevent_default();
                                    let undone = drawing
                                        .try_update(|d| d.as_mut().is_some_and(Drawing::unplace))
                                        .unwrap_or(false);
                                    if !undone {
                                        cancel_drawing();
                                    }
                                    return;
                                }
                                " " => {
                                    event.prevent_default();
                                    drawing.update(|d| {
                                        if let Some(d) = d.as_mut() {
                                            d.flip = !d.flip;
                                        }
                                    });
                                    return;
                                }
                                "Escape" => {
                                    event.prevent_default();
                                    cancel_drawing();
                                    return;
                                }
                                _ => {}
                            }
                        }
                        match event.key().as_str() {
                            "Delete" | "Backspace" => {
                                event.prevent_default();
                                delete_selection();
                            }
                            "Escape" => {
                                // A drag in flight is what Escape is most
                                // often reaching for.
                                drawing.set(None);
                                placing.set(None);
                                place_at.set(None);
                                drag.set(None);
                                ghost.set(None);
                                guides.set((None, None));
                                selected_wire.set(None);
                                selected.set(None);
                                hover_pin.set(None);
                                box_to.set(None);
                                marked.set(Vec::new());
                            }
                            "r" | "R" | " " if !event.ctrl_key() => {
                                if let Some(index) = selected.get_untracked() {
                                    event.prevent_default();
                                    rotate_part(index);
                                }
                            }
                            // KiCad's key for it, and the reason it is not
                            // just another rotation is in `edit::mirror`.
                            "x" | "X" if !event.ctrl_key() => {
                                if let Some(index) = selected.get_untracked() {
                                    event.prevent_default();
                                    mirror_part(index);
                                }
                            }
                            "f" | "F" if !event.ctrl_key() => {
                                event.prevent_default();
                                fit_view();
                            }
                            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
                                if selected.get_untracked().is_some() =>
                            {
                                event.prevent_default();
                                // Shift for the fine grid, as every drawing
                                // tool spells it.
                                let step = if event.shift_key() { 1.0 } else { SNAP };
                                let (dx, dy) = match event.key().as_str() {
                                    "ArrowLeft" => (-step, 0.0),
                                    "ArrowRight" => (step, 0.0),
                                    "ArrowUp" => (0.0, -step),
                                    _ => (0.0, step),
                                };
                                nudge(dx, dy);
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("a") => {
                                event.prevent_default();
                                marked.set((0..parts.with_untracked(Vec::len)).collect());
                                selected_wire.set(None);
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("d") => {
                                if let Some(index) = selected.get_untracked() {
                                    event.prevent_default();
                                    duplicate_part(index);
                                }
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("z") => {
                                event.prevent_default();
                                if event.shift_key() { redo() } else { undo() }
                            }
                            _ if event.ctrl_key()
                                && event.key().eq_ignore_ascii_case("y") => {
                                event.prevent_default();
                                redo();
                            }
                            _ => {}
                        }
                    }
                    on:wheel=move |event: ev::WheelEvent| {
                        event.prevent_default();
                        let Some(element) = canvas.get_untracked() else {
                            return;
                        };
                        let rect = element.get_bounding_client_rect();
                        let cx = f64::from(event.client_x()) - rect.left();
                        let cy = f64::from(event.client_y()) - rect.top();
                        view.update(|(tx, ty, k)| {
                            let factor = if event.delta_y() < 0.0 { 1.12 } else { 1.0 / 1.12 };
                            let next = (*k * factor).clamp(CANVAS_ZOOM_RANGE.0, CANVAS_ZOOM_RANGE.1);
                            let real = next / *k;
                            *tx = cx - (cx - *tx) * real;
                            *ty = cy - (cy - *ty) * real;
                            *k = next;
                        });
                    }
                    on:contextmenu=move |event: ev::MouseEvent| {
                        event.prevent_default();
                        menu.set(Some((
                            f64::from(event.client_x()),
                            f64::from(event.client_y()),
                            MenuTarget::Sheet,
                        )));
                    }
                    on:pointerdown=move |event: ev::PointerEvent| {
                        // Middle-drag pans from anywhere, including over a
                        // part — the gesture every schematic and map editor
                        // shares, and the reason nobody reaches for a
                        // scrollbar.
                        if event.button() == 1 {
                            event.prevent_default();
                            if let Some(element) = canvas.get_untracked() {
                                let _ = element.focus();
                            }
                            let (tx, ty, _) = view.get_untracked();
                            drag.set(Some(Drag::Pan {
                                start_tx: tx,
                                start_ty: ty,
                                px: f64::from(event.client_x()),
                                py: f64::from(event.client_y()),
                            }));
                            return;
                        }
                        if event.button() != 0 {
                            return;
                        }
                        // A press on the sheet puts the floating library
                        // away, as a press outside any popover does.
                        library_open.set(false);
                        // While it runs a drag on the sheet moves the view,
                        // as a map's does: there is nothing to select.
                        if live.get_untracked() {
                            event.prevent_default();
                            if let Some(element) = canvas.get_untracked() {
                                let _ = element.focus();
                            }
                            let (tx, ty, _) = view.get_untracked();
                            drag.set(Some(Drag::Pan {
                                start_tx: tx,
                                start_ty: ty,
                                px: f64::from(event.client_x()),
                                py: f64::from(event.client_y()),
                            }));
                            return;
                        }
                        // An armed part lands where the click says, snapped.
                        if let Some(symbol) = placing.get_untracked() {
                            event.prevent_default();
                            let step = grid.get_untracked();
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            drop_part(symbol, snap_to(world.0, step), snap_to(world.1, step));
                            placing.set(None);
                            place_at.set(None);
                            return;
                        }
                        // Anything a part or a wire handled has stopped the
                        // event already, so what arrives here is the sheet.
                        if let Some(element) = canvas.get_untracked() {
                            let _ = element.focus();
                            selected.set(None);
                            selected_wire.set(None);
                            // A plain drag on the sheet draws the rubber band,
                            // as in KiCad and every drawing tool; panning is
                            // the middle button, or Ctrl/Alt with the left —
                            // the modifier every map has taught.
                            if event.ctrl_key() || event.alt_key() {
                                let (tx, ty, _) = view.get_untracked();
                                drag.set(Some(Drag::Pan {
                                    start_tx: tx,
                                    start_ty: ty,
                                    px: f64::from(event.client_x()),
                                    py: f64::from(event.client_y()),
                                }));
                                return;
                            }
                            // Shift keeps what is marked and lets the band add
                            // to it; a plain press starts the selection over.
                            if !event.shift_key() {
                                marked.set(Vec::new());
                            }
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            box_to.set(None);
                            drag.set(Some(Drag::Box { start: world }));
                        }
                    }
                    on:pointermove=move |event: ev::PointerEvent| {
                        // While it runs the wires let the pointer through to
                        // the parts under them, so which wire it is near is
                        // asked of the geometry — for its highlight and the
                        // reading line.
                        if live.get_untracked() && drag.with_untracked(Option::is_none) {
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            let near = parts.with_untracked(|list| {
                                wires.with_untracked(|all| wire_under(list, all, world, 6.0))
                            });
                            if hover_wire.get_untracked() != near {
                                hover_wire.set(near);
                            }
                        }
                        if placing.with_untracked(Option::is_some) {
                            let step = grid.get_untracked();
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            place_at
                                .set(Some((snap_to(world.0, step), snap_to(world.1, step))));
                        }
                        // A wire being drawn follows the pointer, snapped as
                        // its corners will be, and lights the pin it would
                        // land on — unless the sheet itself is being panned
                        // under it, when the pointer is not aiming at all.
                        if let Some(from) = drawing.with_untracked(|d| d.as_ref().map(|d| d.from.clone()))
                            && !matches!(drag.get_untracked(), Some(Drag::Pan { .. }))
                        {
                            let step = grid.get_untracked();
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            ghost.set(Some((snap_to(world.0, step), snap_to(world.1, step))));
                            let hit = parts
                                .with_untracked(|list| pin_under(list, world, REACH))
                                .filter(|hit| *hit != from);
                            hover_pin.set(hit);
                        }
                        let Some(current) = drag.get_untracked() else {
                            return;
                        };
                        let world = to_world(
                            f64::from(event.client_x()),
                            f64::from(event.client_y()),
                        );
                        match current {
                            Drag::Pan { start_tx, start_ty, px, py } => {
                                view.update(|(tx, ty, _)| {
                                    *tx = start_tx + f64::from(event.client_x()) - px;
                                    *ty = start_ty + f64::from(event.client_y()) - py;
                                });
                            }
                            Drag::Part { index, dx, dy, from, legs } => {
                                let step = grid.get_untracked();
                                let mut x = snap_to(world.0 - dx, step);
                                let mut y = snap_to(world.1 - dy, step);
                                // Line up with what is already on the sheet.
                                // Alignment that only the grid enforces is
                                // alignment nobody can see.
                                let mut guide_x = None;
                                let mut guide_y = None;
                                let others: Vec<(f64, f64)> = parts
                                    .get_untracked()
                                    .iter()
                                    .enumerate()
                                    .filter(|(other, _)| *other != index)
                                    .map(|(_, part)| (part.inst.x, part.inst.y))
                                    .collect();
                                for (ox, oy) in others {
                                    if (ox - x).abs() <= SNAP {
                                        x = ox;
                                        guide_x = Some(ox);
                                    }
                                    if (oy - y).abs() <= SNAP {
                                        y = oy;
                                        guide_y = Some(oy);
                                    }
                                }
                                guides.set((guide_x, guide_y));
                                parts.update(|list| {
                                    if let Some(part) = list.get_mut(index) {
                                        part.inst.x = x;
                                        part.inst.y = y;
                                    }
                                    // The rest of the group follows by the
                                    // same displacement, each from where it
                                    // stood.
                                    if !group_start.with_untracked(Vec::is_empty) {
                                        let group = group_start.get_untracked();
                                        edit::translate(list, &group, x - from.0, y - from.1);
                                    }
                                });
                                // Bends belong to the sheet, exactly as in
                                // KiCad: moving a part stretches only the
                                // leg from its pin to the first bend, and
                                // every later bend stays put. "Stretches" is
                                // literal — the bend slides along the leg's
                                // own axis, so the leg changes length, not
                                // direction.
                                let list = parts.get_untracked();
                                wires.update(|all| {
                                    for (wire_index, from_axis, to_axis) in &legs {
                                        let Some(wire) = all.get_mut(*wire_index) else {
                                            continue;
                                        };
                                        let Some(ends) = wire_ends(&list, wire) else {
                                            continue;
                                        };
                                        if let Some(first) = wire.bends.first_mut() {
                                            follow_bend(ends[0].0, *from_axis, first);
                                        }
                                        if let Some(last) = wire.bends.last_mut() {
                                            follow_bend(ends[1].0, *to_axis, last);
                                        }
                                    }
                                });
                                dirty.set(true);
                            }
                            Drag::Wire { from, .. } => {
                                ghost.set(Some(world));
                                let hit = parts
                                    .with_untracked(|list| pin_under(list, world, REACH))
                                    .filter(|hit| *hit != from);
                                hover_pin.set(hit);
                            }
                            Drag::Box { .. } => {
                                box_to.set(Some(world));
                            }
                            Drag::Segment {
                                wire,
                                first,
                                second,
                                horizontal,
                                grab,
                                base,
                            } => {
                                let axis = if horizontal { world.1 } else { world.0 };
                                let step = grid.get_untracked();
                                let mut value = snap_to(base + axis - grab, step);
                                // The wire's own endpoints outrank the grid:
                                // within a step of either pin's coordinate,
                                // land exactly on it — that is the alignment
                                // the drag was reaching for, and the
                                // simplifier then merges the runs.
                                let anchors = wires.with_untracked(|all| {
                                    let w = all.get(wire)?;
                                    parts.with_untracked(|list| wire_ends(list, w))
                                });
                                if let Some(ends) = anchors {
                                    for end in ends {
                                        let anchor = if horizontal { end.0.1 } else { end.0.0 };
                                        if (value - anchor).abs() <= step.max(SNAP) {
                                            value = anchor;
                                        }
                                    }
                                }
                                wires.update(|all| {
                                    if let Some(w) = all.get_mut(wire) {
                                        for index in [first, second] {
                                            if let Some(point) = w.bends.get_mut(index) {
                                                if horizontal {
                                                    point.1 = value;
                                                } else {
                                                    point.0 = value;
                                                }
                                            }
                                        }
                                    }
                                });
                                dirty.set(true);
                            }
                        }
                    }
                    on:pointerup=move |_| {
                        guides.set((None, None));
                        // A finished segment drag tidies its route: aligned
                        // segments merge, zero-length jogs vanish — so the
                        // next grab moves one segment, not two shards.
                        if let Some(Drag::Segment { wire, .. }) = drag.get_untracked() {
                            let list = parts.get_untracked();
                            wires.update(|all| {
                                if let Some(w) = all.get_mut(wire)
                                    && let Some(ends) = wire_ends(&list, w)
                                {
                                    retidy(w, &ends);
                                }
                            });
                        }
                        // A finished part drag tidies every route it
                        // stretched: a bend that slid into line with the next
                        // one merges away instead of surviving as a
                        // zero-length grab target.
                        if let Some(Drag::Part { legs, .. }) = drag.get_untracked() {
                            let list = parts.get_untracked();
                            wires.update(|all| {
                                for (wire_index, _, _) in &legs {
                                    if let Some(w) = all.get_mut(*wire_index)
                                        && let Some(ends) = wire_ends(&list, w)
                                    {
                                        retidy(w, &ends);
                                    }
                                }
                                // And any route the part was dropped on top
                                // of goes round it. Only those: a wire whose
                                // path crosses nothing is the author's and
                                // is left alone, wherever its bends came
                                // from.
                                layout::reroute_broken(&list, all);
                            });
                            group_start.set(Vec::new());
                        }
                        // The band closes over everything it touched. A press
                        // that never moved — a click on the sheet — marks
                        // nothing, which is the deselect it always was.
                        if let Some(Drag::Box { start }) = drag.get_untracked() {
                            if let Some(to) = box_to.get_untracked()
                                && ((to.0 - start.0).abs() > 2.0 || (to.1 - start.1).abs() > 2.0)
                            {
                                let hits =
                                    parts.with_untracked(|list| parts_in_box(list, start, to));
                                marked.update(|m| {
                                    for hit in hits {
                                        if !m.contains(&hit) {
                                            m.push(hit);
                                        }
                                    }
                                });
                                selected.set(marked.with_untracked(|m| m.first().copied()));
                            }
                            box_to.set(None);
                        }
                        // A wire lands on the pin under the pointer, from
                        // whichever end it was pulled: one assignment for
                        // both directions, so the two gestures cannot
                        // disagree about what wiring means. And a press
                        // that lands nowhere is the start of a wire drawn
                        // click by click, not a wire dropped on the floor.
                        let mut started = None;
                        if let Some(Drag::Wire { from, press }) = drag.get_untracked() {
                            let list = parts.get_untracked();
                            let released = ghost.get_untracked();
                            let moved = released.is_some_and(|at| {
                                (at.0 - press.0).hypot(at.1 - press.1) > CLICK_SLOP
                            });
                            // A pin first, and the middle of a wire only
                            // when no pin is in reach: a branch is what the
                            // gesture means where there was nothing else to
                            // land on, never in place of the pin somebody
                            // was aiming at.
                            let made = if let Some(to) = hover_pin.get_untracked() {
                                checkpoint();
                                wires
                                    .try_update(|all| {
                                        edit::connect(&list, all, (from.0, &from.1), (to.0, &to.1))
                                    })
                                    .flatten()
                            } else if moved && let Some(at) = released {
                                let step = grid.get_untracked();
                                let at = (snap_to(at.0, step), snap_to(at.1, step));
                                let trunk = wires
                                    .with_untracked(|all| wire_under(&list, all, at, REACH));
                                match trunk {
                                    Some(trunk) => {
                                        checkpoint();
                                        wires
                                            .try_update(|all| {
                                                edit::branch(
                                                    &list,
                                                    all,
                                                    (from.0, &from.1),
                                                    trunk,
                                                    at,
                                                )
                                            })
                                            .flatten()
                                    }
                                    None => {
                                        // Pulled out and let go on bare
                                        // sheet: that is the first corner,
                                        // and the wire carries on from it.
                                        let mut draft = Drawing::new(from.clone());
                                        if let Some(part) = list.get(from.0)
                                            && let Some(pin) = part.pin(&from.1)
                                        {
                                            draft.place(pin_point(part, pin), pin_out(part, pin), at);
                                        }
                                        drawing.set(Some(draft));
                                        started = Some(at);
                                        None
                                    }
                                }
                            } else {
                                // A click on the pin: every schematic
                                // editor's gesture for starting a wire.
                                drawing.set(Some(Drawing::new(from.clone())));
                                started = Some(released.unwrap_or(press));
                                None
                            };
                            if made.is_some() {
                                selected.set(None);
                                selected_wire.set(made);
                                dirty.set(true);
                            }
                            hover_pin.set(None);
                        }
                        ghost.set(None);
                        hover_pin.set(None);
                        box_to.set(None);
                        drag.set(None);
                        // A drawing just begun shows at once, from where the
                        // pointer is, rather than waiting for it to move.
                        if let Some(at) = started {
                            let step = grid.get_untracked();
                            ghost.set(Some((snap_to(at.0, step), snap_to(at.1, step))));
                        }
                    }
                    on:pointerleave=move |_| {
                        guides.set((None, None));
                        ghost.set(None);
                        hover_pin.set(None);
                        hover_wire.set(None);
                        box_to.set(None);
                        group_start.set(Vec::new());
                        drag.set(None);
                    }
                    class="relative min-w-0 flex-1 overflow-hidden bg-[#101216] outline-none"
                >
                    // The sheet's own controls, in its corner as every
                    // schematic editor keeps them: Save once there is
                    // something to save, undo and redo, zoom, fit, the snap
                    // grid. Pointer events stop here, or a press on Zoom
                    // would also be a press on the sheet under it.
                    <div
                        class="absolute top-2 right-2 z-20 flex items-center gap-0.5 rounded-[8px] bg-raised p-0.5 ring-1 ring-line-strong"
                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                        on:contextmenu=move |event: ev::MouseEvent| {
                            event.prevent_default();
                            event.stop_propagation();
                        }
                    >
                        // Beside the editor the library is behind this, as
                        // Wokwi's parts are behind its `+`.
                        {compact
                            .then(|| {
                                view! {
                                    <button
                                        type="button"
                                        title=t!("simulate.add-part")
                                        disabled=move || live.get()
                                        on:click=move |_| library_open.update(|open| *open = !*open)
                                        class=move || {
                                            if library_open.get() {
                                                format!("{SHEET_BUTTON} bg-selection text-rust")
                                            } else {
                                                SHEET_BUTTON.to_string()
                                            }
                                        }
                                    >
                                        <IconView icon=Icon::Plus size=14 />
                                    </button>
                                    <span class="mx-0.5 h-4 w-px bg-line" />
                                }
                            })}
                        <button
                            type="button"
                            title=t!("simulate.save")
                            disabled=move || !dirty.get() || live.get()
                            on:click=move |_| save.run(())
                            class=SHEET_BUTTON
                        >
                            <IconView icon=Icon::Save size=14 />
                        </button>
                        // KiCad, both ways. Beside Save because that is what
                        // they are — the same sheet, written somewhere else.
                        // Not beside the editor, where the corner is a
                        // column wide; the panel has them.
                        <span class=move || {
                            if compact { "hidden" } else { "mx-0.5 h-4 w-px bg-line" }
                        } />
                        <button
                            type="button"
                            title=t!("simulate.schematic-import")
                            class:hidden=compact
                            disabled=move || live.get()
                            on:click=move |_| {
                                controller::import_schematic(
                                    state,
                                    Callback::new(move |brought: Sheet| {
                                        checkpoint();
                                        let rows = rows.get_untracked();
                                        // Laid out on arrival: another
                                        // editor's canvas is not this one,
                                        // and a diagram's own coordinates
                                        // land a dozen parts in one square
                                        // inch here — which reads as an
                                        // import that lost half of them.
                                        let mut brought_parts = parts_of(&brought, &rows);
                                        let mut brought_wires = brought.wires.clone();
                                        layout::arrange(&mut brought_parts, &mut brought_wires);
                                        parts.set(brought_parts);
                                        wires.set(brought_wires);
                                        no_connect.set(brought.no_connect.clone());
                                        marked.set(Vec::new());
                                        selected.set(None);
                                        selected_wire.set(None);
                                        dirty.set(true);
                                    }),
                                )
                            }
                            class=SHEET_BUTTON
                        >
                            "⭳"
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.kicad-export")
                            class:hidden=compact
                            on:click=move |_| {
                                let sheet = sheet_now();
                                controller::export_kicad(state, sheet);
                            }
                            class=SHEET_BUTTON
                        >
                            "⭱"
                        </button>
                        <span class="mx-0.5 h-4 w-px bg-line" />
                        <button
                            type="button"
                            title=t!("simulate.undo")
                            disabled=move || history.with(Vec::is_empty) || live.get()
                            on:click=move |_| undo()
                            class=SHEET_BUTTON
                        >
                            "↶"
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.redo")
                            disabled=move || future.with(Vec::is_empty) || live.get()
                            on:click=move |_| redo()
                            class=SHEET_BUTTON
                        >
                            "↷"
                        </button>
                        // Lay the whole sheet out again. One undo step, and
                        // only when asked: a board somebody arranged by hand
                        // is theirs, and a rule that tidied on its own would
                        // move their work out from under them.
                        <button
                            type="button"
                            title=t!("simulate.tidy")
                            class:hidden=compact
                            disabled=move || live.get()
                            on:click=move |_| {
                                checkpoint();
                                parts.update(|list| {
                                    wires.update(|w| layout::arrange(list, w));
                                });
                                dirty.set(true);
                            }
                            class=SHEET_BUTTON
                        >
                            "⌗"
                        </button>
                        // Only while something is running: a Pause on a
                        // sheet with no emulator behind it is a button that
                        // can only refuse.
                        {move || {
                            running
                                .get()
                                .then(|| {
                                    let paused = state.sim.paused;
                                    view! {
                                        <span class="mx-0.5 h-4 w-px bg-line" />
                                        <button
                                            type="button"
                                            title=move || {
                                                if paused.get() {
                                                    t!("simulate.resume")
                                                } else {
                                                    t!("simulate.pause")
                                                }
                                            }
                                            on:click=move |_| {
                                                controller::sim_pause(state, !paused.get_untracked())
                                            }
                                            class=SHEET_BUTTON
                                        >
                                            {move || if paused.get() { "▶" } else { "❚❚" }}
                                        </button>
                                    }
                                })
                        }}
                        <span class="mx-0.5 h-4 w-px bg-line" />
                        <button
                            type="button"
                            title=t!("simulate.zoom-out")
                            on:click=move |_| {
                                view.update(|(_, _, k)| *k = (*k / 1.2).max(CANVAS_ZOOM_RANGE.0))
                            }
                            class=SHEET_BUTTON
                        >
                            "−"
                        </button>
                        <span class="min-w-[5ch] text-center font-mono text-footnote text-label-3">
                            {move || format!("{:.0}%", view.get().2 * 100.0)}
                        </span>
                        <button
                            type="button"
                            title=t!("simulate.zoom-in")
                            on:click=move |_| {
                                view.update(|(_, _, k)| *k = (*k * 1.2).min(CANVAS_ZOOM_RANGE.1))
                            }
                            class=SHEET_BUTTON
                        >
                            "+"
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.fit")
                            on:click=move |_| fit_view()
                            class=SHEET_BUTTON
                        >
                            <IconView icon=Icon::Fit size=14 />
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.grid")
                            class:hidden=compact
                            on:click=move |_| {
                                grid.update(|g| {
                                    *g = match *g as i32 {
                                        1 => 4.0,
                                        4 => 8.0,
                                        8 => 16.0,
                                        _ => 1.0,
                                    }
                                })
                            }
                            class="flex h-7 items-center gap-1 rounded-[6px] px-1.5 font-mono text-caption text-label-2 hover:bg-sunken hover:text-label"
                        >
                            <IconView icon=Icon::Grid size=13 />
                            <span class="tnum leading-none">
                                {move || format!("{}", grid.get() as i32)}
                            </span>
                        </button>
                        // The pane's own two: the whole editor, and away.
                        {compact
                            .then(|| {
                                view! {
                                    <span class="mx-0.5 h-4 w-px bg-line" />
                                    <button
                                        type="button"
                                        title=t!("simulate.open-panel")
                                        on:click=move |_| state.layout.panel.set("simulate".to_string())
                                        class=SHEET_BUTTON
                                    >
                                        <IconView icon=Icon::External size=14 />
                                    </button>
                                    <button
                                        type="button"
                                        title=t!("simulate.hide-board")
                                        on:click=move |_| state.layout.board_beside.set(false)
                                        class=SHEET_BUTTON
                                    >
                                        <IconView icon=Icon::Close size=14 />
                                    </button>
                                }
                            })}
                    </div>
                    <div
                        class="absolute"
                        style=move || {
                            let (tx, ty, k) = view.get();
                            format!(
                                "transform: translate({tx}px, {ty}px) scale({k}); transform-origin: 0 0",
                            )
                        }
                    >
                        // Three layers, because a part must not be able to
                        // hide a wire: the grid sits under everything, the
                        // parts above it, the wires over everything. None
                        // takes the pointer except where a part, a pin or a
                        // wire's own grab handle says so.
                        <svg
                            class="pointer-events-none absolute"
                            style="left: -2000px; top: -2000px"
                            width="6000"
                            height="6000"
                        >
                            <defs>
                                <pattern
                                    id="sheet-grid"
                                    width="16"
                                    height="16"
                                    patternUnits="userSpaceOnUse"
                                >
                                    <circle cx="1" cy="1" r="1" fill="#23262c" />
                                </pattern>
                            </defs>
                            <rect
                                width="6000"
                                height="6000"
                                fill="url(#sheet-grid)"
                                style="pointer-events: none"
                            />
                        </svg>

                        // ── the parts ────────────────────────────────────
                        // One group per part, keyed by index, and every field
                        // a view reads comes through its own memo — so a drag
                        // frame touches one part's transform and nothing else.
                        <svg
                            class="absolute"
                            style="left: -2000px; top: -2000px; pointer-events: none"
                            width="6000"
                            height="6000"
                        >
                            <g transform="translate(2000, 2000)">
                                <For
                                    each=move || 0..parts.with(Vec::len)
                                    key=|index| *index
                                    children=move |index: usize| {
                                        let this = Memo::new(move |_| {
                                            parts.with(|list| list.get(index).cloned())
                                        });
                                        let reference = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().map(|p| p.inst.reference.clone()).unwrap_or_default())
                                        });
                                        let symbol = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().and_then(|p| p.symbol.clone()))
                                        });
                                        let place = Memo::new(move |_| {
                                            this.with(|p| {
                                                p.as_ref()
                                                    .map(|p| (p.inst.x, p.inst.y, p.inst.rot, p.inst.mirror))
                                                    .unwrap_or_default()
                                            })
                                        });
                                        let value = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().map(|p| p.inst.value.clone()).unwrap_or_default())
                                        });
                                        let is_kit = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().is_some_and(|p| p.is_kit()))
                                        });
                                        let behaviour = Memo::new(move |_| {
                                            symbol.with(|s| s.as_ref().map(behaviour_of))
                                        });
                                        let markup = Memo::new(move |_| {
                                            let value = value.get();
                                            symbol.with(|s| {
                                                s.as_ref().map(|s| art::markup(s, &value)).unwrap_or_default()
                                            })
                                        });
                                        // Where the drawing puts the leads,
                                        // the light and the screen.
                                        let plan = Memo::new(move |_| {
                                            let value = value.get();
                                            symbol.with(|s| s.as_ref().map(|s| art::layout(s, &value)))
                                        });
                                        let bbox = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().map(part_box).unwrap_or((0.0, 0.0, 0.0, 0.0)))
                                        });
                                        let is_selected =
                                            Signal::derive(move || selected.get() == Some(index));
                                        let is_marked =
                                            Signal::derive(move || marked.with(|m| m.contains(&index)));
                                        // How brightly its light is drawn,
                                        // 0 to 1: from the current the solver
                                        // says it carries over the period
                                        // when it can say, and otherwise from
                                        // how much of it the rules call the
                                        // part lit — as though lit meant
                                        // fully, which is how every lamp was
                                        // drawn before there were numbers.
                                        let glow = Memo::new(move |_| {
                                            let reference = reference.get();
                                            period.with(|p| {
                                                weights.with(|w| match p.reading(w, &reference) {
                                                    Ok(Some(read)) => glow::of_current(read.through),
                                                    _ => glow::of_share(p.lit(w, &reference)),
                                                })
                                            })
                                        });
                                        let is_pressed = Memo::new(move |_| {
                                            let reference = reference.get();
                                            pressed.with(|p| p.contains(&reference))
                                        });
                                        // `pins` in the drawing's own frame —
                                        // offsets from the part's anchor — so a
                                        // drag frame moves the group and not
                                        // every dot in it.
                                        let pin_dots = Memo::new(move |_| {
                                            this.with(|p| {
                                                let Some(p) = p.as_ref() else {
                                                    return Vec::new();
                                                };
                                                let Some(plan) = part_layout(p) else {
                                                    return Vec::new();
                                                };
                                                plan.spots
                                                    .iter()
                                                    .map(|spot| {
                                                        let ((x, y), _) = spot_on_sheet(p, spot);
                                                        // The pin's place in its symbol travels
                                                        // with the dot: a no-connect is about a
                                                        // pin, and the menu is `Copy`.
                                                        let at = p
                                                            .symbol
                                                            .as_ref()
                                                            .and_then(|s| {
                                                                s.pins
                                                                    .iter()
                                                                    .position(|pin| pin.number == spot.number)
                                                            })
                                                            .unwrap_or(0);
                                                        (spot.number.clone(), x - p.inst.x, y - p.inst.y, at)
                                                    })
                                                    .collect::<Vec<_>>()
                                            })
                                        });
                                        let wired_pins = Memo::new(move |_| {
                                            let list = parts.get();
                                            wires.with(|all| {
                                                pin_dots
                                                    .get()
                                                    .iter()
                                                    .filter(|(number, _, _, _)| {
                                                        !edit::wires_at(&list, all, index, number).is_empty()
                                                    })
                                                    .map(|(number, _, _, _)| number.clone())
                                                    .collect::<Vec<_>>()
                                            })
                                        });
                                        let labels = Memo::new(move |_| {
                                            this.with(|p| {
                                                let Some(p) = p.as_ref() else {
                                                    return Vec::new();
                                                };
                                                pin_labels(p)
                                                    .into_iter()
                                                    .map(|l| Label { x: l.x - p.inst.x, y: l.y - p.inst.y, ..l })
                                                    .collect::<Vec<_>>()
                                            })
                                        });

                                        let start_wire = move |event: ev::PointerEvent, number: String| {
                                            if event.button() != 0 {
                                                return;
                                            }
                                            // Left to the part and the sheet
                                            // under it while the board runs.
                                            if live.get_untracked() {
                                                return;
                                            }
                                            event.prevent_default();
                                            event.stop_propagation();
                                            selected.set(Some(index));
                                            selected_wire.set(None);
                                            if let Some(element) = canvas.get_untracked() {
                                                let _ = element.focus();
                                            }
                                            let press = to_world(
                                                f64::from(event.client_x()),
                                                f64::from(event.client_y()),
                                            );
                                            drag.set(Some(Drag::Wire { from: (index, number), press }));
                                        };

                                        let on_down = move |event: ev::PointerEvent| {
                                            if event.button() != 0 {
                                                return;
                                            }
                                            // While it runs a switch is pressed
                                            // and anything else is the sheet
                                            // under the pointer, which pans.
                                            if live.get_untracked() {
                                                if behaviour.get_untracked() == Some(Behaviour::Switch) {
                                                    event.prevent_default();
                                                    event.stop_propagation();
                                                    press(index, true);
                                                }
                                                return;
                                            }
                                            event.prevent_default();
                                            event.stop_propagation();
                                            if let Some(element) = canvas.get_untracked() {
                                                let _ = element.focus();
                                            }
                                            library_open.set(false);
                                            selected_wire.set(None);
                                            // Shift adds this part to the selection
                                            // or takes it out — KiCad's modifier. A
                                            // plain press selects it alone, unless it
                                            // is already one of several, in which
                                            // case the whole group is what moves.
                                            if event.shift_key() {
                                                marked.update(|m| {
                                                    match m.iter().position(|i| *i == index) {
                                                        Some(at) => {
                                                            m.remove(at);
                                                        }
                                                        None => m.push(index),
                                                    }
                                                });
                                                selected.set(Some(index));
                                                return;
                                            }
                                            selected.set(Some(index));
                                            if !marked.with_untracked(|m| m.contains(&index)) {
                                                marked.set(vec![index]);
                                            }
                                            // A switch is pressed, not dragged, while
                                            // the firmware runs — the thing the sheet
                                            // is for at that moment.
                                            if behaviour.get_untracked() == Some(Behaviour::Switch)
                                                && running.get_untracked()
                                            {
                                                press(index, true);
                                                return;
                                            }
                                            checkpoint();
                                            let world = to_world(
                                                f64::from(event.client_x()),
                                                f64::from(event.client_y()),
                                            );
                                            let (x, y, _, _) = place.get_untracked();
                                            let moving: Vec<usize> = marked.with_untracked(|m| {
                                                let mut all = m.clone();
                                                if !all.contains(&index) {
                                                    all.push(index);
                                                }
                                                all
                                            });
                                            // Judge each wire's legs now, once:
                                            // judged live they would flip as the
                                            // part crosses its own bend.
                                            let legs = parts.with_untracked(|list| {
                                                wires.with_untracked(|all| wire_legs(list, all, &moving))
                                            });
                                            group_start.set(parts.with_untracked(|list| {
                                                moving
                                                    .iter()
                                                    .filter(|i| **i != index)
                                                    .filter_map(|i| list.get(*i).map(|p| (*i, (p.inst.x, p.inst.y))))
                                                    .collect()
                                            }));
                                            drag.set(Some(Drag::Part {
                                                index,
                                                dx: world.0 - x,
                                                dy: world.1 - y,
                                                from: (x, y),
                                                legs,
                                            }));
                                        };
                                        let on_up = move |_| {
                                            if is_pressed.get_untracked() {
                                                press(index, false);
                                            }
                                        };
                                        let on_menu = move |event: ev::MouseEvent| {
                                            event.prevent_default();
                                            event.stop_propagation();
                                            if !live.get_untracked() {
                                                selected.set(Some(index));
                                                selected_wire.set(None);
                                                if !marked.with_untracked(|m| m.contains(&index)) {
                                                    marked.set(vec![index]);
                                                }
                                            }
                                            menu.set(Some((
                                                f64::from(event.client_x()),
                                                f64::from(event.client_y()),
                                                MenuTarget::Part(index),
                                            )));
                                        };

                                        // The body: the devkit's art, or the
                                        // symbol's graphics scaled from
                                        // millimetres and flipped upright.
                                        let body = move || {
                                            if is_kit.get() {
                                                let drawn = rows.get();
                                                let kit_h = kit_height(drawn.len());
                                                let per_side = drawn.len().div_ceil(2).max(1);
                                                let art = chip_label.with_value(|label| kit_art(kit_look, kit_h, label));
                                                let (_, _, rot, mirror) = place.get();
                                                let flip = if mirror { -1 } else { 1 };
                                                let transform = format!("rotate({rot}) scale({flip} 1)");
                                                let labels = drawn
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(row, spec)| {
                                                        // The row's name sits eight pixels in from
                                                        // its pin, inside the board edge, which is
                                                        // where a devkit's silkscreen puts it. The
                                                        // position is turned with the board and the
                                                        // text is not, the way `pin_labels` places
                                                        // every other part's: a name printed
                                                        // sideways is one nobody reads.
                                                        let (px, py) = row_offset(drawn.len(), row);
                                                        let inward = if row < per_side { 1.0 } else { -1.0 };
                                                        let (x, y) =
                                                            orient((px + inward * 8.0, py), rot, mirror);
                                                        let (ox, oy) = orient((inward, 0.0), rot, mirror);
                                                        // Which way "into the board" now points
                                                        // decides how the name hangs off its pin.
                                                        let (anchor, dy) = if ox.abs() > oy.abs() {
                                                            (if ox > 0.0 { "start" } else { "end" }, 3.0)
                                                        } else if oy > 0.0 {
                                                            ("middle", 8.0)
                                                        } else {
                                                            ("middle", -3.0)
                                                        };
                                                        let label = spec.label.clone();
                                                        view! {
                                                            <text
                                                                x=x
                                                                y=y + dy
                                                                text-anchor=anchor
                                                                font-family="ui-monospace"
                                                                font-size="7.5"
                                                                fill="#98a1ae"
                                                                style="pointer-events: none"
                                                            >
                                                                {label}
                                                            </text>
                                                        }
                                                    })
                                                    .collect_view();
                                                view! {
                                                    <g transform=transform inner_html=art></g>
                                                    {labels}
                                                }
                                                    .into_any()
                                            } else {
                                                let (_, _, rot, mirror) = place.get();
                                                let flip = if mirror { -1 } else { 1 };
                                                let transform = format!("rotate({rot}) scale({flip} 1)");
                                                view! {
                                                    <g transform=transform inner_html=move || markup.get()></g>
                                                }
                                                    .into_any()
                                            }
                                        };

                                        // What the rules say about the part,
                                        // drawn over its body: a glow on a lit
                                        // lamp, the mixed colour of an RGB
                                        // lens, the lit segments of a digit,
                                        // the sunk cap of a pressed switch.
                                        let face = move || {
                                            let Some(plan) = plan.get() else {
                                                return ().into_any();
                                            };
                                            let (_, _, rot, mirror) = place.get();
                                            // The drawing turns with the part;
                                            // what is written on it does not.
                                            let turned = move |point: (f64, f64)| orient(point, rot, mirror);
                                            match behaviour.get() {
                                                Some(Behaviour::Led | Behaviour::Rgb) => {
                                                    let Some((lx, ly, r)) = plan.lens else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) = turned((lx, ly));
                                                    let rgb = behaviour.get() == Some(Behaviour::Rgb);
                                                    // A dark lamp is its own
                                                    // colour dimmed, not grey:
                                                    // a red LED is red on the
                                                    // desk with the power off.
                                                    let (on, dark) = lamp_colors(&value.get());
                                                    let dark = if rgb { rgb_color(false, false, false) } else { dark };
                                                    // And the light over it, as
                                                    // a colour and how much of
                                                    // it: the two attributes a
                                                    // duty moves, so a breathing
                                                    // lamp redraws those and not
                                                    // the lamp.
                                                    let light = Memo::new(move |_| {
                                                        if rgb {
                                                            let reference = reference.get();
                                                            let share = |name: &str| lit_share(&reference, name);
                                                            glow::lens(share("R"), share("G"), share("B"))
                                                        } else {
                                                            (on.to_string(), glow.get())
                                                        }
                                                    });
                                                    view! {
                                                        <circle
                                                            cx=cx
                                                            cy=cy
                                                            r=r
                                                            fill=dark
                                                            fill-opacity="0.5"
                                                            stroke="#0b0e12"
                                                            stroke-opacity="0.55"
                                                            stroke-width="0.9"
                                                            style="pointer-events: none"
                                                        />
                                                        // The glow is cast by the
                                                        // light itself, so it fades
                                                        // with it: a drop shadow
                                                        // takes the element's own
                                                        // opacity.
                                                        <circle
                                                            cx=cx
                                                            cy=cy
                                                            r=r
                                                            fill=move || light.with(|(colour, _)| colour.clone())
                                                            fill-opacity=move || light.with(|(_, level)| format!("{:.3}", 0.95 * level))
                                                            style=move || {
                                                                light.with(|(colour, level)| {
                                                                    if *level > 0.0 {
                                                                        format!(
                                                                            "filter: drop-shadow(0 0 5px {colour}) drop-shadow(0 0 13px {colour}); pointer-events: none",
                                                                        )
                                                                    } else {
                                                                        "pointer-events: none".to_string()
                                                                    }
                                                                })
                                                            }
                                                        />
                                                        <ellipse
                                                            cx=cx - r * 0.3
                                                            cy=cy - r * 0.35
                                                            rx=r * 0.28
                                                            ry=r * 0.42
                                                            fill="#ffffff"
                                                            fill-opacity=move || light.with(|(_, level)| format!("{:.3}", 0.16 + 0.34 * level))
                                                            style="pointer-events: none"
                                                        />
                                                    }
                                                        .into_any()
                                                }
                                                Some(Behaviour::Seven) => {
                                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) =
                                                        turned((fx + fw / 2.0, fy + fh / 2.0));
                                                    // Each segment lit for its
                                                    // own share of the period,
                                                    // so a digit dimmed by PWM
                                                    // is a dim digit.
                                                    let seg = move |name: &'static str| {
                                                        move || {
                                                            let share = lit_share(&reference.get(), name);
                                                            glow::mix("#3a2323", "#ff5c5c", glow::of_share(share))
                                                        }
                                                    };
                                                    let transform =
                                                        format!("translate({} {})", cx - 13.0, cy - 21.0);
                                                    view! {
                                                        <g transform=transform style="pointer-events: none">
                                                            <rect x="6" y="2" width="14" height="4" rx="2" fill=seg("a") />
                                                            <rect x="19" y="5" width="4" height="13" rx="2" fill=seg("b") />
                                                            <rect x="19" y="23" width="4" height="13" rx="2" fill=seg("c") />
                                                            <rect x="6" y="36" width="14" height="4" rx="2" fill=seg("d") />
                                                            <rect x="3" y="23" width="4" height="13" rx="2" fill=seg("e") />
                                                            <rect x="3" y="5" width="4" height="13" rx="2" fill=seg("f") />
                                                            <rect x="6" y="19" width="14" height="4" rx="2" fill=seg("g") />
                                                        </g>
                                                    }
                                                        .into_any()
                                                }
                                                // A matrix keypad: sixteen
                                                // caps, each its own target,
                                                // because a key is what is
                                                // pressed and the part is
                                                // only what carries them.
                                                Some(Behaviour::Keypad) => {
                                                    let keys = art::keypad_keys(&plan);
                                                    let running_now = running.get();
                                                    keys.into_iter()
                                                        .enumerate()
                                                        .map(|(at, (label, kx, ky, half))| {
                                                            let (row, column) = (at / 4, at % 4);
                                                            let (cx, cy) = turned((kx, ky));
                                                            let held = key_down.get()
                                                                == Some((index, row, column));
                                                            // Pressed rather than
                                                            // dragged while the
                                                            // firmware runs, as a
                                                            // switch is; with
                                                            // nothing running the
                                                            // press falls through
                                                            // to the part, which
                                                            // is what moves it.
                                                            let down = move |event: ev::PointerEvent| {
                                                                if event.button() != 0 || !running_now {
                                                                    return;
                                                                }
                                                                event.prevent_default();
                                                                event.stop_propagation();
                                                                press_key(index, row, column, true);
                                                            };
                                                            let up = move |_: ev::PointerEvent| {
                                                                press_key(index, row, column, false);
                                                            };
                                                            let fill = if held { "#4b5361" } else { "transparent" };
                                                            let style = if running_now {
                                                                "cursor: pointer"
                                                            } else {
                                                                "pointer-events: none"
                                                            };
                                                            view! {
                                                                <rect
                                                                    x=cx - half
                                                                    y=cy - half
                                                                    width=half * 2.0
                                                                    height=half * 2.0
                                                                    rx="2"
                                                                    fill=fill
                                                                    style=style
                                                                    on:pointerdown=down
                                                                    on:pointerup=up
                                                                    on:pointerleave=up
                                                                >
                                                                    <title>{label.to_string()}</title>
                                                                </rect>
                                                            }
                                                        })
                                                        .collect_view()
                                                        .into_any()
                                                }
                                                // A chain of addressable LEDs,
                                                // lit by the bytes the wire
                                                // carried rather than by a
                                                // level on a pin: one
                                                // transmission sets all of
                                                // them, and the order on the
                                                // wire is the chain's own.
                                                Some(Behaviour::Strip) => {
                                                    let reference = reference.get();
                                                    let colours = gpio_for(&reference, "DIN")
                                                        .map(|gpio| {
                                                            state.sim.rmt.with(|rmt| {
                                                                rmt.get(&gpio)
                                                                    .map(|bytes| {
                                                                        rusty_embed::strip_colours(bytes)
                                                                    })
                                                                    .unwrap_or_default()
                                                            })
                                                        })
                                                        .unwrap_or_default();
                                                    let lenses = art::strip_lenses(&plan);
                                                    lenses
                                                        .into_iter()
                                                        .enumerate()
                                                        .map(|(index, (lx, ly, r))| {
                                                            // Dark is dark: an LED told
                                                            // to be black and one the
                                                            // firmware has not reached
                                                            // are the same on the desk.
                                                            let (red, green, blue) = colours
                                                                .get(index)
                                                                .copied()
                                                                .unwrap_or((0, 0, 0));
                                                            let lit = red as u16 + green as u16 + blue as u16 > 0;
                                                            let (cx, cy) = turned((lx, ly));
                                                            let fill = format!("rgb({red} {green} {blue})");
                                                            let style = if lit {
                                                                format!(
                                                                    "pointer-events: none; filter: drop-shadow(0 0 {:.1}px rgb({red} {green} {blue}))",
                                                                    r * 0.9,
                                                                )
                                                            } else {
                                                                "pointer-events: none".to_string()
                                                            };
                                                            view! {
                                                                <rect
                                                                    x=cx - r * 0.72
                                                                    y=cy - r * 0.72
                                                                    width=r * 1.44
                                                                    height=r * 1.44
                                                                    rx="1"
                                                                    fill=fill
                                                                    style=style
                                                                />
                                                            }
                                                        })
                                                        .collect_view()
                                                        .into_any()
                                                }
                                                // What the firmware prints, on
                                                // the screen it prints it to —
                                                // upright, whichever way the
                                                // module is turned. Or, where the
                                                // sheet says which controller is
                                                // behind the glass, what the
                                                // firmware's own display driver
                                                // drew: an SSD1306 has no state a
                                                // driver reads back, so the bus
                                                // carries the whole picture.
                                                Some(Behaviour::Display) => {
                                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) =
                                                        turned((fx + fw / 2.0, fy + fh / 2.0));
                                                    let drawn = Memo::new(move |_| {
                                                        let address = this
                                                            .with(|p| p.as_ref().and_then(display_address))?;
                                                        state.sim.screens.with(|screens| {
                                                            let screen = screens.get(&address)?;
                                                            Some((
                                                                pixel_path(screen),
                                                                (fw / screen.width() as f64)
                                                                    .min(fh / screen.height() as f64),
                                                                (screen.width() as f64, screen.height() as f64),
                                                                screen.is_on(),
                                                            ))
                                                        })
                                                    });
                                                    view! {
                                                        {move || {
                                                            let Some((path, scale, (w, h), on)) = drawn.get() else {
                                                                return view! {
                                                                    <text
                                                                        x=cx
                                                                        y=cy + 3.0
                                                                        text-anchor="middle"
                                                                        font-family="ui-monospace"
                                                                        font-size="8.5"
                                                                        fill="#3ddc84"
                                                                        style="pointer-events: none"
                                                                    >
                                                                        {move || {
                                                                            let text = state.sim.display.get();
                                                                            if text.is_empty() {
                                                                                "········".to_string()
                                                                            } else {
                                                                                text
                                                                            }
                                                                        }}
                                                                    </text>
                                                                }
                                                                    .into_any();
                                                            };
                                                            // In the screen's own pixels, scaled onto
                                                            // the glass: the path is the decoder's
                                                            // answer and nothing here does arithmetic
                                                            // on it. A panel the driver has not
                                                            // switched on is dimmed rather than
                                                            // blanked — dark glass and a firmware that
                                                            // drew nothing look the same on the desk
                                                            // and are different faults.
                                                            let transform = format!(
                                                                "translate({} {}) scale({scale})",
                                                                cx - w * scale / 2.0,
                                                                cy - h * scale / 2.0,
                                                            );
                                                            view! {
                                                                <g
                                                                    transform=transform
                                                                    opacity=if on { "1" } else { "0.15" }
                                                                    style="pointer-events: none"
                                                                >
                                                                    <path
                                                                        d=path
                                                                        stroke="#3ddc84"
                                                                        stroke-width="1"
                                                                        fill="none"
                                                                        shape-rendering="crispEdges"
                                                                    />
                                                                </g>
                                                            }
                                                                .into_any()
                                                        }}
                                                    }
                                                        .into_any()
                                                }
                                                // A sounder says it is
                                                // sounding: rings rather than
                                                // a noise nobody asked their
                                                // machine to make.
                                                Some(Behaviour::Buzzer) => {
                                                    let Some((lx, ly, r)) = plan.lens else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) = turned((lx, ly));
                                                    let on = glow.get() > 0.0;
                                                    view! {
                                                        <circle
                                                            cx=cx
                                                            cy=cy
                                                            r=r
                                                            fill="none"
                                                            stroke=if on { "#ffd75c" } else { "#4a515c" }
                                                            stroke-width=if on { "2.4" } else { "1" }
                                                            stroke-dasharray="3 3"
                                                            style="pointer-events: none"
                                                        >
                                                            {on
                                                                .then(|| {
                                                                    view! {
                                                                        <animate
                                                                            attributeName="r"
                                                                            values=format!("{};{}", r * 0.6, r)
                                                                            dur="0.5s"
                                                                            repeatCount="indefinite"
                                                                        />
                                                                    }
                                                                })}
                                                        </circle>
                                                    }
                                                        .into_any()
                                                }
                                                // The horn follows the duty on
                                                // the signal pin, which is what
                                                // a servo is told and all a
                                                // sheet can honestly show.
                                                Some(Behaviour::Servo) => {
                                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) =
                                                        turned((fx + fw / 2.0 + 18.0, fy + fh / 2.0));
                                                    let reference = reference.get();
                                                    let drive = gpio_for(&reference, "SIG").and_then(|gpio| {
                                                        state.sim.pwm.with(|pwm| pwm.get(&gpio).copied())
                                                    });
                                                    // Where the horn stands, from the
                                                    // width of the pulse when the
                                                    // emulator said how often — a servo
                                                    // answers to that and not to how
                                                    // hard. The ends are the part's,
                                                    // because 500..2500 and 1000..2000
                                                    // are forty degrees apart at each
                                                    // end and both are ordinary.
                                                    let (min_us, max_us) = this.with(|p| {
                                                        let props = p.as_ref().map(|p| &p.inst);
                                                        (
                                                            props.and_then(|i| i.prop("min")).unwrap_or(SERVO_MIN_US),
                                                            props.and_then(|i| i.prop("max")).unwrap_or(SERVO_MAX_US),
                                                        )
                                                    });
                                                    let angle = drive
                                                        .map(|d| f64::from(d.servo_angle(min_us, max_us)) - 90.0);
                                                    let transform = format!(
                                                        "translate({cx} {cy}) rotate({:.1})",
                                                        angle.unwrap_or(0.0)
                                                    );
                                                    view! {
                                                        <g transform=transform style="pointer-events: none">
                                                            <rect
                                                                x="-2"
                                                                y="-16"
                                                                width="4"
                                                                height="18"
                                                                rx="2"
                                                                fill=if angle.is_some() { "#e3e7ec" } else { "#5a626e" }
                                                            />
                                                            <circle cx="0" cy="0" r="3" fill="#9aa2ae" />
                                                        </g>
                                                    }
                                                        .into_any()
                                                }
                                                // The cap sinks as well as
                                                // colouring: a tactile switch
                                                // moves, and the eye reads the
                                                // movement before the colour.
                                                Some(Behaviour::Switch) => {
                                                    let Some((lx, ly, r)) = plan.lens else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) = turned((lx, ly));
                                                    let down = is_pressed.get();
                                                    view! {
                                                        <circle
                                                            cx=cx
                                                            cy=cy
                                                            r=if down { r - 1.2 } else { r }
                                                            fill=if down { "#e05d38" } else { "#39404a" }
                                                            stroke="#5a626e"
                                                            stroke-width="1"
                                                            style="pointer-events: none"
                                                        />
                                                    }
                                                        .into_any()
                                                }
                                                _ => ().into_any(),
                                            }
                                        };

                                        // The gold dots wires pull out of; a
                                        // dot that nothing reaches pulses so
                                        // an unwired part reads as unfinished
                                        // rather than as broken.
                                        let dots = move || {
                                            pin_dots
                                                .get()
                                                .into_iter()
                                                .map(|(number, dx, dy, at)| {
                                                    // A `Copy` handle to the number, so the
                                                    // closures below can each be used more
                                                    // than once.
                                                    let number = StoredValue::new(number);
                                                    let wired = move || {
                                                        wired_pins.with(|w| number.with_value(|n| w.contains(n)))
                                                    };
                                                    let target = move || {
                                                        hover_pin.with(|h| {
                                                            h.as_ref().is_some_and(|(p, n)| {
                                                                *p == index && number.with_value(|mine| n == mine)
                                                            })
                                                        })
                                                    };
                                                    // Answered for: KiCad's cross, and no more
                                                    // pulsing — the pulse means "unfinished" and
                                                    // this one is finished.
                                                    let answered = move || {
                                                        parts.with(|list| {
                                                            let Some(part) = list.get(index) else {
                                                                return false;
                                                            };
                                                            let Some(symbol) = part.symbol.as_ref() else {
                                                                return false;
                                                            };
                                                            let Some(found) = number.with_value(|n| {
                                                                symbol.pins.iter().find(|p| p.number == *n).cloned()
                                                            }) else {
                                                                return false;
                                                            };
                                                            let named = PinRef::new(
                                                                &part.inst.reference,
                                                                symbol.wire_key(&found),
                                                            );
                                                            no_connect.with(|marks| marks.contains(&named))
                                                        })
                                                    };
                                                    view! {
                                                        <circle
                                                            cx=dx
                                                            cy=dy
                                                            r=move || if target() { 5.5 } else { 3.4 }
                                                            fill=move || if target() { "#ffd75c" } else if wired() { "#c9a227" } else { "#e0a838" }
                                                            class=move || if wired() || target() || answered() { "" } else { "animate-pulse" }
                                                            style=move || {
                                                                if live.get() {
                                                                    "pointer-events: all; cursor: inherit"
                                                                } else {
                                                                    "pointer-events: all; cursor: crosshair"
                                                                }
                                                            }
                                                            on:pointerdown=move |event: ev::PointerEvent| start_wire(event, number.get_value())
                                                            on:dblclick=move |event: ev::MouseEvent| {
                                                                if live.get_untracked() {
                                                                    return;
                                                                }
                                                                event.stop_propagation();
                                                                disconnect_pin(index, number.get_value());
                                                            }
                                                            on:contextmenu=move |event: ev::MouseEvent| {
                                                                event.prevent_default();
                                                                event.stop_propagation();
                                                                menu.set(Some((
                                                                    f64::from(event.client_x()),
                                                                    f64::from(event.client_y()),
                                                                    MenuTarget::Pin(index, at),
                                                                )));
                                                            }
                                                        >
                                                            <title>{t!("simulate.pin-hint")}</title>
                                                        </circle>
                                                        <Show when=answered>
                                                            <g
                                                                stroke="#d05a5a"
                                                                stroke-width="1.6"
                                                                stroke-linecap="round"
                                                                style="pointer-events: none"
                                                            >
                                                                <line x1=dx - 4.0 y1=dy - 4.0 x2=dx + 4.0 y2=dy + 4.0 />
                                                                <line x1=dx - 4.0 y1=dy + 4.0 x2=dx + 4.0 y2=dy - 4.0 />
                                                            </g>
                                                        </Show>
                                                    }
                                                })
                                                .collect_view()
                                        };

                                        // Pin names and numbers, upright
                                        // whatever the body does.
                                        let texts = move || {
                                            if is_kit.get() {
                                                return ().into_any();
                                            }
                                            labels
                                                .get()
                                                .into_iter()
                                                .map(|label| {
                                                    view! {
                                                        <text
                                                            x=label.x
                                                            y=label.y
                                                            text-anchor=label.anchor
                                                            font-family="ui-monospace"
                                                            font-size=label.size
                                                            fill="#98a1ae"
                                                            style="pointer-events: none"
                                                        >
                                                            {label.text}
                                                        </text>
                                                    }
                                                })
                                                .collect_view()
                                                .into_any()
                                        };

                                        // The reference above the body and the
                                        // value below, KiCad's placement; an
                                        // unknown symbol says so where its body
                                        // would be.
                                        let captions = move || {
                                            if is_kit.get() {
                                                return ().into_any();
                                            }
                                            let (x0, y0, x1, y1) = bbox.get();
                                            let (px, py, _, _) = place.get();
                                            let cx = (x0 + x1) / 2.0 - px;
                                            let top = y0 - py - 5.0;
                                            let bottom = y1 - py + 11.0;
                                            let unknown = symbol.with(Option::is_none);
                                            let missing = unknown.then(|| {
                                                let id = this.with(|p| p.as_ref().map(|p| p.inst.symbol.clone()).unwrap_or_default());
                                                let (w, h) = UNKNOWN_BOX;
                                                view! {
                                                    <rect x=-w / 2.0 y=-h / 2.0 width=w height=h rx="4" fill="#2c313a" stroke="#e05d38" stroke-dasharray="4 3" style="pointer-events: none" />
                                                    <text x="0" y="3" text-anchor="middle" font-family="ui-monospace" font-size="7" fill="#e05d38" style="pointer-events: none">{id}</text>
                                                }
                                            });
                                            // A rail and a label are their
                                            // own value, printed inside the
                                            // drawing; a second copy under it
                                            // reads as a mistake. Their
                                            // reference is `#PWR`, which
                                            // nobody needs to see either.
                                            let own = symbol
                                                .with(|s| s.as_ref().is_some_and(art::draws_own_value));
                                            view! {
                                                {missing}
                                                {(!own)
                                                    .then(|| {
                                                        view! {
                                                            <text x=cx y=top text-anchor="middle" font-family="ui-monospace" font-size="9" fill="#5fd0c8" style="pointer-events: none">
                                                                {move || reference.get()}
                                                            </text>
                                                            <text x=cx y=bottom text-anchor="middle" font-family="ui-monospace" font-size="8" fill="#d7dce3" style="pointer-events: none">
                                                                {move || value.get()}
                                                            </text>
                                                        }
                                                    })}
                                            }
                                                .into_any()
                                        };

                                        let ring = move || {
                                            let (x0, y0, x1, y1) = bbox.get();
                                            let (px, py, _, _) = place.get();
                                            let stroke = if is_selected.get() {
                                                "#e05d38"
                                            } else if is_marked.get() {
                                                "#e05d3899"
                                            } else {
                                                "transparent"
                                            };
                                            view! {
                                                <rect
                                                    x=x0 - px - 4.0
                                                    y=y0 - py - 4.0
                                                    width=x1 - x0 + 8.0
                                                    height=y1 - y0 + 8.0
                                                    rx="4"
                                                    fill="transparent"
                                                    stroke=stroke
                                                    stroke-width="1.2"
                                                    stroke-dasharray="4 3"
                                                    style="pointer-events: all"
                                                />
                                            }
                                        };

                                        let cursor = move || {
                                            let grabbing = matches!(drag.get(), Some(Drag::Part { .. }))
                                                && (is_selected.get() || is_marked.get());
                                            if grabbing {
                                                "cursor: grabbing"
                                            } else if behaviour.get() == Some(Behaviour::Switch) && running.get() {
                                                "cursor: pointer"
                                            } else if live.get() {
                                                "cursor: default"
                                            } else {
                                                "cursor: grab"
                                            }
                                        };

                                        view! {
                                            <g
                                                transform=move || {
                                                    let (x, y, _, _) = place.get();
                                                    format!("translate({x} {y})")
                                                }
                                                style=cursor
                                                on:pointerdown=on_down
                                                on:pointerup=on_up
                                                on:pointerenter=move |_| hover_part.set(Some(index))
                                                on:pointerleave=move |event| {
                                                    on_up(event);
                                                    hover_part.set(None);
                                                }
                                                on:contextmenu=on_menu
                                            >
                                                {ring}
                                                {body}
                                                {face}
                                                {texts}
                                                {captions}
                                                {dots}
                                            </g>
                                        }
                                    }
                                />
                            </g>
                        </svg>

                        // The footprint a moving part left behind — KiCad's
                        // ghost. Drawn only once the part has actually moved,
                        // so a click that never becomes a drag flashes nothing.
                        {move || {
                            let Some(Drag::Part { index, from, .. }) = drag.get() else {
                                return None;
                            };
                            let part = parts.with(|list| list.get(index).cloned())?;
                            if (part.inst.x, part.inst.y) == from {
                                return None;
                            }
                            let (x0, y0, x1, y1) = part_box(&part);
                            let (dx, dy) = (from.0 - part.inst.x, from.1 - part.inst.y);
                            Some(view! {
                                <div
                                    class="pointer-events-none absolute rounded-[6px] border-2 border-dashed border-[#e05d38]/60"
                                    style=format!(
                                        "left: {}px; top: {}px; width: {}px; height: {}px",
                                        x0 + dx - 4.0,
                                        y0 + dy - 4.0,
                                        x1 - x0 + 8.0,
                                        y1 - y0 + 8.0,
                                    )
                                />
                            })
                        }}

                        // ── the interactive faces ─────────────────────────
                        // A knob, a source, a screen, a rotor: HTML, placed
                        // under the symbol's body. Each takes the pointer
                        // only where it must, so a press beside the slider
                        // is still a press on the sheet.
                        <For
                            each=move || 0..parts.with(Vec::len)
                            key=|index| *index
                            children=move |index: usize| {
                                let this = Memo::new(move |_| parts.with(|list| list.get(index).cloned()));
                                let behaviour = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().and_then(|p| p.symbol.as_ref()).map(behaviour_of))
                                });
                                let reference = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().map(|p| p.inst.reference.clone()).unwrap_or_default())
                                });
                                let anchor = Memo::new(move |_| {
                                    this.with(|p| {
                                        p.as_ref().map(|p| {
                                            let (x0, _, x1, y1) = part_box(p);
                                            ((x0 + x1) / 2.0, y1 + 14.0)
                                        }).unwrap_or_default()
                                    })
                                });
                                let gpio_at = move |pin: &str| gpio_for(&reference.get_untracked(), pin);
                                move || {
                                    let (cx, cy) = anchor.get();
                                    let style = format!("left: {cx}px; top: {cy}px; transform: translateX(-50%)");
                                    match behaviour.get() {
                                        // The module the firmware asked to be
                                        // fed. Its value names the channel;
                                        // a channel the firmware never
                                        // declared gets no slider, for the
                                        // reason the tunables get none — a
                                        // range rusty invented is how
                                        // somebody injects 2000 deg/s into a
                                        // loop written for 250.
                                        // A sensor rusty answers for register by
                                        // register: a slider per reading, in the
                                        // units a person reads. Between runs a
                                        // slider says where the next run starts,
                                        // which is the sheet's; during one it
                                        // moves the reading the firmware gets.
                                        Some(Behaviour::Sensor)
                                            if this.with(|p| {
                                                p.as_ref()
                                                    .and_then(|p| p.inst.props.get("model"))
                                                    .is_some_and(|id| {
                                                        sensors.with_value(|all| {
                                                            rusty_embed::sensor::Spec::find(all, id).is_some()
                                                        })
                                                    })
                                            }) =>
                                        {
                                            let (model, props) = this.with(|p| {
                                                let inst = &p.as_ref()?.inst;
                                                let model = sensors.with_value(|all| {
                                                    rusty_embed::sensor::Spec::find(all, inst.props.get("model")?).cloned()
                                                })?;
                                                Some((model, inst.props.clone()))
                                            })?;
                                            let part_ref = reference.get_untracked();
                                            Some(view! {
                                                <div class="pointer-events-none absolute flex flex-col gap-0.5" style=style>
                                                    <span class="font-mono text-caption text-label-3">{model.name.clone()}</span>
                                                    {model
                                                        .channels
                                                        .iter()
                                                        .map(|channel| {
                                                            let key = channel.key.clone();
                                                            let (min, max, unit) = (channel.min, channel.max, channel.unit.clone());
                                                            let start = props
                                                                .get(&key)
                                                                .and_then(|text| text.trim().parse::<f64>().ok())
                                                                .unwrap_or(channel.rest);
                                                            let shown = {
                                                                let part_ref = part_ref.clone();
                                                                let key = key.clone();
                                                                Memo::new(move |_| {
                                                                    state.sim.readings.with(|held| {
                                                                        held.get(&(part_ref.clone(), key.clone())).copied()
                                                                    })
                                                                    .unwrap_or(start)
                                                                })
                                                            };
                                                            let moved = part_ref.clone();
                                                            let typed = key.clone();
                                                            let written = key.clone();
                                                            view! {
                                                                <span class="flex items-center gap-1.5">
                                                                    <span class="w-[11ch] truncate text-caption text-label-3">
                                                                        {reading_label(&key)}
                                                                    </span>
                                                                    // Any value, not a grid: a stepped slider shows
                                                                    // 0.5 g as 0.52 and moves the reading the moment
                                                                    // it is touched.
                                                                    <input
                                                                        type="range"
                                                                        min=min
                                                                        max=max
                                                                        step="any"
                                                                        prop:value=move || shown.get().to_string()
                                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                                            event.stop_propagation()
                                                                        }
                                                                        on:input=move |event: ev::Event| {
                                                                            if let Ok(value) = event_target_value(&event).parse::<f64>() {
                                                                                controller::sim_reading(state, moved.clone(), typed.clone(), value);
                                                                            }
                                                                        }
                                                                        on:change=move |event: ev::Event| {
                                                                            if state.app.session_running.get_untracked() {
                                                                                return;
                                                                            }
                                                                            if let Ok(value) = event_target_value(&event).parse::<f64>() {
                                                                                checkpoint();
                                                                                parts.update(|list| {
                                                                                    edit::set_prop(list, index, &written, &reading_text(value))
                                                                                });
                                                                                dirty.set(true);
                                                                            }
                                                                        }
                                                                        class="pointer-events-auto w-[70px] accent-[#5fd0c8]"
                                                                    />
                                                                    <span class="w-[10ch] text-right font-mono text-caption text-label-2">
                                                                        {move || format!("{} {unit}", reading_text(shown.get()))}
                                                                    </span>
                                                                </span>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </div>
                                            }
                                                .into_any())
                                        }
                                        Some(Behaviour::Sensor) => {
                                            let wanted = this
                                                .with(|p| p.as_ref().map(|p| p.inst.value.trim().to_string()))
                                                .unwrap_or_default();
                                            let declared = state
                                                .sim
                                                .sensors
                                                .with(|all| all.iter().find(|s| s.name == wanted).cloned());
                                            let Some(def) = declared else {
                                                return Some(view! {
                                                    <div class="pointer-events-none absolute max-w-[190px]" style=style>
                                                        <span class="rounded-[4px] bg-raised px-1.5 py-1 text-caption leading-snug text-label-4 ring-1 ring-line">
                                                            {if wanted.is_empty() {
                                                                t!("simulate.sensor-unnamed")
                                                            } else {
                                                                t!("simulate.sensor-undeclared", name = wanted.clone())
                                                            }}
                                                        </span>
                                                    </div>
                                                }
                                                    .into_any());
                                            };
                                            let count = def.components.max(1) as usize;
                                            let min = def.min.unwrap_or(-1.0);
                                            let max = def.max.unwrap_or(1.0);
                                            let unit = def.unit.clone().unwrap_or_default();
                                            let name = def.name.clone();
                                            let held = {
                                                let name = name.clone();
                                                move || {
                                                    state.sim.sensor_values.with(|all| {
                                                        all.get(&name).cloned().unwrap_or_else(|| vec![0.0; count])
                                                    })
                                                }
                                            };
                                            Some(view! {
                                                <div class="pointer-events-none absolute flex flex-col gap-0.5" style=style>
                                                    <span class="font-mono text-caption text-label-3">
                                                        {format!("{name} {unit}")}
                                                    </span>
                                                    {(0..count)
                                                        .map(|axis| {
                                                            let name = name.clone();
                                                            let held = held.clone();
                                                            // A `Copy` handle, so the slider and the number beside it
                                                            // can both read the sample.
                                                            let sample = StoredValue::new(held.clone());
                                                            let shown = move || {
                                                                sample.with_value(|held| held().get(axis).copied().unwrap_or(0.0))
                                                            };
                                                            view! {
                                                                <span class="flex items-center gap-1.5">
                                                                    <input
                                                                        type="range"
                                                                        min=min
                                                                        max=max
                                                                        step=(max - min) / 200.0
                                                                        prop:value=move || shown().to_string()
                                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                                            event.stop_propagation()
                                                                        }
                                                                        on:input=move |event: ev::Event| {
                                                                            let Ok(value) = event_target_value(&event).parse::<f32>()
                                                                            else {
                                                                                return;
                                                                            };
                                                                            let mut sample = held();
                                                                            sample.resize(count, 0.0);
                                                                            sample[axis] = value;
                                                                            controller::sim_sensor(state, name.clone(), sample);
                                                                        }
                                                                        class="pointer-events-auto w-[70px] accent-[#5fd0c8]"
                                                                    />
                                                                    <span class="w-[5ch] text-right font-mono text-caption text-label-2">
                                                                        {move || format!("{:.2}", shown())}
                                                                    </span>
                                                                </span>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </div>
                                            }
                                                .into_any())
                                        }
                                        Some(Behaviour::Pot) => {
                                            // Where the sheet says this knob starts, read the way
                                            // the analog source reads its own — the backend sends
                                            // the matching counts as soon as a run connects, so
                                            // the panel and the converter agree before the first
                                            // drag rather than after it.
                                            let start_turn = this.with(|p| {
                                                p.as_ref().map_or(rusty_embed::nets::POT_REST, |p| {
                                                    rusty_embed::nets::pot_start(&p.inst)
                                                })
                                            });
                                            let turned = RwSignal::new(start_turn);
                                            let span = pot_span_for(&reference.get_untracked());
                                            // The converter's full scale, named the same way the
                                            // analog source names it, so one part does not read
                                            // on a different scale from its neighbour.
                                            let max = this.with(|p| {
                                                p.as_ref().map_or(rusty_embed::nets::ADC_MAX, |p| {
                                                    rusty_embed::nets::adc_max(&p.inst)
                                                })
                                            });
                                            let angle = move || -135.0 + f64::from(turned.get()) / 255.0 * 270.0;
                                            Some(view! {
                                                <div class="pointer-events-none absolute flex items-center gap-1.5" style=style>
                                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full bg-[#3a404a] ring-1 ring-[#5a626e]">
                                                        <span
                                                            class="absolute top-[3px] left-1/2 h-[7px] w-[2px] rounded-full bg-[#c9a227]"
                                                            style=move || format!("transform-origin: 1px 7px; transform: translateX(-1px) rotate({:.0}deg)", angle())
                                                        />
                                                    </span>
                                                    <input
                                                        type="range"
                                                        min="0"
                                                        max="255"
                                                        value=start_turn
                                                        title=t!("simulate.pot-hint")
                                                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) = event_target_value(&event).parse::<u8>() {
                                                                turned.set(value);
                                                                // The text line, for firmware that
                                                                // reads rusty's protocol.
                                                                if let Some(gpio) = gpio_at("W") {
                                                                    controller::sim_pot(state, gpio, value);
                                                                }
                                                                // And the counts, for firmware that
                                                                // just calls read_oneshot() — but
                                                                // only where the sheet said what
                                                                // the track's ends are on.
                                                                if let Some(span) = span {
                                                                    controller::sim_analog(
                                                                        state,
                                                                        span.gpio,
                                                                        span.counts(value, max),
                                                                    );
                                                                }
                                                            }
                                                        }
                                                        class="pointer-events-auto w-[56px] accent-[#c9a227]"
                                                    />
                                                </div>
                                            }.into_any())
                                        }
                                        Some(Behaviour::Analog) => {
                                            // Counts, and the count is what is
                                            // shown. rusty does not know the
                                            // divider on this board, so it does
                                            // not print a voltage it cannot
                                            // stand behind.
                                            let max = this.with(|p| {
                                                p.as_ref().map_or(rusty_embed::nets::ADC_MAX, |p| {
                                                    rusty_embed::nets::adc_max(&p.inst)
                                                })
                                            });
                                            // Where the sheet says this source
                                            // starts, until somebody moves it.
                                            // The backend sends the same value
                                            // down the pin channel as soon as
                                            // a run connects, so the slider and
                                            // the converter agree before the
                                            // first drag rather than after it.
                                            let start = this.with(|p| {
                                                p.as_ref().map_or(0, |p| rusty_embed::nets::analog_start(&p.inst))
                                            });
                                            let held = move || {
                                                gpio_at("OUT")
                                                    .map(|gpio| state.sim.analog.with(|a| a.get(&gpio).copied().unwrap_or(start)))
                                                    .unwrap_or(start)
                                            };
                                            Some(view! {
                                                <div class="pointer-events-none absolute flex items-center gap-2" style=style>
                                                    <input
                                                        type="range"
                                                        min="0"
                                                        max=max
                                                        prop:value=move || held().to_string()
                                                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) = event_target_value(&event).parse::<u16>()
                                                                && let Some(gpio) = gpio_at("OUT")
                                                            {
                                                                controller::sim_analog(state, gpio, value);
                                                            }
                                                        }
                                                        class="pointer-events-auto w-[68px] accent-[#4aa8ff]"
                                                    />
                                                    // What the firmware's own
                                                    // converter last took off
                                                    // this pin, which is the
                                                    // difference between a
                                                    // slider that does nothing
                                                    // and firmware that is not
                                                    // reading. Only rusty's
                                                    // emulator can say, so no
                                                    // claim when it has not.
                                                    <span
                                                        class="w-[4ch] text-right font-mono text-caption text-label-2"
                                                        title=move || {
                                                            match gpio_at("OUT")
                                                                .and_then(|gpio| state.sim.adc.with(|a| a.get(&gpio).copied()))
                                                            {
                                                                Some(counts) => t!("simulate.adc-read", counts = counts),
                                                                None => t!("simulate.adc-unread"),
                                                            }
                                                        }
                                                    >
                                                        {held}
                                                    </span>
                                                </div>
                                            }.into_any())
                                        }
                                        Some(Behaviour::Motor) => {
                                            let duty = move || {
                                                gpio_at("PWM").and_then(|gpio| state.sim.pwm.with(|pwm| pwm.get(&gpio).copied()))
                                            };
                                            let drive = move || {
                                                let reference = reference.get();
                                                // A direction pin under PWM has
                                                // no one level, and reads as
                                                // none rather than as either.
                                                let level = |pin: &str| match level_of(&PinRef::new(reference.as_str(), pin)) {
                                                    Some(period::Level::High) => Some(true),
                                                    Some(period::Level::Low) => Some(false),
                                                    _ => None,
                                                };
                                                // A fan has no direction pins,
                                                // and reading two unwired inputs
                                                // as two lows would call it
                                                // COAST and stop a motor that has
                                                // nowhere to say otherwise.
                                                match (level("IN1"), level("IN2")) {
                                                    (None, None) => rusty_embed::Drive::Forward,
                                                    (a, b) => rusty_embed::Drive::from_inputs(a.unwrap_or(false), b.unwrap_or(false)),
                                                }
                                            };
                                            let spin = move || match duty().map(|d| d.duty) {
                                                Some(d) if d > 0.01 && drive().turns() => {
                                                    let seconds = (0.25 / d).clamp(0.25, 4.0);
                                                    let way = match drive() {
                                                        rusty_embed::Drive::Reverse => "reverse",
                                                        _ => "normal",
                                                    };
                                                    format!("animation-duration: {seconds:.2}s; animation-direction: {way}")
                                                }
                                                _ => "animation: none".to_string(),
                                            };
                                            let readout = move || match duty() {
                                                None => t!("simulate.no-duty"),
                                                Some(d) => {
                                                    format!("{:.0}% {}", d.duty * 100.0, drive().label())
                                                }
                                            };
                                            let tone = move || match duty() {
                                                None => "text-label-3",
                                                Some(_) if !drive().turns() => "text-label-2",
                                                Some(_) => "text-label",
                                            };
                                            Some(view! {
                                                <div class="pointer-events-none absolute flex items-center gap-2" style=style>
                                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full border border-line-strong bg-sunken">
                                                        <span class="absolute inset-[3px] animate-spin" style=spin>
                                                            <span class="absolute top-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-label-2" />
                                                            <span class="absolute bottom-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-line-strong" />
                                                        </span>
                                                        <span class="size-1 rounded-full bg-label-3" />
                                                    </span>
                                                    <span class=move || format!("font-mono text-caption {}", tone())>{readout}</span>
                                                </div>
                                            }.into_any())
                                        }
                                        _ => None,
                                    }
                                }
                            }
                        />

                        <svg
                            class="pointer-events-none absolute"
                            style="left: -2000px; top: -2000px"
                            width="6000"
                            height="6000"
                        >
                            <g transform="translate(2000, 2000)">
                                // ── wires: grab a segment, push it ──────────
                                {move || {
                                    let list = parts.get();
                                    let picked = selected_wire.get();
                                    let hovered = hover_wire.get();
                                    let powered = running.get();
                                    wires
                                        .get()
                                        .iter()
                                        .enumerate()
                                        .filter_map(|(wire_index, wire)| {
                                            let ends = wire_ends(&list, wire)?;
                                            let points = wire_path(&ends, &wire.bends);
                                            let from = points[0];
                                            let to = *points.last()?;
                                            let is_picked = picked == Some(wire_index);
                                            let is_hovered = hovered == Some(wire_index);
                                            // A net's level colours its wires
                                            // while the firmware runs: high is
                                            // green, low is dim, unknown is the
                                            // sheet's grey — and a net under
                                            // PWM is green in pulses, because
                                            // it is high for part of every
                                            // period and has no one level.
                                            let tone = powered
                                                .then(|| tones.with(|all| all.get(wire_index).copied().flatten()))
                                                .flatten();
                                            let stroke = if is_picked {
                                                "#e05d38"
                                            } else if is_hovered {
                                                "#b7c0cc"
                                            } else {
                                                match tone {
                                                    Some(Tone::High | Tone::Switching) => "#5ecf7a",
                                                    Some(Tone::Low) => "#5b6472",
                                                    None => "#7d8694",
                                                }
                                            };
                                            let dashes = if tone == Some(Tone::Switching) { "7 4" } else { "none" };
                                            let width = if is_picked {
                                                "2.4"
                                            } else if is_hovered {
                                                "2.0"
                                            } else {
                                                "1.6"
                                            };
                                            let path = points
                                                .iter()
                                                .map(|(x, y)| format!("{x},{y}"))
                                                .collect::<Vec<_>>()
                                                .join(" ");

                                            // One grab handle per segment.
                                            // Pushing a segment is how a
                                            // schematic editor moves a corner
                                            // — you never hunt for the vertex.
                                            let grabs = points
                                                .windows(2)
                                                .enumerate()
                                                .map(|(seg, pair)| {
                                                    let (a, b) = (pair[0], pair[1]);
                                                    let horizontal = (a.1 - b.1).abs() < 0.5;
                                                    let cursor = if horizontal { "row-resize" } else { "col-resize" };
                                                    let drawn = points.clone();
                                                    view! {
                                                        <line
                                                            x1=a.0
                                                            y1=a.1
                                                            x2=b.0
                                                            y2=b.1
                                                            stroke="transparent"
                                                            stroke-width="12"
                                                            // Through to the parts while it runs:
                                                            // a wire drawn across a switch's cap
                                                            // took every press meant for it.
                                                            style=move || {
                                                                if live.get() {
                                                                    "pointer-events: none".to_string()
                                                                } else {
                                                                    format!("pointer-events: stroke; cursor: {cursor}")
                                                                }
                                                            }
                                                            on:pointerenter=move |_| hover_wire.set(Some(wire_index))
                                                            on:pointerleave=move |_| hover_wire.set(None)
                                                            on:contextmenu=move |event: ev::MouseEvent| {
                                                                event.prevent_default();
                                                                event.stop_propagation();
                                                                if !live.get_untracked() {
                                                                    selected.set(None);
                                                                    selected_wire.set(Some(wire_index));
                                                                }
                                                                menu.set(Some((
                                                                    f64::from(event.client_x()),
                                                                    f64::from(event.client_y()),
                                                                    MenuTarget::Wire(wire_index),
                                                                )));
                                                            }
                                                            on:pointerdown=move |event: ev::PointerEvent| {
                                                                if event.button() != 0 {
                                                                    return;
                                                                }
                                                                // The sheet's, which pans, while it runs.
                                                                if live.get_untracked() {
                                                                    return;
                                                                }
                                                                event.prevent_default();
                                                                event.stop_propagation();
                                                                selected.set(None);
                                                                selected_wire.set(Some(wire_index));
                                                                if let Some(element) = canvas.get_untracked() {
                                                                    let _ = element.focus();
                                                                }
                                                                checkpoint();
                                                                // Freeze the drawn path into
                                                                // real bends, so the segment
                                                                // has movable points on both
                                                                // sides — and the two anchored
                                                                // ends stay put by growing an
                                                                // elbow.
                                                                let handles = wires
                                                                    .try_update(|all| {
                                                                        let w = all.get_mut(wire_index)?;
                                                                        let mut inner: Vec<(f64, f64)> = drawn[1..drawn.len() - 1].to_vec();
                                                                        let mut first = seg as isize - 1;
                                                                        let mut second = seg as isize;
                                                                        if first < 0 {
                                                                            inner.insert(0, drawn[0]);
                                                                            first = 0;
                                                                            second = 1;
                                                                        }
                                                                        if second as usize >= inner.len() {
                                                                            inner.push(to);
                                                                            second = inner.len() as isize - 1;
                                                                        }
                                                                        w.bends = inner;
                                                                        Some((first as usize, second as usize))
                                                                    })
                                                                    .flatten();
                                                                if let Some((first, second)) = handles {
                                                                    let world = to_world(
                                                                        f64::from(event.client_x()),
                                                                        f64::from(event.client_y()),
                                                                    );
                                                                    drag.set(Some(Drag::Segment {
                                                                        wire: wire_index,
                                                                        first,
                                                                        second,
                                                                        horizontal,
                                                                        grab: if horizontal { world.1 } else { world.0 },
                                                                        base: if horizontal { a.1 } else { a.0 },
                                                                    }));
                                                                }
                                                            }
                                                        />
                                                    }
                                                })
                                                .collect_view();

                                            // Corner pips, so the selected
                                            // wire shows where its bends are.
                                            let bends = is_picked.then(|| {
                                                points[1..points.len() - 1]
                                                    .iter()
                                                    .map(|(bx, by)| {
                                                        let (bx, by) = (*bx, *by);
                                                        view! {
                                                            <rect
                                                                x=bx - 3.5
                                                                y=by - 3.5
                                                                width="7"
                                                                height="7"
                                                                fill="#e05d38"
                                                                style="pointer-events: auto; cursor: pointer"
                                                                on:dblclick=move |event: ev::MouseEvent| {
                                                                    if live.get_untracked() {
                                                                        return;
                                                                    }
                                                                    event.stop_propagation();
                                                                    checkpoint();
                                                                    wires.update(|all| {
                                                                        if let Some(w) = all.get_mut(wire_index) {
                                                                            w.bends.retain(|(wx, wy)| (wx - bx).abs() > 0.5 || (wy - by).abs() > 0.5);
                                                                        }
                                                                    });
                                                                    dirty.set(true);
                                                                }
                                                            />
                                                        }
                                                    })
                                                    .collect_view()
                                            });

                                            Some(view! {
                                                <polyline
                                                    points=path
                                                    fill="none"
                                                    stroke=stroke
                                                    stroke-width=width
                                                    stroke-dasharray=dashes
                                                    style="pointer-events: none"
                                                />
                                                <circle cx=from.0 cy=from.1 r="2.2" fill="#c9a227" style="pointer-events: none" />
                                                <circle cx=to.0 cy=to.1 r="2.2" fill="#c9a227" style="pointer-events: none" />
                                                {bends}
                                                {grabs}
                                            })
                                        })
                                        .collect_view()
                                }}

                                // Junctions: where wires *join*, as opposed
                                // to where they cross. Two ends at a pin is
                                // one — the pin is a conductor too — and so
                                // is an end landing on another wire's line,
                                // which a branch makes and which the old
                                // rule, reading pins alone, drew nothing for.
                                {move || {
                                    let list = parts.get();
                                    let all = wires.get();
                                    layout::junctions(&list, &all)
                                        .into_iter()
                                        .map(|(x, y)| {
                                            view! {
                                                <circle cx=x cy=y r="3.6" fill="#c9a227" style="pointer-events: none" />
                                            }
                                        })
                                        .collect_view()
                                }}

                                // The armed part's ghost: the part itself,
                                // where a click would plant it. A bare
                                // rectangle said only "something goes here".
                                {move || {
                                    let symbol = placing.get()?;
                                    let (x, y) = place_at.get()?;
                                    let plan = art::layout(&symbol, "");
                                    let (x0, y0, x1, y1) = plan.bounds;
                                    let drawn = art::markup(&symbol, "");
                                    Some(view! {
                                        <g
                                            transform=format!("translate({x} {y})")
                                            opacity="0.7"
                                            style="pointer-events: none"
                                        >
                                            <rect
                                                x=x0 - 5.0
                                                y=y0 - 5.0
                                                width=x1 - x0 + 10.0
                                                height=y1 - y0 + 10.0
                                                rx="4"
                                                fill="#e05d38"
                                                fill-opacity="0.10"
                                                stroke="#e05d38"
                                                stroke-width="1.2"
                                                stroke-dasharray="4 3"
                                            />
                                            <g inner_html=drawn></g>
                                        </g>
                                    })
                                }}

                                // ── alignment guides ────────────────────────
                                {move || {
                                    let (gx, gy) = guides.get();
                                    view! {
                                        {gx.map(|x| view! {
                                            <line x1=x y1=-2000 x2=x y2=4000 stroke="#e0a838" stroke-width="0.8" stroke-dasharray="4 4" style="pointer-events: none" />
                                        })}
                                        {gy.map(|y| view! {
                                            <line x1=-2000 y1=y x2=4000 y2=y stroke="#e0a838" stroke-width="0.8" stroke-dasharray="4 4" style="pointer-events: none" />
                                        })}
                                    }
                                }}

                                // ── the ghost while pulling a new wire ──────
                                //
                                // The route the wire will actually take
                                // once it lands on a pin, computed by the
                                // same `connection` that will make it — a
                                // preview that showed a diagonal and then
                                // drew an orthogonal route somewhere else
                                // is a preview of nothing. With no pin in
                                // reach it is the elbow out of the pin,
                                // which is the shape a schematic wire has
                                // whatever it ends on.
                                // A wire being drawn click by click is drawn
                                // the whole way: what the clicks have fixed,
                                // solid, and the leg following the pointer,
                                // dashed — both from the `Drawing` the next
                                // click will use, so the click makes what
                                // the screen showed.
                                {move || {
                                    let target = ghost.get()?;
                                    let draft = drawing.get()?;
                                    let landing = hover_pin.get();
                                    let (fixed, live) = parts.with(|list| {
                                        let part = list.get(draft.from.0)?;
                                        let pin = part.pin(&draft.from.1)?;
                                        let (start, out) = (pin_point(part, pin), pin_out(part, pin));
                                        let mut fixed = vec![start];
                                        fixed.extend(draft.placed.iter().copied());
                                        let anchor = *fixed.last()?;
                                        let mut live = vec![anchor];
                                        match landing.as_ref() {
                                            // Pin to pin with nothing laid
                                            // between is the routed wire a
                                            // drag makes, and so is its
                                            // preview.
                                            Some(to) if draft.placed.is_empty() => {
                                                let routed = wires.with(|all| {
                                                    edit::connection(
                                                        list,
                                                        all,
                                                        (draft.from.0, &draft.from.1),
                                                        (to.0, &to.1),
                                                    )
                                                });
                                                match routed.and_then(|w| {
                                                    wire_ends(list, &w).map(|e| wire_path(&e, &w.bends))
                                                }) {
                                                    Some(path) => live = path,
                                                    None => live.extend(draft.leg_to(start, out, target)),
                                                }
                                            }
                                            Some(to) => {
                                                let end_part = list.get(to.0)?;
                                                let end_pin = end_part.pin(&to.1)?;
                                                live.extend(last_leg(
                                                    anchor,
                                                    pin_point(end_part, end_pin),
                                                    pin_out(end_part, end_pin),
                                                ));
                                            }
                                            None => live.extend(draft.leg_to(start, out, target)),
                                        }
                                        Some((fixed, simplify_route(live)))
                                    })?;
                                    let join = |points: &[(f64, f64)]| {
                                        points
                                            .iter()
                                            .map(|(x, y)| format!("{x},{y}"))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    };
                                    let solid = (fixed.len() >= 2).then(|| {
                                        view! {
                                            <polyline
                                                points=join(&fixed)
                                                fill="none"
                                                stroke="#e0a838"
                                                stroke-width="1.8"
                                                style="pointer-events: none"
                                            />
                                        }
                                    });
                                    Some(view! {
                                        <g>
                                            {solid}
                                            <polyline
                                                points=join(&live)
                                                fill="none"
                                                stroke="#e0a838"
                                                stroke-width="1.8"
                                                stroke-dasharray="5 4"
                                                style="pointer-events: none"
                                            />
                                        </g>
                                    })
                                }}
                                {move || {
                                    let target = ghost.get()?;
                                    let Some(Drag::Wire { from, .. }) = drag.get() else {
                                        return None;
                                    };
                                    let landing = hover_pin.get();
                                    let points = parts.with(|list| {
                                        let part = list.get(from.0)?;
                                        let pin = part.pin(&from.1)?;
                                        let start = pin_point(part, pin);
                                        let out = pin_out(part, pin);
                                        if let Some(to) = landing.as_ref() {
                                            let wire = wires.with(|all| {
                                                edit::connection(
                                                    list,
                                                    all,
                                                    (from.0, &from.1),
                                                    (to.0, &to.1),
                                                )
                                            });
                                            if let Some(wire) = wire
                                                && let Some(ends) = wire_ends(list, &wire)
                                            {
                                                return Some(wire_path(&ends, &wire.bends));
                                            }
                                        }
                                        // Out along the pin, then one turn
                                        // towards the pointer.
                                        let step = ROW_PITCH;
                                        let stub = (start.0 + out.0 * step, start.1 + out.1 * step);
                                        let corner = if out.0.abs() > out.1.abs() {
                                            (target.0, stub.1)
                                        } else {
                                            (stub.0, target.1)
                                        };
                                        Some(vec![start, stub, corner, target])
                                    })?;
                                    let path = points
                                        .iter()
                                        .map(|(x, y)| format!("{x},{y}"))
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    Some(view! {
                                        <polyline
                                            points=path
                                            fill="none"
                                            stroke="#e0a838"
                                            stroke-width="1.8"
                                            stroke-dasharray="5 4"
                                            style="pointer-events: none"
                                        />
                                    })
                                }}

                                // ── the rubber band ─────────────────────────
                                {move || {
                                    let Some(Drag::Box { start }) = drag.get() else {
                                        return None;
                                    };
                                    let to = box_to.get()?;
                                    Some(view! {
                                        <rect
                                            x=start.0.min(to.0)
                                            y=start.1.min(to.1)
                                            width=(start.0 - to.0).abs()
                                            height=(start.1 - to.1).abs()
                                            fill="#e0a838"
                                            fill-opacity="0.08"
                                            stroke="#e0a838"
                                            stroke-width="1"
                                            stroke-dasharray="4 3"
                                            style="pointer-events: none"
                                        />
                                    })
                                }}
                            </g>
                        </svg>
                    </div>

                    // While a wire is being drawn the whole sheet is one
                    // click target: a click is a corner, a pin or a wire
                    // finishes it, and nothing under the pointer — a part,
                    // a wire's grab handle — takes the press for itself.
                    // Under the corner controls (z-20), which stay usable;
                    // the middle button and Ctrl/Alt with the left fall
                    // through to the canvas, which pans, and the drawing
                    // survives the pan.
                    {move || {
                        drawing.with(Option::is_some).then(|| {
                            view! {
                                <div
                                    class="absolute inset-0 z-10 cursor-crosshair"
                                    on:pointerdown=move |event: ev::PointerEvent| {
                                        match event.button() {
                                            0 if !(event.ctrl_key() || event.alt_key()) => {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                if let Some(element) = canvas.get_untracked() {
                                                    let _ = element.focus();
                                                }
                                                drawing_click(to_world(
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                ));
                                            }
                                            2 => {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                cancel_drawing();
                                            }
                                            _ => {}
                                        }
                                    }
                                    on:contextmenu=move |event: ev::MouseEvent| {
                                        event.prevent_default();
                                        event.stop_propagation();
                                    }
                                >
                                    // What the keys do while drawing, where
                                    // KiCad puts it: said while it applies
                                    // and gone the moment it does not.
                                    <span class="pointer-events-none absolute top-2 left-3 rounded-[6px] bg-raised/90 px-2 py-1 text-caption text-label-2 ring-1 ring-line">
                                        {t!("simulate.drawing-hint")}
                                    </span>
                                </div>
                            }
                        })
                    }}

                    // What the rules found, and where the pin levels come
                    // from — on the board rather than in a rail beside it,
                    // because both are claims about what you are looking at.
                    <div class="pointer-events-none absolute bottom-2 left-3 flex max-w-[calc(100%-1.5rem)] flex-col gap-1">
                        {move || {
                            let findings = findings.get();
                            (!findings.is_empty()).then(|| {
                                view! {
                                    <div class="pointer-events-auto flex max-w-[60ch] flex-col gap-0.5 rounded-[6px] bg-amber-fill/90 px-2 py-1.5 ring-1 ring-line">
                                        {findings
                                            .iter()
                                            .map(|warning| {
                                                view! {
                                                    <span class="text-caption leading-snug text-label-2">{warning_text(warning)}</span>
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                }
                            })
                        }}
                        // While it runs, what the pointer is over, measured:
                        // the probe and the inspector's numbers, said on the
                        // board instead of in a panel over it.
                        {move || {
                            if !live.get() {
                                return None;
                            }
                            let text = if let Some(index) = hover_part.get() {
                                let reference = parts
                                    .with(|list| list.get(index).map(|p| p.inst.reference.clone()))?;
                                let reading = measured(&reference)?;
                                let (volts, amps) =
                                    (readout::volts(reading.across), readout::amps(reading.through));
                                // An average no instant has — a lamp under
                                // PWM never sits at its average voltage —
                                // says that it is one.
                                if reading.steady {
                                    format!("{reference} · {volts} · {amps}")
                                } else {
                                    format!(
                                        "{reference} · {}",
                                        t!("simulate.reading-average", volts = volts, amps = amps)
                                    )
                                }
                            } else {
                                let index = hover_wire.get()?;
                                let from = wires.with(|all| all.get(index).map(|w| w.from.clone()))?;
                                let word = level_word(level_of(&from));
                                match volts_of(&from).map(readout::volts) {
                                    Some(volts) => format!("{word} · {volts}"),
                                    None => word,
                                }
                            };
                            Some(view! {
                                <span class="self-start rounded-[6px] bg-raised/90 px-2 py-1 font-mono text-caption text-label ring-1 ring-line">
                                    {text}
                                </span>
                            })
                        }}
                        {move || {
                            let (label, detail) = match state.sim.pin_source.get() {
                                rusty_embed::PinSource::Emulator => (
                                    t!("simulate.pins-emulator"),
                                    t!("simulate.pins-emulator-detail"),
                                ),
                                rusty_embed::PinSource::Firmware => (
                                    t!("simulate.pins-firmware"),
                                    t!("simulate.pins-firmware-detail"),
                                ),
                            };
                            running.get().then(|| {
                                view! {
                                    <span
                                        class="pointer-events-auto cursor-help self-start text-footnote text-label-3 underline decoration-dotted underline-offset-2"
                                        title=detail
                                    >
                                        {label}
                                    </span>
                                }
                            })
                        }}
                    </div>

                    {move || {
                        let (x, y, target) = menu.get()?;
                        let close = Callback::new(move |_| menu.set(None));
                        // A running board's menu is the view's, whatever it
                        // was opened on: nothing in it edits.
                        let target = if live.get_untracked() { MenuTarget::Sheet } else { target };
                        let items = match target {
                            MenuTarget::Wire(index) => view! {
                                <MenuItem
                                    label=t!("simulate.straighten")
                                    on_select=Callback::new(move |_| {
                                        straighten_wire(index);
                                        menu.set(None);
                                    })
                                />
                                <MenuSeparator />
                                <MenuItem
                                    label=t!("simulate.disconnect")
                                    shortcut="Del"
                                    danger=true
                                    on_select=Callback::new(move |_| {
                                        remove_wire(index);
                                        menu.set(None);
                                    })
                                />
                            }
                                .into_any(),
                            MenuTarget::Part(index) => {
                                let (is_kit, has_wires) = parts.with_untracked(|list| {
                                    let part = list.get(index);
                                    let is_kit = part.is_some_and(|p| p.is_kit());
                                    let reference = part.map(|p| p.inst.reference.clone()).unwrap_or_default();
                                    let has_wires = wires.with_untracked(|all| {
                                        all.iter().any(|w| w.from.part == reference || w.to.part == reference)
                                    });
                                    (is_kit, has_wires)
                                });
                                view! {
                                    <MenuItem
                                        label=t!("simulate.rotate")
                                        shortcut="Space"
                                        on_select=Callback::new(move |_| {
                                            rotate_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.mirror")
                                        shortcut="X"
                                        on_select=Callback::new(move |_| {
                                            mirror_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.duplicate")
                                        shortcut="Ctrl+D"
                                        disabled=is_kit
                                        on_select=Callback::new(move |_| {
                                            duplicate_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.disconnect-wires")
                                        disabled=!has_wires
                                        on_select=Callback::new(move |_| {
                                            disconnect_all(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuSeparator />
                                    <MenuItem
                                        label=t!("simulate.remove")
                                        shortcut="Del"
                                        danger=true
                                        disabled=is_kit
                                        on_select=Callback::new(move |_| {
                                            remove_part(index);
                                            menu.set(None);
                                        })
                                    />
                                }
                                    .into_any()
                            }
                            MenuTarget::Pin(part, pin) => {
                                let marked_already = parts.with_untracked(|list| {
                                    list.get(part)
                                        .and_then(|p| p.symbol.as_ref())
                                        .and_then(|s| s.pins.get(pin).map(|f| (s, f)))
                                        .zip(list.get(part))
                                        .is_some_and(|((symbol, found), owner)| {
                                            let named = PinRef::new(
                                                &owner.inst.reference,
                                                symbol.wire_key(found),
                                            );
                                            no_connect.with_untracked(|m| m.contains(&named))
                                        })
                                });
                                let label = if marked_already {
                                    t!("simulate.connected-again")
                                } else {
                                    t!("simulate.not-connected")
                                };
                                view! {
                                    <MenuItem
                                        label=label
                                        on_select=Callback::new(move |_| {
                                            toggle_no_connect(part, pin);
                                            menu.set(None);
                                        })
                                    />
                                }
                                .into_any()
                            }
                            MenuTarget::Sheet => view! {
                                {(!live.get_untracked())
                                    .then(|| {
                                        view! {
                                            <MenuItem
                                                label=t!("menu.edit.undo")
                                                shortcut="Ctrl+Z"
                                                disabled=history.with_untracked(Vec::is_empty)
                                                on_select=Callback::new(move |_| {
                                                    undo();
                                                    menu.set(None);
                                                })
                                            />
                                            <MenuItem
                                                label=t!("menu.edit.redo")
                                                shortcut="Ctrl+Y"
                                                disabled=future.with_untracked(Vec::is_empty)
                                                on_select=Callback::new(move |_| {
                                                    redo();
                                                    menu.set(None);
                                                })
                                            />
                                            <MenuItem
                                                label=t!("simulate.select-all")
                                                shortcut="Ctrl+A"
                                                on_select=Callback::new(move |_| {
                                                    marked.set((0..parts.with_untracked(Vec::len)).collect());
                                                    selected_wire.set(None);
                                                    menu.set(None);
                                                })
                                            />
                                            <MenuSeparator />
                                        }
                                    })}
                                {state
                                    .sim.plan
                                    .with_untracked(|plan| {
                                        plan.as_ref()
                                            .and_then(|p| p.debug.as_ref())
                                            .map(|d| d.gdb_command.clone())
                                    })
                                    .map(|command| {
                                        view! {
                                            <MenuItem
                                                label=t!("simulate.open-gdb")
                                                on_select=Callback::new(move |_| {
                                                    controller::attach_debugger_terminal(
                                                        state,
                                                        command.clone(),
                                                    );
                                                    menu.set(None);
                                                })
                                            />
                                            <MenuSeparator />
                                        }
                                    })}
                                <MenuItem
                                    label=t!("simulate.fit-contents")
                                    shortcut="F"
                                    on_select=Callback::new(move |_| {
                                        fit_view();
                                        menu.set(None);
                                    })
                                />
                                <MenuItem
                                    label=t!("simulate.reset-view")
                                    shortcut="1:1"
                                    on_select=Callback::new(move |_| {
                                        view.set((0.0, 0.0, 1.0));
                                        menu.set(None);
                                    })
                                />
                            }
                                .into_any(),
                        };
                        Some(view! { <ContextMenu x=x y=y on_close=close>{items}</ContextMenu> })
                    }}
                </div>

                // ── properties ───────────────────────────────────────────
                <div class=move || {
                    if !compact {
                        "flex w-[200px] flex-none flex-col overflow-y-auto border-l border-line bg-sidebar"
                    } else if inspecting() {
                        if inspector_left() {
                            "absolute top-12 left-2 z-30 flex max-h-[calc(100%-3.5rem)] w-[220px] flex-col overflow-y-auto rounded-[8px] bg-sidebar shadow-2xl ring-1 ring-line-strong"
                        } else {
                            "absolute top-12 right-2 z-30 flex max-h-[calc(100%-3.5rem)] w-[220px] flex-col overflow-y-auto rounded-[8px] bg-sidebar shadow-2xl ring-1 ring-line-strong"
                        }
                    } else {
                        "hidden"
                    }
                }>
                    {move || {
                        // A selected wire outranks a selected part.
                        if let Some(index) = selected_wire.get() {
                            let Some(wire) = wires.with(|all| all.get(index).cloned()) else {
                                return ().into_any();
                            };
                            let bends = wire.bends.len();
                            return view! {
                                <div class="flex flex-col gap-2 p-3">
                                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        {t!("simulate.wire")}
                                    </span>
                                    <p class="font-mono text-footnote text-label-2">
                                        {format!("{} → {}", wire.from, wire.to)}
                                    </p>
                                    <p class="text-footnote text-label-4">
                                        {t!("simulate.wire-bends", bends = bends.to_string())}
                                    </p>
                                    // The probe. A wire on a sheet is a
                                    // question — what is this actually
                                    // joined to, and what is it sitting at
                                    // — and the answer was only ever the
                                    // colour of the line while it ran.
                                    {move || {
                                        let Some(members) = period.with(|p| {
                                            let nets = p.nets();
                                            nets.net_of(&wire.from).map(|net| {
                                                nets.members(net)
                                                    .iter()
                                                    .map(PinRef::to_string)
                                                    .collect::<Vec<String>>()
                                            })
                                        }) else {
                                            return ().into_any();
                                        };
                                        let level = level_of(&wire.from);
                                        let tone = match level {
                                            Some(period::Level::High | period::Level::Switching(_)) => "text-[#5ecf7a]",
                                            Some(period::Level::Low) => "text-label-2",
                                            None => "text-label-4",
                                        };
                                        let word = level_word(level);
                                        // And what it is *at*. High and low
                                        // are the rules' reading; this is
                                        // the solver's, and on a divider
                                        // they are the same net saying two
                                        // different useful things — "not
                                        // being driven" and "1.65 V".
                                        //
                                        // Where there is no number, the
                                        // reason goes in its place. This is
                                        // the moment somebody asked what a
                                        // net is at, so it is the moment to
                                        // say which property would answer
                                        // them — a probe that silently
                                        // showed nothing would read as a
                                        // feature that does not work.
                                        let at = period.with(|p| {
                                            weights.with(|w| match p.volts_at(w, &wire.from) {
                                                Ok(volts) => Ok(volts.map(readout::volts)),
                                                Err(why) => Err(unsolved_text(why)),
                                            })
                                        });
                                        view! {
                                            <div class="flex flex-col gap-1 border-t border-line pt-2">
                                                <span class="text-caption text-label-4">
                                                    {t!("simulate.net")}
                                                </span>
                                                <div class="flex items-baseline gap-2">
                                                    <p class=format!("font-mono text-footnote {tone}")>{word}</p>
                                                    {at
                                                        .as_ref()
                                                        .ok()
                                                        .and_then(|volts| volts.clone())
                                                        .map(|volts| {
                                                            view! {
                                                                <p class="font-mono text-footnote text-label">{volts}</p>
                                                            }
                                                        })}
                                                </div>
                                                {at
                                                    .err()
                                                    .map(|why| {
                                                        view! {
                                                            <p class="text-caption leading-snug text-label-4">{why}</p>
                                                        }
                                                    })}
                                                <p class="font-mono text-caption leading-snug text-label-3 select-text">
                                                    {members.join("  ")}
                                                </p>
                                            </div>
                                        }
                                            .into_any()
                                    }}
                                    <button
                                        type="button"
                                        on:click=move |_| straighten_wire(index)
                                        class="rounded-[6px] px-2 py-1 text-footnote text-label-2 ring-1 ring-line hover:bg-sunken hover:text-label"
                                    >
                                        {t!("simulate.straighten")}
                                    </button>
                                    <button
                                        type="button"
                                        on:click=move |_| delete_selection()
                                        class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                                    >
                                        {t!("simulate.disconnect-del")}
                                    </button>
                                </div>
                            }
                                .into_any();
                        }
                        // Several parts under the band: how many, and what
                        // the keys do to them.
                        let count = marked.with(Vec::len);
                        if count > 1 {
                            return view! {
                                <div class="flex flex-col gap-2 p-3">
                                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        {t!("simulate.selected-count", count = count)}
                                    </span>
                                    <p class="text-caption leading-snug text-label-4">
                                        {t!("simulate.group-hint")}
                                    </p>
                                    <button
                                        type="button"
                                        on:click=move |_| remove_marked()
                                        class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                                    >
                                        {t!("simulate.remove-selected")}
                                    </button>
                                </div>
                            }
                                .into_any();
                        }
                        let Some(index) = selected.get() else {
                            return view! {
                                <p class="p-3 text-footnote text-label-4">
                                    {if live.get() {
                                        t!("simulate.live-hint")
                                    } else {
                                        t!("simulate.nothing-selected")
                                    }}
                                </p>
                            }
                                .into_any();
                        };
                        let Some(part) = parts.with(|list| list.get(index).cloned()) else {
                            return ().into_any();
                        };
                        if part.is_kit() {
                            return view! {
                                <div class="flex flex-col gap-2 p-3">
                                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        {part.inst.reference.clone()}
                                    </span>
                                    <p class="font-mono text-footnote text-label-2">{part.inst.value.clone()}</p>
                                    <p class="text-caption leading-snug text-label-4">{t!("simulate.kit-hint")}</p>
                                </div>
                            }
                                .into_any();
                        }

                        let behaviour = part.symbol.as_ref().map(behaviour_of);
                        let symbol_id = part.inst.symbol.clone();
                        let symbol_name = part
                            .symbol
                            .as_ref()
                            .map(|s| s.name.clone())
                            .unwrap_or_else(|| symbol_id.clone());
                        let description = part.symbol.as_ref().and_then(|s| s.description.clone());
                        let reference = part.inst.reference.clone();
                        // Every pin, with what it is wired to — the panel and
                        // the drawing cannot call one pin two things because
                        // both read the symbol.
                        let list = parts.get();
                        let all = wires.get();
                        let pin_rows: Vec<(String, String)> = part
                            .pins()
                            .iter()
                            .map(|pin| {
                                let mut ends: Vec<String> = edit::wires_at(&list, &all, index, &pin.number)
                                    .into_iter()
                                    .filter_map(|w| {
                                        let wire = all.get(w)?;
                                        let other = if edit::end_is(&list, &wire.from, index, &pin.number) {
                                            &wire.to
                                        } else {
                                            &wire.from
                                        };
                                        Some(other.to_string())
                                    })
                                    .collect();
                                ends.sort();
                                let label = if pin.name == "~" || pin.name == pin.number {
                                    pin.number.clone()
                                } else {
                                    format!("{} ({})", pin.name, pin.number)
                                };
                                (label, ends.join(", "))
                            })
                            .collect();
                        let own_warnings: Vec<String> = findings.with(|found| {
                            found
                                .iter()
                                .filter(|w| match w {
                                    Warning::LedWithoutResistor { part } | Warning::SwitchDrivesNothing { part } => {
                                        *part == reference
                                    }
                                    _ => false,
                                })
                                .map(warning_text)
                                .collect()
                        });
                        let analog_max = part.inst.props.get("max").cloned().unwrap_or_default();
                        // The bus fields are offered to a part that has the
                        // pins for it and to no other: a lamp with an I2C
                        // address is a claim about a part that cannot carry
                        // one, and the emulator would honour it.
                        let on_a_bus = part
                            .pins()
                            .iter()
                            .any(|pin| pin.name.eq_ignore_ascii_case("SDA"));
                        let bus_addr = part.inst.props.get("addr").cloned().unwrap_or_default();
                        let bus_regs = part.inst.props.get("regs").cloned().unwrap_or_default();
                        // What this address has actually done on the bus, most
                        // recent last. The point of showing it beside the
                        // fields is that "the slider does nothing" and "the
                        // firmware never asked" look identical without it.
                        let bus_address = rusty_embed::nets::hex_address(&bus_addr);
                        // The wire's half of the same idea, offered to a part
                        // that has a clock pin.
                        let on_a_wire = part
                            .pins()
                            .iter()
                            .any(|pin| pin.name.eq_ignore_ascii_case("SCK"));
                        let wire_select = part.inst.props.get("cs").cloned().unwrap_or_default();
                        let wire_miso = part.inst.props.get("miso").cloned().unwrap_or_default();
                        let wire_line = wire_select.trim().parse::<u8>().ok();
                        let wire_traffic = move || {
                            let Some(select) = wire_line else {
                                return Vec::new();
                            };
                            state.sim.spi.with(|wire| {
                                wire.iter()
                                    .filter(|report| report.select == select)
                                    .rev()
                                    .take(6)
                                    .map(|report| {
                                        let bytes: String = report
                                            .bytes
                                            .iter()
                                            .map(|b| format!("{b:02x}"))
                                            .collect();
                                        format!("{} {bytes}", report.verb)
                                    })
                                    .collect::<Vec<_>>()
                            })
                        };
                        let bus_traffic = move || {
                            let Some(address) = bus_address else {
                                return Vec::new();
                            };
                            state.sim.i2c.with(|bus| {
                                bus.iter()
                                    .filter(|report| report.address == address)
                                    .rev()
                                    .take(6)
                                    .map(|report| {
                                        let bytes: String = report
                                            .bytes
                                            .iter()
                                            .map(|b| format!("{b:02x}"))
                                            .collect();
                                        format!("{} {bytes}", report.verb)
                                    })
                                    .collect::<Vec<_>>()
                            })
                        };

                        view! {
                            <div class="flex flex-col gap-2 p-3">
                                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    {symbol_name}
                                </span>
                                <p class="font-mono text-caption text-label-4 select-text" title=description.clone().unwrap_or_default()>
                                    {symbol_id.clone()}
                                </p>
                                {part.symbol.is_none().then(|| view! {
                                    <p class="text-caption leading-snug text-crimson">{t!("simulate.unknown-symbol")}</p>
                                })}
                                // The reference, KiCad's word for the name on
                                // the sheet: renaming it renames the wires'
                                // ends, and a taken name is refused.
                                <input
                                    type="text"
                                    title=t!("simulate.reference-hint")
                                    prop:value=part.inst.reference.clone()
                                    on:change=move |event| {
                                        checkpoint();
                                        let text = event_target_value(&event);
                                        let ok = parts
                                            .try_update(|list| {
                                                wires.try_update(|all| edit::rename(list, all, index, &text)).unwrap_or(false)
                                            })
                                            .unwrap_or(false);
                                        if ok {
                                            dirty.set(true);
                                        }
                                    }
                                    class="h-[26px] rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                />
                                <input
                                    type="text"
                                    title=t!("simulate.value-hint")
                                    placeholder=t!("simulate.value")
                                    prop:value=part.inst.value.clone()
                                    on:change=move |event| {
                                        checkpoint();
                                        let text = event_target_value(&event);
                                        parts.update(|list| edit::set_value(list, index, &text));
                                        dirty.set(true);
                                    }
                                    class="h-[26px] rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                />
                                {(behaviour == Some(Behaviour::Led)).then(|| {
                                    view! {
                                        <div class="flex items-center gap-1.5">
                                            {[
                                                ("green", "bg-[#3ddc84]", t!("parts.color-green")),
                                                ("blue", "bg-[#4aa8ff]", t!("parts.color-blue")),
                                                ("red", "bg-[#ff5c5c]", t!("parts.color-red")),
                                                ("yellow", "bg-[#ffd75c]", t!("parts.color-yellow")),
                                            ]
                                                .into_iter()
                                                .map(|(name, swatch, label)| {
                                                    view! {
                                                        <button
                                                            type="button"
                                                            title=label
                                                            on:click=move |_| {
                                                                checkpoint();
                                                                parts.update(|list| edit::set_value(list, index, name));
                                                                dirty.set(true);
                                                            }
                                                            class=format!("size-5 rounded-full ring-1 ring-line hover:ring-2 {swatch}")
                                                        />
                                                    }
                                                })
                                                .collect_view()}
                                        </div>
                                    }
                                })}
                                {(behaviour == Some(Behaviour::Analog)).then(|| {
                                    view! {
                                        <label class="flex items-center gap-2 text-footnote text-label-2">
                                            <span class="shrink-0">{t!("simulate.analog-max")}</span>
                                            <input
                                                type="text"
                                                title=t!("simulate.max-hint")
                                                placeholder="4095"
                                                prop:value=analog_max.clone()
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let text = event_target_value(&event);
                                                    parts.update(|list| edit::set_prop(list, index, "max", &text));
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                    }
                                })}
                                // The two pulse widths this servo answers
                                // to. Stated rather than assumed: 500..2500
                                // and 1000..2000 are both ordinary, and
                                // reading one as the other puts the horn
                                // forty degrees from where the firmware asked
                                // at each end. Empty is the first pair.
                                {(behaviour == Some(Behaviour::Servo)).then(|| {
                                    let low = part.inst.props.get("min").cloned().unwrap_or_default();
                                    let high = part.inst.props.get("max").cloned().unwrap_or_default();
                                    let set = move |key: &'static str| {
                                        move |event: web_sys::Event| {
                                            checkpoint();
                                            let text = event_target_value(&event);
                                            parts.update(|list| edit::set_prop(list, index, key, &text));
                                            dirty.set(true);
                                        }
                                    };
                                    view! {
                                        <label
                                            class="flex items-center gap-2 text-footnote text-label-2"
                                            title=t!("simulate.servo-pulse-hint")
                                        >
                                            <span class="shrink-0">{t!("simulate.servo-pulse")}</span>
                                            <input
                                                type="text"
                                                placeholder="500"
                                                prop:value=low
                                                on:change=set("min")
                                                class="h-[26px] w-0 min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                            <input
                                                type="text"
                                                placeholder="2500"
                                                prop:value=high
                                                on:change=set("max")
                                                class="h-[26px] w-0 min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                    }
                                })}
                                // Which part a sensor module is: one rusty
                                // answers for register by register, or the
                                // channel the firmware declared.
                                {(behaviour == Some(Behaviour::Sensor)).then(|| {
                                    let current = part.inst.props.get("model").cloned().unwrap_or_default();
                                    let known = sensors.with_value(|all| {
                                        rusty_embed::sensor::Spec::find(all, &current).map(|spec| spec.id.clone())
                                    });
                                    let has_address = part
                                        .inst
                                        .props
                                        .get("addr")
                                        .is_some_and(|address| !address.trim().is_empty());
                                    view! {
                                        <label
                                            class="flex items-center gap-2 text-footnote text-label-2"
                                            title=t!("simulate.sensor-model-hint")
                                        >
                                            <span class="shrink-0">{t!("simulate.sensor-model")}</span>
                                            <select
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let id = event_target_value(&event);
                                                    let first = sensors.with_value(|all| {
                                                        rusty_embed::sensor::Spec::find(all, &id)
                                                            .and_then(|spec| spec.addresses.first().copied())
                                                    });
                                                    parts.update(|list| {
                                                        edit::set_prop(list, index, "model", &id);
                                                        // The address the breakout ships with,
                                                        // unless the sheet already said one.
                                                        if let Some(address) = first
                                                            && !has_address
                                                        {
                                                            let address = format!("{address:02x}");
                                                            edit::set_prop(list, index, "addr", &address);
                                                        }
                                                    });
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-1.5 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            >
                                                <option value="" selected=known.is_none()>
                                                    {t!("simulate.sensor-model-declared")}
                                                </option>
                                                {sensors
                                                    .get_value()
                                                    .into_iter()
                                                    .map(|spec| {
                                                        let chosen = known.as_deref() == Some(spec.id.as_str());
                                                        view! {
                                                            <option value=spec.id selected=chosen>
                                                                {spec.name}
                                                            </option>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </select>
                                        </label>
                                    }
                                })}
                                // Which controller is behind the glass, so
                                // the bytes on the bus can be read as a
                                // picture. Named by the sheet rather than
                                // taken from the traffic: the SH1106's window
                                // sits two columns into its RAM, and a
                                // decoder that chose for itself would draw
                                // something nobody could check. Until it is
                                // named the screen shows what the firmware
                                // prints to `[rusty:disp]`, as it always has.
                                {(behaviour == Some(Behaviour::Display)).then(|| {
                                    let current = part.inst.props.get("panel").cloned().unwrap_or_default();
                                    let current_panel = rusty_embed::screen::Panel::from_id(&current);
                                    let has_address = part
                                        .inst
                                        .props
                                        .get("addr")
                                        .is_some_and(|address| !address.trim().is_empty());
                                    view! {
                                        <label
                                            class="flex items-center gap-2 text-footnote text-label-2"
                                            title=t!("simulate.screen-panel-hint")
                                        >
                                            <span class="shrink-0">{t!("simulate.screen-panel")}</span>
                                            <select
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let id = event_target_value(&event);
                                                    let named = rusty_embed::screen::Panel::from_id(&id).is_some();
                                                    parts.update(|list| {
                                                        edit::set_prop(list, index, "panel", &id);
                                                        // The address these modules ship
                                                        // with, unless the sheet said one.
                                                        if named && !has_address {
                                                            edit::set_prop(list, index, "addr", "3c");
                                                        }
                                                    });
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-1.5 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            >
                                                <option value="" selected=current_panel.is_none()>
                                                    {t!("simulate.screen-panel-text")}
                                                </option>
                                                {rusty_embed::screen::Panel::ALL
                                                    .into_iter()
                                                    .map(|panel| {
                                                        let chosen = current_panel == Some(panel);
                                                        view! {
                                                            <option value=panel.id() selected=chosen>
                                                                {panel.name()}
                                                            </option>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </select>
                                        </label>
                                    }
                                })}
                                // A panel nobody has switched on is drawn
                                // faintly, and here is where it says why: a
                                // dark screen and a driver that never sent
                                // `0xAF` look identical on the desk.
                                {move || {
                                    let address = parts
                                        .with(|list| list.get(index).and_then(display_address))?;
                                    let dark = state.sim.screens.with(|screens| {
                                        screens.get(&address).is_some_and(|screen| {
                                            screen.written() && !screen.is_on()
                                        })
                                    });
                                    dark.then(|| {
                                        view! {
                                            <p class="text-footnote text-label-3">
                                                {t!("simulate.screen-off")}
                                            </p>
                                        }
                                    })
                                }}
                                {on_a_bus.then(|| {
                                    view! {
                                        <label class="flex items-center gap-2 text-footnote text-label-2">
                                            <span class="shrink-0">{t!("simulate.bus-address")}</span>
                                            <input
                                                type="text"
                                                title=t!("simulate.bus-address-hint")
                                                placeholder="68"
                                                prop:value=bus_addr.clone()
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let text = event_target_value(&event);
                                                    parts.update(|list| edit::set_prop(list, index, "addr", &text));
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                        <label class="flex items-center gap-2 text-footnote text-label-2">
                                            <span class="shrink-0">{t!("simulate.bus-registers")}</span>
                                            <input
                                                type="text"
                                                title=t!("simulate.bus-registers-hint")
                                                placeholder="75=68,3b=010203040506"
                                                prop:value=bus_regs.clone()
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let text = event_target_value(&event);
                                                    parts.update(|list| edit::set_prop(list, index, "regs", &text));
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                        {move || {
                                            let traffic = bus_traffic();
                                            (!traffic.is_empty()).then(|| {
                                                view! {
                                                    <div class="flex flex-col gap-0.5">
                                                        <span class="text-caption text-label-4">
                                                            {t!("simulate.bus-traffic")}
                                                        </span>
                                                        {traffic
                                                            .into_iter()
                                                            .map(|line| view! {
                                                                <p class="font-mono text-caption text-label-3">{line}</p>
                                                            })
                                                            .collect_view()}
                                                    </div>
                                                }
                                            })
                                        }}
                                    }
                                })}
                                {on_a_wire.then(|| {
                                    view! {
                                        <label class="flex items-center gap-2 text-footnote text-label-2">
                                            <span class="shrink-0">{t!("simulate.wire-select")}</span>
                                            <input
                                                type="text"
                                                title=t!("simulate.wire-select-hint")
                                                placeholder="0"
                                                prop:value=wire_select.clone()
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let text = event_target_value(&event);
                                                    parts.update(|list| edit::set_prop(list, index, "cs", &text));
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                        <label class="flex items-center gap-2 text-footnote text-label-2">
                                            <span class="shrink-0">{t!("simulate.wire-answer")}</span>
                                            <input
                                                type="text"
                                                title=t!("simulate.wire-answer-hint")
                                                placeholder="1a68"
                                                prop:value=wire_miso.clone()
                                                on:change=move |event| {
                                                    checkpoint();
                                                    let text = event_target_value(&event);
                                                    parts.update(|list| edit::set_prop(list, index, "miso", &text));
                                                    dirty.set(true);
                                                }
                                                class="h-[26px] min-w-0 flex-1 rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                            />
                                        </label>
                                        {move || {
                                            let traffic = wire_traffic();
                                            (!traffic.is_empty()).then(|| {
                                                view! {
                                                    <div class="flex flex-col gap-0.5">
                                                        <span class="text-caption text-label-4">
                                                            {t!("simulate.wire-traffic")}
                                                        </span>
                                                        {traffic
                                                            .into_iter()
                                                            .map(|line| view! {
                                                                <p class="font-mono text-caption text-label-3">{line}</p>
                                                            })
                                                            .collect_view()}
                                                    </div>
                                                }
                                            })
                                        }}
                                    }
                                })}
                                // What the solver makes of this part: what
                                // is across it and what is going through
                                // it, under the value that decides both, so
                                // changing 330 to 1k and watching the
                                // current move is one glance.
                                //
                                // Silent for a part with no electrical
                                // model — a label, a display, a rail — and
                                // for a refusal about some *other* part,
                                // which belongs beside that one.
                                {
                                    let reference = reference.clone();
                                    move || {
                                        period
                                            .with(|p| {
                                                weights.with(|w| match p.reading(w, &reference) {
                                                    Ok(found) => found.map(|read| {
                                                        Ok((
                                                            readout::volts(read.across),
                                                            readout::amps(read.through),
                                                            readout::watts(read.watts),
                                                            read.steady,
                                                        ))
                                                    }),
                                                    Err(why) => (why.part() == Some(reference.as_str()))
                                                        .then(|| Err(unsolved_text(why))),
                                                })
                                            })
                                            .map(|reading| match reading {
                                                Ok((across, through, watts, steady)) => {
                                                    view! {
                                                        <div class="flex flex-col gap-0.5">
                                                            <span class="text-caption text-label-4">
                                                                {if steady {
                                                                    t!("simulate.measured")
                                                                } else {
                                                                    t!("simulate.measured-average")
                                                                }}
                                                            </span>
                                                            // One line at this column's width:
                                                            // three significant figures caps
                                                            // each figure, so the widest a
                                                            // reading gets is three negative
                                                            // ones — 154px of the 176 there
                                                            // are, measured in the panel.
                                                            <p class="flex items-baseline gap-2 font-mono text-footnote text-label">
                                                                <span>{across}</span>
                                                                <span class="text-label-3">{through}</span>
                                                                <span class="text-caption text-label-4">{watts}</span>
                                                            </p>
                                                        </div>
                                                    }
                                                        .into_any()
                                                }
                                                Err(why) => {
                                                    view! {
                                                        <p class="rounded-[6px] bg-sunken px-2 py-1 text-caption leading-snug text-label-3">
                                                            {why}
                                                        </p>
                                                    }
                                                        .into_any()
                                                }
                                            })
                                    }
                                }
                                <div class="flex flex-col gap-1">
                                    <span class="text-caption text-label-4">{t!("simulate.pins")}</span>
                                    {pin_rows
                                        .into_iter()
                                        .map(|(label, ends)| {
                                            let unwired = ends.is_empty();
                                            view! {
                                                <p class="flex items-baseline gap-2 font-mono text-footnote text-label-2">
                                                    <span class="w-[7ch] shrink-0 truncate text-label-3">{label}</span>
                                                    <span class=if unwired { "text-label-4" } else { "" }>
                                                        {if unwired { t!("simulate.unwired") } else { ends }}
                                                    </span>
                                                </p>
                                            }
                                        })
                                        .collect_view()}
                                    <p class="text-caption leading-snug text-label-4">
                                        {t!("simulate.wire-hint")}
                                    </p>
                                </div>
                                {own_warnings
                                    .into_iter()
                                    .map(|text| view! {
                                        <p class="rounded-[6px] bg-amber-fill px-2 py-1 text-caption leading-snug text-label-2">{text}</p>
                                    })
                                    .collect_view()}
                                <button
                                    type="button"
                                    on:click=move |_| remove_part(index)
                                    class="rounded-[6px] px-2 py-1 text-footnote text-crimson ring-1 ring-line hover:bg-sunken"
                                >
                                    {t!("simulate.remove-del")}
                                </button>
                            </div>
                        }
                            .into_any()
                    }}
                </div>
            </div>
        </div>
    }
}
