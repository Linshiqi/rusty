//! The board being edited, as every piece of the editor shares it: the
//! parts and wires, what is selected and in hand, the view, the readings of
//! the rules and the solver — and the commands those pieces run on them.
//!
//! It was forty-odd locals and thirty closures at the top of one
//! 4,700-line component, every one captured by whichever part of its view
//! needed it. `Board` is those locals as fields and those closures as
//! methods, `Copy` like the signals it holds, so a piece of the editor can
//! be a component of its own that takes one.

use super::*;

/// Everything the sheet editor's pieces share. See the module header.
#[derive(Clone, Copy)]
pub(super) struct Board {
    pub state: AppState,
    /// Beside the editor: the library and the inspector float over the
    /// sheet when wanted instead of standing either side of it.
    pub compact: bool,
    pub running: RwSignal<bool>,
    /// While the firmware runs, the sheet is the board on the desk and not
    /// a drawing. See where `BoardEditor` derives it.
    pub live: Signal<bool>,
    /// The part under the pointer while it runs, for the reading line.
    pub hover_part: RwSignal<Option<usize>>,
    /// The floating library, beside the editor.
    pub library_open: RwSignal<bool>,
    pub sensors: StoredValue<Vec<rusty_embed::sensor::Spec>>,
    pub chip_id: StoredValue<String>,
    pub chip_label: StoredValue<String>,
    pub kit_look: KitStyle,
    /// The pin rows this part actually has, from the catalogue.
    pub rows: Memo<Vec<Row>>,
    pub parts: RwSignal<Vec<EditPart>>,
    pub wires: RwSignal<Vec<Wire>>,
    /// Symbols imported during this session, beside the plan's library.
    pub extra: RwSignal<Vec<Symbol>>,
    pub importing: RwSignal<bool>,
    pub dirty: RwSignal<bool>,
    pub selected: RwSignal<Option<usize>>,
    pub selected_wire: RwSignal<Option<usize>>,
    /// The wire under the pointer, for the brightening that says "this one".
    pub hover_wire: RwSignal<Option<usize>>,
    /// (client x, client y, what was clicked)
    pub menu: RwSignal<Option<(f64, f64, MenuTarget)>>,
    /// Alignment guides shown while a part is dragged into line.
    pub guides: RwSignal<(Option<f64>, Option<f64>)>,
    /// The active grid step.
    pub grid: RwSignal<f64>,
    pub drag: RwSignal<Option<Drag>>,
    /// While pulling a wire: the cursor in world coordinates.
    pub ghost: RwSignal<Option<(f64, f64)>>,
    /// The pin the cursor is within reach of while pulling a wire.
    pub hover_pin: RwSignal<Option<(usize, String)>>,
    /// A wire being drawn click by click.
    pub drawing: RwSignal<Option<Drawing>>,
    /// Every part in the selection.
    pub marked: RwSignal<Vec<usize>>,
    /// Where each other marked part stood when a group drag began.
    pub group_start: RwSignal<Vec<GroupStart>>,
    /// The rubber band's moving corner while a box drag is in flight.
    pub box_to: RwSignal<Option<(f64, f64)>>,
    /// Pan x, pan y, zoom.
    pub view: RwSignal<(f64, f64, f64)>,
    pub canvas: NodeRef<leptos::html::Div>,
    /// The switches held down right now, by reference.
    pub pressed: RwSignal<HashSet<String>>,
    /// The pins the author has said reach nothing on purpose.
    pub no_connect: RwSignal<Vec<PinRef>>,
    pub history: RwSignal<Vec<Snapshot>>,
    pub future: RwSignal<Vec<Snapshot>>,
    /// The sheet read over one PWM period; see `BoardEditor`.
    pub period: Memo<Period>,
    pub weights: Memo<Vec<f64>>,
    pub findings: Memo<Vec<Warning>>,
    pub tones: Memo<Vec<Option<Tone>>>,
    /// A picked part, armed to the cursor until a click plants it.
    pub placing: RwSignal<Option<Symbol>>,
    pub place_at: RwSignal<Option<(f64, f64)>>,
    /// Which key of which keypad the pointer is holding.
    pub key_down: RwSignal<Option<(usize, usize, usize)>>,
    pub save: Callback<()>,
}

/// The sheet as the rules read it: parts, wires, and the symbols the parts
/// carry. Built untracked for a command, tracked for the reading.
pub(super) fn sheet_with_symbols(
    chip: &str,
    list: &[EditPart],
    wires: &[Wire],
    marks: &[PinRef],
) -> Sheet {
    let mut sheet = sheet_of(chip, list, wires, marks);
    for part in list {
        if let Some(symbol) = &part.symbol
            && !sheet.symbols.iter().any(|s| s.id() == symbol.id())
        {
            sheet.symbols.push(symbol.clone());
        }
    }
    sheet
}

/// What level a pin sits at over the period — what a wire's colour and the
/// probe say. A function of the two memos rather than only a method, because
/// the wires' colours are read before there is a `Board` to ask.
pub(super) fn level_in(
    period: Memo<Period>,
    weights: Memo<Vec<f64>>,
    pin: &PinRef,
) -> Option<period::Level> {
    period.with(|p| weights.with(|w| p.level(w, pin)))
}

impl Board {
    pub(super) fn checkpoint(self) {
        let Self {
            history,
            future,
            parts,
            wires,
            no_connect,
            ..
        } = self;
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
    }

    pub(super) fn undo(self) {
        let Self {
            history,
            future,
            parts,
            wires,
            no_connect,
            dirty,
            ..
        } = self;
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
    }

    pub(super) fn redo(self) {
        let Self {
            history,
            future,
            parts,
            wires,
            no_connect,
            dirty,
            ..
        } = self;
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
    }

    /// "Yes, that pin reaches nothing, on purpose." The one answer the
    /// loose-pin finding can be given, and the reason it is allowed to exist.
    pub(super) fn toggle_no_connect(self, part: usize, pin: usize) {
        let Self {
            parts,
            no_connect,
            dirty,
            ..
        } = self;
        let Some(named) = parts.with_untracked(|list| {
            let part = list.get(part)?;
            let symbol = part.symbol.as_ref()?;
            let found = symbol.pins.get(pin)?;
            Some(PinRef::new(&part.inst.reference, symbol.wire_key(found)))
        }) else {
            return;
        };
        self.checkpoint();
        no_connect.update(|marks| match marks.iter().position(|p| *p == named) {
            Some(at) => {
                marks.remove(at);
            }
            None => marks.push(named),
        });
        dirty.set(true);
    }

    /// The sheet as it stands, untracked — what a command reads.
    pub(super) fn sheet_now(self) -> Sheet {
        sheet_with_symbols(
            &self.chip_id.get_value(),
            &self.parts.get_untracked(),
            &self.wires.get_untracked(),
            &self.no_connect.get_untracked(),
        )
    }

    // The period, asked in the shapes the board's readers want.

    pub(super) fn lit_share(self, reference: &str, pin: &str) -> f64 {
        let Self {
            period, weights, ..
        } = self;
        period.with(|p| weights.with(|w| p.pin_lit(w, reference, pin)))
    }

    pub(super) fn level_of(self, pin: &PinRef) -> Option<period::Level> {
        level_in(self.period, self.weights, pin)
    }

    pub(super) fn measured(self, reference: &str) -> Option<period::Measured> {
        let Self {
            period, weights, ..
        } = self;
        period.with(|p| weights.with(|w| p.reading(w, reference).ok().flatten()))
    }

    pub(super) fn volts_of(self, pin: &PinRef) -> Option<f64> {
        let Self {
            period, weights, ..
        } = self;
        period.with(|p| weights.with(|w| p.volts_at(w, pin).ok().flatten()))
    }

    /// The GPIO a part's pin reaches through the wires — what a knob, a
    /// source or a motor is *on*, in the firmware's terms.
    pub(super) fn gpio_for(self, reference: &str, pin: &str) -> Option<u8> {
        nets::gpio_of(
            &self.sheet_now(),
            &self.rows.get_untracked(),
            reference,
            pin,
        )
    }

    /// Where a pot's track runs between the rails, when the sheet says. The
    /// knob then reads as ADC counts through `adc.read_oneshot()` and not
    /// only as the text protocol's `P<pin>=`; `None` is a pot whose ends the
    /// sheet has not committed to, which gets the text line alone as before.
    pub(super) fn pot_span_for(self, reference: &str) -> Option<nets::PotSpan> {
        nets::pot_span(&self.sheet_now(), &self.rows.get_untracked(), reference)
    }

    pub(super) fn drop_part(self, symbol: Symbol, x: f64, y: f64) {
        let Self {
            parts,
            selected,
            marked,
            dirty,
            ..
        } = self;
        self.checkpoint();
        parts.update(|list| selected.set(Some(edit::add(list, &symbol, x, y))));
        marked.set(selected.get_untracked().into_iter().collect());
        dirty.set(true);
    }

    /// A new part arrives unwired: picking one arms it to the cursor — a
    /// ghost follows the mouse, a click plants it there, Escape puts it back.
    pub(super) fn add_part(self, symbol: Symbol) {
        let Self {
            live,
            placing,
            place_at,
            ..
        } = self;
        if live.get_untracked() {
            return;
        }
        placing.set(Some(symbol));
        place_at.set(None);
    }

    pub(super) fn import(self, number: String) {
        let Self {
            state,
            importing,
            extra,
            ..
        } = self;
        importing.set(true);
        controller::import_symbol(
            state,
            number,
            Callback::new(move |symbol: Option<Symbol>| {
                importing.set(false);
                if let Some(symbol) = symbol {
                    extra.update(|list| list.push(symbol.clone()));
                    self.add_part(symbol);
                }
            }),
        );
    }

    /// A point on the screen as a point on the sheet.
    pub(super) fn to_world(self, client_x: f64, client_y: f64) -> (f64, f64) {
        let Self { canvas, view, .. } = self;
        let Some(element) = canvas.get_untracked() else {
            return (client_x, client_y);
        };
        let rect = element.get_bounding_client_rect();
        let (tx, ty, k) = view.get_untracked();
        (
            (client_x - rect.left() - tx) / k,
            (client_y - rect.top() - ty) / k,
        )
    }

    pub(super) fn rotate_part(self, index: usize) {
        self.checkpoint();
        self.parts.update(|list| edit::rotate(list, index));
        self.dirty.set(true);
    }

    pub(super) fn mirror_part(self, index: usize) {
        self.checkpoint();
        self.parts.update(|list| edit::mirror(list, index));
        self.dirty.set(true);
    }

    pub(super) fn nudge(self, dx: f64, dy: f64) {
        let Some(index) = self.selected.get_untracked() else {
            return;
        };
        self.checkpoint();
        self.parts.update(|list| edit::nudge(list, index, dx, dy));
        self.dirty.set(true);
    }

    /// Frame everything the sheet holds, the way every canvas tool's F does.
    /// Everything on screen, no larger than `cap`, centred in whichever
    /// direction has room to spare.
    ///
    /// Every read is a `try_`: the fit on open runs a frame after mount, and
    /// by then this editor may already be gone — replaced when the plan
    /// loads a second time — and reading a disposed handle panics, which in
    /// wasm takes the whole window with it.
    pub(super) fn fit_within(self, cap: f64) {
        let Self {
            canvas,
            parts,
            view,
            ..
        } = self;
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
    }

    pub(super) fn fit_view(self) {
        self.fit_within(CANVAS_ZOOM_RANGE.1)
    }

    pub(super) fn straighten_wire(self, index: usize) {
        self.checkpoint();
        let list = self.parts.get_untracked();
        self.wires.update(|all| edit::straighten(&list, all, index));
        self.dirty.set(true);
    }

    pub(super) fn remove_wire(self, index: usize) {
        self.checkpoint();
        self.wires.update(|list| edit::remove_wire(list, index));
        self.selected_wire.set(None);
        self.hover_wire.set(None);
        self.dirty.set(true);
    }

    pub(super) fn disconnect_pin(self, index: usize, number: String) {
        self.checkpoint();
        let list = self.parts.get_untracked();
        self.wires
            .update(|w| edit::disconnect_pin(&list, w, index, &number));
        self.selected_wire.set(None);
        self.dirty.set(true);
    }

    pub(super) fn disconnect_all(self, index: usize) {
        self.checkpoint();
        let list = self.parts.get_untracked();
        self.wires.update(|w| edit::disconnect_all(&list, w, index));
        self.selected_wire.set(None);
        self.dirty.set(true);
    }

    pub(super) fn remove_part(self, index: usize) {
        let Self {
            parts,
            wires,
            selected,
            selected_wire,
            marked,
            dirty,
            ..
        } = self;
        self.checkpoint();
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
    }

    /// Every marked part at once — Delete on a rubber-band selection.
    pub(super) fn remove_marked(self) {
        let Self {
            parts,
            wires,
            selected,
            selected_wire,
            marked,
            dirty,
            ..
        } = self;
        let group = marked.get_untracked();
        if group.is_empty() {
            return;
        }
        self.checkpoint();
        parts.update(|list| wires.update(|w| edit::remove_many(list, w, &group)));
        selected.set(None);
        selected_wire.set(None);
        marked.set(Vec::new());
        dirty.set(true);
    }

    pub(super) fn duplicate_part(self, index: usize) {
        let Self {
            parts,
            selected,
            marked,
            dirty,
            ..
        } = self;
        self.checkpoint();
        parts.update(|list| {
            if let Some(copy) = edit::duplicate(list, index) {
                selected.set(Some(copy));
                marked.set(vec![copy]);
            }
        });
        dirty.set(true);
    }

    /// Remove the selected wire, or the selected part — through the same
    /// commands the menu uses, rather than a third copy of each.
    pub(super) fn delete_selection(self) {
        if let Some(index) = self.selected_wire.get_untracked() {
            self.remove_wire(index);
        } else if self.marked.with_untracked(|m| m.len() > 1) {
            self.remove_marked();
        } else if let Some(index) = self.selected.get_untracked() {
            self.remove_part(index);
        }
    }

    /// A wire being drawn, abandoned: Escape, a right-click, or a click back
    /// on the pin it started from.
    pub(super) fn cancel_drawing(self) {
        self.drawing.set(None);
        self.ghost.set(None);
        self.hover_pin.set(None);
    }

    /// One click while a wire is being drawn, at `world` on the sheet. A pin
    /// in reach finishes it there — its own start pin takes it back — a wire
    /// in reach finishes it as a branch, and anywhere else is a corner. The
    /// preview under the pointer is drawn from the same `Drawing`, so what
    /// was on screen before the click is what the click makes.
    pub(super) fn drawing_click(self, world: (f64, f64)) {
        let Self {
            drawing,
            parts,
            wires,
            grid,
            selected,
            selected_wire,
            dirty,
            ..
        } = self;
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
            self.cancel_drawing();
            return;
        };
        let from = (draft.from.0, draft.from.1.as_str());
        let made = if let Some(hit) = pin_under(&list, world, REACH) {
            if hit == draft.from {
                self.cancel_drawing();
                return;
            }
            let to = (hit.0, hit.1.as_str());
            if draft.placed.is_empty() {
                // Pin to pin with nothing laid between: the same routed
                // wire a drag makes, which is what the preview showed.
                self.checkpoint();
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
                    self.checkpoint();
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
                    self.checkpoint();
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
        self.cancel_drawing();
    }

    /// A key of a matrix keypad, pressed or released. It **joins** its row
    /// to its column rather than driving either — see `nets::keypad_tie` —
    /// so it goes as a switch and the console hears nothing.
    pub(super) fn press_key(self, index: usize, row: usize, column: usize, down: bool) {
        let Self {
            key_down,
            running,
            parts,
            rows,
            state,
            ..
        } = self;
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
        if let Some((a, b)) = nets::keypad_tie(
            &self.sheet_now(),
            &rows.get_untracked(),
            &reference,
            row,
            column,
        ) {
            controller::sim_switch(state, a, b, down);
        }
    }

    /// A switch pressed on the sheet: the rules see it as conducting, and
    /// while a session runs the GPIO it reaches is driven to the level its
    /// other side holds — through the same message the old buttons sent, so
    /// firmware written for `B<pin>=1` hears it too.
    pub(super) fn press(self, index: usize, down: bool) {
        let Self {
            parts,
            pressed,
            running,
            rows,
            state,
            ..
        } = self;
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
        let sheet = self.sheet_now();
        let rows = rows.get_untracked();
        if let Some((a, b)) = nets::switch_tie(&sheet, &rows, &reference) {
            controller::sim_switch(state, a, b, down);
        } else if let Some((gpio, _)) = nets::button_drives(&sheet, &rows, &reference) {
            controller::sim_press(state, gpio, down);
        }
    }

    /// Whether the inspector has anything to say — beside the editor it is
    /// drawn only then.
    pub(super) fn inspecting(self) -> bool {
        self.selected.get().is_some()
            || self.selected_wire.get().is_some()
            || self.marked.with(|m| m.len() > 1)
    }

    /// Beside the editor it floats on the side of the pane away from the
    /// part it describes, so what is being edited is never under the panel
    /// editing it — on the right, the ESP32's parts all were.
    pub(super) fn inspector_left(self) -> bool {
        let Self {
            selected,
            parts,
            view,
            canvas,
            ..
        } = self;
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
    }
}
