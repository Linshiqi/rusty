//! The board as a drawing: while nothing runs, a press on a part selects
//! and drags it, a press on a pin starts a wire, a press on a wire selects
//! it, and the inspector and the keys change what is selected. What each
//! gesture *does* to the parts and the wires is `rusty_embed::board::edit`
//! and `geometry`, pure and tested; this is the bookkeeping around them —
//! what is selected, what is in hand, the undo history — and the save.
//!
//! The rules the old window taught are kept: bends belong to the sheet, so
//! moving a part stretches only the leg from its pin to the first bend; a
//! wire lands on the pin under the pointer or on nothing, never on the
//! nearest pin forty pixels away; a press on a pin that does not travel is
//! the start of a wire drawn click by click; and the file is written a
//! moment after the last change, in the format a person writes by hand.

use std::{rc::Rc, time::Duration};

use rusty_embed::{
    PinRef, Sheet, Symbol,
    board::{
        edit,
        geometry::{
            self, CLICK_SLOP, Drawing, EditPart, Snapshot, WireStart, follow_bend, part_box,
            pin_out, pin_point, pin_under, retidy, snap_to, wire_ends, wire_legs, wire_under,
        },
        layout,
    },
};
use rusty_i18n::t;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{BoardProp, LibraryRow, Shell, work};

/// How near a pin or a wire the pointer has to be, in sheet units.
const REACH: f64 = 12.0;

/// The grid parts and corners snap to.
const GRID: f64 = 8.0;

/// What the editor holds between gestures.
#[derive(Default)]
pub(super) struct Edit {
    /// The part the inspector describes.
    pub(super) selected: Option<usize>,
    /// Or the wire.
    pub(super) wire: Option<usize>,
    drag: Option<Drag>,
    /// A wire being drawn click by click.
    drawing: Option<Drawing>,
    /// Where the pointer is, while a wire is being drawn.
    pointer: Option<(f64, f64)>,
    /// The pin under the pointer.
    pub(super) target: Option<(usize, String)>,
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    /// Counts saves asked for, so only the last of a burst writes.
    saves: u64,
}

impl Edit {
    /// A board loaded again may have fewer parts than the selection named.
    pub(super) fn forget_beyond(&mut self, parts: usize) {
        self.selected = self.selected.filter(|&index| index < parts);
        self.wire = None;
        self.drag = None;
        self.drawing = None;
        self.target = None;
    }
}

/// What a press has in hand until it is let go.
enum Drag {
    /// A part, grabbed `grab` from its anchor, with the legs of the wires
    /// touching it judged when it was picked up.
    Part {
        index: usize,
        grab: (f64, f64),
        legs: Vec<WireStart>,
        before: Snapshot,
        moved: bool,
    },
    /// A wire pulled out of a pin.
    Wire {
        from: (usize, String),
        press: (f64, f64),
    },
}

/// The sheet the rules read from what the editor holds: the parts, the
/// wires, the no-connect marks, and every symbol the parts carry.
pub(super) fn sheet_with_symbols(
    chip: &str,
    parts: &[EditPart],
    wires: &[rusty_embed::Wire],
    marks: &[PinRef],
) -> Sheet {
    let mut sheet = geometry::sheet_of(chip, parts, wires, marks);
    for part in parts {
        if let Some(symbol) = &part.symbol
            && !sheet.symbols.iter().any(|s| s.id() == symbol.id())
        {
            sheet.symbols.push(symbol.clone());
        }
    }
    sheet
}

impl App {
    /// Whether the sheet is a drawing now: no simulation is under way.
    pub(super) fn board_editing(&self) -> bool {
        self.activity_kind() != Some(rusty_embed::activity::Kind::Simulate)
    }

    /// A press on the sheet. `true` when the editor took it; otherwise the
    /// view pans.
    pub(super) fn board_down(self: &Rc<Self>, x: f32, y: f32, _shift: bool) -> bool {
        if !self.board_editing() {
            return false;
        }
        let at = (f64::from(x), f64::from(y));
        // A wire being drawn: every click is a corner, a pin or a wire
        // finishes it.
        if self.board.borrow().edit.drawing.is_some() {
            self.drawing_click(at);
            return true;
        }
        let mut board = self.board.borrow_mut();
        let board = &mut *board;
        if board.sheet.is_none() {
            return false;
        }
        let parts = &board.parts;
        if let Some(pin) = pin_under(parts, at, REACH) {
            board.edit.drag = Some(Drag::Wire {
                from: pin,
                press: at,
            });
            return true;
        }
        // The part on top: the last drawn, the devkit only where nothing
        // else is.
        let hit = (0..parts.len()).rev().find(|&index| {
            let (x0, y0, x1, y1) = part_box(&parts[index]);
            at.0 >= x0 && at.0 <= x1 && at.1 >= y0 && at.1 <= y1
        });
        if let Some(index) = hit {
            let part = &parts[index];
            let grab = (at.0 - part.inst.x, at.1 - part.inst.y);
            let legs = wire_legs(parts, &board.sheet_wires(), &[index]);
            let before = board.snapshot();
            board.edit.selected = Some(index);
            board.edit.wire = None;
            board.edit.drag = Some(Drag::Part {
                index,
                grab,
                legs,
                before,
                moved: false,
            });
            self.sync_after_select();
            return true;
        }
        let wires = board.sheet_wires();
        if let Some(index) = wire_under(parts, &wires, at, REACH) {
            board.edit.selected = None;
            board.edit.wire = Some(index);
            self.sync_after_select();
            return true;
        }
        // The bare sheet: nothing selected, and the view pans.
        let had = board.edit.selected.take().is_some() | board.edit.wire.take().is_some();
        if had {
            self.sync_after_select();
        }
        false
    }

    /// What is selected changed: drawn once the press has been answered,
    /// since the board is borrowed while it is.
    fn sync_after_select(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(app) = weak.upgrade() {
                app.sync_board_parts();
                app.sync_board_live();
                app.sync_inspector();
            }
        });
    }

    /// The pointer moving with a press held.
    pub(super) fn board_drag(self: &Rc<Self>, x: f32, y: f32) {
        let at = (f64::from(x), f64::from(y));
        if !self.drag_moved(at) {
            return;
        }
        self.sync_board_parts();
        self.sync_board_live();
        self.sync_ghost();
    }

    /// What is in hand follows the pointer; `false` when nothing changed.
    fn drag_moved(&self, at: (f64, f64)) -> bool {
        let mut board = self.board.borrow_mut();
        let board = &mut *board;
        match &mut board.edit.drag {
            Some(Drag::Part {
                index,
                grab,
                legs,
                moved,
                ..
            }) => {
                let index = *index;
                let place = (snap_to(at.0 - grab.0, GRID), snap_to(at.1 - grab.1, GRID));
                let Some(part) = board.parts.get_mut(index) else {
                    return false;
                };
                if (part.inst.x, part.inst.y) == place {
                    return false;
                }
                part.inst.x = place.0;
                part.inst.y = place.1;
                *moved = true;
                // Bends belong to the sheet, as in KiCad: moving a part
                // stretches only the leg from its pin to the first bend.
                let legs = legs.clone();
                let Some(sheet) = board.sheet.as_mut() else {
                    return false;
                };
                for (wire_index, from_axis, to_axis) in legs {
                    let Some(wire) = sheet.wires.get_mut(wire_index) else {
                        continue;
                    };
                    let Some(ends) = wire_ends(&board.parts, wire) else {
                        continue;
                    };
                    if let Some(first) = wire.bends.first_mut() {
                        follow_bend(ends[0].0, from_axis, first);
                    }
                    if let Some(last) = wire.bends.last_mut() {
                        follow_bend(ends[1].0, to_axis, last);
                    }
                }
                board.moved_parts();
            }
            Some(Drag::Wire { from, .. }) => {
                let from = from.clone();
                board.edit.pointer = Some(at);
                board.edit.target = pin_under(&board.parts, at, REACH).filter(|hit| *hit != from);
            }
            None => return false,
        }
        true
    }

    /// Letting go: whatever was in hand lands.
    pub(super) fn board_up(self: &Rc<Self>, x: f32, y: f32) {
        let at = (f64::from(x), f64::from(y));
        let drag = self.board.borrow_mut().edit.drag.take();
        match drag {
            Some(Drag::Part {
                legs,
                before,
                moved: true,
                ..
            }) => {
                {
                    let mut board = self.board.borrow_mut();
                    let board = &mut *board;
                    let parts = board.parts.clone();
                    if let Some(sheet) = board.sheet.as_mut() {
                        // Every route it stretched is tidied, and any route
                        // the part was dropped on goes round it.
                        for (wire_index, _, _) in &legs {
                            if let Some(wire) = sheet.wires.get_mut(*wire_index)
                                && let Some(ends) = wire_ends(&parts, wire)
                            {
                                retidy(wire, &ends);
                            }
                        }
                        layout::reroute_broken(&parts, &mut sheet.wires);
                    }
                    board.remember(before);
                }
                self.board_edited(false);
            }
            Some(Drag::Part { .. }) => {}
            Some(Drag::Wire { from, press }) => self.wire_released(from, press, at),
            None => {}
        }
    }

    /// A wire pulled from `from` let go at `at`: onto a pin it joins them,
    /// onto a wire it branches, and anywhere else it is the start of a wire
    /// drawn click by click — a click on a pin as much as a drag that ended
    /// on bare sheet.
    fn wire_released(self: &Rc<Self>, from: (usize, String), press: (f64, f64), at: (f64, f64)) {
        let moved = (at.0 - press.0).hypot(at.1 - press.1) > CLICK_SLOP;
        let made = {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let target = pin_under(&board.parts, at, REACH).filter(|hit| *hit != from);
            let before = board.snapshot();
            let parts = board.parts.clone();
            let Some(sheet) = board.sheet.as_mut() else {
                return;
            };
            let snapped = (snap_to(at.0, GRID), snap_to(at.1, GRID));
            if let Some(to) = target {
                let made =
                    edit::connect(&parts, &mut sheet.wires, (from.0, &from.1), (to.0, &to.1));
                made.inspect(|_| board.remember(before))
            } else if moved && let Some(trunk) = wire_under(&parts, &sheet.wires, snapped, REACH) {
                let made =
                    edit::branch(&parts, &mut sheet.wires, (from.0, &from.1), trunk, snapped);
                made.inspect(|_| board.remember(before))
            } else {
                let mut drawing = Drawing::new(from.clone());
                if moved
                    && let Some(part) = parts.get(from.0)
                    && let Some(pin) = part.pin(&from.1)
                {
                    drawing.place(pin_point(part, pin), pin_out(part, pin), snapped);
                }
                board.edit.drawing = Some(drawing);
                board.edit.pointer = Some(snapped);
                None
            }
        };
        self.board.borrow_mut().edit.target = None;
        if let Some(index) = made {
            {
                let mut board = self.board.borrow_mut();
                board.edit.selected = None;
                board.edit.wire = Some(index);
            }
            self.board_edited(true);
        }
        self.sync_ghost();
    }

    /// A click while a wire is being drawn: a corner, or the end of it.
    fn drawing_click(self: &Rc<Self>, at: (f64, f64)) {
        let snapped = (snap_to(at.0, GRID), snap_to(at.1, GRID));
        let made = {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let Some(mut drawing) = board.edit.drawing.take() else {
                return;
            };
            let parts = board.parts.clone();
            let Some(start) = parts.get(drawing.from.0) else {
                return;
            };
            let Some(start_pin) = start.pin(&drawing.from.1) else {
                return;
            };
            let (pin, out) = (pin_point(start, start_pin), pin_out(start, start_pin));
            let before = board.snapshot();
            let Some(sheet) = board.sheet.as_mut() else {
                return;
            };
            let from = drawing.from.clone();
            if let Some(to) = pin_under(&parts, at, REACH).filter(|hit| *hit != from) {
                let Some(end_part) = parts.get(to.0) else {
                    return;
                };
                let Some(end_pin) = end_part.pin(&to.1) else {
                    return;
                };
                let route = drawing.route_into(
                    pin,
                    out,
                    pin_point(end_part, end_pin),
                    pin_out(end_part, end_pin),
                );
                let made = edit::connect_drawn(
                    &parts,
                    &mut sheet.wires,
                    (from.0, &from.1),
                    (to.0, &to.1),
                    route,
                );
                made.inspect(|_| board.remember(before))
            } else if let Some(trunk) = wire_under(&parts, &sheet.wires, snapped, REACH) {
                let route = drawing.route_to(pin, out, snapped);
                let made =
                    edit::branch_drawn(&parts, &mut sheet.wires, (from.0, &from.1), trunk, route);
                made.inspect(|_| board.remember(before))
            } else {
                drawing.place(pin, out, snapped);
                board.edit.drawing = Some(drawing);
                None
            }
        };
        if let Some(index) = made {
            {
                let mut board = self.board.borrow_mut();
                board.edit.selected = None;
                board.edit.wire = Some(index);
                board.edit.pointer = None;
            }
            self.board_edited(true);
        }
        self.sync_ghost();
    }

    /// The pointer over the sheet with nothing pressed: the wire being
    /// drawn follows it, and the pin under it is marked.
    pub(super) fn board_hover(self: &Rc<Self>, x: f32, y: f32) {
        let at = (f64::from(x), f64::from(y));
        let changed = {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let drawing = board.edit.drawing.as_ref().map(|d| d.from.clone());
            let target =
                pin_under(&board.parts, at, REACH).filter(|hit| Some(hit) != drawing.as_ref());
            let pointer = drawing
                .as_ref()
                .map(|_| (snap_to(at.0, GRID), snap_to(at.1, GRID)));
            let changed = board.edit.target != target || board.edit.pointer != pointer;
            board.edit.target = target;
            board.edit.pointer = pointer;
            changed
        };
        if changed {
            self.sync_ghost();
        }
    }

    /// A key while the sheet has focus. `true` when the editor took it.
    pub(super) fn board_key(self: &Rc<Self>, text: &str, control: bool, shift: bool) -> bool {
        use slint::platform::Key;
        if !self.board_editing() {
            return false;
        }
        let is = |key: Key| text == SharedString::from(key).as_str();
        let drawing = self.board.borrow().edit.drawing.is_some();
        if drawing {
            if is(Key::Escape) {
                self.cancel_drawing();
                return true;
            }
            if is(Key::Backspace) {
                let mut board = self.board.borrow_mut();
                if let Some(drawing) = board.edit.drawing.as_mut()
                    && !drawing.unplace()
                {
                    board.edit.drawing = None;
                }
                drop(board);
                self.sync_ghost();
                return true;
            }
            if text == " " {
                if let Some(drawing) = self.board.borrow_mut().edit.drawing.as_mut() {
                    drawing.flip = !drawing.flip;
                }
                self.sync_ghost();
                return true;
            }
        }
        let id = match text {
            _ if is(Key::Delete) || is(Key::Backspace) => "board.delete",
            _ if is(Key::Escape) => "board.deselect",
            "r" | "R" if !control => "board.rotate",
            "m" | "M" if !control => "board.mirror",
            "d" | "D" if control => "board.duplicate",
            "z" | "Z" if control && shift => "board.redo",
            "z" | "Z" if control => "board.undo",
            "y" | "Y" if control => "board.redo",
            _ => return false,
        };
        self.board_act(id);
        true
    }

    fn cancel_drawing(self: &Rc<Self>) {
        {
            let mut board = self.board.borrow_mut();
            board.edit.drawing = None;
            board.edit.pointer = None;
            board.edit.target = None;
        }
        self.sync_ghost();
    }

    /// The editor's verbs, by their action ids.
    pub(super) fn board_act(self: &Rc<Self>, id: &str) {
        if !self.board_editing() {
            return;
        }
        let structural = {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let before = board.snapshot();
            let selected = board.edit.selected;
            let wire = board.edit.wire;
            let parts = &mut board.parts;
            let Some(sheet) = board.sheet.as_mut() else {
                return;
            };
            let changed = match (id, selected, wire) {
                ("board.rotate", Some(index), _) => {
                    edit::rotate(parts, index);
                    Some(false)
                }
                ("board.mirror", Some(index), _) => {
                    edit::mirror(parts, index);
                    Some(false)
                }
                ("board.duplicate", Some(index), _) => edit::duplicate(parts, index).map(|copy| {
                    board.edit.selected = Some(copy);
                    true
                }),
                ("board.delete", Some(index), _) => edit::remove(parts, &mut sheet.wires, index)
                    .then(|| {
                        board.edit.selected = None;
                        true
                    }),
                ("board.delete", None, Some(index)) => {
                    edit::remove_wire(&mut sheet.wires, index);
                    board.edit.wire = None;
                    Some(true)
                }
                ("board.straighten", None, Some(index)) => {
                    edit::straighten(parts, &mut sheet.wires, index);
                    Some(false)
                }
                ("board.deselect", ..) => {
                    board.edit.selected = None;
                    board.edit.wire = None;
                    None
                }
                ("board.undo", ..) => board.step(true).then_some(true),
                ("board.redo", ..) => board.step(false).then_some(true),
                _ => None,
            };
            if changed.is_some() && !matches!(id, "board.undo" | "board.redo") {
                board.remember(before);
            }
            changed
        };
        match structural {
            Some(structural) => self.board_edited(structural),
            None => {
                self.sync_board_parts();
                self.sync_board_live();
                self.sync_inspector();
            }
        }
    }

    /// A field of the inspector accepted.
    pub(super) fn board_rename(self: &Rc<Self>, name: &str) {
        let renamed = {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let Some(index) = board.edit.selected else {
                return;
            };
            let before = board.snapshot();
            let parts = &mut board.parts;
            let Some(sheet) = board.sheet.as_mut() else {
                return;
            };
            let renamed = parts
                .get(index)
                .is_some_and(|p| p.inst.reference == name.trim())
                || edit::rename(parts, &mut sheet.wires, index, name);
            if renamed {
                board.remember(before);
            }
            renamed
        };
        if let Some(window) = self.window() {
            window
                .global::<Shell>()
                .set_board_ref_problem(SharedString::from(if renamed {
                    String::new()
                } else {
                    t!("desk.board-reference-taken", name = name.trim())
                }));
        }
        if renamed {
            self.board_edited(true);
        }
    }

    pub(super) fn board_revalue(self: &Rc<Self>, value: &str) {
        self.board_change(|parts, index| edit::set_value(parts, index, value));
    }

    pub(super) fn board_reprop(self: &Rc<Self>, key: &str, value: &str) {
        self.board_change(|parts, index| edit::set_prop(parts, index, key, value));
    }

    fn board_change(self: &Rc<Self>, change: impl FnOnce(&mut Vec<EditPart>, usize)) {
        {
            let mut board = self.board.borrow_mut();
            let Some(index) = board.edit.selected else {
                return;
            };
            let before = board.snapshot();
            change(&mut board.parts, index);
            board.remember(before);
        }
        self.board_edited(true);
    }

    /// The library narrowed to what is typed.
    pub(super) fn board_library_query(&self, query: &str) {
        let Some(window) = self.window() else { return };
        let wanted = query.trim().to_lowercase();
        let board = self.board.borrow();
        let rows: Vec<LibraryRow> = board
            .library
            .iter()
            .filter(|symbol| {
                wanted.is_empty()
                    || symbol.name.to_lowercase().contains(&wanted)
                    || symbol.library.to_lowercase().contains(&wanted)
                    || symbol
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&wanted))
            })
            .take(200)
            .map(|symbol| LibraryRow {
                id: symbol.id().into(),
                name: symbol.name.as_str().into(),
                detail: match &symbol.description {
                    Some(about) => format!("{} · {about}", symbol.library),
                    None => symbol.library.clone(),
                }
                .into(),
            })
            .collect();
        window
            .global::<Shell>()
            .set_board_library(ModelRc::new(VecModel::from(rows)));
    }

    /// A symbol from the library placed beside the devkit, where the
    /// sheet has room.
    pub(super) fn board_add(self: &Rc<Self>, id: &str) {
        {
            let mut board = self.board.borrow_mut();
            let board = &mut *board;
            let Some(symbol): Option<Symbol> = board.library.iter().find(|s| s.id() == id).cloned()
            else {
                return;
            };
            let before = board.snapshot();
            // Left of the devkit, level with its top: where parts go on
            // every sheet rusty writes. `add` finds the nearest free spot.
            let (x0, y0, _, _) = board
                .parts
                .first()
                .map(part_box)
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            let at = (snap_to(x0 - 120.0, GRID), snap_to(y0 + 40.0, GRID));
            let index = edit::add(&mut board.parts, &symbol, at.0, at.1);
            board.edit.selected = Some(index);
            board.edit.wire = None;
            board.remember(before);
        }
        self.board_edited(true);
    }

    /// Something on the sheet changed: draw it, read it again, and write it
    /// a moment after the last change. `structural` when what is joined to
    /// what may have changed, not only where things are.
    pub(super) fn board_edited(self: &Rc<Self>, structural: bool) {
        {
            let mut board = self.board.borrow_mut();
            board.rebuild(structural);
        }
        self.sync_board_parts();
        self.sync_board_live();
        self.sync_inspector();
        self.sync_ghost();
        self.save_board_soon();
    }

    fn save_board_soon(self: &Rc<Self>) {
        let generation = {
            let mut board = self.board.borrow_mut();
            board.edit.saves += 1;
            board.edit.saves
        };
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(400), move || {
            let Some(app) = weak.upgrade() else { return };
            let (sheet, root) = {
                let board = app.board.borrow();
                if board.edit.saves != generation {
                    return;
                }
                (board.sheet.clone(), app.root())
            };
            let (Some(sheet), Some(root)) = (sheet, root) else {
                return;
            };
            work::spawn(
                move || {
                    let firmware = rusty_embed::project::firmware_root(&root);
                    rusty_embed::simulate::save_board(&firmware, &sheet).map_err(|e| e.to_string())
                },
                |app, saved| {
                    if let Err(reason) = saved {
                        app.note(1, t!("desk.board-not-saved", reason = reason));
                    }
                },
            );
        });
    }

    /// The inspector: what is selected, and its fields.
    pub(super) fn sync_inspector(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let board = self.board.borrow();
        let part = board
            .edit
            .selected
            .and_then(|index| board.parts.get(index))
            .filter(|part| !part.is_kit());
        match part {
            Some(part) => {
                shell.set_board_inspect("part".into());
                shell.set_board_ref(part.inst.reference.as_str().into());
                shell.set_board_value(part.inst.value.as_str().into());
                shell.set_board_symbol(part.inst.symbol.as_str().into());
                let props: Vec<BoardProp> = part
                    .inst
                    .props
                    .iter()
                    .map(|(key, value)| BoardProp {
                        key: key.as_str().into(),
                        value: value.as_str().into(),
                    })
                    .collect();
                shell.set_board_props(ModelRc::new(VecModel::from(props)));
            }
            None if board.edit.wire.is_some() => shell.set_board_inspect("wire".into()),
            None => shell.set_board_inspect(SharedString::new()),
        }
    }

    /// The wire being drawn or pulled, and the pin it would land on.
    pub(super) fn sync_ghost(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let board = self.board.borrow();
        let edit = &board.edit;
        let from = match (&edit.drawing, &edit.drag) {
            (Some(drawing), _) => Some((drawing.from.clone(), Some(drawing))),
            (None, Some(Drag::Wire { from, .. })) => Some((from.clone(), None)),
            _ => None,
        };
        let ghost = from.and_then(|((index, number), drawing)| {
            let part = board.parts.get(index)?;
            let pin = part.pin(&number)?;
            let (at, out) = (pin_point(part, pin), pin_out(part, pin));
            let to = edit.pointer?;
            let fresh = Drawing::new((index, number));
            let drawing = drawing.unwrap_or(&fresh);
            let route = match edit.target.as_ref().and_then(|(i, n)| {
                let end = board.parts.get(*i)?;
                let end_pin = end.pin(n)?;
                Some((pin_point(end, end_pin), pin_out(end, end_pin)))
            }) {
                Some((end, end_out)) => drawing.route_into(at, out, end, end_out),
                None => drawing.route_to(at, out, to),
            };
            Some(super::board::commands(&route))
        });
        shell.set_board_ghost(ghost.unwrap_or_default().into());
        let target = edit.target.as_ref().and_then(|(index, number)| {
            let part = board.parts.get(*index)?;
            Some(pin_point(part, part.pin(number)?))
        });
        shell.set_board_target_shown(target.is_some());
        if let Some((x, y)) = target {
            shell.set_board_target_x(x as f32);
            shell.set_board_target_y(y as f32);
        }
    }
}

/// The probe's view of the sheet.
impl App {
    /// Each part's reference and place, and each wire's ends.
    pub fn board_report(&self) -> String {
        let board = self.board.borrow();
        let parts: Vec<String> = board
            .parts
            .iter()
            .map(|p| {
                format!(
                    "{}@{},{} r{}",
                    p.inst.reference, p.inst.x, p.inst.y, p.inst.rot
                )
            })
            .collect();
        let wires: Vec<String> = board
            .sheet_wires()
            .iter()
            .map(|w| format!("{}.{}-{}.{}", w.from.part, w.from.pin, w.to.part, w.to.pin))
            .collect();
        format!(
            "parts {parts:?}; {} wires {wires:?}; selected {:?}",
            wires.len(),
            board.edit.selected
        )
    }

    pub fn board_parts(&self) -> usize {
        self.board.borrow().parts.len()
    }

    pub fn board_last_reference(&self) -> String {
        self.board
            .borrow()
            .parts
            .last()
            .map(|p| p.inst.reference.clone())
            .unwrap_or_default()
    }

    /// The middle of a part's drawing, in sheet units.
    pub fn board_centre(&self, index: usize) -> Option<(f32, f32)> {
        let board = self.board.borrow();
        let (x0, y0, x1, y1) = part_box(board.parts.get(index)?);
        Some((((x0 + x1) / 2.0) as f32, ((y0 + y1) / 2.0) as f32))
    }

    /// Where a pin is on the sheet.
    pub fn board_pin_point(&self, reference: &str, pin: &str) -> Option<(f32, f32)> {
        let board = self.board.borrow();
        let part = board.parts.iter().find(|p| p.inst.reference == reference)?;
        let (x, y) = pin_point(part, part.pin(pin)?);
        Some((x as f32, y as f32))
    }
}

impl super::board::Board {
    fn sheet_wires(&self) -> Vec<rusty_embed::Wire> {
        self.sheet
            .as_ref()
            .map(|s| s.wires.clone())
            .unwrap_or_default()
    }

    fn snapshot(&self) -> Snapshot {
        (
            self.parts.clone(),
            self.sheet_wires(),
            self.no_connect.clone(),
        )
    }

    /// An edit about to land: what was before it goes on the undo list,
    /// and whatever was undone is no longer redoable.
    fn remember(&mut self, before: Snapshot) {
        edit::remember(&mut self.edit.past, before);
        self.edit.future.clear();
    }

    /// Undo (`back`) or redo: the other list gets what is here now.
    fn step(&mut self, back: bool) -> bool {
        let now = self.snapshot();
        let taken = if back {
            self.edit.past.pop()
        } else {
            self.edit.future.pop()
        };
        let Some((parts, wires, marks)) = taken else {
            return false;
        };
        if back {
            self.edit.future.push(now);
        } else {
            edit::remember(&mut self.edit.past, now);
        }
        self.parts = parts;
        if let Some(sheet) = self.sheet.as_mut() {
            sheet.wires = wires;
        }
        self.no_connect = marks;
        self.edit.selected = self.edit.selected.filter(|&i| i < self.parts.len());
        self.edit.wire = None;
        true
    }
}
