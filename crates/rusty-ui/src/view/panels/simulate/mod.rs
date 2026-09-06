//! Running firmware with no hardware on the desk — and wiring the desk up.
//!
//! This page is a project of its own, aimed at Wokwi-grade simulation:
//! code on one side, the living board on the other, a growing part
//! catalogue, and user-defined parts via `.rusty/parts/*.toml` so a device
//! rusty never heard of can still be drawn and driven. The serial protocol
//! (`[rusty:gpio]`, and friends to come) is the contract every part speaks.
//!
//! The page is a small board editor in the Wokwi shape: a component library
//! on the left, a canvas with the devkit on the right, and the sheet's own
//! controls in the canvas's corner.
//! LEDs are added from the library, dragged into place, given a pin and a
//! colour, and saved into the project's `.rusty/sim.toml` — a file diffed
//! and reviewed like any other. At run time each LED lights from the pin
//! levels the firmware reports over serial, and the caption says exactly
//! that: the QEMU peripheral models expose no GPIO readback to do better.

use leptos::{ev, prelude::*};

mod edit;
mod geometry;
mod library;

use geometry::*;
use library::Library;
use rusty_embed::SimBoard;

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
                    // chip. This used to default to "esp32", which drew the
                    // classic 30-pin header — GPIO34–39 included — over a C3
                    // project, and a wire could be dropped on a pin the part
                    // does not have. No chip draws rails only, which is the
                    // honest picture of a project rusty could not identify.
                    board=plan.board.clone().unwrap_or_else(|| {
                        let chip = state
                            .project
                            .detected
                            .with_untracked(|p| p.as_ref().and_then(|p| p.chip.clone()))
                            .unwrap_or_default();
                        geometry::empty_board(&chip, None)
                    })
                    user_parts=plan.parts.clone()
                />


            </div>
        }
        .into_any()
    }
}

/// The editor: library, canvas, corner controls. Local state until Save writes it
/// into `.rusty/sim.toml` and the plan reloads.
#[component]
fn BoardEditor(board: SimBoard, user_parts: Vec<rusty_embed::PartDef>) -> impl IntoView {
    let state = AppState::expect();
    let running = state.app.session_running;
    let chip = board.chip.clone();
    let chip_label = board.chip.to_uppercase();

    // The pin rows this part actually has. From the catalogue, so a chip
    // added tomorrow draws its own pins rather than the ESP32 devkit's —
    // which is what every board used to show, C3 boards included, labelled
    // with GPIO36/39/34/35 that the part does not have.
    let rows = {
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
            kit_rows(&chip, &gpio)
        })
    };

    let parts = RwSignal::new(parts_of(&board));
    let kit_pos = RwSignal::new((board.kit_x.unwrap_or(460.0), board.kit_y.unwrap_or(40.0)));
    let dirty = RwSignal::new(false);
    let selected = RwSignal::new(None::<usize>);
    let selected_wire = RwSignal::new(None::<(usize, usize)>);
    // The wire under the pointer, for the brightening that says "this one".
    let hover_wire = RwSignal::new(None::<(usize, usize)>);
    // (client x, client y, what was clicked)
    let menu = RwSignal::new(None::<(f64, f64, MenuTarget)>);
    // Alignment guides shown while a part is being dragged into line with
    // another one — the quiet confirmation every drawing tool gives.
    let guides = RwSignal::new((None::<f64>, None::<f64>));
    // The active grid step. Coarse grids place, fine grids nudge — and the
    // dial exists because no single step suits both.
    let grid = RwSignal::new(SNAP);
    let drag = RwSignal::new(None::<Drag>);
    // While pulling a wire: current cursor in world coords, and the row the
    // cursor hovers, when it is one that accepts wires.
    let ghost = RwSignal::new(None::<(f64, f64)>);
    let hover_row = RwSignal::new(None::<usize>);
    // A wire pulled from a chip pin: the part stub under the pointer, when
    // one is within reach — the dot that lights up to take it.
    let hover_stub = RwSignal::new(None::<(usize, usize)>);
    // Every part in the selection: the one under the ring plus whatever a
    // rubber band or a Shift-click added. `selected` stays the one the
    // inspector describes and the keys act on when the group is one part.
    let marked = RwSignal::new(Vec::<usize>::new());
    // Where each other marked part stood when a group drag began, with its
    // routes' first-leg axes, so every frame is start plus one displacement
    // rather than an accumulation of snapped deltas.
    let group_start = RwSignal::new(Vec::<GroupStart>::new());
    // The rubber band's moving corner while a box drag is in flight.
    let box_to = RwSignal::new(None::<(f64, f64)>);
    let view = RwSignal::new((0.0f64, 0.0f64, 1.0f64));
    let canvas: NodeRef<leptos::html::Div> = NodeRef::new();

    let history = RwSignal::new(Vec::<Snapshot>::new());
    let future = RwSignal::new(Vec::<Snapshot>::new());
    let checkpoint = move || {
        history.update(|h| edit::remember(h, (parts.get_untracked(), kit_pos.get_untracked())));
        future.set(Vec::new());
    };
    let undo = move || {
        let Some((p, k)) = history.try_update(|h| h.pop()).flatten() else {
            return;
        };
        future.update(|f| f.push((parts.get_untracked(), kit_pos.get_untracked())));
        parts.set(p);
        kit_pos.set(k);
        dirty.set(true);
    };
    let redo = move || {
        let Some((p, k)) = future.try_update(|f| f.pop()).flatten() else {
            return;
        };
        history.update(|h| h.push((parts.get_untracked(), kit_pos.get_untracked())));
        parts.set(p);
        kit_pos.set(k);
        dirty.set(true);
    };

    // A new part arrives unwired: connecting it is the user's move, made by
    // pulling its stub to a chip pin. Auto-wiring guessed; this asks.
    // KiCad's placement: picking a part arms it to the cursor — a ghost
    // follows the mouse, a click plants it there, Escape puts it back.
    // Dropping parts at a fixed corner made every placement start with a
    // drag nobody asked for.
    let placing = RwSignal::new(None::<(PartKind, String)>);
    let place_at = RwSignal::new(None::<(f64, f64)>);

    let drop_part = move |kind: PartKind, label_stub: String, x: f64, y: f64| {
        checkpoint();
        parts.update(|list| selected.set(Some(edit::add(list, kind, &label_stub, x, y))));
        dirty.set(true);
    };
    let add_part = move |kind: PartKind, label_stub: String| {
        placing.set(Some((kind, label_stub)));
        place_at.set(None);
    };

    let save = Callback::new(move |_: ()| {
        let board = board_of(&chip, kit_pos.get_untracked(), &parts.get_untracked());
        controller::save_sim_board(state, board, dirty);
    });

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
    let flip_part = move |index: usize| {
        checkpoint();
        parts.update(|list| edit::flip(list, index));
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
        let kit_size = (KIT_W, kit_height(rows.get_untracked().len()));
        let (min, max) = edit::bounds(&parts.get_untracked(), kit_pos.get_untracked(), kit_size);
        let (w, h) = (max.0 - min.0 + 80.0, max.1 - min.1 + 80.0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let k = (rect.width() / w)
            .min(rect.height() / h)
            .clamp(CANVAS_ZOOM_RANGE.0, CANVAS_ZOOM_RANGE.1);
        view.set((-(min.0 - 40.0) * k, -(min.1 - 40.0) * k, k));
    };

    let straighten = move |part_index: usize, slot: usize| {
        checkpoint();
        parts.update(|list| edit::straighten(list, part_index, slot));
        dirty.set(true);
    };
    let disconnect = move |part_index: usize, slot: usize| {
        checkpoint();
        parts.update(|list| edit::disconnect(list, part_index, slot));
        selected_wire.set(None);
        dirty.set(true);
    };
    let remove_part = move |index: usize| {
        checkpoint();
        parts.update(|list| edit::remove(list, index));
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
        parts.update(|list| edit::remove_many(list, &group));
        selected.set(None);
        selected_wire.set(None);
        marked.set(Vec::new());
        dirty.set(true);
    };
    let duplicate_part = move |index: usize| {
        checkpoint();
        parts.update(|list| selected.set(edit::duplicate(list, index)));
        dirty.set(true);
    };

    // Disconnect the selected wire, or remove the selected part — through the
    // same two commands the menu uses, rather than a third copy of each. The
    // copies had already drifted: this one never cleared `selected_wire` on
    // the delete path.
    let delete_selection = move || {
        if let Some((part, slot)) = selected_wire.get_untracked() {
            disconnect(part, slot);
        } else if marked.with_untracked(|m| m.len() > 1) {
            remove_marked();
        } else if let Some(index) = selected.get_untracked() {
            remove_part(index);
        }
    };

    view! {
        <div class="flex min-h-0 flex-1 flex-col">

            <div class="flex min-h-0 flex-1">
                <Library user_parts=user_parts on_add=Callback::new(move |(kind, label): (PartKind, String)| add_part(kind, label)) />

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
                                hover_stub.set(None);
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
                            // just another rotation is in `edit::flip`.
                            "x" | "X" if !event.ctrl_key() => {
                                if let Some(index) = selected.get_untracked() {
                                    event.prevent_default();
                                    flip_part(index);
                                }
                            }
                            "f" | "F" if !event.ctrl_key() => {
                                event.prevent_default();
                                fit_view();
                            }
                            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown"
                                if selected.get_untracked().is_some() =>
                            {
                                {
                                    event.prevent_default();
                                    // Shift for the fine grid, as every
                                    // drawing tool spells it.
                                    let step = if event.shift_key() { 1.0 } else { SNAP };
                                    let (dx, dy) = match event.key().as_str() {
                                        "ArrowLeft" => (-step, 0.0),
                                        "ArrowRight" => (step, 0.0),
                                        "ArrowUp" => (0.0, -step),
                                        _ => (0.0, step),
                                    };
                                    nudge(dx, dy);
                                }
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
                        if let Some((kind, label)) = placing.get_untracked() {
                            event.prevent_default();
                            let step = grid.get_untracked();
                            let world = to_world(
                                f64::from(event.client_x()),
                                f64::from(event.client_y()),
                            );
                            drop_part(
                                kind,
                                label,
                                snap_to(world.0, step),
                                snap_to(world.1, step),
                            );
                            placing.set(None);
                            place_at.set(None);
                            return;
                        }
                        if let Some(element) = canvas.get_untracked()
                            && let Some(target) = event.target()
                            && let Ok(node) = wasm_bindgen::JsCast::dyn_into::<web_sys::Node>(target)
                            && (element.is_same_node(Some(&node))
                                || node.node_name() == "svg"
                                || node.node_name() == "rect")
                        {
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
                            Drag::Part {
                                index,
                                dx,
                                dy,
                                axes,
                                from,
                            } => {
                                let step = grid.get_untracked();
                                let mut x = snap_to(world.0 - dx, step);
                                let mut y = snap_to(world.1 - dy, step);
                                // Line up with what is already on the sheet.
                                // Alignment that only the grid enforces is
                                // alignment nobody can see.
                                let mut guide_x = None;
                                let mut guide_y = None;
                                let (kx, ky) = kit_pos.get_untracked();
                                let others: Vec<(f64, f64)> = parts
                                    .get_untracked()
                                    .iter()
                                    .enumerate()
                                    .filter(|(other, _)| *other != index)
                                    .map(|(_, part)| (part.x, part.y))
                                    .chain(std::iter::once((kx, ky)))
                                    .collect();
                                for (ox, oy) in others {
                                    if (ox - x).abs() <= SNAP {
                                        // Edge alignment stays on the base
                                        // step whatever the dial says: lining
                                        // up with a neighbour is the intent
                                        // fine grids exist to serve.

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
                                        // Bends belong to the sheet, exactly as
                                        // in KiCad: moving a part stretches only
                                        // the segment from its stub to the first
                                        // bend, and every later bend stays put.
                                        // "Stretches" is literal — the first
                                        // bend slides along the segment's own
                                        // axis, so the segment changes length,
                                        // not direction. Left planted on both
                                        // axes, a bend at the old height grows
                                        // a wall of wire back up to where the
                                        // part used to be.
                                        part.x = x;
                                        part.y = y;
                                        let stubs: Vec<(f64, f64)> = (0..part.kind.wires())
                                            .map(|slot| stub_point(part, slot))
                                            .collect();
                                        for (slot, stub) in stubs.into_iter().enumerate() {
                                            if let Some(first) =
                                                part.waypoints[slot].first_mut()
                                            {
                                                follow_first_bend(stub, axes[slot], first);
                                            }
                                        }
                                    }
                                });
                                // The rest of the group follows by the same
                                // displacement, each from where it stood.
                                if !group_start.with_untracked(Vec::is_empty) {
                                    let group = group_start.get_untracked();
                                    parts.update(|list| {
                                        edit::translate(list, &group, x - from.0, y - from.1);
                                    });
                                }
                                dirty.set(true);
                            }
                            Drag::Kit { dx, dy, .. } => {
                                let step = grid.get_untracked();
                                kit_pos.set((
                                    snap_to(world.0 - dx, step),
                                    snap_to(world.1 - dy, step),
                                ));
                                dirty.set(true);
                            }
                            Drag::Wire { .. } => {
                                ghost.set(Some(world));
                                let drawn = rows.get_untracked();
                                let row = row_under(kit_pos.get_untracked(), drawn.len(), world)
                                    .filter(|r| drawn.get(*r).is_some_and(|r| r.1.is_some()));
                                hover_row.set(row);
                            }
                            Drag::WireFromPin { .. } => {
                                ghost.set(Some(world));
                                // Generous, and in sheet units so it does not
                                // shrink as the view zooms out: a stub is a
                                // 9px dot and the hand has a wire to mind.
                                hover_stub.set(
                                    parts.with_untracked(|list| stub_under(list, world, 14.0)),
                                );
                            }
                            Drag::Box { .. } => {
                                box_to.set(Some(world));
                            }
                            Drag::Segment {
                                part,
                                slot,
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
                                // within a step of the stub's or the pin's
                                // coordinate, land exactly on it — that is
                                // the alignment the drag was reaching for,
                                // and the simplifier then merges the runs.
                                let anchors = parts.with_untracked(|list| {
                                    let p = list.get(part)?;
                                    let stub = stub_point(p, slot);
                                    let pin = row_of_gpio(&rows.get_untracked(), p.pins[slot])
                                        .map(|row| row_point(kit_pos.get_untracked(), rows.get_untracked().len(), row));
                                    Some((stub, pin))
                                });
                                if let Some((stub, pin)) = anchors {
                                    let candidates = [
                                        Some(if horizontal { stub.1 } else { stub.0 }),
                                        pin.map(|p| if horizontal { p.1 } else { p.0 }),
                                    ];
                                    for anchor in candidates.into_iter().flatten() {
                                        if (value - anchor).abs() <= step.max(SNAP) {
                                            value = anchor;
                                        }
                                    }
                                }
                                parts.update(|list| {
                                    if let Some(p) = list.get_mut(part) {
                                        for index in [first, second] {
                                            if let Some(point) =
                                                p.waypoints[slot].get_mut(index)
                                            {
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
                        if let Some(Drag::Segment { part, slot, .. }) = drag.get_untracked() {
                            let kit = kit_pos.get_untracked();
                            parts.update(|list| {
                                if let Some(p) = list.get_mut(part) {
                                    retidy(p, slot, kit, &rows.get_untracked());
                                }
                            });
                        }
                        // A finished part drag tidies every route it stretched:
                        // a first bend that slid into line with the next one
                        // merges away instead of surviving as a zero-length
                        // grab target.
                        if let Some(Drag::Part { index, .. }) = drag.get_untracked() {
                            let kit = kit_pos.get_untracked();
                            let drawn = rows.get_untracked();
                            let mut moved: Vec<usize> = group_start
                                .get_untracked()
                                .iter()
                                .map(|(i, _, _)| *i)
                                .collect();
                            moved.push(index);
                            parts.update(|list| {
                                for index in moved {
                                    if let Some(p) = list.get_mut(index) {
                                        for slot in 0..p.kind.wires() {
                                            if p.waypoints[slot].is_empty() {
                                                continue;
                                            }
                                            retidy(p, slot, kit, &drawn);
                                        }
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
                        // A wire pulled from a chip pin lands on a stub: the
                        // same assignment as the other direction, so the two
                        // gestures cannot disagree about what wiring means.
                        if let Some(Drag::WireFromPin { row }) = drag.get_untracked() {
                            if let Some((part, slot)) = hover_stub.get_untracked()
                                && let Some(gpio) = rows.get_untracked().get(row).and_then(|r| r.1)
                            {
                                checkpoint();
                                parts.update(|list| {
                                    if let Some(p) = list.get_mut(part) {
                                        p.pins[slot] = gpio;
                                        p.waypoints[slot].clear();
                                        if p.kind.wires() == 1
                                            && edit::is_auto_label(&p.kind, &p.label)
                                        {
                                            p.label = single_pin_label(&p.kind, gpio);
                                        }
                                    }
                                });
                                selected.set(Some(part));
                                marked.set(vec![part]);
                                selected_wire.set(Some((part, slot)));
                                dirty.set(true);
                            }
                            hover_stub.set(None);
                        }
                        if let Some(Drag::Wire { part, slot }) = drag.get_untracked() {
                            // Landing on a GPIO row wires the pin; anywhere
                            // else cancels. Wiring IS pin assignment.
                            if let Some(row) = hover_row.get_untracked()
                                && let Some(gpio) = rows.get_untracked().get(row).and_then(|r| r.1)
                            {
                                checkpoint();
                                parts.update(|list| {
                                    if let Some(p) = list.get_mut(part) {
                                        p.pins[slot] = gpio;
                                        p.waypoints[slot].clear();
                                        // A name the user typed outlives a
                                        // rewire; the editor's own follows it.
                                        if p.kind.wires() == 1
                                            && edit::is_auto_label(&p.kind, &p.label)
                                        {
                                            p.label = single_pin_label(&p.kind, gpio);
                                        }
                                    }
                                });
                                selected_wire.set(Some((part, slot)));
                                dirty.set(true);
                            }
                        }
                        ghost.set(None);
                        hover_row.set(None);
                        hover_stub.set(None);
                        box_to.set(None);
                        drag.set(None);
                    }
                    on:pointerleave=move |_| {
                        guides.set((None, None));
                        ghost.set(None);
                        hover_row.set(None);
                        hover_stub.set(None);
                        box_to.set(None);
                        group_start.set(Vec::new());
                        drag.set(None);
                    }
                    class="relative min-w-0 flex-1 overflow-hidden bg-[#101216] outline-none"
                >
                    // The sheet's own controls, in its corner as every
                    // schematic editor keeps them: Save once there is
                    // something to save, undo and redo, zoom, fit, the snap
                    // grid. They were in the rail, where a zoom percentage sat
                    // in a 46px column between Run and Stop. Pointer events
                    // stop here, or a press on Zoom would also be a press on
                    // the sheet under it — dropping an armed part, or starting
                    // a pan.
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
                            // leading-none, or the caption line box out-talls the
                            // icon and the digit prints below the glyph's centre.
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
                        // Two layers, because a part must not be able to
                        // hide a wire: the grid sits under everything, the
                        // wires over everything. Neither takes the pointer
                        // except where a wire's own grab handle says so.
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

                        // the devkit — labelled pins, drop targets, draggable
                        {move || {
                            let (kx, ky) = kit_pos.get();
                            let hovered = hover_row.get();
                            // Drawn from the part's own pins: the height, the
                            // split down the middle, and the labels all follow
                            // from how many it has.
                            let drawn = rows.get();
                            let per_side = left_rows(drawn.len()).max(1);
                            let kit_h = kit_height(drawn.len());
                            view! {
                                <div
                                    on:pointerdown=move |event: ev::PointerEvent| {
                                        if event.button() != 0 {
                                            return;
                                        }
                                        event.prevent_default();
                                        event.stop_propagation();
                                        checkpoint();
                                        let world = to_world(
                                            f64::from(event.client_x()),
                                            f64::from(event.client_y()),
                                        );
                                        let (kx, ky) = kit_pos.get_untracked();
                                        drag.set(Some(Drag::Kit {
                                            dx: world.0 - kx,
                                            dy: world.1 - ky,
                                            from: (kx, ky),
                                        }));
                                    }
                                    class="absolute cursor-grab"
                                    style=format!("left: {kx}px; top: {ky}px")
                                >
                                    <svg
                                        width=KIT_W
                                        height=kit_h
                                        viewBox=format!("0 0 {KIT_W} {kit_h}")
                                    >
                                        <rect x="4" y="2" width=KIT_W - 8.0 height=kit_h - 4.0 rx="10" fill="#1a1d23" stroke="#454b56" stroke-width="1.5" />
                                        {drawn
                                            .into_iter()
                                            .enumerate()
                                            .map(|(row, (name, gpio))| {
                                                let left = row < per_side;
                                                let y = 16 + (row % per_side) as i32 * ROW_PITCH as i32;
                                                let cx = if left { 10.0 } else { KIT_W - 10.0 };
                                                let hot = hovered == Some(row);
                                                let fill = if hot {
                                                    "#e0a838"
                                                } else if gpio.is_some() {
                                                    "#c9a227"
                                                } else {
                                                    "#5a5142"
                                                };
                                                let r = if hot { 5.0 } else { 3.2 };
                                                // A press on a wireable pin starts
                                                // a wire from the chip's side — and
                                                // must not start the devkit's drag.
                                                let wireable = gpio.is_some();
                                                view! {
                                                    <circle
                                                        cx=cx
                                                        cy=y
                                                        r=r
                                                        fill=fill
                                                        style=if wireable { "cursor: crosshair" } else { "" }
                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                            if !wireable || event.button() != 0 {
                                                                return;
                                                            }
                                                            event.prevent_default();
                                                            event.stop_propagation();
                                                            selected.set(None);
                                                            selected_wire.set(None);
                                                            hover_row.set(Some(row));
                                                            drag.set(Some(Drag::WireFromPin { row }));
                                                        }
                                                    />
                                                    <text
                                                        x=if left { 18.0 } else { KIT_W - 18.0 }
                                                        y=y + 3
                                                        text-anchor=if left { "start" } else { "end" }
                                                        font-family="ui-monospace"
                                                        font-size="7.5"
                                                        fill="#98a1ae"
                                                    >
                                                        {name}
                                                    </text>
                                                }
                                            })
                                            .collect_view()}
                                        <rect x="42" y="12" width=KIT_W - 84.0 height="84" rx="4" fill="#2e333b" stroke="#4a515d" />
                                        <text x=KIT_W / 2.0 y="58" text-anchor="middle" font-family="ui-monospace" font-size="12" fill="#aab3c0">
                                            {chip_label.clone()}
                                        </text>
                                        <rect x=KIT_W / 2.0 - 15.0 y=kit_h - 22.0 width="30" height="14" rx="2" fill="#3a3e46" />
                                    </svg>
                                </div>
                            }
                        }}

                        // One view per part, keyed by index, and every field
                        // a view reads comes through its own memo — so a drag
                        // frame touches one part's `style` and nothing else.
                        // This used to rebuild every part's DOM on every
                        // pointer move: fine with three parts, a stutter with
                        // thirty, and the reason a hover on one stub could
                        // not be cheap.
                        <For
                            each=move || 0..parts.with(Vec::len)
                            key=|index| *index
                            children=move |index: usize| {
                                let this = Memo::new(move |_| {
                                    parts.with(|list| list.get(index).cloned())
                                });
                                let kind = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().map(|p| p.kind.clone()))
                                });
                                let pins = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().map_or([UNWIRED; 7], |p| p.pins))
                                });
                                let place = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().map(|p| (p.x, p.y, p.rot, p.flip)))
                                });
                                let label = Memo::new(move |_| {
                                    this.with(|p| {
                                        p.as_ref().map(|p| p.label.clone()).unwrap_or_default()
                                    })
                                });
                                let active_low = Memo::new(move |_| {
                                    this.with(|p| p.as_ref().is_some_and(|p| p.active_low))
                                });
                                let pin = move |slot: usize| pins.get()[slot];
                                // Mirroring the body mirrors its writing too;
                                // readouts and the label undo whatever the
                                // body did, the rule that keeps a turned part
                                // readable.
                                let readable = move || {
                                    let (_, _, rot, flip) =
                                        place.get().unwrap_or((0.0, 0.0, 0, false));
                                    match (rot == 180, flip) {
                                        (true, true) => "transform: rotate(180deg) scaleX(-1)",
                                        (true, false) => "transform: rotate(180deg)",
                                        (false, true) => "transform: scaleX(-1)",
                                        (false, false) => "",
                                    }
                                };
                                let is_selected =
                                    Signal::derive(move || selected.get() == Some(index));
                                let is_marked =
                                    Signal::derive(move || marked.with(|m| m.contains(&index)));
                                let level = move |pin: u8| {
                                    state
                                        .sim
                                        .gpio
                                        .with(|gpio| gpio.get(&pin).copied().unwrap_or(false))
                                };
                                // `None` means the firmware has never reported
                                // a duty for this pin, which is not the same
                                // as reporting zero — see the field's own
                                // note. A motor draws the difference.
                                let duty = move |pin: u8| {
                                    state.sim.pwm.with(|pwm| pwm.get(&pin).copied())
                                };
                                // What the firmware set, read through the
                                // part's wiring: a lamp wired active-low is
                                // lit when its pin is low, and a pull-up
                                // button is pressed when its pin is low.
                                // Drawing the level itself was the confident
                                // wrong answer for both.
                                let lit = move |slot: usize| level(pin(slot)) != active_low.get();

                                // The face follows the kind and only the kind;
                                // what it shows follows the pins and the
                                // firmware through the closures inside it.
                                let face = move || {
                                    let Some(kind) = kind.get() else {
                                        return ().into_any();
                                    };
                                    match kind {
                                        PartKind::Led { color } => {
                                            let (on, off) = lamp_colors(&color);
                                            lamp_dome(move || if lit(0) { on } else { off }, move || lit(0))
                                        }
                                        PartKind::Rgb => lamp_dome(
                                            move || rgb_color(lit(0), lit(1), lit(2)),
                                            move || lit(0) || lit(1) || lit(2),
                                        ),
                                        PartKind::Seven => view! {
                                            // A digit mounted upside-down
                                            // still reads upright — KiCad
                                            // keeps symbol text readable
                                            // whatever the body does.
                                            <svg
                                                width="26"
                                                height="42"
                                                viewBox="0 0 26 42"
                                                class="shrink-0"
                                                style=readable
                                            >
                                                <rect x="0" y="0" width="26" height="42" rx="3" fill="#1a1114" />
                                                {
                                                    let seg = move |slot: usize| {
                                                        if lit(slot) { "#ff5c5c" } else { "#3a2323" }
                                                    };
                                                    view! {
                                                        <rect x="6" y="2" width="14" height="4" rx="2" fill=move || seg(0) />
                                                        <rect x="19" y="5" width="4" height="13" rx="2" fill=move || seg(1) />
                                                        <rect x="19" y="23" width="4" height="13" rx="2" fill=move || seg(2) />
                                                        <rect x="6" y="36" width="14" height="4" rx="2" fill=move || seg(3) />
                                                        <rect x="3" y="23" width="4" height="13" rx="2" fill=move || seg(4) />
                                                        <rect x="3" y="5" width="4" height="13" rx="2" fill=move || seg(5) />
                                                        <rect x="6" y="19" width="14" height="4" rx="2" fill=move || seg(6) />
                                                    }
                                                }
                                            </svg>
                                        }
                                            .into_any(),
                                        PartKind::Display => view! {
                                            <span
                                                class="grid min-h-[34px] min-w-[110px] place-items-center rounded-[4px] bg-[#0d1a12] px-2 py-1 font-mono text-caption text-[#3ddc84] ring-1 ring-[#1d4a2f]"
                                                style=readable
                                            >
                                                {move || {
                                                    let text = state.sim.display.get();
                                                    if text.is_empty() {
                                                        "········".to_string()
                                                    } else {
                                                        text
                                                    }
                                                }}
                                            </span>
                                        }
                                            .into_any(),
                                        PartKind::Motor => {
                                            // A fan has no direction pins, and
                                            // reading two unwired inputs as two
                                            // lows would call it COAST and stop
                                            // a motor that has nowhere to say
                                            // otherwise.
                                            let bridged =
                                                move || pin(1) != UNWIRED || pin(2) != UNWIRED;
                                            let drive = move || {
                                                if bridged() {
                                                    rusty_embed::Drive::from_inputs(
                                                        level(pin(1)),
                                                        level(pin(2)),
                                                    )
                                                } else {
                                                    rusty_embed::Drive::Forward
                                                }
                                            };
                                            // Faster duty, faster rotor. Capped
                                            // at four turns a second because
                                            // past that a spoke reads as a blur
                                            // and the direction stops being
                                            // legible, which is the one thing
                                            // this drawing is for.
                                            let spin = move || match duty(pin(0)) {
                                                Some(d) if d > 0.01 && drive().turns() => {
                                                    let seconds = (0.25 / d).clamp(0.25, 4.0);
                                                    let way = match drive() {
                                                        rusty_embed::Drive::Reverse => "reverse",
                                                        _ => "normal",
                                                    };
                                                    format!(
                                                        "animation-duration: {seconds:.2}s; \
                                                         animation-direction: {way}",
                                                    )
                                                }
                                                // Still, and for two different
                                                // reasons the readout spells out.
                                                _ => "animation: none".to_string(),
                                            };
                                            let readout = move || match duty(pin(0)) {
                                                None => t!("simulate.no-duty"),
                                                Some(d) => {
                                                    format!("{:.0}% {}", d * 100.0, drive().label())
                                                }
                                            };
                                            let tone = move || match duty(pin(0)) {
                                                // Never reported: the same grey
                                                // every other "rusty does not
                                                // know" reads in.
                                                None => "text-label-3",
                                                Some(_) if !drive().turns() => "text-label-2",
                                                Some(_) => "text-label",
                                            };
                                            view! {
                                                <span class="flex items-center gap-2">
                                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full border border-line-strong bg-sunken">
                                                        <span
                                                            class="absolute inset-[3px] animate-spin"
                                                            style=spin
                                                        >
                                                            <span class="absolute top-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-label-2" />
                                                            <span class="absolute bottom-0 left-1/2 h-1/2 w-px -translate-x-1/2 bg-line-strong" />
                                                        </span>
                                                        <span class="size-1 rounded-full bg-label-3" />
                                                    </span>
                                                    <span class=move || {
                                                        format!("font-mono text-caption {}", tone())
                                                    }>{readout}</span>
                                                </span>
                                            }
                                                .into_any()
                                        }
                                        PartKind::Analog => {
                                            // Counts, and the count is what is
                                            // shown. rusty does not know the
                                            // divider on this board, so it
                                            // does not print a voltage it
                                            // cannot stand behind.
                                            let held = move || {
                                                state
                                                    .sim
                                                    .analog
                                                    .with(|a| a.get(&pin(0)).copied().unwrap_or(0))
                                            };
                                            view! {
                                                <span class="flex items-center gap-2">
                                                    <input
                                                        type="range"
                                                        min="0"
                                                        max="4095"
                                                        prop:value=move || held().to_string()
                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                            event.stop_propagation();
                                                        }
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) =
                                                                event_target_value(&event).parse::<u16>()
                                                            {
                                                                controller::sim_analog(state, pin(0), value);
                                                            }
                                                        }
                                                        class="w-[68px] accent-[#4aa8ff]"
                                                    />
                                                    <span class="w-[4ch] text-right font-mono text-caption text-label-2">
                                                        {held}
                                                    </span>
                                                </span>
                                            }
                                                .into_any()
                                        }
                                        PartKind::Pot => {
                                            // The knob turns with the slider: a
                                            // potentiometer is a shaft, and its
                                            // angle is the reading at a glance.
                                            let turned = RwSignal::new(128u8);
                                            let angle = move || {
                                                -135.0 + f64::from(turned.get()) / 255.0 * 270.0
                                            };
                                            view! {
                                                <span class="flex items-center gap-1.5">
                                                    <span class="relative grid size-5 shrink-0 place-items-center rounded-full bg-[#3a404a] ring-1 ring-[#5a626e]">
                                                        <span
                                                            class="absolute top-[3px] left-1/2 h-[7px] w-[2px] rounded-full bg-[#c9a227]"
                                                            style=move || {
                                                                format!(
                                                                    "transform-origin: 1px 7px; transform: translateX(-1px) rotate({:.0}deg)",
                                                                    angle(),
                                                                )
                                                            }
                                                        />
                                                    </span>
                                                    <input
                                                        type="range"
                                                        min="0"
                                                        max="255"
                                                        value="128"
                                                        on:pointerdown=move |event: ev::PointerEvent| {
                                                            event.stop_propagation();
                                                        }
                                                        on:input=move |event: ev::Event| {
                                                            if let Ok(value) =
                                                                event_target_value(&event).parse::<u8>()
                                                            {
                                                                turned.set(value);
                                                                controller::sim_pot(state, pin(0), value);
                                                            }
                                                        }
                                                        class="w-[56px] accent-[#c9a227]"
                                                    />
                                                </span>
                                            }
                                                .into_any()
                                        }
                                        PartKind::Button => {
                                            // The press the button itself
                                            // shows. A button is an input:
                                            // waiting for the firmware to
                                            // report it back meant no click
                                            // ever looked like anything.
                                            let held = RwSignal::new(false);
                                            view! {
                                                <span
                                                    on:pointerdown=move |event: ev::PointerEvent| {
                                                        event.stop_propagation();
                                                        held.set(true);
                                                        if running.get_untracked() {
                                                            controller::sim_press(state, pin(0), true);
                                                        }
                                                    }
                                                    on:pointerup=move |_| {
                                                        held.set(false);
                                                        if running.get_untracked() {
                                                            controller::sim_press(state, pin(0), false);
                                                        }
                                                    }
                                                    on:pointerleave=move |_| {
                                                        if held.get_untracked() {
                                                            held.set(false);
                                                            if running.get_untracked() {
                                                                controller::sim_press(state, pin(0), false);
                                                            }
                                                        }
                                                    }
                                                    class=move || {
                                                        let pressed =
                                                            held.get() || (running.get() && lit(0));
                                                        // The cap sinks as well as
                                                        // colouring: a tactile switch
                                                        // moves, and the eye reads the
                                                        // movement before the colour.
                                                        format!(
                                                            "grid size-5 shrink-0 cursor-pointer place-items-center rounded-[5px] ring-1 ring-[#5a626e] transition-transform duration-75 {}",
                                                            if pressed {
                                                                "bg-rust scale-90"
                                                            } else {
                                                                "bg-[#3a404a]"
                                                            },
                                                        )
                                                    }
                                                >
                                                    <span class="size-2 rounded-full bg-[#9aa3b0]" />
                                                </span>
                                            }
                                                .into_any()
                                        }
                                    }
                                };

                                // The gold dots wires pull out of — and, on a
                                // part with more than one, the name beside
                                // each, as KiCad names its pins: which dot is
                                // `b` has to be readable before the wire lands.
                                let stubs = move || {
                                    let Some(kind) = kind.get() else {
                                        return ().into_any();
                                    };
                                    let wires = kind.wires();
                                    (0..wires)
                                        .map(|slot| {
                                            // Centre the 9px dot exactly on the
                                            // wire's anchor: same constants as
                                            // stub_point, so the dot and the wire
                                            // cannot disagree again.
                                            let top = STUB_OFFSET - 4.5 + slot as f64 * SLOT_PITCH;
                                            let name = (wires > 1).then(|| {
                                                let name = stub_names(&kind)[slot];
                                                view! {
                                                    <span
                                                        class="pointer-events-none absolute font-mono text-[7px] leading-none text-[#98a1ae]"
                                                        style=move || {
                                                            format!("right: 8px; top: {}px; {}", top + 1.0, readable())
                                                        }
                                                    >
                                                        {name}
                                                    </span>
                                                }
                                            });
                                            view! {
                                                {name}
                                                <span
                                                    title=move || {
                                                        if pin(slot) == UNWIRED {
                                                            t!("simulate.stub-unwired")
                                                        } else {
                                                            t!("simulate.stub-wired")
                                                        }
                                                    }
                                                    on:pointerdown=move |event: ev::PointerEvent| {
                                                        event.prevent_default();
                                                        event.stop_propagation();
                                                        selected.set(Some(index));
                                                        selected_wire.set(None);
                                                        drag.set(Some(Drag::Wire { part: index, slot }));
                                                    }
                                                    class=move || {
                                                        // Lit up while a wire pulled from
                                                        // a chip pin hovers over it: the
                                                        // one dot that will take it.
                                                        let target = hover_stub.get() == Some((index, slot));
                                                        let unwired = pin(slot) == UNWIRED;
                                                        format!(
                                                            "absolute size-[9px] cursor-crosshair rounded-full ring-1 ring-[#101216] transition-transform {}",
                                                            if target {
                                                                "scale-150 bg-[#ffd75c]"
                                                            } else if unwired {
                                                                "animate-pulse bg-[#e0a838]"
                                                            } else {
                                                                "bg-[#c9a227]"
                                                            },
                                                        )
                                                    }
                                                    style=format!("right: -4.5px; top: {top}px")
                                                />
                                            }
                                        })
                                        .collect_view()
                                        .into_any()
                                };

                                view! {
                                    <div
                                        on:contextmenu=move |event: ev::MouseEvent| {
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
                                        }
                                        on:pointerdown=move |event: ev::PointerEvent| {
                                            if event.button() != 0 {
                                                return;
                                            }
                                            event.prevent_default();
                                            event.stop_propagation();
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
                                            if matches!(kind.get_untracked(), Some(PartKind::Button))
                                                && running.get_untracked()
                                            {
                                                return;
                                            }
                                            checkpoint();
                                            let world = to_world(
                                                f64::from(event.client_x()),
                                                f64::from(event.client_y()),
                                            );
                                            let (x, y, _, _) = place.get_untracked().unwrap_or_default();
                                            // Judge each route's first-leg axis now,
                                            // once: judged live it would flip as the
                                            // part crosses its own bend.
                                            let axes = parts
                                                .with_untracked(|list| list.get(index).map(first_leg_axes))
                                                .unwrap_or([None; 7]);
                                            // The rest of the group starts where it
                                            // stands; every frame moves it by the
                                            // grabbed part's displacement.
                                            group_start.set(parts.with_untracked(|list| {
                                                marked.with_untracked(|m| {
                                                    m.iter()
                                                        .filter(|i| **i != index)
                                                        .filter_map(|i| {
                                                            list.get(*i).map(|p| {
                                                                (*i, (p.x, p.y), first_leg_axes(p))
                                                            })
                                                        })
                                                        .collect()
                                                })
                                            }));
                                            drag.set(Some(Drag::Part {
                                                index,
                                                dx: world.0 - x,
                                                dy: world.1 - y,
                                                axes,
                                                from: (x, y),
                                            }));
                                        }
                                        class=move || {
                                            let ring = if is_selected.get() {
                                                "ring-2 ring-rust"
                                            } else if is_marked.get() {
                                                "ring-2 ring-rust/60"
                                            } else {
                                                "ring-1 ring-[#515a68]"
                                            };
                                            // Lifted while it moves: a shadow and
                                            // the grabbing cursor say the part is
                                            // in the hand; the dashed footprint
                                            // drawn below says where it came from.
                                            let lifted = match drag.get() {
                                                Some(Drag::Part { index: moving, .. }) => {
                                                    moving == index
                                                        || group_start
                                                            .with(|g| g.iter().any(|(i, _, _)| *i == index))
                                                }
                                                _ => false,
                                            };
                                            let lift = if lifted {
                                                "z-20 cursor-grabbing opacity-90 shadow-[0_12px_28px_rgba(0,0,0,0.55)]"
                                            } else {
                                                "cursor-grab"
                                            };
                                            format!(
                                                "absolute flex items-center gap-1.5 rounded-[8px] bg-[#2c313a] px-1.5 py-1 select-none {ring} {lift}",
                                            )
                                        }
                                        style=move || {
                                            let (x, y, rot, flip) = place.get().unwrap_or_default();
                                            let (width, height) = kind
                                                .get()
                                                .map(|k| (k.width(), k.height()))
                                                .unwrap_or_default();
                                            format!(
                                                "left: {x}px; top: {y}px; width: {width}px; \
                                                 height: {height}px; transform: rotate({rot}deg){}",
                                                if flip { " scaleX(-1)" } else { "" },
                                            )
                                        }
                                    >
                                        {face}
                                        // The label counter-rotates with the
                                        // readouts: the body turns, the writing
                                        // stays readable.
                                        <span
                                            class="min-w-0 flex-1 truncate font-mono text-caption text-[#d7dce3]"
                                            style=readable
                                        >
                                            {move || label.get()}
                                        </span>
                                        {stubs}
                                    </div>
                                }
                            }
                        />

                        // The footprint a moving part left behind — KiCad's
                        // ghost. Drawn only once the part has actually moved,
                        // so a click that never becomes a drag flashes nothing.
                        {move || {
                            let (from, width, height, rot, flip, moved) = match drag.get()? {
                                Drag::Part { index, from, .. } => {
                                    let part = parts.with(|list| list.get(index).cloned())?;
                                    let moved = (part.x, part.y) != from;
                                    (
                                        from,
                                        part.kind.width(),
                                        part.kind.height(),
                                        part.rot,
                                        part.flip,
                                        moved,
                                    )
                                }
                                Drag::Kit { from, .. } => {
                                    let here = kit_pos.get();
                                    (
                                        from,
                                        KIT_W,
                                        kit_height(rows.get().len()),
                                        0,
                                        false,
                                        here != from,
                                    )
                                }
                                _ => return None,
                            };
                            moved.then(|| {
                                view! {
                                    <div
                                        class="pointer-events-none absolute rounded-[8px] border-2 border-dashed border-[#e05d38]/60"
                                        style=format!(
                                            "left: {}px; top: {}px; width: {width}px; height: {height}px; \
                                             transform: rotate({rot}deg){}",
                                            from.0,
                                            from.1,
                                            if flip { " scaleX(-1)" } else { "" },
                                        )
                                    />
                                }
                            })
                        }}

                        <svg
                            class="pointer-events-none absolute"
                            style="left: -2000px; top: -2000px"
                            width="6000"
                            height="6000"
                        >
                            <g transform="translate(2000, 2000)">
                                // ── wires: grab a segment, push it ──────────
                                {move || {
                                    let kit = kit_pos.get();
                                    let picked = selected_wire.get();
                                    let hovered = hover_wire.get();
                                    parts
                                        .get()
                                        .iter()
                                        .enumerate()
                                        .flat_map(|(part_index, part)| {
                                            (0..part.kind.wires())
                                                .filter_map(|slot| {
                                                    let points = wire_path(part, slot, kit, &rows.get_untracked())?;
                                                    let from = points[0];
                                                    let to = *points.last()?;
                                                    let is_picked =
                                                        picked == Some((part_index, slot));
                                                    // Brightens under the pointer, so
                                                    // the wire about to be grabbed is
                                                    // the one that answers.
                                                    let is_hovered =
                                                        hovered == Some((part_index, slot));
                                                    let stroke = if is_picked {
                                                        "#e05d38"
                                                    } else if is_hovered {
                                                        "#b7c0cc"
                                                    } else {
                                                        "#7d8694"
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

                                                    // One grab handle per
                                                    // segment. Pushing a
                                                    // segment is how a
                                                    // schematic editor moves
                                                    // a corner — you never
                                                    // hunt for the vertex.
                                                    let grabs = points
                                                        .windows(2)
                                                        .enumerate()
                                                        .map(|(seg, pair)| {
                                                            let (a, b) = (pair[0], pair[1]);
                                                            let horizontal =
                                                                (a.1 - b.1).abs() < 0.5;
                                                            let cursor = if horizontal {
                                                                "row-resize"
                                                            } else {
                                                                "col-resize"
                                                            };
                                                            let drawn = points.clone();
                                                            let menu_at = menu;
                                                            view! {
                                                                <line
                                                                    x1=a.0
                                                                    y1=a.1
                                                                    x2=b.0
                                                                    y2=b.1
                                                                    stroke="transparent"
                                                                    stroke-width="12"
                                                                    style=format!(
                                                                        "pointer-events: stroke; cursor: {cursor}",
                                                                    )
                                                                    on:pointerenter=move |_| {
                                                                        hover_wire.set(Some((part_index, slot)))
                                                                    }
                                                                    on:pointerleave=move |_| hover_wire.set(None)
                                                                    on:contextmenu=move |event: ev::MouseEvent| {
                                                                        event.prevent_default();
                                                                        event.stop_propagation();
                                                                        selected.set(None);
                                                                        selected_wire
                                                                            .set(Some((part_index, slot)));
                                                                        menu_at
                                                                            .set(Some((
                                                                                f64::from(event.client_x()),
                                                                                f64::from(event.client_y()),
                                                                                MenuTarget::Wire(part_index, slot),
                                                                            )));
                                                                    }
                                                                    on:pointerdown=move |event: ev::PointerEvent| {
                                                                        if event.button() != 0 {
                                                                            return;
                                                                        }
                                                                        event.prevent_default();
                                                                        event.stop_propagation();
                                                                        selected.set(None);
                                                                        selected_wire
                                                                            .set(Some((part_index, slot)));
                                                                        if let Some(element) =
                                                                            canvas.get_untracked()
                                                                        {
                                                                            let _ = element.focus();
                                                                        }
                                                                        checkpoint();
                                                                        // Freeze the drawn path
                                                                        // into real bends, so the
                                                                        // segment has movable
                                                                        // points on both sides —
                                                                        // and the two anchored
                                                                        // ends stay put by
                                                                        // growing an elbow.
                                                                        let ends = parts
                                                                            .try_update(|list| {
                                                                                let p = list.get_mut(part_index)?;
                                                                                let mut inner: Vec<(f64, f64)> = drawn
                                                                                    [1..drawn.len() - 1]
                                                                                    .to_vec();
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
                                                                                p.waypoints[slot] = inner;
                                                                                Some((first as usize, second as usize))
                                                                            })
                                                                            .flatten();
                                                                        if let Some((first, second)) = ends {
                                                                            let world = to_world(
                                                                                f64::from(event.client_x()),
                                                                                f64::from(event.client_y()),
                                                                            );
                                                                            drag.set(Some(Drag::Segment {
                                                                                part: part_index,
                                                                                slot,
                                                                                first,
                                                                                second,
                                                                                horizontal,
                                                                                grab: if horizontal {
                                                                                    world.1
                                                                                } else {
                                                                                    world.0
                                                                                },
                                                                                base: if horizontal { a.1 } else { a.0 },
                                                                            }));
                                                                        }
                                                                    }
                                                                />
                                                            }
                                                        })
                                                        .collect_view();

                                                    // Corner pips, so the
                                                    // selected wire shows
                                                    // where its bends are.
                                                    let bends = is_picked
                                                        .then(|| {
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
                                                                                // A bend is removable on
                                                                                // its own, not only by
                                                                                // Straighten-all: find the
                                                                                // stored waypoint under
                                                                                // this pip and drop it.
                                                                                checkpoint();
                                                                                parts.update(|list| {
                                                                                    if let Some(p) =
                                                                                        list.get_mut(part_index)
                                                                                    {
                                                                                        p.waypoints[slot].retain(
                                                                                            |(wx, wy)| {
                                                                                                (wx - bx).abs() > 0.5
                                                                                                    || (wy - by).abs()
                                                                                                        > 0.5
                                                                                            },
                                                                                        );
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
                                                        <circle cx=from.0 cy=from.1 r="2.6" fill="#c9a227" style="pointer-events: none" />
                                                        <circle cx=to.0 cy=to.1 r="2.6" fill="#c9a227" style="pointer-events: none" />
                                                        {bends}
                                                        {grabs}
                                                    })
                                                })
                                                .collect::<Vec<_>>()
                                        })
                                        .collect_view()
                                }}

                                // The armed part's ghost: where a click
                                // would plant it, at its real footprint.
                                {move || {
                                    let (kind, _) = placing.get()?;
                                    let (x, y) = place_at.get()?;
                                    let width = kind.width();
                                    let height = kind.height();
                                    Some(
                                        view! {
                                            <div
                                                class="pointer-events-none absolute rounded-[8px] bg-selection opacity-80 ring-2 ring-rust"
                                                style=format!(
                                                    "left: {x}px; top: {y}px; width: {width}px; height: {height}px",
                                                )
                                            />
                                        },
                                    )
                                }}

                                // ── alignment guides ────────────────────────
                                {move || {
                                    let (gx, gy) = guides.get();
                                    view! {
                                        {gx
                                            .map(|x| {
                                                view! {
                                                    <line
                                                        x1=x
                                                        y1=-2000
                                                        x2=x
                                                        y2=4000
                                                        stroke="#e0a838"
                                                        stroke-width="0.8"
                                                        stroke-dasharray="4 4"
                                                        style="pointer-events: none"
                                                    />
                                                }
                                            })}
                                        {gy
                                            .map(|y| {
                                                view! {
                                                    <line
                                                        x1=-2000
                                                        y1=y
                                                        x2=4000
                                                        y2=y
                                                        stroke="#e0a838"
                                                        stroke-width="0.8"
                                                        stroke-dasharray="4 4"
                                                        style="pointer-events: none"
                                                    />
                                                }
                                            })}
                                    }
                                }}

                                // ── the ghost while pulling a new wire ──────
                                {move || {
                                    let target = ghost.get()?;
                                    // From a stub towards the chip, or from a
                                    // chip pin towards a stub: one dashed line,
                                    // anchored at the end the hand holds still.
                                    let from = match drag.get()? {
                                        Drag::Wire { part, slot } => parts
                                            .with(|list| list.get(part).map(|p| stub_point(p, slot)))?,
                                        Drag::WireFromPin { row } => {
                                            row_point(kit_pos.get(), rows.get().len(), row)
                                        }
                                        _ => return None,
                                    };
                                    Some(view! {
                                        <line
                                            x1=from.0
                                            y1=from.1
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

                    // Where these pin levels come from, on the board rather
                    // than in the rail beside it: it is a claim about what you
                    // are looking at, and the rail is a column of 46px actions
                    // that a sentence cannot live in.
                    //
                    // It has to follow the emulator actually running, because
                    // the answer changed. With rusty's QEMU a pin has state and
                    // the board shows it; with Espressif's the write handler is
                    // empty, so the board can only repeat what the firmware
                    // printed about itself. A user whose LED stays dark needs to
                    // know which, or they check their wiring when the bug is a
                    // missing `println!` — or the reverse.
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
                        running
                            .get()
                            .then(|| {
                                view! {
                                    // The tooltip carries the whole of it; the
                                    // line carries enough to make somebody
                                    // hover. `pointer-events-none` so a caption
                                    // parked over the sheet cannot eat a drag —
                                    // the label itself opts back in for its
                                    // tooltip.
                                    <div class="pointer-events-none absolute bottom-2 left-3 max-w-[calc(100%-1.5rem)]">
                                        <span
                                            class="pointer-events-auto cursor-help text-footnote text-label-3 underline decoration-dotted underline-offset-2"
                                            title=detail
                                        >
                                            {label}
                                        </span>
                                    </div>
                                }
                            })
                    }}

                    {move || {
                        let (x, y, target) = menu.get()?;
                        let close = Callback::new(move |_| menu.set(None));
                        let items = match target {
                            MenuTarget::Wire(part_index, slot) => {
                                view! {
                                    <MenuItem
                                        label=t!("simulate.straighten")
                                        on_select=Callback::new(move |_| {
                                            straighten(part_index, slot);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuSeparator />
                                    <MenuItem
                                        label=t!("simulate.disconnect")
                                        shortcut="Del"
                                        danger=true
                                        on_select=Callback::new(move |_| {
                                            disconnect(part_index, slot);
                                            menu.set(None);
                                        })
                                    />
                                }
                                    .into_any()
                            }
                            MenuTarget::Part(index) => {
                                let wires = parts
                                    .with_untracked(|list| {
                                        list.get(index).map(|p| p.kind.wires()).unwrap_or(0)
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
                                    // Mirroring, not a second rotation: a part
                                    // on the chip's right wants its stubs on
                                    // the near edge *in the same order*, and
                                    // turning it 180° reverses them.
                                    <MenuItem
                                        label=t!("simulate.mirror")
                                        shortcut="X"
                                        on_select=Callback::new(move |_| {
                                            flip_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.duplicate")
                                        shortcut="Ctrl+D"
                                        on_select=Callback::new(move |_| {
                                            duplicate_part(index);
                                            menu.set(None);
                                        })
                                    />
                                    <MenuItem
                                        label=t!("simulate.disconnect-wires")
                                        disabled=wires == 0
                                        on_select=Callback::new(move |_| {
                                            for slot in 0..wires {
                                                disconnect(index, slot);
                                            }
                                            menu.set(None);
                                        })
                                    />
                                    <MenuSeparator />
                                    <MenuItem
                                        label=t!("simulate.remove")
                                        shortcut="Del"
                                        danger=true
                                        on_select=Callback::new(move |_| {
                                            remove_part(index);
                                            menu.set(None);
                                        })
                                    />
                                }
                                    .into_any()
                            }
                            MenuTarget::Sheet => {
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
                                        disabled=parts.with_untracked(Vec::is_empty)
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
                                    .into_any()
                            }
                        };
                        Some(view! { <ContextMenu x=x y=y on_close=close>{items}</ContextMenu> })
                    }}
                </div>

                <div class="flex w-[190px] flex-none flex-col border-l border-line bg-sidebar">
                    {move || {
                        // A selected wire outranks a selected part.
                        if let Some((part_index, slot)) = selected_wire.get() {
                            let part = parts.with(|l| l.get(part_index).cloned());
                            let Some(part) = part else {
                                return ().into_any();
                            };
                            let pin = part.pins[slot];
                            let bends = part.waypoints[slot].len();
                            return view! {
                                <div class="flex flex-col gap-2 p-3">
                                    <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                        {t!("simulate.wire")}
                                    </span>
                                    <p class="text-footnote text-label-2">
                                        {format!("{} → GPIO{pin}", part.label)}
                                    </p>
                                    <p class="text-footnote text-label-4">
                                        {t!("simulate.wire-bends", bends = bends.to_string())}
                                    </p>
                                    <button
                                        type="button"
                                        on:click=move |_| straighten(part_index, slot)
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
                        // the keys do to them. The single-part inspector below
                        // would describe one of them and invite an edit that
                        // applied to that one alone.
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

                        let pin_text = move |slot: usize| {
                            let pin = parts
                                .with_untracked(|list| {
                                    list.get(index).map(|p| p.pins[slot]).unwrap_or(UNWIRED)
                                });
                            if pin == UNWIRED {
                                "—".to_string()
                            } else {
                                pin.to_string()
                            }
                        };

                        view! {
                            <div class="flex flex-col gap-2 p-3">
                                <span class="text-caption font-semibold tracking-[0.06em] text-label-3 uppercase">
                                    {match part.kind {
                                        PartKind::Led { .. } => t!("parts.led"),
                                        PartKind::Button => t!("parts.button"),
                                        PartKind::Rgb => t!("parts.rgb"),
                                        PartKind::Seven => t!("parts.seven"),
                                        PartKind::Display => t!("parts.display"),
                                        PartKind::Pot => t!("parts.pot"),
                                        PartKind::Motor => t!("parts.motor"),
                                        PartKind::Analog => t!("parts.analog"),
                                    }}
                                </span>
                                // The name on the sheet. The editor's own —
                                // `GPIO26` — follows the wiring; one typed
                                // here is kept through a rewire, as KiCad
                                // keeps a reference the user set.
                                <input
                                    type="text"
                                    title=t!("simulate.label-hint")
                                    prop:value=part.label.clone()
                                    on:change=move |event| {
                                        checkpoint();
                                        let text = event_target_value(&event);
                                        parts.update(|list| edit::rename(list, index, &text));
                                        dirty.set(true);
                                    }
                                    class="h-[26px] rounded-[6px] bg-sunken px-2 font-mono text-footnote text-label outline-none ring-1 ring-line focus:ring-rust"
                                />
                                {(part.kind.wires() > 0)
                                    .then(|| {
                                        // The same names the stubs wear on the
                                        // sheet, so the panel and the drawing
                                        // cannot call one pin two things.
                                        let names = stub_names(&part.kind);
                                        view! {
                                            <div class="flex flex-col gap-1">
                                                {names
                                                    .iter()
                                                    .enumerate()
                                                    .map(|(slot, name)| {
                                                        view! {
                                                            <p class="flex items-center gap-2 font-mono text-footnote text-label-2">
                                                                <span class="w-[3ch] text-label-3">
                                                                    {*name}
                                                                </span>
                                                                <span>{pin_text(slot)}</span>
                                                            </p>
                                                        }
                                                    })
                                                    .collect_view()}
                                                <p class="text-caption leading-snug text-label-4">
                                                    {t!("simulate.wire-hint")}
                                                </p>
                                            </div>
                                        }
                                    })}
                                // Polarity: the one fact about a lamp or a
                                // button the sheet cannot see and the firmware
                                // cannot be judged without. A devkit's onboard
                                // LED is usually active-low; a button is
                                // usually to ground with a pull-up.
                                {matches!(
                                    part.kind,
                                    PartKind::Led { .. }
                                        | PartKind::Rgb
                                        | PartKind::Seven
                                        | PartKind::Button
                                )
                                    .then(|| {
                                        let is_button = matches!(part.kind, PartKind::Button);
                                        let on = part.active_low;
                                        view! {
                                            <label class="flex items-start gap-1.5 text-footnote text-label-2 select-none">
                                                <input
                                                    type="checkbox"
                                                    class="mt-0.5"
                                                    prop:checked=on
                                                    on:change=move |event| {
                                                        checkpoint();
                                                        let on = event_target_checked(&event);
                                                        parts.update(|list| edit::set_active_low(list, index, on));
                                                        dirty.set(true);
                                                    }
                                                />
                                                <span>
                                                    {if is_button {
                                                        t!("simulate.active-low-button")
                                                    } else {
                                                        t!("simulate.active-low-lamp")
                                                    }}
                                                </span>
                                            </label>
                                        }
                                    })}
                                {matches!(part.kind, PartKind::Led { .. })
                                    .then(|| {
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
                                                                    parts.update(|list| {
                                                                        if let Some(part) =
                                                                            list.get_mut(index)
                                                                        {
                                                                            part.kind = PartKind::Led {
                                                                                color: name.to_string(),
                                                                            };
                                                                        }
                                                                    });
                                                                    dirty.set(true);
                                                                }
                                                                class=format!(
                                                                    "size-5 rounded-full ring-1 ring-line hover:ring-2 {swatch}",
                                                                )
                                                            />
                                                        }
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                                <button
                                    type="button"
                                    // Through `remove_part`, like the menu and
                                    // the Delete key. This button had its own
                                    // copy of the removal, and it was the one
                                    // that forgot to clear the selected wire.
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

/// A 5 mm LED as a schematic draws it — dome, flange, two legs — in place of
/// the coloured disc that read as a status dot. The lens takes the colour the
/// closures give it and glows when lit: the glow is what reads as "on" from
/// across the room, the way the part itself does. `color` answers the lit or
/// the dark shade; `lit` decides the glow and the highlight's strength.
fn lamp_dome(
    color: impl Fn() -> &'static str + Copy + Send + Sync + 'static,
    lit: impl Fn() -> bool + Copy + Send + Sync + 'static,
) -> AnyView {
    let glow = move || {
        if lit() {
            format!(
                "filter: drop-shadow(0 0 4px {0}) drop-shadow(0 0 9px {0})",
                color()
            )
        } else {
            String::new()
        }
    };
    let sheen = move || if lit() { "0.55" } else { "0.16" };
    view! {
        <svg width="16" height="20" viewBox="0 0 16 20" class="shrink-0 overflow-visible">
            <line x1="5.5" y1="14.5" x2="5.5" y2="20" stroke="#8a929e" stroke-width="1.2" />
            <line x1="10.5" y1="14.5" x2="10.5" y2="20" stroke="#8a929e" stroke-width="1.2" />
            <rect x="1" y="12" width="14" height="3" rx="1" fill=color fill-opacity="0.8" />
            <path d="M2.5 12.5V6.5A5.5 5.5 0 0 1 13.5 6.5V12.5Z" fill=color style=glow />
            <ellipse cx="6" cy="5.5" rx="1.5" ry="2.6" fill="#ffffff" fill-opacity=sheen />
        </svg>
    }
    .into_any()
}
