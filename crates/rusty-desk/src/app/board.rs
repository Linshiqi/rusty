//! The board beside the code, Wokwi's layout: the sheet the project
//! declares (`.rusty/sim.toml`) drawn as the components on the desk, its
//! lamps lit and its wires coloured by what the firmware is doing, its
//! switches and keys pressed, its knobs turned and its sensors slid with
//! the pointer while a run goes on.
//!
//! Drawn in two layers. The picture — the devkit and every part's body —
//! is one SVG composed by `rusty_embed::board::picture`, and changes only
//! when the sheet does. Everything that moves is the window's own: a wire
//! is a path coloured by its net's level, a lamp a lens as bright as its
//! current, a digit's segments, a strip's pixels as the bytes on its wire
//! said, a screen as the firmware's driver drew it, a servo's horn where
//! the pulse puts it. What the board says about the pins goes through
//! `rusty_embed::period` — the rules and the circuit read at each moment of
//! one PWM period, which is also the answer with nothing on PWM — re-read
//! when a level or a press changes, re-weighted when only a duty does.
//!
//! The live layer's models are made once and changed row by row
//! ([`refresh`]): a model replaced under a held switch takes the switch's
//! press area with it, and its release goes nowhere.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use rusty_embed::{
    Drive, Instance, PinRef, Sheet, SimPlan,
    activity::Kind,
    board::{
        art, geometry,
        geometry::{EditPart, SERVO_MAX_US, SERVO_MIN_US},
        glow, picture,
    },
    nets::{self, Behaviour, Row, behaviour_of},
    period::{Level, Period},
    screen::Screen,
};
use rusty_i18n::t;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::App;
use crate::{
    AppWindow, BoardControl, BoardLamp, BoardPart, BoardRotor, BoardScreen, BoardShape,
    BoardSwitch, BoardWire, BoardWord, Shell, work,
};

/// What the board panel shows, and what it needs to answer a press.
#[derive(Default)]
pub(super) struct Board {
    /// The sheet as the rules read it — the parts' instances, the wires,
    /// every symbol the parts carry — kept in step with `parts`.
    pub(super) sheet: Option<Sheet>,
    rows: Vec<Row>,
    pub(super) parts: Vec<EditPart>,
    /// Which GPIO each part's named pins reach, worked out once a load:
    /// the conducting graph is the sheet's whole partition.
    gpio: HashMap<(String, String), u8>,
    /// What a part's picture is drawn with: the devkit's board, the chip's
    /// name on its can.
    kit: Option<geometry::KitStyle>,
    chip_label: String,
    /// Every symbol the sheet may place.
    pub(super) library: Vec<rusty_embed::Symbol>,
    /// The pins the author marked as reaching nothing on purpose.
    pub(super) no_connect: Vec<PinRef>,
    /// Each part's picture, by what it looks like, so moving a part draws
    /// nothing again.
    pictures: HashMap<String, Picture>,
    /// The editor: what is selected, what is in hand, the undo history.
    pub(super) edit: super::sheet::Edit,
    /// The switches held down, by reference.
    pressed: HashSet<String>,
    /// The keypad key held down: the part, its row and its column.
    key_down: Option<(usize, usize, usize)>,
    /// The screens the sheet declares, by bus address, as their drivers
    /// drew them.
    screens: HashMap<u8, Screen>,
    /// The sheet read over a PWM period, and what it was read for.
    period: Option<Period>,
    read_for: Option<ReadFor>,
    /// A redraw is on its way.
    due: bool,
}

/// What a period was read for — the levels, the PWM pins in their rank,
/// the switches held — so it is read again only when one of them moves.
#[derive(PartialEq)]
struct ReadFor {
    levels: Vec<(u8, bool)>,
    ranking: Vec<u8>,
    held: Vec<String>,
}

/// One part's picture, and where it sits relative to the part's anchor.
#[derive(Clone)]
struct Picture {
    image: slint::Image,
    origin: (f64, f64),
    size: (f64, f64),
}

/// Room round the parts in the view's box.
const MARGIN: f64 = 24.0;

/// The live layer's models, made once with the window.
pub(super) struct Models {
    parts: Rc<VecModel<BoardPart>>,
    wires: Rc<VecModel<BoardWire>>,
    lamps: Rc<VecModel<BoardLamp>>,
    switches: Rc<VecModel<BoardSwitch>>,
    shapes: Rc<VecModel<BoardShape>>,
    screens: Rc<VecModel<BoardScreen>>,
    rotors: Rc<VecModel<BoardRotor>>,
    controls: Rc<VecModel<BoardControl>>,
    texts: Rc<VecModel<BoardWord>>,
}

impl Models {
    pub(super) fn new(window: &AppWindow) -> Models {
        let models = Models {
            parts: Rc::default(),
            wires: Rc::default(),
            lamps: Rc::default(),
            switches: Rc::default(),
            shapes: Rc::default(),
            screens: Rc::default(),
            rotors: Rc::default(),
            controls: Rc::default(),
            texts: Rc::default(),
        };
        let shell = window.global::<Shell>();
        shell.set_board_parts(ModelRc::from(models.parts.clone()));
        shell.set_board_wires(ModelRc::from(models.wires.clone()));
        shell.set_board_lamps(ModelRc::from(models.lamps.clone()));
        shell.set_board_switches(ModelRc::from(models.switches.clone()));
        shell.set_board_shapes(ModelRc::from(models.shapes.clone()));
        shell.set_board_screens(ModelRc::from(models.screens.clone()));
        shell.set_board_rotors(ModelRc::from(models.rotors.clone()));
        shell.set_board_controls(ModelRc::from(models.controls.clone()));
        shell.set_board_texts(ModelRc::from(models.texts.clone()));
        models
    }

    fn clear(&self) {
        self.parts.set_vec(Vec::new());
        self.wires.set_vec(Vec::new());
        self.lamps.set_vec(Vec::new());
        self.switches.set_vec(Vec::new());
        self.shapes.set_vec(Vec::new());
        self.screens.set_vec(Vec::new());
        self.rotors.set_vec(Vec::new());
        self.controls.set_vec(Vec::new());
        self.texts.set_vec(Vec::new());
    }
}

/// A model brought to `rows`, changing only the rows that differ — the
/// elements drawn for the others are left as they are.
fn refresh<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model.row_data(index).as_ref() != Some(&row) {
            model.set_row_data(index, row);
        }
    }
}

/// The controls brought to `rows`. A slider moved by hand stops following
/// its row, so the sliders are made again whenever what they are changes —
/// a run starting or ending, the part they belong to — and otherwise only
/// what they say is.
fn refresh_controls(model: &VecModel<BoardControl>, rows: Vec<BoardControl>) {
    let same = model.row_count() == rows.len()
        && rows.iter().enumerate().all(|(index, row)| {
            model.row_data(index).is_some_and(|had| {
                had.index == row.index && had.key == row.key && had.live == row.live
            })
        });
    if same {
        refresh(model, rows);
    } else {
        model.set_vec(rows);
    }
}

/// The board, worked out off the window's thread: the picture is the
/// sheet's every symbol drawn, and the rows ask the catalogue.
struct Loaded {
    sheet: Sheet,
    rows: Vec<Row>,
    parts: Vec<EditPart>,
    gpio: HashMap<(String, String), u8>,
    kit: geometry::KitStyle,
    chip_label: String,
    library: Vec<rusty_embed::Symbol>,
}

/// The pins a part's face reads a GPIO through.
const FACE_PINS: [&str; 7] = ["W", "OUT", "DIN", "SIG", "PWM", "IN1", "IN2"];

fn load(root: PathBuf, plan: Option<SimPlan>) -> Option<Loaded> {
    let firmware = rusty_embed::project::firmware_root(&root);
    let plan = match plan {
        Some(plan) => plan,
        None => {
            let detected = rusty_embed::project::detect(&firmware).ok()?;
            rusty_embed::simulate::plan(&detected, false)
        }
    };
    let sheet = plan.board?;
    let rows = rusty_embed::simulate::kit_rows_for(&firmware, &sheet.chip);
    let catalog = rusty_embed::catalog::Catalog::load(Some(&root));
    let part = catalog.chip(&sheet.chip);
    let kit = geometry::kit_style(
        part.and_then(|c| c.kit.as_ref()),
        part.map_or("", |c| c.vendor_name.as_str()),
    );
    let parts = geometry::parts_of(&sheet, &rows);
    let gpio = face_gpios(&sheet, &rows, &parts);
    Some(Loaded {
        chip_label: sheet.chip.to_uppercase(),
        sheet,
        rows,
        parts,
        gpio,
        kit,
        library: plan.library,
    })
}

/// Which GPIO each face's pins reach, from the conducting graph.
fn face_gpios(sheet: &Sheet, rows: &[Row], parts: &[EditPart]) -> HashMap<(String, String), u8> {
    let mut gpio = HashMap::new();
    for part in parts {
        let Some(symbol) = part.symbol.as_ref() else {
            continue;
        };
        for pin in FACE_PINS {
            if symbol.pins.iter().any(|p| p.name == pin || p.number == pin)
                && let Some(at) = nets::gpio_of(sheet, rows, &part.inst.reference, pin)
            {
                gpio.insert((part.inst.reference.clone(), pin.to_string()), at);
            }
        }
    }
    gpio
}

/// What a part looks like, as a key: two parts that key alike draw alike.
fn look_of(part: &EditPart) -> String {
    let inst = &part.inst;
    if part.is_kit() {
        return format!("kit|{}|{}", inst.rot, inst.mirror);
    }
    format!(
        "{}|{}|{}|{}",
        inst.symbol, inst.value, inst.rot, inst.mirror
    )
}

impl Board {
    /// Each part's picture, placed — drawn once per look — and the words
    /// beside them.
    fn placed(&mut self) -> (Vec<BoardPart>, Vec<picture::Word>) {
        let Board {
            parts,
            rows,
            kit,
            chip_label,
            pictures,
            edit,
            ..
        } = self;
        let Some(kit) = kit.as_ref() else {
            return (Vec::new(), Vec::new());
        };
        let placed = parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let drawn = pictures.entry(look_of(part)).or_insert_with(|| {
                    let drawn = picture::part_picture(part, rows, kit, chip_label);
                    Picture {
                        image: slint::Image::load_from_svg_data(drawn.svg.as_bytes())
                            .unwrap_or_default(),
                        origin: drawn.origin,
                        size: drawn.size,
                    }
                });
                BoardPart {
                    x: (part.inst.x + drawn.origin.0) as f32,
                    y: (part.inst.y + drawn.origin.1) as f32,
                    w: drawn.size.0 as f32,
                    h: drawn.size.1 as f32,
                    image: drawn.image.clone(),
                    selected: edit.selected == Some(index),
                }
            })
            .collect();
        (placed, picture::words(parts, rows))
    }

    /// The parts moved but nothing was joined or parted: the sheet the
    /// rules read follows the parts, and is read again.
    pub(super) fn moved_parts(&mut self) {
        self.rebuild(false);
    }

    /// The sheet the rules read, from the parts and the wires as they are
    /// now; with `structural`, which GPIO each face reaches as well.
    pub(super) fn rebuild(&mut self, structural: bool) {
        let Some(old) = self.sheet.as_ref() else {
            return;
        };
        let sheet =
            super::sheet::sheet_with_symbols(&old.chip, &self.parts, &old.wires, &self.no_connect);
        if structural {
            self.gpio = face_gpios(&sheet, &self.rows, &self.parts);
        }
        self.sheet = Some(sheet);
        self.period = None;
        self.read_for = None;
    }
}

/// A polyline as path commands, in sheet units.
pub(super) fn commands(points: &[(f64, f64)]) -> String {
    let mut out = String::new();
    for (index, (x, y)) in points.iter().enumerate() {
        out.push_str(if index == 0 { "M " } else { " L " });
        out.push_str(&format!("{x:.2} {y:.2}"));
    }
    out
}

/// A box with rounded corners, as path commands.
fn rounded(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let r = r.min(w / 2.0).min(h / 2.0);
    format!(
        "M {:.2} {y:.2} H {:.2} A {r:.2} {r:.2} 0 0 1 {:.2} {:.2} V {:.2} A {r:.2} {r:.2} 0 0 1 {:.2} {:.2} \
         H {:.2} A {r:.2} {r:.2} 0 0 1 {x:.2} {:.2} V {:.2} A {r:.2} {r:.2} 0 0 1 {:.2} {y:.2} Z",
        x + r,
        x + w - r,
        x + w,
        y + r,
        y + h - r,
        x + w - r,
        y + h,
        x + r,
        y + h - r,
        y + r,
        x + r,
    )
}

/// A circle, as path commands.
fn circle(x: f64, y: f64, r: f64) -> String {
    format!(
        "M {:.2} {y:.2} A {r:.2} {r:.2} 0 1 0 {:.2} {y:.2} A {r:.2} {r:.2} 0 1 0 {:.2} {y:.2} Z",
        x - r,
        x + r,
        x - r
    )
}

/// What a sensor's reading is called on the board, in the window's language.
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

/// A reading as a person reads it: no trailing zeros, no `-0`.
fn reading_text(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// `#rrggbb` as a colour.
fn parse_colour(hex: &str) -> slint::Color {
    super::colour::hex(hex)
}

/// Everything the live layer draws, gathered before any model is touched.
#[derive(Default)]
struct Live {
    wires: Vec<BoardWire>,
    lamps: Vec<BoardLamp>,
    switches: Vec<BoardSwitch>,
    shapes: Vec<BoardShape>,
    screens: Vec<BoardScreen>,
    rotors: Vec<BoardRotor>,
    controls: Vec<BoardControl>,
    texts: Vec<BoardWord>,
}

/// What the simulator holds that the faces read, copied out of it.
struct Heard {
    levels: HashMap<u8, bool>,
    duties: HashMap<u8, rusty_embed::Duty>,
    display: Option<String>,
    rmt: HashMap<u8, Vec<u8>>,
    adc: HashMap<u8, u16>,
    analog: HashMap<u8, u16>,
    turns: HashMap<String, u8>,
    readings: HashMap<(String, String), f64>,
    sensors: Vec<rusty_embed::SensorDef>,
    sensor_values: HashMap<String, Vec<f32>>,
    specs: Vec<rusty_embed::sensor::Spec>,
    live: bool,
}

impl App {
    /// Read the board the project declares — from `plan` when a run has
    /// just made one — and draw it.
    pub(super) fn load_board(self: &Rc<Self>, plan: Option<SimPlan>) {
        let Some(root) = self.root() else { return };
        work::spawn(
            move || load(root, plan),
            |app, loaded| {
                let Some(window) = app.window() else { return };
                let shell = window.global::<Shell>();
                let Some(loaded) = loaded else {
                    // No board: nothing of the last project's left drawn.
                    *app.board.borrow_mut() = Board::default();
                    shell.set_board_shown(false);
                    shell.set_board_words(ModelRc::default());
                    app.board_models.clear();
                    return;
                };
                shell.set_board_shown(true);
                {
                    // A screen for every display part that names its
                    // controller: which controller is behind the glass
                    // decides how the bytes read, and that is the sheet's
                    // to say, never the traffic's.
                    let screens = loaded
                        .parts
                        .iter()
                        .filter(|part| {
                            part.symbol.as_ref().map(behaviour_of) == Some(Behaviour::Display)
                        })
                        .filter_map(|part| {
                            let panel =
                                rusty_embed::screen::Panel::from_id(part.inst.props.get("panel")?)?;
                            Some((geometry::display_address(part)?, Screen::of(panel)))
                        })
                        .collect();
                    let mut board = app.board.borrow_mut();
                    let pressed = std::mem::take(&mut board.pressed);
                    // A board loaded again keeps its history and its
                    // pictures: a run reloads what the editor just wrote.
                    let edit = std::mem::take(&mut board.edit);
                    let pictures = std::mem::take(&mut board.pictures);
                    *board = Board {
                        no_connect: loaded.sheet.no_connect.clone(),
                        sheet: Some(loaded.sheet),
                        rows: loaded.rows,
                        parts: loaded.parts,
                        gpio: loaded.gpio,
                        kit: Some(loaded.kit),
                        chip_label: loaded.chip_label,
                        library: loaded.library,
                        screens,
                        pressed,
                        edit,
                        pictures,
                        ..Board::default()
                    };
                    let count = board.parts.len();
                    board.edit.forget_beyond(count);
                }
                shell.set_board_fit(shell.get_board_fit() + 1);
                app.sync_board_parts();
                app.sync_board_live();
                app.sync_inspector();
            },
        );
    }

    /// The parts' pictures where the parts are, the words beside them, and
    /// the box the view frames.
    pub(super) fn sync_board_parts(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let (placed, words, ((x0, y0), (x1, y1))) = {
            let mut board = self.board.borrow_mut();
            let (placed, words) = board.placed();
            (placed, words, geometry::bounds(&board.parts))
        };
        refresh(&self.board_models.parts, placed);
        let words: Vec<BoardWord> = words
            .iter()
            .map(|word| BoardWord {
                x: word.x as f32,
                y: word.y as f32,
                anchor: match word.anchor {
                    "start" => 0,
                    "end" => 2,
                    _ => 1,
                },
                text: word.text.as_str().into(),
                size: word.size as f32,
                kind: match word.kind {
                    "reference" => 1,
                    "value" => 2,
                    _ => 0,
                },
            })
            .collect();
        shell.set_board_words(ModelRc::new(VecModel::from(words)));
        shell.set_board_x((x0 - MARGIN) as f32);
        shell.set_board_y((y0 - MARGIN) as f32);
        shell.set_board_width((x1 - x0 + 2.0 * MARGIN).max(1.0) as f32);
        shell.set_board_height((y1 - y0 + 2.0 * MARGIN).max(1.0) as f32);
    }

    /// The pins moved: the board redraws once a frame at most, since a
    /// breathing lamp reports a hundred duties a second.
    pub(super) fn board_moved(self: &Rc<Self>) {
        {
            let mut board = self.board.borrow_mut();
            if board.due || board.sheet.is_none() {
                return;
            }
            board.due = true;
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::from_millis(33), move || {
            if let Some(app) = weak.upgrade() {
                app.board.borrow_mut().due = false;
                app.sync_board_live();
            }
        });
    }

    /// An I2C write on its way past: drawn on the screen at that address,
    /// if the sheet declares one. `w+` is more of the message before it.
    pub(super) fn screen_heard(&self, report: &rusty_embed::I2cReport) -> bool {
        let mut board = self.board.borrow_mut();
        let Some(screen) = board.screens.get_mut(&report.address) else {
            return false;
        };
        match report.verb.as_str() {
            "w" => screen.i2c(&report.bytes),
            "w+" => screen.i2c_more(&report.bytes),
            _ => return false,
        }
        true
    }

    /// Whether what is moved on the board reaches the firmware now: a
    /// simulation is running and has started its emulator.
    pub(super) fn sim_live(&self) -> bool {
        self.activity_kind() == Some(Kind::Simulate)
            && (self.runner.takes_input() || self.state.borrow().sim.has_pins())
    }

    fn board_heard(&self) -> Heard {
        let live = self.sim_live();
        let state = self.state.borrow();
        let sim = &state.sim;
        Heard {
            levels: sim.levels.iter().map(|(p, l)| (*p, *l)).collect(),
            duties: sim.duties.iter().map(|(p, d)| (*p, *d)).collect(),
            display: sim.display.clone(),
            rmt: sim.rmt.clone(),
            adc: sim.adc.clone(),
            analog: sim.analog.clone(),
            turns: sim.turns.clone(),
            readings: sim.readings.clone(),
            sensors: sim.sensors.clone(),
            sensor_values: sim.sensor_values.clone(),
            specs: sim
                .plan
                .as_ref()
                .map(|plan| plan.parts.clone())
                .unwrap_or_default(),
            live,
        }
    }

    /// The live layer: every wire's level, every part's face, every control.
    pub(super) fn sync_board_live(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let heard = self.board_heard();
        let mut board = self.board.borrow_mut();
        let Board {
            sheet: Some(sheet),
            rows,
            parts,
            gpio,
            pressed,
            key_down,
            screens,
            period,
            read_for,
            edit,
            ..
        } = &mut *board
        else {
            return;
        };
        let selected_wire = edit.wire;
        // Read again only when what the reading depends on moved.
        let wired = rusty_embed::period::wired_gpios(&sheet.wires, rows);
        let ranking = rusty_embed::period::ranking(&heard.duties, &wired);
        let mut sorted_levels: Vec<(u8, bool)> =
            heard.levels.iter().map(|(p, l)| (*p, *l)).collect();
        sorted_levels.sort();
        let mut held: Vec<String> = pressed.iter().cloned().collect();
        held.sort();
        let key = ReadFor {
            levels: sorted_levels,
            ranking: ranking.clone(),
            held,
        };
        if read_for.as_ref() != Some(&key) || period.is_none() {
            *period = Some(Period::read(sheet, rows, pressed, &heard.levels, ranking));
            *read_for = Some(key);
        }
        let Some(period) = period.as_ref() else {
            return;
        };
        let weights = period.weights(&heard.duties);

        let mut live = Live::default();
        for wire in &sheet.wires {
            let Some(ends) = geometry::wire_ends(parts, wire) else {
                continue;
            };
            let points = geometry::wire_path(&ends, &wire.bends);
            let tone = match period.level(&weights, &wire.from) {
                Some(Level::High) => 2,
                Some(Level::Low) => 1,
                Some(Level::Switching(_)) => 3,
                None => 0,
            };
            live.wires.push(BoardWire {
                commands: commands(&points).into(),
                tone,
                selected: selected_wire == Some(live.wires.len()),
            });
        }

        let faces = Faces {
            period,
            weights: &weights,
            gpio,
            heard: &heard,
            screens,
            pressed,
            key_down: *key_down,
        };
        for (index, part) in parts.iter().enumerate() {
            faces.draw(index, part, &mut live);
        }
        drop(board);
        let models = &self.board_models;
        refresh(&models.wires, live.wires);
        refresh(&models.lamps, live.lamps);
        refresh(&models.switches, live.switches);
        refresh(&models.shapes, live.shapes);
        refresh(&models.screens, live.screens);
        refresh(&models.rotors, live.rotors);
        refresh(&models.texts, live.texts);
        refresh_controls(&models.controls, live.controls);
        shell.set_board_editing(self.board_editing());
        let source = self.state.borrow().sim.source;
        shell.set_board_caption(SharedString::from(match source {
            Some(rusty_embed::PinSource::Emulator) => t!("desk.board-emulator"),
            Some(rusty_embed::PinSource::Firmware) => t!("desk.board-firmware"),
            None => String::new(),
        }));
    }

    /// A switch or a key pressed or let go with the pointer: held on the
    /// board, and told to the emulator while a run goes on — a key between
    /// two GPIOs joins them, a switch to a rail drives one.
    pub(super) fn board_press(self: &Rc<Self>, index: usize, key: i32, down: bool) {
        let (tie, drive) = {
            let mut board = self.board.borrow_mut();
            let Some(part) = board.parts.get(index) else {
                return;
            };
            let reference = part.inst.reference.clone();
            let Some(sheet) = board.sheet.clone() else {
                return;
            };
            if key >= 0 {
                let (row, column) = (key as usize / 4, key as usize % 4);
                if down {
                    board.key_down = Some((index, row, column));
                } else if board.key_down == Some((index, row, column)) {
                    board.key_down = None;
                } else {
                    return;
                }
                (
                    nets::keypad_tie(&sheet, &board.rows, &reference, row, column),
                    None,
                )
            } else {
                if down {
                    board.pressed.insert(reference.clone());
                } else {
                    board.pressed.remove(&reference);
                }
                (
                    nets::switch_tie(&sheet, &board.rows, &reference),
                    nets::button_drives(&sheet, &board.rows, &reference),
                )
            }
        };
        self.sim_press(tie, drive.map(|(gpio, _)| gpio), down);
        self.sync_board_live();
    }

    /// A slider under a part moved: a knob turned, a source slid, one
    /// reading of a sensor. Held for the board, and while a run goes on,
    /// sent where the firmware reads it.
    pub(super) fn board_control(self: &Rc<Self>, index: usize, key: &str, value: f32) {
        let (inst, sheet, rows, gpio) = {
            let board = self.board.borrow();
            let Some(part) = board.parts.get(index) else {
                return;
            };
            let Some(sheet) = board.sheet.clone() else {
                return;
            };
            (
                part.inst.clone(),
                sheet,
                board.rows.clone(),
                board.gpio.clone(),
            )
        };
        let reference = inst.reference.clone();
        let at = |pin: &str| gpio.get(&(reference.clone(), pin.to_string())).copied();
        match key {
            "pot" => {
                let turn = value.round().clamp(0.0, 255.0) as u8;
                self.state
                    .borrow_mut()
                    .sim
                    .turns
                    .insert(reference.clone(), turn);
                // The text line, for firmware that reads rusty's protocol.
                if let Some(wiper) = at("W") {
                    self.sim_send(&rusty_embed::protocol::pot_line(u32::from(wiper), turn));
                }
                // And the counts, for firmware that just converts — only
                // where the sheet said what the track's ends are on.
                if let Some(span) = nets::pot_span(&sheet, &rows, &reference) {
                    let counts = span.counts(turn, nets::adc_max(&inst));
                    self.sim_send(&rusty_embed::analog_line(span.gpio, counts));
                }
            }
            "analog" => {
                let Some(pin) = at("OUT") else { return };
                let counts = value.round().clamp(0.0, f32::from(nets::adc_max(&inst))) as u16;
                self.state.borrow_mut().sim.analog.insert(pin, counts);
                self.sim_send(&rusty_embed::analog_line(pin, counts));
            }
            _ => {
                if let Some(reading) = key.strip_prefix("reg:") {
                    let value = f64::from(value);
                    self.state
                        .borrow_mut()
                        .sim
                        .readings
                        .insert((reference.clone(), reading.to_string()), value);
                    if self.sim_live()
                        && let Some(refused) = self.sim_sensor_set(&reference, reading, value)
                    {
                        self.note_transient(1, refused);
                    }
                } else if let Some(axis) = key.strip_prefix("axis:") {
                    let Ok(axis) = axis.parse::<usize>() else {
                        return;
                    };
                    let name = inst.value.trim().to_string();
                    let line = {
                        let mut state = self.state.borrow_mut();
                        let sim = &mut state.sim;
                        let count = sim
                            .sensors
                            .iter()
                            .find(|s| s.name == name)
                            .map_or(1, |s| s.components.max(1) as usize);
                        // A sample travels whole: the other axes go with it
                        // as they stand.
                        let sample = sim
                            .sensor_values
                            .entry(name.clone())
                            .or_insert_with(|| vec![0.0; count]);
                        sample.resize(count, 0.0);
                        if let Some(slot) = sample.get_mut(axis) {
                            *slot = value;
                        }
                        rusty_embed::sensor_line(&name, sample)
                    };
                    self.sim_send_console(&line);
                }
            }
        }
        self.sync_board_live();
    }
}

/// What every face is drawn from, for one redraw.
struct Faces<'a> {
    period: &'a Period,
    weights: &'a [f64],
    gpio: &'a HashMap<(String, String), u8>,
    heard: &'a Heard,
    screens: &'a HashMap<u8, Screen>,
    pressed: &'a HashSet<String>,
    key_down: Option<(usize, usize, usize)>,
}

impl Faces<'_> {
    fn gpio(&self, reference: &str, pin: &str) -> Option<u8> {
        self.gpio
            .get(&(reference.to_string(), pin.to_string()))
            .copied()
    }

    fn level(&self, reference: &str, pin: &str) -> Option<bool> {
        match self
            .period
            .level(self.weights, &PinRef::new(reference, pin))
        {
            Some(Level::High) => Some(true),
            Some(Level::Low) => Some(false),
            _ => None,
        }
    }

    fn draw(&self, index: usize, part: &EditPart, live: &mut Live) {
        let Some(symbol) = part.symbol.as_ref() else {
            return;
        };
        let inst = &part.inst;
        let reference = inst.reference.as_str();
        let plan = art::layout(symbol, &inst.value);
        // The drawing turns with the part; what is written on it does not.
        let on_sheet = |(x, y): (f64, f64)| {
            let (dx, dy) = geometry::orient((x, y), inst.rot, inst.mirror);
            (inst.x + dx, inst.y + dy)
        };
        // Under the part, where what is handled hangs.
        let (x0, _, x1, y1) = geometry::part_box(part);
        let below = (((x0 + x1) / 2.0) as f32, (y1 + 14.0) as f32);
        match behaviour_of(symbol) {
            Behaviour::Led | Behaviour::Rgb => {
                let Some((lx, ly, r)) = plan.lens else { return };
                let (x, y) = on_sheet((lx, ly));
                let (lit, dark, light) = if behaviour_of(symbol) == Behaviour::Rgb {
                    let share = |pin: &str| self.period.pin_lit(self.weights, reference, pin);
                    let (colour, level) = glow::lens(share("R"), share("G"), share("B"));
                    (colour, geometry::rgb_color(false, false, false), level)
                } else {
                    // As bright as its current where the circuit is
                    // solved; where it is not, the rules' lit share at full
                    // current.
                    let light = match self.period.reading(self.weights, reference) {
                        Ok(Some(measured)) => glow::of_current(measured.through.abs()),
                        _ => glow::of_share(self.period.lit(self.weights, reference)),
                    };
                    let (lit, dark) = geometry::lamp_colors(&inst.value);
                    (lit.to_string(), dark, light)
                };
                live.lamps.push(BoardLamp {
                    x: x as f32,
                    y: y as f32,
                    r: r as f32,
                    lit: parse_colour(&lit),
                    dark: parse_colour(dark),
                    light: light as f32,
                    square: false,
                });
            }
            Behaviour::Switch => {
                let Some((lx, ly, r)) = plan.lens else { return };
                let (x, y) = on_sheet((lx, ly));
                live.switches.push(BoardSwitch {
                    x: x as f32,
                    y: y as f32,
                    w: (r * 2.0) as f32,
                    h: (r * 2.0) as f32,
                    radius: r as f32,
                    held: self.pressed.contains(reference),
                    index: index as i32,
                    key: -1,
                });
            }
            // Sixteen caps, each its own target: a key is what is pressed,
            // and the part only what carries them.
            Behaviour::Keypad => {
                for (at, (_, kx, ky, half)) in art::keypad_keys(&plan).into_iter().enumerate() {
                    let (row, column) = (at / 4, at % 4);
                    let (x, y) = on_sheet((kx, ky));
                    live.switches.push(BoardSwitch {
                        x: x as f32,
                        y: y as f32,
                        w: (half * 2.0) as f32,
                        h: (half * 2.0) as f32,
                        radius: 2.0,
                        held: self.key_down == Some((index, row, column)),
                        index: index as i32,
                        key: at as i32,
                    });
                }
            }
            // Each segment lit for its own share of the period, so a digit
            // dimmed by PWM is a dim digit.
            Behaviour::Seven => {
                let Some((fx, fy, fw, fh)) = plan.face else {
                    return;
                };
                let (cx, cy) = on_sheet((fx + fw / 2.0, fy + fh / 2.0));
                let (ox, oy) = (cx - 13.0, cy - 21.0);
                for (name, x, y, w, h) in [
                    ("a", 6.0, 2.0, 14.0, 4.0),
                    ("b", 19.0, 5.0, 4.0, 13.0),
                    ("c", 19.0, 23.0, 4.0, 13.0),
                    ("d", 6.0, 36.0, 14.0, 4.0),
                    ("e", 3.0, 23.0, 4.0, 13.0),
                    ("f", 3.0, 5.0, 4.0, 13.0),
                    ("g", 6.0, 19.0, 14.0, 4.0),
                ] {
                    let share = self.period.pin_lit(self.weights, reference, name);
                    let fill = glow::mix("#3a2323", "#ff5c5c", glow::of_share(share));
                    live.shapes.push(BoardShape {
                        commands: rounded(ox + x, oy + y, w, h, 2.0).into(),
                        fill: parse_colour(&fill),
                        stroke: slint::Color::from_argb_u8(0, 0, 0, 0),
                        width: 0.0,
                    });
                }
            }
            // A chain of addressable LEDs, lit by the bytes its wire
            // carried rather than by a level: dark is dark, whether told to
            // be black or not yet reached.
            Behaviour::Strip => {
                let colours = self
                    .gpio(reference, "DIN")
                    .and_then(|pin| self.heard.rmt.get(&pin))
                    .map(|bytes| rusty_embed::strip_colours(bytes))
                    .unwrap_or_default();
                for (at, (lx, ly, r)) in art::strip_lenses(&plan).into_iter().enumerate() {
                    let (red, green, blue) = colours.get(at).copied().unwrap_or((0, 0, 0));
                    let lit = u16::from(red) + u16::from(green) + u16::from(blue) > 0;
                    let (x, y) = on_sheet((lx, ly));
                    live.lamps.push(BoardLamp {
                        x: x as f32,
                        y: y as f32,
                        r: (r * 0.72) as f32,
                        lit: slint::Color::from_rgb_u8(red, green, blue),
                        dark: parse_colour("#15171b"),
                        light: if lit { 1.0 } else { 0.0 },
                        square: true,
                    });
                }
            }
            // What the firmware's display driver drew, where the sheet says
            // which controller is behind the glass; otherwise what the
            // firmware prints to its display line. Upright either way.
            Behaviour::Display => {
                let Some((fx, fy, fw, fh)) = plan.face else {
                    return;
                };
                let (cx, cy) = on_sheet((fx + fw / 2.0, fy + fh / 2.0));
                match geometry::display_address(part).and_then(|a| self.screens.get(&a)) {
                    Some(screen) => {
                        let (w, h) = (screen.width() as f64, screen.height() as f64);
                        let scale = (fw / w).min(fh / h);
                        live.screens.push(BoardScreen {
                            x: (cx - w * scale / 2.0) as f32,
                            y: (cy - h * scale / 2.0) as f32,
                            w: (w * scale) as f32,
                            h: (h * scale) as f32,
                            commands: geometry::pixel_path(screen).into(),
                            columns: w as f32,
                            rows: h as f32,
                            on: screen.is_on(),
                        });
                    }
                    None => {
                        let text = match self.heard.display.as_deref() {
                            Some(text) if !text.is_empty() => text.to_string(),
                            _ => "········".to_string(),
                        };
                        live.texts.push(BoardWord {
                            x: cx as f32,
                            y: (cy + 3.0) as f32,
                            anchor: 1,
                            text: text.into(),
                            size: 8.5,
                            kind: 3,
                        });
                    }
                }
            }
            // A sounder says it is sounding — a ring rather than a noise
            // nobody asked their machine to make.
            Behaviour::Buzzer => {
                let Some((lx, ly, r)) = plan.lens else { return };
                let (x, y) = on_sheet((lx, ly));
                let on = self.period.lit(self.weights, reference) > 0.0;
                live.shapes.push(BoardShape {
                    commands: circle(x, y, r).into(),
                    fill: slint::Color::from_argb_u8(0, 0, 0, 0),
                    stroke: parse_colour(if on { "#ffd75c" } else { "#4a515c" }),
                    width: if on { 2.4 } else { 1.0 },
                });
            }
            // The horn follows the pulse on the signal pin, which is what a
            // servo is told and all a sheet can honestly show. The ends are
            // the part's: 500..2500 and 1000..2000 are both ordinary.
            Behaviour::Servo => {
                let Some((fx, fy, fw, fh)) = plan.face else {
                    return;
                };
                let (cx, cy) = on_sheet((fx + fw / 2.0 + 18.0, fy + fh / 2.0));
                let drive = self
                    .gpio(reference, "SIG")
                    .and_then(|pin| self.heard.duties.get(&pin));
                let min = inst.prop::<f32>("min").unwrap_or(SERVO_MIN_US);
                let max = inst.prop::<f32>("max").unwrap_or(SERVO_MAX_US);
                let angle = drive.map(|d| f64::from(d.servo_angle(min, max)) - 90.0);
                let (sin, cos) = angle.unwrap_or(0.0).to_radians().sin_cos();
                let turn = |(x, y): (f64, f64)| (cx + x * cos - y * sin, cy + x * sin + y * cos);
                let horn: Vec<(f64, f64)> = [(-2.0, -16.0), (2.0, -16.0), (2.0, 2.0), (-2.0, 2.0)]
                    .into_iter()
                    .map(turn)
                    .collect();
                live.shapes.push(BoardShape {
                    commands: format!("{} Z", commands(&horn)).into(),
                    fill: parse_colour(if angle.is_some() {
                        "#e3e7ec"
                    } else {
                        "#5a626e"
                    }),
                    stroke: slint::Color::from_argb_u8(0, 0, 0, 0),
                    width: 0.0,
                });
                live.shapes.push(BoardShape {
                    commands: circle(cx, cy, 3.0).into(),
                    fill: parse_colour("#9aa2ae"),
                    stroke: slint::Color::from_argb_u8(0, 0, 0, 0),
                    width: 0.0,
                });
            }
            // Duty on PWM, direction from the bridge's two inputs; a braked
            // or coasting rotor stands, whatever the duty.
            Behaviour::Motor => {
                let duty = self
                    .gpio(reference, "PWM")
                    .and_then(|pin| self.heard.duties.get(&pin))
                    .map(|d| d.duty);
                // A fan has no direction pins, and two unwired inputs read
                // as two lows would call it COAST.
                let drive = match (self.level(reference, "IN1"), self.level(reference, "IN2")) {
                    (None, None) => Drive::Forward,
                    (a, b) => Drive::from_inputs(a.unwrap_or(false), b.unwrap_or(false)),
                };
                let speed = match duty {
                    Some(d) if d > 0.01 && drive.turns() => {
                        let turns = 1.0 / (0.25 / d).clamp(0.25, 4.0);
                        if drive == Drive::Reverse {
                            -turns
                        } else {
                            turns
                        }
                    }
                    _ => 0.0,
                };
                let (x, y) = on_sheet((-1.0, 0.0));
                live.rotors.push(BoardRotor {
                    x: x as f32,
                    y: y as f32,
                    r: 13.0,
                    speed,
                });
                live.texts.push(BoardWord {
                    x: below.0,
                    y: below.1 + 10.0,
                    anchor: 1,
                    text: match duty {
                        None => t!("simulate.no-duty").into(),
                        Some(d) => format!("{:.0}% {}", d * 100.0, drive.label()).into(),
                    },
                    size: 9.0,
                    kind: if duty.is_some() && drive.turns() {
                        4
                    } else {
                        5
                    },
                });
            }
            // A knob: the text line for firmware reading rusty's protocol,
            // and counts where the sheet says what the track's ends are on.
            Behaviour::Pot => {
                let turn = self
                    .heard
                    .turns
                    .get(reference)
                    .copied()
                    .unwrap_or_else(|| nets::pot_start(inst));
                live.controls.push(self.control(
                    index,
                    "pot",
                    format!("{reference} {}", inst.value),
                    Some((0.0, 255.0, f32::from(turn))),
                    turn.to_string(),
                    t!("simulate.pot-hint"),
                ));
            }
            // Counts, and the count is what is shown: rusty does not know
            // the divider on this board.
            Behaviour::Analog => {
                let pin = self.gpio(reference, "OUT");
                let start = nets::analog_start(inst);
                let held = pin
                    .and_then(|p| self.heard.analog.get(&p).copied())
                    .unwrap_or(start);
                // What the firmware's own converter last took off the pin:
                // the difference between a slider that does nothing and
                // firmware that is not reading.
                let tip = match pin.and_then(|p| self.heard.adc.get(&p)) {
                    Some(counts) => t!("simulate.adc-read", counts = counts),
                    None => t!("simulate.adc-unread"),
                };
                live.controls.push(self.control(
                    index,
                    "analog",
                    format!("{reference} {}", inst.value),
                    Some((0.0, f32::from(nets::adc_max(inst)), f32::from(held))),
                    held.to_string(),
                    tip,
                ));
            }
            Behaviour::Sensor => self.sensor(index, inst, live),
            _ => {}
        }
    }

    /// A sensor's sliders: one per reading of a part rusty answers for
    /// register by register, or one per axis of a channel the firmware
    /// declared — and none for a channel it never declared, since a range
    /// rusty invented is how somebody injects 2000 °/s into a loop
    /// written for 250.
    fn sensor(&self, index: usize, inst: &Instance, live: &mut Live) {
        let reference = inst.reference.as_str();
        let spec = inst
            .props
            .get("model")
            .and_then(|id| rusty_embed::sensor::Spec::find(&self.heard.specs, id));
        if let Some(spec) = spec {
            live.controls
                .push(self.heading(index, format!("{reference} {}", spec.name)));
            for channel in &spec.channels {
                let start = inst
                    .props
                    .get(&channel.key)
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .unwrap_or(channel.rest);
                let value = self
                    .heard
                    .readings
                    .get(&(reference.to_string(), channel.key.clone()))
                    .copied()
                    .unwrap_or(start);
                live.controls.push(self.control(
                    index,
                    &format!("reg:{}", channel.key),
                    reading_label(&channel.key),
                    Some((channel.min as f32, channel.max as f32, value as f32)),
                    format!("{} {}", reading_text(value), channel.unit),
                    String::new(),
                ));
            }
            return;
        }
        // A part on the bus with no model answers the bytes its `regs`
        // fix, and there is nothing in them to slide.
        if inst.props.contains_key("addr") {
            return;
        }
        let wanted = inst.value.trim();
        let Some(def) = self.heard.sensors.iter().find(|s| s.name == wanted) else {
            let said = if wanted.is_empty() {
                t!("simulate.sensor-unnamed")
            } else {
                t!("simulate.sensor-undeclared", name = wanted)
            };
            live.controls
                .push(self.heading(index, format!("{reference} · {said}")));
            return;
        };
        let count = def.components.max(1) as usize;
        let (min, max) = (def.min.unwrap_or(-1.0), def.max.unwrap_or(1.0));
        let unit = def.unit.clone().unwrap_or_default();
        live.controls
            .push(self.heading(index, format!("{reference} {} {unit}", def.name)));
        let held = self.heard.sensor_values.get(&def.name);
        for axis in 0..count {
            let value = held.and_then(|v| v.get(axis).copied()).unwrap_or(0.0);
            let label = match (count, axis) {
                (3, 0) => "x".to_string(),
                (3, 1) => "y".to_string(),
                (3, 2) => "z".to_string(),
                _ => format!("{axis}"),
            };
            live.controls.push(self.control(
                index,
                &format!("axis:{axis}"),
                label,
                Some((min, max, value)),
                format!("{value:.2}"),
                String::new(),
            ));
        }
    }

    fn heading(&self, index: usize, text: String) -> BoardControl {
        self.control(index, "", text, None, String::new(), String::new())
    }

    fn control(
        &self,
        index: usize,
        key: &str,
        label: String,
        slider: Option<(f32, f32, f32)>,
        text: String,
        tip: String,
    ) -> BoardControl {
        let (min, max, value) = slider.unwrap_or_default();
        BoardControl {
            index: index as i32,
            key: key.into(),
            label: label.into(),
            slider: slider.is_some(),
            min,
            max,
            value,
            text: text.into(),
            tip: tip.into(),
            live: self.heard.live,
        }
    }
}
