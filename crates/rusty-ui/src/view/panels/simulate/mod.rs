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
mod board;
mod controls;
mod edit;
mod faces;
mod geometry;
mod gestures;
mod glow;
mod inspector;
mod layout;
mod library;
mod menu;
mod parts;
mod reading;
mod readout;
mod wires;
mod words;

use board::{Board, level_in, sheet_with_symbols};
use controls::SheetControls;
use faces::Faces;
use geometry::*;
use inspector::Inspector;
use library::Library;
use menu::SheetMenu;
use parts::PartsLayer;
use reading::ReadingLine;
use rusty_embed::circuit;
use rusty_embed::nets::{self, Behaviour, Row, Warning, behaviour_of};
use rusty_embed::period::{self, Period};
use rusty_embed::{PinRef, Sheet, Symbol, Wire};
use wires::WireLayer;
use words::*;

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

/// A net's colour while the board runs.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tone {
    High,
    Low,
    /// Under PWM: high for part of every period, drawn as the pulses it is.
    Switching,
}

/// The editor: library, sheet, corner controls, properties. Local state
/// until Save writes it into `.rusty/sim.toml` and the plan reloads.
///
/// What its pieces share — the parts and wires, what is in hand, the view,
/// the readings — is a [`Board`], built here, and each piece is a component
/// that takes one: the corner's controls, the parts, their faces, the
/// wires, the reading line, the menu and the inspector.
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
                    level_in(period, weights, &wire.from).map(|level| match level {
                        period::Level::High => Tone::High,
                        period::Level::Low => Tone::Low,
                        period::Level::Switching(_) => Tone::Switching,
                    })
                })
                .collect()
        })
    });

    // A new part arrives unwired: connecting it is the user's move, made by
    // pulling a pin to another pin. KiCad's placement: picking a part arms
    // it to the cursor — a ghost follows the mouse, a click plants it there,
    // Escape puts it back.
    let placing = RwSignal::new(None::<Symbol>);
    let place_at = RwSignal::new(None::<(f64, f64)>);

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

    // Which key of which keypad the pointer is holding: one at a time,
    // because a pointer is one finger. `(part index, row, column)`.
    let key_down = RwSignal::new(None::<(usize, usize, usize)>);

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

    let board = Board {
        state,
        compact,
        running,
        live,
        hover_part,
        library_open,
        sensors,
        chip_id,
        chip_label,
        kit_look,
        rows,
        parts,
        wires,
        extra,
        importing,
        dirty,
        selected,
        selected_wire,
        hover_wire,
        menu,
        guides,
        grid,
        drag,
        ghost,
        hover_pin,
        drawing,
        marked,
        group_start,
        box_to,
        view,
        canvas,
        pressed,
        no_connect,
        history,
        future,
        period,
        weights,
        findings,
        tones,
        placing,
        place_at,
        key_down,
        save,
    };

    // Beside the editor the pane is a column of whatever width it was
    // dragged to, so the board opens fitted to it, no larger than life —
    // left at the origin, a devkit placed for the panel's width stood half
    // off the pane's edge.
    if compact {
        Effect::new(move |fitted: Option<bool>| {
            if fitted == Some(true) || canvas.get().is_none() {
                return fitted == Some(true);
            }
            request_animation_frame(move || board.fit_within(1.0));
            true
        });
    }

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
                            board.add_part(symbol);
                            library_open.set(false);
                        })
                        on_import=Callback::new(move |number: String| board.import(number))
                        importing=Signal::derive(move || importing.get())
                    />
                </div>

                <div
                    node_ref=canvas
                    tabindex="0"
                    on:keydown=move |event: ev::KeyboardEvent| board.on_keydown(event)
                    on:wheel=move |event: ev::WheelEvent| board.on_wheel(event)
                    on:contextmenu=move |event: ev::MouseEvent| {
                        event.prevent_default();
                        menu.set(Some((
                            f64::from(event.client_x()),
                            f64::from(event.client_y()),
                            MenuTarget::Sheet,
                        )));
                    }
                    on:pointerdown=move |event: ev::PointerEvent| board.on_pointerdown(event)
                    on:pointermove=move |event: ev::PointerEvent| board.on_pointermove(event)
                    on:pointerup=move |_| board.on_pointerup()
                    on:pointerleave=move |_| board.on_pointerleave()
                    class="relative min-w-0 flex-1 overflow-hidden bg-[#101216] outline-none"
                >
                    // The sheet's own controls, in its corner as every
                    // schematic editor keeps them: Save once there is
                    // something to save, undo and redo, zoom, fit, the snap
                    // grid. Pointer events stop here, or a press on Zoom
                    // would also be a press on the sheet under it.
                    <SheetControls board=board />
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
                        <PartsLayer board=board />

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
                        <Faces board=board />

                        <WireLayer board=board />
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
                                                board.drawing_click(board.to_world(
                                                    f64::from(event.client_x()),
                                                    f64::from(event.client_y()),
                                                ));
                                            }
                                            2 => {
                                                event.prevent_default();
                                                event.stop_propagation();
                                                board.cancel_drawing();
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
                    <ReadingLine board=board />

                    <SheetMenu board=board />
                </div>

                // ── properties ───────────────────────────────────────────
                <Inspector board=board />
            </div>
        </div>
    }
}
