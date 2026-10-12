//! The sheet under the pointer and the keys: what a press, a drag, a
//! release, the wheel and a key do to the board. Methods on `Board`, so the
//! canvas's handlers are one line each.

use super::*;

impl Board {
    /// A key on the focused sheet: the editing keys while it is a drawing, the
    /// wire's own keys while one is being drawn, and F alone while it runs.
    pub(super) fn on_keydown(self, event: ev::KeyboardEvent) {
        let Self {
            live,
            parts,
            selected,
            selected_wire,
            guides,
            drag,
            ghost,
            hover_pin,
            drawing,
            marked,
            box_to,
            placing,
            place_at,
            ..
        } = self;
        // A running board takes no edits; F still fits.
        if live.get_untracked() {
            if matches!(event.key().as_str(), "f" | "F") && !event.ctrl_key() {
                event.prevent_default();
                self.fit_view();
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
                        self.cancel_drawing();
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
                    self.cancel_drawing();
                    return;
                }
                _ => {}
            }
        }
        match event.key().as_str() {
            "Delete" | "Backspace" => {
                event.prevent_default();
                self.delete_selection();
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
                    self.rotate_part(index);
                }
            }
            // KiCad's key for it, and the reason it is not
            // just another rotation is in `edit::mirror`.
            "x" | "X" if !event.ctrl_key() => {
                if let Some(index) = selected.get_untracked() {
                    event.prevent_default();
                    self.mirror_part(index);
                }
            }
            "f" | "F" if !event.ctrl_key() => {
                event.prevent_default();
                self.fit_view();
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
                self.nudge(dx, dy);
            }
            _ if event.ctrl_key() && event.key().eq_ignore_ascii_case("a") => {
                event.prevent_default();
                marked.set((0..parts.with_untracked(Vec::len)).collect());
                selected_wire.set(None);
            }
            _ if event.ctrl_key() && event.key().eq_ignore_ascii_case("d") => {
                if let Some(index) = selected.get_untracked() {
                    event.prevent_default();
                    self.duplicate_part(index);
                }
            }
            _ if event.ctrl_key() && event.key().eq_ignore_ascii_case("z") => {
                event.prevent_default();
                if event.shift_key() {
                    self.redo()
                } else {
                    self.undo()
                }
            }
            _ if event.ctrl_key() && event.key().eq_ignore_ascii_case("y") => {
                event.prevent_default();
                self.redo();
            }
            _ => {}
        }
    }

    /// The wheel zooms about the pointer, as every map does.
    pub(super) fn on_wheel(self, event: ev::WheelEvent) {
        let Self { view, canvas, .. } = self;
        event.prevent_default();
        let Some(element) = canvas.get_untracked() else {
            return;
        };
        let rect = element.get_bounding_client_rect();
        let cx = f64::from(event.client_x()) - rect.left();
        let cy = f64::from(event.client_y()) - rect.top();
        view.update(|(tx, ty, k)| {
            let factor = if event.delta_y() < 0.0 {
                1.12
            } else {
                1.0 / 1.12
            };
            let next = (*k * factor).clamp(CANVAS_ZOOM_RANGE.0, CANVAS_ZOOM_RANGE.1);
            let real = next / *k;
            *tx = cx - (cx - *tx) * real;
            *ty = cy - (cy - *ty) * real;
            *k = next;
        });
    }

    /// A press on the sheet itself — anything a part or a wire handled has
    /// stopped the event already: a pan, a planted part, or the start of a
    /// rubber band.
    pub(super) fn on_pointerdown(self, event: ev::PointerEvent) {
        let Self {
            live,
            library_open,
            selected,
            selected_wire,
            grid,
            drag,
            marked,
            box_to,
            view,
            canvas,
            placing,
            place_at,
            ..
        } = self;
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
            let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
            self.drop_part(symbol, snap_to(world.0, step), snap_to(world.1, step));
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
            let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
            box_to.set(None);
            drag.set(Some(Drag::Box { start: world }));
        }
    }

    /// The pointer moving over the sheet: whatever is in hand follows it.
    pub(super) fn on_pointermove(self, event: ev::PointerEvent) {
        let Self {
            live,
            parts,
            wires,
            dirty,
            hover_wire,
            guides,
            grid,
            drag,
            ghost,
            hover_pin,
            drawing,
            group_start,
            box_to,
            view,
            placing,
            place_at,
            ..
        } = self;
        // While it runs the wires let the pointer through to
        // the parts under them, so which wire it is near is
        // asked of the geometry — for its highlight and the
        // reading line.
        if live.get_untracked() && drag.with_untracked(Option::is_none) {
            let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
            let near = parts.with_untracked(|list| {
                wires.with_untracked(|all| wire_under(list, all, world, 6.0))
            });
            if hover_wire.get_untracked() != near {
                hover_wire.set(near);
            }
        }
        if placing.with_untracked(Option::is_some) {
            let step = grid.get_untracked();
            let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
            place_at.set(Some((snap_to(world.0, step), snap_to(world.1, step))));
        }
        // A wire being drawn follows the pointer, snapped as
        // its corners will be, and lights the pin it would
        // land on — unless the sheet itself is being panned
        // under it, when the pointer is not aiming at all.
        if let Some(from) = drawing.with_untracked(|d| d.as_ref().map(|d| d.from.clone()))
            && !matches!(drag.get_untracked(), Some(Drag::Pan { .. }))
        {
            let step = grid.get_untracked();
            let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
            ghost.set(Some((snap_to(world.0, step), snap_to(world.1, step))));
            let hit = parts
                .with_untracked(|list| pin_under(list, world, REACH))
                .filter(|hit| *hit != from);
            hover_pin.set(hit);
        }
        let Some(current) = drag.get_untracked() else {
            return;
        };
        let world = self.to_world(f64::from(event.client_x()), f64::from(event.client_y()));
        match current {
            Drag::Pan {
                start_tx,
                start_ty,
                px,
                py,
            } => {
                view.update(|(tx, ty, _)| {
                    *tx = start_tx + f64::from(event.client_x()) - px;
                    *ty = start_ty + f64::from(event.client_y()) - py;
                });
            }
            Drag::Part {
                index,
                dx,
                dy,
                from,
                legs,
            } => {
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

    /// Letting go: whatever was in hand lands, and what it made is tidied.
    pub(super) fn on_pointerup(self) {
        let Self {
            parts,
            wires,
            dirty,
            selected,
            selected_wire,
            guides,
            grid,
            drag,
            ghost,
            hover_pin,
            drawing,
            marked,
            group_start,
            box_to,
            ..
        } = self;
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
                let hits = parts.with_untracked(|list| parts_in_box(list, start, to));
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
            let moved =
                released.is_some_and(|at| (at.0 - press.0).hypot(at.1 - press.1) > CLICK_SLOP);
            // A pin first, and the middle of a wire only
            // when no pin is in reach: a branch is what the
            // gesture means where there was nothing else to
            // land on, never in place of the pin somebody
            // was aiming at.
            let made = if let Some(to) = hover_pin.get_untracked() {
                self.checkpoint();
                wires
                    .try_update(|all| edit::connect(&list, all, (from.0, &from.1), (to.0, &to.1)))
                    .flatten()
            } else if moved && let Some(at) = released {
                let step = grid.get_untracked();
                let at = (snap_to(at.0, step), snap_to(at.1, step));
                let trunk = wires.with_untracked(|all| wire_under(&list, all, at, REACH));
                match trunk {
                    Some(trunk) => {
                        self.checkpoint();
                        wires
                            .try_update(|all| {
                                edit::branch(&list, all, (from.0, &from.1), trunk, at)
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

    /// The pointer leaving the sheet puts down whatever was in hand.
    pub(super) fn on_pointerleave(self) {
        let Self {
            hover_wire,
            guides,
            drag,
            ghost,
            hover_pin,
            group_start,
            box_to,
            ..
        } = self;
        guides.set((None, None));
        ghost.set(None);
        hover_pin.set(None);
        hover_wire.set(None);
        box_to.set(None);
        group_start.set(Vec::new());
        drag.set(None);
    }
}
