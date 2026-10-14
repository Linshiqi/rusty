//! The board beside the code, Wokwi's layout: the sheet the project
//! declares (`.rusty/sim.toml`) drawn as the components on the desk, its
//! lamps lit and its wires coloured by what the firmware is doing, its
//! switches pressed with the pointer while a run goes on.
//!
//! Drawn in two layers. The picture — the devkit and every part's body —
//! is one SVG composed by `rusty_embed::board::picture`, and changes only
//! when the sheet does. Everything that moves is the window's own: a wire
//! is a path coloured by its net's level, a lamp a lens as bright as its
//! current, a switch's cap sunk while held. What the board says about the
//! pins goes through `rusty_embed::period` — the rules and the circuit read
//! at each moment of one PWM period, which is also the answer with nothing
//! on PWM — re-read when a level or a press changes, re-weighted when only
//! a duty does.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use rusty_embed::{
    Sheet, SimPlan,
    board::{art, geometry, glow, picture},
    nets::{Behaviour, Row, behaviour_of},
    period::{Level, Period},
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::{BoardLamp, BoardSwitch, BoardWire, BoardWord, Shell, work};

/// What the board panel shows, and what it needs to answer a press.
#[derive(Default)]
pub(super) struct Board {
    sheet: Option<Sheet>,
    rows: Vec<Row>,
    parts: Vec<geometry::EditPart>,
    /// The switches held down, by reference.
    pressed: HashSet<String>,
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

/// The board, worked out off the window's thread: the picture is the
/// sheet's every symbol drawn, and the rows ask the catalogue.
struct Loaded {
    sheet: Sheet,
    rows: Vec<Row>,
    parts: Vec<geometry::EditPart>,
    picture: picture::Picture,
    words: Vec<picture::Word>,
}

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
    let picture = picture::picture(&parts, &rows, &kit, &sheet.chip.to_uppercase());
    let words = picture::words(&parts, &rows);
    Some(Loaded {
        sheet,
        rows,
        parts,
        picture,
        words,
    })
}

/// A polyline as path commands, in sheet units.
fn commands(points: &[(f64, f64)]) -> String {
    let mut out = String::new();
    for (index, (x, y)) in points.iter().enumerate() {
        out.push_str(if index == 0 { "M " } else { " L " });
        out.push_str(&format!("{x:.2} {y:.2}"));
    }
    out
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
                    shell.set_board_wires(ModelRc::default());
                    shell.set_board_lamps(ModelRc::default());
                    shell.set_board_switches(ModelRc::default());
                    shell.set_board_words(ModelRc::default());
                    return;
                };
                let image = slint::Image::load_from_svg_data(loaded.picture.svg.as_bytes())
                    .unwrap_or_default();
                shell.set_board_image(image);
                shell.set_board_x(loaded.picture.origin.0 as f32);
                shell.set_board_y(loaded.picture.origin.1 as f32);
                shell.set_board_width(loaded.picture.size.0 as f32);
                shell.set_board_height(loaded.picture.size.1 as f32);
                let words: Vec<BoardWord> = loaded
                    .words
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
                shell.set_board_shown(true);
                {
                    let mut board = app.board.borrow_mut();
                    let pressed = std::mem::take(&mut board.pressed);
                    *board = Board {
                        sheet: Some(loaded.sheet),
                        rows: loaded.rows,
                        parts: loaded.parts,
                        pressed,
                        ..Board::default()
                    };
                }
                app.sync_board_live();
            },
        );
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

    /// The live layer: every wire's level, every lamp's light, every switch.
    pub(super) fn sync_board_live(&self) {
        let Some(window) = self.window() else { return };
        let shell = window.global::<Shell>();
        let (levels, duties) = {
            let state = self.state.borrow();
            let gpio: HashMap<u8, bool> = state.sim.levels.iter().map(|(p, l)| (*p, *l)).collect();
            let duties: HashMap<u8, rusty_embed::Duty> =
                state.sim.duties.iter().map(|(p, d)| (*p, *d)).collect();
            (gpio, duties)
        };
        let mut board = self.board.borrow_mut();
        let Board {
            sheet: Some(sheet),
            rows,
            parts,
            pressed,
            period,
            read_for,
            ..
        } = &mut *board
        else {
            return;
        };
        // Read again only when what the reading depends on moved.
        let wired = rusty_embed::period::wired_gpios(&sheet.wires, rows);
        let ranking = rusty_embed::period::ranking(&duties, &wired);
        let mut sorted_levels: Vec<(u8, bool)> = levels.iter().map(|(p, l)| (*p, *l)).collect();
        sorted_levels.sort();
        let mut held: Vec<String> = pressed.iter().cloned().collect();
        held.sort();
        let key = ReadFor {
            levels: sorted_levels,
            ranking: ranking.clone(),
            held,
        };
        if read_for.as_ref() != Some(&key) || period.is_none() {
            *period = Some(Period::read(sheet, rows, pressed, &levels, ranking));
            *read_for = Some(key);
        }
        let Some(period) = period.as_ref() else {
            return;
        };
        let weights = period.weights(&duties);

        let mut wires = Vec::new();
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
            wires.push(BoardWire {
                commands: commands(&points).into(),
                tone,
            });
        }

        let mut lamps = Vec::new();
        let mut switches = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let Some(symbol) = part.symbol.as_ref() else {
                continue;
            };
            let behaviour = behaviour_of(symbol);
            let Some((cx, cy, r)) = art::layout(symbol, &part.inst.value).lens else {
                continue;
            };
            let (dx, dy) = geometry::orient((cx, cy), part.inst.rot, part.inst.mirror);
            let (x, y) = (part.inst.x + dx, part.inst.y + dy);
            let reference = &part.inst.reference;
            match behaviour {
                Behaviour::Led => {
                    // As bright as its current where the circuit is solved;
                    // where it is not, the rules' lit share at full current.
                    let light = match period.reading(&weights, reference) {
                        Ok(Some(measured)) => glow::of_current(measured.through.abs()),
                        _ => glow::of_share(period.lit(&weights, reference)),
                    };
                    let (lit, dark) = geometry::lamp_colors(&part.inst.value);
                    lamps.push(BoardLamp {
                        x: x as f32,
                        y: y as f32,
                        r: r as f32,
                        lit: parse_colour(lit),
                        dark: parse_colour(dark),
                        light: light as f32,
                    });
                }
                Behaviour::Switch => switches.push(BoardSwitch {
                    x: x as f32,
                    y: y as f32,
                    r: r as f32,
                    held: pressed.contains(reference),
                    index: index as i32,
                }),
                _ => {}
            }
        }
        drop(board);
        shell.set_board_wires(ModelRc::new(VecModel::from(wires)));
        shell.set_board_lamps(ModelRc::new(VecModel::from(lamps)));
        shell.set_board_switches(ModelRc::new(VecModel::from(switches)));
        let source = self.state.borrow().sim.source;
        shell.set_board_caption(SharedString::from(match source {
            Some(rusty_embed::PinSource::Emulator) => rusty_i18n::t!("desk.board-emulator"),
            Some(rusty_embed::PinSource::Firmware) => rusty_i18n::t!("desk.board-firmware"),
            None => String::new(),
        }));
    }

    /// A switch pressed or let go with the pointer: held on the board, and
    /// told to the emulator while a run goes on — a key between two GPIOs
    /// joins them, a switch to a rail drives one.
    pub(super) fn board_press(self: &Rc<Self>, index: usize, down: bool) {
        let (tie, drive) = {
            let mut board = self.board.borrow_mut();
            let Some(part) = board.parts.get(index) else {
                return;
            };
            let reference = part.inst.reference.clone();
            if down {
                board.pressed.insert(reference.clone());
            } else {
                board.pressed.remove(&reference);
            }
            let Some(sheet) = board.sheet.as_ref() else {
                return;
            };
            (
                rusty_embed::nets::switch_tie(sheet, &board.rows, &reference),
                rusty_embed::nets::button_drives(sheet, &board.rows, &reference),
            )
        };
        self.sim_press(tie, drive.map(|(gpio, _)| gpio), down);
        self.sync_board_live();
    }
}

/// `#rrggbb` as a colour.
fn parse_colour(hex: &str) -> slint::Color {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)
}
