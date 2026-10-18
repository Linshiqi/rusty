//! The math toolbox: the vectors, quaternions and frames a flight
//! controller is made of, worked out a row at a time and drawn — an
//! attitude as the aircraft it is, every step of the working in space, and
//! the firmware's own estimate beside the simulator's truth while a run
//! goes on.
//!
//! The arithmetic and the sheet's language are `rusty_embed::spatial`, the
//! picture `rusty_embed::scene` (`scene::math` composes it); this is the
//! page. The sheet is the project's `.rusty/math.toml`, read once per
//! project, saved a moment after typing stops, and never saved over when it
//! did not read.

use std::{path::PathBuf, rc::Rc, time::Duration};

use rusty_embed::{
    scene::{
        self, Camera, Ink, Item, Preset, Projector,
        math::{compose_plane, compose_space, plane_suits, turns},
        model,
        sketch::{PALETTE, nice_step},
    },
    spatial::{
        Frame, Quat, instrument,
        sheet::{EXAMPLES, Live, MathSheet, Row, Unit, Value, evaluate, set_literal, steps},
    },
};
use rusty_i18n::t;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{App, Tab, draw, math_words as words};
use crate::{MathLine, MathRow, MathStep, Shell, work};

/// How long typing has to pause before the sheet is written.
const SAVE_AFTER: Duration = Duration::from_millis(600);
/// How long one turn takes to play.
const SECONDS_PER_TURN: f64 = 0.9;
const TICK: Duration = Duration::from_millis(16);

pub(super) struct Math {
    sheet: MathSheet,
    hidden: Vec<bool>,
    selected: Option<usize>,
    step: Option<usize>,
    progress: f64,
    playing: bool,
    live: Live,
    /// The project whose sheet this is, once read.
    home: Option<PathBuf>,
    unreadable: Option<String>,
    edits: u64,
    camera: Camera,
    /// The plane rather than space, when chosen; `None` follows the sheet.
    flat: Option<bool>,
    /// The plane's centre and zoom.
    plane: (f64, f64, f64),
    size: (f64, f64),
    help: bool,
    ticker: Option<slint::Timer>,
    /// The rows model, kept while only outcomes change so a row being typed
    /// in keeps its field.
    rows: Option<Rc<VecModel<MathRow>>>,
}

impl Default for Math {
    fn default() -> Self {
        Math {
            sheet: EXAMPLES[0].sheet(),
            hidden: Vec::new(),
            selected: None,
            step: None,
            progress: 1.0,
            playing: false,
            live: Live::default(),
            home: None,
            unreadable: None,
            edits: 0,
            camera: Camera::preset(Preset::Chase, Frame::ZUp),
            flat: None,
            plane: (0.0, 0.0, 1.0),
            size: (800.0, 480.0),
            help: false,
            ticker: None,
            rows: None,
        }
    }
}

/// A row's colour, the view's palette.
fn colour(index: usize) -> slint::Color {
    super::colour::hex(PALETTE[index % PALETTE.len()])
}

/// Whether any row reads the firmware or the plant — asked of the text, so
/// a sheet that does not is never re-evaluated ten times a second.
fn reads_live(sheet: &MathSheet) -> bool {
    sheet
        .rows
        .iter()
        .any(|row| row.contains("tel(") || row.contains("truth"))
}

impl App {
    pub(super) fn math_act(self: &Rc<Self>, verb: &str) {
        match verb {
            "open" => {
                self.open_tab(Tab::Page("math"));
                self.math_shown();
            }
            "shown" => self.math_shown(),
            "add" => self.math_add(None),
            "z-up" => self.math_frame(Frame::ZUp),
            "z-down" => self.math_frame(Frame::ZDown),
            "help" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.math.help = !state.math.help;
                }
                self.sync_math(false);
            }
            "play" => self.math_play(),
            "flat" => {
                {
                    let mut state = self.state.borrow_mut();
                    let flat = self.math_flat_now(&state.math);
                    state.math.flat = Some(!flat);
                }
                self.sync_math(false);
            }
            "chase" => self.math_preset(Preset::Chase),
            "top" => self.math_preset(Preset::Top),
            "front" => self.math_preset(Preset::Front),
            "side" => self.math_preset(Preset::Side),
            "fit" => self.math_fit(),
            "copy" => {
                let literal = {
                    let state = self.state.borrow();
                    let rows = evaluate(&state.math.sheet, &state.math.live);
                    state
                        .math
                        .selected
                        .and_then(|i| rows.get(i).cloned())
                        .and_then(|r| r.value.and_then(Result::ok))
                        .and_then(|v| words::rust(&v))
                };
                if let Some(literal) = literal {
                    super::git::copy_text(&literal);
                    self.note_transient(0, t!("math.copied"));
                }
            }
            _ => {}
        }
    }

    /// The page on screen: the project's sheet, read once per project, and
    /// the clock that plays turns and reads a running simulation.
    fn math_shown(self: &Rc<Self>) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_math_examples(ModelRc::new(VecModel::from(example_names())));
            shell.set_math_help_rows(ModelRc::new(VecModel::from(help_rows())));
            shell.set_symbols_font(SharedString::from(if cfg!(windows) {
                "Segoe UI Symbol"
            } else if cfg!(target_os = "macos") {
                "Apple Symbols"
            } else {
                "DejaVu Sans"
            }));
        }
        self.math_load();
        if self.state.borrow().math.ticker.is_none() {
            let timer = slint::Timer::default();
            let weak = Rc::downgrade(self);
            let mut since_live = Duration::ZERO;
            timer.start(slint::TimerMode::Repeated, TICK, move || {
                let Some(app) = weak.upgrade() else { return };
                if !matches!(app.active_tab(), Some(Tab::Page("math"))) {
                    return;
                }
                app.math_tick();
                since_live += TICK;
                // What a run says, ten times a second and not per line.
                if since_live >= Duration::from_millis(100) {
                    since_live = Duration::ZERO;
                    app.math_live();
                }
            });
            self.state.borrow_mut().math.ticker = Some(timer);
        }
        self.sync_math(true);
    }

    fn math_load(self: &Rc<Self>) {
        let root = self.root();
        if root == self.state.borrow().math.home {
            return;
        }
        let Some(root) = root else { return };
        work::spawn(
            {
                let root = root.clone();
                move || rusty_embed::spatial::sheet_file::load(&root).map_err(|e| e.to_string())
            },
            move |app, found| {
                if app.root().as_ref() != Some(&root) {
                    return;
                }
                {
                    let mut state = app.state.borrow_mut();
                    let math = &mut state.math;
                    match found {
                        Ok(found) => {
                            if let Some(sheet) = found {
                                show(math, sheet);
                            }
                            math.unreadable = None;
                            math.home = Some(root);
                        }
                        Err(why) => {
                            math.home = None;
                            math.unreadable = Some(why);
                        }
                    }
                }
                app.sync_math(true);
            },
        );
    }

    /// Save once the typing stops — only to a project whose sheet was read,
    /// so a file that does not read is never written over.
    fn math_edited(self: &Rc<Self>) {
        let edits = {
            let mut state = self.state.borrow_mut();
            state.math.edits += 1;
            if state.math.home.is_none() {
                return;
            }
            state.math.edits
        };
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(SAVE_AFTER, move || {
            let Some(app) = weak.upgrade() else { return };
            let (root, sheet) = {
                let state = app.state.borrow();
                if state.math.edits != edits {
                    return;
                }
                let Some(root) = state.math.home.clone() else {
                    return;
                };
                (root, state.math.sheet.clone())
            };
            work::spawn(
                move || {
                    rusty_embed::spatial::sheet_file::save(&root, &sheet).map_err(|e| e.to_string())
                },
                |app, saved| {
                    if let Err(why) = saved {
                        app.note(2, why);
                    }
                },
            );
        });
    }

    /// One row's text, typed.
    pub(super) fn math_row(self: &Rc<Self>, index: i32, text: &str) {
        let changed = {
            let mut state = self.state.borrow_mut();
            match usize::try_from(index)
                .ok()
                .and_then(|i| state.math.sheet.rows.get_mut(i))
            {
                Some(row) if row != text => {
                    *row = text.to_string();
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.math_edited();
            self.sync_math(false);
        }
    }

    /// A key in a row's field: Enter makes the next row, Backspace in an
    /// empty one takes it away, the arrows move between them.
    pub(super) fn math_row_key(self: &Rc<Self>, index: i32, key: &str) -> bool {
        let Ok(index) = usize::try_from(index) else {
            return false;
        };
        let (count, empty) = {
            let state = self.state.borrow();
            let rows = &state.math.sheet.rows;
            (rows.len(), rows.get(index).is_some_and(|r| r.is_empty()))
        };
        match key {
            "enter" => {
                self.math_add(Some(index));
                true
            }
            "backspace" if empty => {
                self.math_remove(index as i32);
                self.math_focus(index.saturating_sub(1));
                true
            }
            "up" if index > 0 => {
                self.math_select(index as i32 - 1);
                self.math_focus(index - 1);
                true
            }
            "down" if index + 1 < count => {
                self.math_select(index as i32 + 1);
                self.math_focus(index + 1);
                true
            }
            _ => false,
        }
    }

    fn math_focus(&self, index: usize) {
        if let Some(window) = self.window() {
            let shell = window.global::<Shell>();
            shell.set_math_focus(index as i32);
            shell.set_math_focus_tick(shell.get_math_focus_tick().wrapping_add(1));
        }
    }

    /// A new, empty row after `after` (at the end for `None`).
    fn math_add(self: &Rc<Self>, after: Option<usize>) {
        let at = {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            let at = after
                .map_or(math.sheet.rows.len(), |i| i + 1)
                .min(math.sheet.rows.len());
            math.hidden.resize(math.sheet.rows.len(), false);
            math.sheet.rows.insert(at, String::new());
            math.hidden.insert(at, false);
            math.selected = Some(at);
            math.step = None;
            at
        };
        self.math_edited();
        self.sync_math(true);
        self.math_focus(at);
    }

    pub(super) fn math_remove(self: &Rc<Self>, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            if index >= math.sheet.rows.len() {
                return;
            }
            math.hidden.resize(math.sheet.rows.len(), false);
            math.sheet.rows.remove(index);
            math.hidden.remove(index);
            let left = math.sheet.rows.len();
            math.selected = math.selected.and_then(|s| {
                (left > 0).then(|| {
                    if s > index {
                        s - 1
                    } else if s == index {
                        index.saturating_sub(1).min(left - 1)
                    } else {
                        s
                    }
                })
            });
            math.step = None;
        }
        self.math_edited();
        self.sync_math(true);
    }

    pub(super) fn math_toggle(self: &Rc<Self>, index: i32) {
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let hidden = &mut state.math.hidden;
            if hidden.len() <= index {
                hidden.resize(index + 1, false);
            }
            hidden[index] = !hidden[index];
        }
        self.sync_math(false);
    }

    pub(super) fn math_select(self: &Rc<Self>, index: i32) {
        let index = usize::try_from(index).ok();
        {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            if math.selected == index {
                return;
            }
            math.selected = index;
            math.step = None;
            math.playing = false;
            math.progress = 1.0;
        }
        self.sync_math(false);
    }

    /// Which way the world's Z points — and the camera goes where that
    /// frame's chase view is, since up has changed ends.
    fn math_frame(self: &Rc<Self>, frame: Frame) {
        {
            let mut state = self.state.borrow_mut();
            if state.math.sheet.frame == frame {
                return;
            }
            state.math.sheet.frame = frame;
            state.math.camera = Camera::preset(Preset::Chase, frame);
        }
        self.math_edited();
        self.sync_math(false);
    }

    /// An example in place of the sheet, asking first when that would lose
    /// rows nobody else has.
    pub(super) fn math_example(self: &Rc<Self>, index: i32) {
        let Some(example) = usize::try_from(index - 1)
            .ok()
            .and_then(|i| EXAMPLES.get(i))
        else {
            return;
        };
        let own_work = {
            let state = self.state.borrow();
            let rows = &state.math.sheet.rows;
            rows.iter().any(|r| !r.trim().is_empty())
                && !EXAMPLES.iter().any(|e| e.sheet().rows == *rows)
        };
        if own_work && !super::git::confirm(&t!("math.replace-sheet")) {
            return;
        }
        show(&mut self.state.borrow_mut().math, example.sheet());
        self.math_edited();
        self.sync_math(true);
    }

    /// A row's slider moved: its number rewritten in its own unit.
    pub(super) fn math_slider(self: &Rc<Self>, index: i32, value: f32) {
        let text = {
            let state = self.state.borrow();
            usize::try_from(index)
                .ok()
                .and_then(|i| state.math.sheet.rows.get(i).cloned())
        };
        if let Some(next) = text.and_then(|t| set_literal(&t, f64::from(value))) {
            self.math_row(index, &next);
            // The field shows the new text: the outcomes are not enough.
            self.sync_math(true);
        }
    }

    // ---- the working and the view -------------------------------------------

    pub(super) fn math_step(self: &Rc<Self>, index: i32) {
        {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            let Ok(index) = usize::try_from(index) else {
                return;
            };
            math.playing = false;
            math.step = Some(index);
            let rows = evaluate(&math.sheet, &math.live);
            if let Some(row) = math.selected.and_then(|s| rows.get(s)) {
                let all = turns(row);
                if let Some(k) = all.iter().position(|(i, _)| *i == index) {
                    math.progress = (k + 1) as f64 / all.len() as f64;
                }
            }
        }
        self.sync_math(false);
    }

    fn math_play(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            if math.playing {
                math.playing = false;
            } else {
                if math.progress >= 1.0 {
                    math.progress = 0.0;
                }
                math.step = None;
                math.playing = true;
            }
        }
        self.sync_math(false);
    }

    pub(super) fn math_scrub(self: &Rc<Self>, at: f32) {
        {
            let mut state = self.state.borrow_mut();
            state.math.playing = false;
            state.math.step = None;
            state.math.progress = f64::from(at).clamp(0.0, 1.0);
        }
        self.sync_math(false);
    }

    fn math_tick(self: &Rc<Self>) {
        let moved = {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            if !math.playing {
                return;
            }
            let rows = evaluate(&math.sheet, &math.live);
            let n = math
                .selected
                .and_then(|s| rows.get(s))
                .map_or(0, |r| turns(r).len());
            if n == 0 {
                math.playing = false;
                true
            } else {
                let step = TICK.as_secs_f64() / (SECONDS_PER_TURN * n as f64);
                math.progress = (math.progress + step).min(1.0);
                if math.progress >= 1.0 {
                    math.playing = false;
                }
                true
            }
        };
        if moved {
            self.sync_math(false);
        }
    }

    /// What the firmware last printed on every channel, and the plant while
    /// the Flight tab closes the loop — for a sheet that reads either.
    fn math_live(self: &Rc<Self>) {
        if !reads_live(&self.state.borrow().math.sheet) {
            return;
        }
        let live = self.live_now();
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.math.live == live {
                false
            } else {
                state.math.live = live;
                true
            }
        };
        if changed {
            self.sync_math(false);
        }
    }

    /// What a running simulation offers a sheet right now.
    pub(super) fn live_now(&self) -> Live {
        let state = self.state.borrow();
        let channels = state
            .sim
            .plot
            .channels
            .iter()
            .filter_map(|(name, samples)| {
                samples.last().map(|(_, v)| (name.clone(), f64::from(*v)))
            })
            .collect();
        let (truth, truth_rate) = match state.flight.truth() {
            Some((q, r)) => (Some(q), Some(r)),
            None => (None, None),
        };
        Live {
            channels,
            truth,
            truth_rate,
        }
    }

    fn math_flat_now(&self, math: &Math) -> bool {
        math.flat.unwrap_or_else(|| {
            let rows = evaluate(&math.sheet, &math.live);
            plane_suits(&rows, math.selected)
        })
    }

    fn math_preset(self: &Rc<Self>, preset: Preset) {
        {
            let mut state = self.state.borrow_mut();
            let frame = state.math.sheet.frame;
            let distance = state.math.camera.distance;
            state.math.camera = Camera {
                distance,
                ..Camera::preset(preset, frame)
            };
        }
        self.sync_math(false);
    }

    fn math_fit(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let math = &mut state.math;
            if self.math_flat_now(math) {
                math.plane = (0.0, 0.0, 1.0);
            } else {
                let rows = evaluate(&math.sheet, &math.live);
                let items: Vec<Item> = compose_space(
                    &rows,
                    &math.hidden,
                    math.selected,
                    math.step,
                    1.0,
                    math.sheet.frame,
                )
                .into_iter()
                .filter(|(item, _)| !matches!(item, Item::Line { ink: Ink::Grid, .. }))
                .map(|(item, _)| item)
                .collect();
                let extent = model::extent(&items).max(1.3);
                math.camera = math.camera.fitting(extent);
            }
        }
        self.sync_math(false);
    }

    pub(super) fn math_resized(self: &Rc<Self>, w: f32, h: f32) {
        let size = (f64::from(w.max(1.0)), f64::from(h.max(1.0)));
        if self.state.borrow().math.size == size {
            return;
        }
        self.state.borrow_mut().math.size = size;
        self.sync_math(false);
    }

    /// A drag on the view: space turns, the plane pans.
    pub(super) fn math_orbit(self: &Rc<Self>, dx: f32, dy: f32) {
        {
            let mut state = self.state.borrow_mut();
            let flat = self.math_flat_now(&state.math);
            let math = &mut state.math;
            let (dx, dy) = (f64::from(dx), f64::from(dy));
            if flat {
                let scale = plane_scale(math);
                math.plane.0 -= dx / scale;
                math.plane.1 += dy / scale;
            } else {
                math.camera = math.camera.orbit(dx, dy, math.sheet.frame);
            }
        }
        self.sync_math(false);
    }

    pub(super) fn math_zoom(self: &Rc<Self>, delta: f32) {
        {
            let mut state = self.state.borrow_mut();
            let flat = self.math_flat_now(&state.math);
            let math = &mut state.math;
            let factor = (f64::from(-delta) * 0.0015).exp();
            if flat {
                math.plane.2 = (math.plane.2 / factor).clamp(0.05, 50.0);
            } else {
                math.camera = math.camera.zoomed(factor);
            }
        }
        self.sync_math(false);
    }

    // ---- drawing -------------------------------------------------------------

    /// The page from the state. `structure` rebuilds the rows' fields — a
    /// row added, removed or loaded; otherwise only what each came to
    /// changes, and a field being typed in keeps its caret.
    pub(super) fn sync_math(&self, structure: bool) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let mut state = self.state.borrow_mut();
        let has_project = state.project.is_some();
        let math = &mut state.math;
        let rows = evaluate(&math.sheet, &math.live);
        math.hidden.resize(math.sheet.rows.len(), false);

        shell.set_math_frame_up(math.sheet.frame == Frame::ZUp);
        shell.set_math_help(math.help);
        shell.set_math_saved(
            if math.home.is_some() {
                t!("math.saved-in-project")
            } else if math.unreadable.is_some() || !has_project {
                String::new()
            } else {
                t!("math.not-saved")
            }
            .into(),
        );
        shell.set_math_unreadable(
            math.unreadable
                .as_ref()
                .map(|why| t!("math.unreadable", reason = why.clone()))
                .unwrap_or_default()
                .into(),
        );

        // The rows.
        let made: Vec<MathRow> = rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                row_of(
                    i,
                    &math.sheet.rows[i],
                    row,
                    math.selected == Some(i),
                    math.hidden[i],
                )
            })
            .collect();
        match (&math.rows, structure) {
            (Some(model), false) if model.row_count() == made.len() => {
                for (i, row) in made.into_iter().enumerate() {
                    model.set_row_data(i, row);
                }
            }
            _ => {
                let model = Rc::new(VecModel::from(made));
                shell.set_math_rows(ModelRc::from(model.clone()));
                math.rows = Some(model);
            }
        }

        // The working.
        let chosen = math.selected.and_then(|i| rows.get(i));
        let steps: Vec<MathStep> = chosen
            .map(|row| {
                row.steps
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.lines.is_empty() || !s.remarks.is_empty())
                    .map(|(i, step)| MathStep {
                        index: i as i32,
                        title: words::what(step.what).into(),
                        lines: step.lines.join("\n").into(),
                        remarks: step
                            .remarks
                            .iter()
                            .map(|r| words::remark(*r))
                            .collect::<Vec<_>>()
                            .join("\n")
                            .into(),
                        chosen: math.step == Some(i),
                    })
                    .collect()
            })
            .unwrap_or_default();
        shell.set_math_steps(ModelRc::new(VecModel::from(steps)));
        shell.set_math_steps_note(
            match chosen {
                None => t!("math.no-row"),
                Some(row) if row.steps.is_empty() => t!("math.no-steps"),
                Some(_) => String::new(),
            }
            .into(),
        );
        shell.set_math_turns(chosen.map_or(0, |r| turns(r).len()) as i32);
        shell.set_math_playing(math.playing);
        shell.set_math_progress(math.progress as f32);

        // The value.
        let value = chosen
            .and_then(|r| r.value.as_ref())
            .and_then(|v| v.as_ref().ok());
        readout(&shell, value, math.sheet.frame);

        // The view.
        let flat = math
            .flat
            .unwrap_or_else(|| plane_suits(&rows, math.selected));
        shell.set_math_flat(flat);
        let (w, h) = math.size;
        let drawn = if flat {
            let scale = plane_scale_of(&rows, math);
            let (cx, cy, _) = math.plane;
            let half = (w.max(h) / scale) * 0.6;
            let items = compose_plane(
                &rows,
                &math.hidden,
                math.selected,
                math.step,
                half,
                nice_step(80.0 / scale),
                (cx, cy),
            );
            scene::render(&items, &Projector::plane(scale, (cx, cy), w, h))
        } else {
            let items = compose_space(
                &rows,
                &math.hidden,
                math.selected,
                math.step,
                math.progress,
                math.sheet.frame,
            );
            scene::render(
                &items,
                &Projector::new(&math.camera, math.sheet.frame, w, h),
            )
        };
        shell.set_math_pieces(ModelRc::new(VecModel::from(draw::pieces_of(drawn))));
        shell.set_math_view_w(w as f32);
        shell.set_math_view_h(h as f32);
    }
}

/// A sheet put on screen: its rows all shown, nothing selected, the camera
/// where its frame's chase view is.
fn show(math: &mut Math, sheet: MathSheet) {
    math.hidden = vec![false; sheet.rows.len()];
    math.selected = None;
    math.step = None;
    math.progress = 1.0;
    math.playing = false;
    math.camera = Camera::preset(Preset::Chase, sheet.frame);
    math.sheet = sheet;
}

/// The plane's scale: the vectors fitted, times the user's zoom.
fn plane_scale_of(rows: &[Row], math: &Math) -> f64 {
    let (w, h) = math.size;
    let extent = rows
        .iter()
        .filter_map(|r| match &r.value {
            Some(Ok(Value::Vec2(v))) => Some(v.norm()),
            _ => None,
        })
        .fold(1.0_f64, f64::max);
    (w.min(h) * 0.42 / extent) * math.plane.2
}

fn plane_scale(math: &Math) -> f64 {
    plane_scale_of(&evaluate(&math.sheet, &math.live), math)
}

/// A row as its field and what it came to.
fn row_of(index: usize, text: &str, row: &Row, selected: bool, hidden: bool) -> MathRow {
    let (value, problem) = match &row.value {
        None => (String::new(), String::new()),
        Some(Ok(value)) => (words::show(value), String::new()),
        Some(Err(problem)) => (String::new(), words::problem(problem)),
    };
    let mut out = MathRow {
        text: text.into(),
        colour: colour(index),
        hidden,
        selected,
        value: value.into(),
        problem: problem.into(),
        notes: row
            .notes
            .iter()
            .map(words::note)
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
        live: row.live,
        ..MathRow::default()
    };
    if let Some(slider) = row.slider {
        out.slider = true;
        out.min = slider.min as f32;
        out.max = slider.max as f32;
        out.at = slider.value as f32;
        out.shown = match slider.unit {
            Unit::Degrees => format!("{}°", words::trimmed(slider.value, 1)),
            Unit::Radians => format!("{} rad", words::trimmed(slider.value, 3)),
            Unit::Plain => words::trimmed(slider.value, 2),
        }
        .into();
    }
    out
}

/// The selected row's value in full: an attitude as its quaternion, axis and
/// angle, Euler angles, matrix and the instrument a pilot reads it on.
fn readout(shell: &Shell, value: Option<&Value>, frame: Frame) {
    let line = |label: &str, value: String| MathLine {
        label: label.into(),
        value: value.into(),
    };
    let mut lines = Vec::new();
    let mut matrix: Vec<SharedString> = Vec::new();
    let mut extra = String::new();
    let mut kind = 0;
    let mut nose = 0.0;
    let mut bank = 0.0;
    let attitude = |q: Quat,
                    given: Option<rusty_embed::spatial::Euler>,
                    lines: &mut Vec<MathLine>,
                    matrix: &mut Vec<SharedString>,
                    extra: &mut String|
     -> instrument::Reading {
        let unit = q.normalized().unwrap_or(Quat::IDENTITY);
        let e = given.unwrap_or_else(|| q.to_euler());
        let (axis, angle) = unit
            .axis_angle()
            .map(|(n, a)| {
                (
                    format!(
                        "({}, {}, {})",
                        steps::num(n.x),
                        steps::num(n.y),
                        steps::num(n.z)
                    ),
                    steps::deg(a),
                )
            })
            .unwrap_or_else(|| ("—".into(), "0°".into()));
        lines.push(line(
            "q",
            format!(
                "({}, {}, {}, {})",
                steps::num(q.w),
                steps::num(q.x),
                steps::num(q.y),
                steps::num(q.z)
            ),
        ));
        lines.push(line("|q|", steps::precise(q.norm())));
        lines.push(line(&t!("math.readout.axis"), axis));
        lines.push(line(&t!("math.readout.angle"), angle));
        lines.push(line(&t!("math.readout.roll"), steps::deg(e.roll)));
        lines.push(line(&t!("math.readout.pitch"), steps::deg(e.pitch)));
        lines.push(line(&t!("math.readout.yaw"), steps::deg(e.yaw)));
        let reading = instrument::read(unit, frame);
        let m = unit.to_mat3();
        matrix.extend(
            m.m.iter()
                .flat_map(|r| r.iter().map(|v| SharedString::from(steps::num(*v)))),
        );
        *extra = format!(
            "{}\n{}",
            t!(
                "math.readout.instrument",
                nose = steps::deg(reading.nose_up),
                bank = steps::deg(reading.bank_right),
                heading = steps::deg(reading.heading)
            ),
            orthonormal(m)
        );
        reading
    };
    let mut copyable = false;
    match value {
        None => {}
        Some(Value::Quat(q)) => {
            kind = 1;
            let r = attitude(*q, None, &mut lines, &mut matrix, &mut extra);
            (nose, bank) = (r.nose_up, r.bank_right);
            copyable = true;
        }
        Some(Value::Euler(e)) => {
            kind = 1;
            let r = attitude(
                Quat::from_euler(*e),
                Some(*e),
                &mut lines,
                &mut matrix,
                &mut extra,
            );
            (nose, bank) = (r.nose_up, r.bank_right);
            copyable = true;
        }
        Some(Value::Mat3(m)) => {
            kind = 2;
            matrix.extend(
                m.m.iter()
                    .flat_map(|r| r.iter().map(|v| SharedString::from(steps::num(*v)))),
            );
            extra = format!(
                "det = {} · |RᵀR − I| = {}",
                steps::num(m.det()),
                steps::num(m.orthonormal_error())
            );
            copyable = true;
        }
        Some(Value::Verdict(v)) => {
            kind = 3;
            let error = if v.angle {
                steps::deg(v.error)
            } else {
                steps::num(v.error)
            };
            lines.push(line(&words::relation(v.relation), format!("Δ = {error}")));
        }
        Some(other) => {
            kind = 4;
            lines.push(line("", words::show(other)));
            match other {
                Value::Vec3(v) => extra = format!("|v| = {}", steps::num(v.norm())),
                Value::Vec2(v) => {
                    extra = format!(
                        "|v| = {} · ∠X = {}",
                        steps::num(v.norm()),
                        steps::deg(v.y.atan2(v.x))
                    )
                }
                _ => {}
            }
            copyable = true;
        }
    }
    shell.set_math_value_kind(kind);
    shell.set_math_value_lines(ModelRc::new(VecModel::from(lines)));
    shell.set_math_matrix(ModelRc::new(VecModel::from(matrix)));
    shell.set_math_value_extra(extra.into());
    shell.set_math_nose(nose.to_degrees() as f32);
    shell.set_math_bank(bank.to_degrees() as f32);
    shell.set_math_copyable(copyable);
}

/// How far a matrix is from a rotation: its determinant and `|RᵀR − I|`.
fn orthonormal(m: rusty_embed::spatial::Mat3) -> String {
    format!(
        "det = {} · |RᵀR − I| = {}",
        steps::num(m.det()),
        steps::num(m.orthonormal_error())
    )
}

/// The examples, for the page's menu: a heading row first.
pub(super) fn example_names() -> Vec<SharedString> {
    std::iter::once(SharedString::from(t!("math.examples")))
        .chain(
            EXAMPLES
                .iter()
                .map(|e| SharedString::from(words::example(e.id))),
        )
        .collect()
}

/// The reference, a group at a time.
pub(super) fn help_rows() -> Vec<MathLine> {
    let mut out = Vec::new();
    for (group, functions) in words::groups() {
        out.push(MathLine {
            label: group.into(),
            value: SharedString::new(),
        });
        for (code, what) in functions {
            out.push(MathLine {
                label: code.into(),
                value: what.into(),
            });
        }
    }
    out
}
