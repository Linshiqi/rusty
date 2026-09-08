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

use std::collections::HashSet;

use leptos::{ev, prelude::*};

mod art;
mod edit;
mod geometry;
mod library;

use geometry::*;
use library::Library;
use rusty_embed::nets::{self, Behaviour, Evaluation, Row, Warning, behaviour_of};
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
pub fn Simulate() -> impl IntoView {
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
        Warning::WireSelectUnreadable { part, value } => {
            t!("simulate.warning-wire-select", part = part, value = value)
        }
        Warning::WireNotWired { part } => t!("simulate.warning-wire-wiring", part = part),
    }
}

/// The editor: library, sheet, corner controls, properties. Local state
/// until Save writes it into `.rusty/sim.toml` and the plan reloads.
#[component]
fn BoardEditor(board: Sheet, library: Vec<Symbol>) -> impl IntoView {
    let state = AppState::expect();
    let running = state.app.session_running;
    let chip = board.chip.clone();
    // Copy handles to the chip's name, so the closures the parts' views
    // share can be `Copy` themselves — a `String` captured by move is what
    // stops a closure being used twice.
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

    let history = RwSignal::new(Vec::<Snapshot>::new());
    let future = RwSignal::new(Vec::<Snapshot>::new());
    let checkpoint = move || {
        history.update(|h| edit::remember(h, (parts.get_untracked(), wires.get_untracked())));
        future.set(Vec::new());
    };
    let undo = move || {
        let Some((p, w)) = history.try_update(|h| h.pop()).flatten() else {
            return;
        };
        future.update(|f| f.push((parts.get_untracked(), wires.get_untracked())));
        parts.set(p);
        wires.set(w);
        dirty.set(true);
    };
    let redo = move || {
        let Some((p, w)) = future.try_update(|f| f.pop()).flatten() else {
            return;
        };
        history.update(|h| h.push((parts.get_untracked(), wires.get_untracked())));
        parts.set(p);
        wires.set(w);
        dirty.set(true);
    };

    // The sheet as the rules read it: parts, wires, and the symbols the
    // parts carry. Built untracked for a command, tracked for the reading.
    let sheet_with_symbols = |chip: &str, list: &[EditPart], wires: &[Wire]| -> Sheet {
        let mut sheet = sheet_of(chip, list, wires);
        for part in list {
            if let Some(symbol) = &part.symbol
                && !sheet.symbols.iter().any(|s| s.id() == symbol.id())
            {
                sheet.symbols.push(symbol.clone());
            }
        }
        sheet
    };
    let sheet_now = move || {
        sheet_with_symbols(
            &chip_id.get_value(),
            &parts.get_untracked(),
            &wires.get_untracked(),
        )
    };
    // Everything the rules say about the sheet at this moment: which lamps
    // are lit, what level every pin sits at, what is wrong. One reading for
    // every part, recomputed when the sheet, the firmware's levels or a
    // held button change.
    let eval: Memo<Evaluation> = {
        let chip = chip.clone();
        Memo::new(move |_| {
            let sheet = sheet_with_symbols(&chip, &parts.get(), &wires.get());
            let rows = rows.get();
            let gpio = state.sim.gpio.get();
            let held = pressed.get();
            nets::evaluate(nets::Inputs {
                sheet: &sheet,
                rows: &rows,
                gpio: &gpio,
                pressed: &held,
            })
        })
    };
    // The GPIO a part's pin reaches through the wires — what a knob, a
    // source or a motor is *on*, in the firmware's terms.
    let gpio_for = move |reference: &str, pin: &str| -> Option<u8> {
        nets::gpio_of(&sheet_now(), &rows.get_untracked(), reference, pin)
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
            let sheet = sheet_of(&chip, &parts.get_untracked(), &wires.get_untracked());
            controller::save_sim_board(state, sheet, dirty);
        })
    };

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
    let fit_view = move || {
        let Some(element) = canvas.get_untracked() else {
            return;
        };
        let rect = element.get_bounding_client_rect();
        let (min, max) = bounds(&parts.get_untracked());
        let (w, h) = (max.0 - min.0 + 80.0, max.1 - min.1 + 80.0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let k = (rect.width() / w)
            .min(rect.height() / h)
            .clamp(CANVAS_ZOOM_RANGE.0, CANVAS_ZOOM_RANGE.1);
        view.set((-(min.0 - 40.0) * k, -(min.1 - 40.0) * k, k));
    };

    let straighten_wire = move |index: usize| {
        checkpoint();
        wires.update(|list| edit::straighten(list, index));
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

    // A switch pressed on the sheet: the rules see it as conducting, and
    // while a session runs the GPIO it reaches is driven to the level its
    // other side holds — through the same message the old buttons sent, so
    // firmware written for `B<pin>=1` hears it too.
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
        if let Some((gpio, _)) =
            nets::button_drives(&sheet_now(), &rows.get_untracked(), &reference)
        {
            controller::sim_press(state, gpio, down);
        }
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">
            <div class="flex min-h-0 flex-1">
                <Library
                    symbols=symbols
                    on_add=Callback::new(move |symbol: Symbol| add_part(symbol))
                    on_import=Callback::new(move |number: String| import(number))
                    importing=Signal::derive(move || importing.get())
                />

                <div
                    node_ref=canvas
                    tabindex="0"
                    on:keydown=move |event: ev::KeyboardEvent| {
                        match event.key().as_str() {
                            "Delete" | "Backspace" => {
                                event.prevent_default();
                                delete_selection();
                            }
                            "Escape" => {
                                // A drag in flight is what Escape is most
                                // often reaching for.
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
                        if placing.with_untracked(Option::is_some) {
                            let step = grid.get_untracked();
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            place_at
                                .set(Some((snap_to(world.0, step), snap_to(world.1, step))));
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
                            Drag::Wire { from } => {
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
                        // disagree about what wiring means.
                        if let Some(Drag::Wire { from }) = drag.get_untracked() {
                            if let Some(to) = hover_pin.get_untracked() {
                                checkpoint();
                                let list = parts.get_untracked();
                                let made = wires
                                    .try_update(|all| {
                                        edit::connect(&list, all, (from.0, &from.1), (to.0, &to.1))
                                    })
                                    .flatten();
                                if made.is_some() {
                                    selected.set(None);
                                    selected_wire.set(made);
                                    dirty.set(true);
                                }
                            }
                            hover_pin.set(None);
                        }
                        ghost.set(None);
                        hover_pin.set(None);
                        box_to.set(None);
                        drag.set(None);
                    }
                    on:pointerleave=move |_| {
                        guides.set((None, None));
                        ghost.set(None);
                        hover_pin.set(None);
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
                        <button
                            type="button"
                            title=t!("simulate.save")
                            disabled=move || !dirty.get()
                            on:click=move |_| save.run(())
                            class=SHEET_BUTTON
                        >
                            <IconView icon=Icon::Save size=14 />
                        </button>
                        <span class="mx-0.5 h-4 w-px bg-line" />
                        <button
                            type="button"
                            title=t!("simulate.undo")
                            disabled=move || history.with(Vec::is_empty)
                            on:click=move |_| undo()
                            class=SHEET_BUTTON
                        >
                            "↶"
                        </button>
                        <button
                            type="button"
                            title=t!("simulate.redo")
                            disabled=move || future.with(Vec::is_empty)
                            on:click=move |_| redo()
                            class=SHEET_BUTTON
                        >
                            "↷"
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
                                            symbol.with(|s| s.as_ref().map(art::layout))
                                        });
                                        let bbox = Memo::new(move |_| {
                                            this.with(|p| p.as_ref().map(part_box).unwrap_or((0.0, 0.0, 0.0, 0.0)))
                                        });
                                        let is_selected =
                                            Signal::derive(move || selected.get() == Some(index));
                                        let is_marked =
                                            Signal::derive(move || marked.with(|m| m.contains(&index)));
                                        let is_lit = Memo::new(move |_| {
                                            let reference = reference.get();
                                            eval.with(|e| e.is_lit(&reference))
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
                                                        (spot.number.clone(), x - p.inst.x, y - p.inst.y)
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
                                                    .filter(|(number, _, _)| {
                                                        !edit::wires_at(&list, all, index, number).is_empty()
                                                    })
                                                    .map(|(number, _, _)| number.clone())
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
                                            event.prevent_default();
                                            event.stop_propagation();
                                            selected.set(Some(index));
                                            selected_wire.set(None);
                                            if let Some(element) = canvas.get_untracked() {
                                                let _ = element.focus();
                                            }
                                            drag.set(Some(Drag::Wire { from: (index, number) }));
                                        };

                                        let on_down = move |event: ev::PointerEvent| {
                                            if event.button() != 0 {
                                                return;
                                            }
                                            event.prevent_default();
                                            event.stop_propagation();
                                            if let Some(element) = canvas.get_untracked() {
                                                let _ = element.focus();
                                            }
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
                                            selected.set(Some(index));
                                            selected_wire.set(None);
                                            if !marked.with_untracked(|m| m.contains(&index)) {
                                                marked.set(vec![index]);
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
                                                let labels = drawn
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(row, spec)| {
                                                        let left = row < per_side;
                                                        let (_, y) = row_offset(drawn.len(), row);
                                                        let label = spec.label.clone();
                                                        view! {
                                                            <text
                                                                x=if left { 18.0 } else { KIT_W - 18.0 }
                                                                y=y + 3.0
                                                                text-anchor=if left { "start" } else { "end" }
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
                                                    <g inner_html=art></g>
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
                                                    let reference = reference.get();
                                                    let rgb = behaviour.get() == Some(Behaviour::Rgb);
                                                    let (colour, lit) = if rgb {
                                                        let channel = |name: &str| {
                                                            eval.with(|e| e.is_pin_lit(&reference, name))
                                                        };
                                                        let (r, g, b) =
                                                            (channel("R"), channel("G"), channel("B"));
                                                        (rgb_color(r, g, b), r || g || b)
                                                    } else {
                                                        let (on, off) = lamp_colors(&value.get());
                                                        let lit = is_lit.get();
                                                        (if lit { on } else { off }, lit)
                                                    };
                                                    // A dark lamp is its own
                                                    // colour dimmed, not grey:
                                                    // a red LED is red on the
                                                    // desk with the power off.
                                                    let glow = if lit {
                                                        format!(
                                                            "filter: drop-shadow(0 0 5px {colour}) drop-shadow(0 0 13px {colour}); pointer-events: none",
                                                        )
                                                    } else {
                                                        "pointer-events: none".to_string()
                                                    };
                                                    view! {
                                                        <circle
                                                            cx=cx
                                                            cy=cy
                                                            r=r
                                                            fill=colour
                                                            fill-opacity=if lit { "0.95" } else { "0.5" }
                                                            stroke="#0b0e12"
                                                            stroke-opacity="0.55"
                                                            stroke-width="0.9"
                                                            style=glow
                                                        />
                                                        <ellipse
                                                            cx=cx - r * 0.3
                                                            cy=cy - r * 0.35
                                                            rx=r * 0.28
                                                            ry=r * 0.42
                                                            fill="#ffffff"
                                                            fill-opacity=if lit { "0.5" } else { "0.16" }
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
                                                    let reference = reference.get();
                                                    let seg = move |name: &str| {
                                                        if eval.with(|e| e.is_pin_lit(&reference, name)) {
                                                            "#ff5c5c"
                                                        } else {
                                                            "#3a2323"
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
                                                // What the firmware prints,
                                                // on the screen it prints it
                                                // to — upright, whichever way
                                                // the module is turned.
                                                Some(Behaviour::Display) => {
                                                    let Some((fx, fy, fw, fh)) = plan.face else {
                                                        return ().into_any();
                                                    };
                                                    let (cx, cy) =
                                                        turned((fx + fw / 2.0, fy + fh / 2.0));
                                                    view! {
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
                                                    let on = is_lit.get();
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
                                                    let duty = gpio_for(&reference, "SIG").and_then(|gpio| {
                                                        state.sim.pwm.with(|pwm| pwm.get(&gpio).copied())
                                                    });
                                                    let angle = duty.map(|d| -90.0 + f64::from(d) * 180.0);
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
                                                .map(|(number, dx, dy)| {
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
                                                    view! {
                                                        <circle
                                                            cx=dx
                                                            cy=dy
                                                            r=move || if target() { 5.5 } else { 3.4 }
                                                            fill=move || if target() { "#ffd75c" } else if wired() { "#c9a227" } else { "#e0a838" }
                                                            class=move || if wired() || target() { "" } else { "animate-pulse" }
                                                            style="pointer-events: all; cursor: crosshair"
                                                            on:pointerdown=move |event: ev::PointerEvent| start_wire(event, number.get_value())
                                                            on:dblclick=move |event: ev::MouseEvent| {
                                                                event.stop_propagation();
                                                                disconnect_pin(index, number.get_value());
                                                            }
                                                        >
                                                            <title>{t!("simulate.pin-hint")}</title>
                                                        </circle>
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
                                                on:pointerleave=on_up
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
                                            let turned = RwSignal::new(128u8);
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
                                                        value="128"
                                                        title=t!("simulate.pot-hint")
                                                        on:pointerdown=move |event: ev::PointerEvent| event.stop_propagation()
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) = event_target_value(&event).parse::<u8>() {
                                                                turned.set(value);
                                                                if let Some(gpio) = gpio_at("W") {
                                                                    controller::sim_pot(state, gpio, value);
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
                                            let max = this.with(|p| p.as_ref().and_then(|p| p.inst.prop::<u16>("max"))).unwrap_or(4095);
                                            // Where the sheet says this source
                                            // starts, until somebody moves it.
                                            // The backend sends the same value
                                            // down the pin channel as soon as
                                            // a run connects, so the slider and
                                            // the converter agree before the
                                            // first drag rather than after it.
                                            let start = this.with(|p| p.as_ref().and_then(|p| p.inst.prop::<u16>("start"))).unwrap_or(0);
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
                                                let level = |pin: &str| eval.with(|e| e.level(&reference, pin));
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
                                            let spin = move || match duty() {
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
                                                Some(d) => format!("{:.0}% {}", d * 100.0, drive().label()),
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
                                    let live = running.get();
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
                                            // sheet's grey.
                                            let level = live
                                                .then(|| eval.with(|e| e.levels.get(&wire.from).copied().flatten()))
                                                .flatten();
                                            let stroke = if is_picked {
                                                "#e05d38"
                                            } else if is_hovered {
                                                "#b7c0cc"
                                            } else {
                                                match level {
                                                    Some(true) => "#5ecf7a",
                                                    Some(false) => "#5b6472",
                                                    None => "#7d8694",
                                                }
                                            };
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
                                                            style=format!("pointer-events: stroke; cursor: {cursor}")
                                                            on:pointerenter=move |_| hover_wire.set(Some(wire_index))
                                                            on:pointerleave=move |_| hover_wire.set(None)
                                                            on:contextmenu=move |event: ev::MouseEvent| {
                                                                event.prevent_default();
                                                                event.stop_propagation();
                                                                selected.set(None);
                                                                selected_wire.set(Some(wire_index));
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

                                // Junctions: a pin two or more wires meet at
                                // gets the dot every schematic draws there.
                                {move || {
                                    let list = parts.get();
                                    let all = wires.get();
                                    let mut seen: Vec<(f64, f64)> = Vec::new();
                                    let mut dots = Vec::new();
                                    for (index, part) in list.iter().enumerate() {
                                        for pin in part.pins() {
                                            if edit::wires_at(&list, &all, index, &pin.number).len() >= 2 {
                                                let point = pin_point(part, pin);
                                                if !seen.contains(&point) {
                                                    seen.push(point);
                                                    dots.push(view! {
                                                        <circle cx=point.0 cy=point.1 r="3.6" fill="#c9a227" style="pointer-events: none" />
                                                    });
                                                }
                                            }
                                        }
                                    }
                                    dots.collect_view()
                                }}

                                // The armed part's ghost: the part itself,
                                // where a click would plant it. A bare
                                // rectangle said only "something goes here".
                                {move || {
                                    let symbol = placing.get()?;
                                    let (x, y) = place_at.get()?;
                                    let plan = art::layout(&symbol);
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
                                {move || {
                                    let target = ghost.get()?;
                                    let Some(Drag::Wire { from }) = drag.get() else {
                                        return None;
                                    };
                                    let start = parts.with(|list| {
                                        let part = list.get(from.0)?;
                                        let pin = part.pin(&from.1)?;
                                        Some(pin_point(part, pin))
                                    })?;
                                    Some(view! {
                                        <line
                                            x1=start.0
                                            y1=start.1
                                            x2=target.0
                                            y2=target.1
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

                    // What the rules found, and where the pin levels come
                    // from — on the board rather than in a rail beside it,
                    // because both are claims about what you are looking at.
                    <div class="pointer-events-none absolute bottom-2 left-3 flex max-w-[calc(100%-1.5rem)] flex-col gap-1">
                        {move || {
                            let findings = eval.with(|e| e.warnings.clone());
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
                                        disabled=is_kit
                                        on_select=Callback::new(move |_| {
                                            rotate_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.mirror")
                                        shortcut="X"
                                        disabled=is_kit
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
                            MenuTarget::Sheet => view! {
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
                <div class="flex w-[200px] flex-none flex-col overflow-y-auto border-l border-line bg-sidebar">
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
                                        let reading = eval.get();
                                        let Some(net) = reading.net_of(&wire.from) else {
                                            return ().into_any();
                                        };
                                        let level = reading
                                            .levels
                                            .get(&wire.from)
                                            .copied()
                                            .flatten();
                                        let (word, tone) = match level {
                                            Some(true) => (t!("simulate.net-high"), "text-[#5ecf7a]"),
                                            Some(false) => (t!("simulate.net-low"), "text-label-2"),
                                            None => (t!("simulate.net-floating"), "text-label-4"),
                                        };
                                        let members: Vec<String> = reading
                                            .members(net)
                                            .iter()
                                            .map(PinRef::to_string)
                                            .collect();
                                        view! {
                                            <div class="flex flex-col gap-1 border-t border-line pt-2">
                                                <span class="text-caption text-label-4">
                                                    {t!("simulate.net")}
                                                </span>
                                                <p class=format!("font-mono text-footnote {tone}")>{word}</p>
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
                                    {t!("simulate.nothing-selected")}
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
                        let own_warnings: Vec<String> = eval.with(|e| {
                            e.warnings
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
                        let bus_address = u8::from_str_radix(
                            bus_addr.trim().trim_start_matches("0x"),
                            16,
                        )
                        .ok();
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
