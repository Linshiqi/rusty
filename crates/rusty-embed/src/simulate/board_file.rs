//! `.rusty/sim.toml`, as it is written on disk.
//!
//! Its own types, converted to and from the wire model — the file/wire split
//! every user-authored TOML here gets, so a panel refactor cannot silently
//! break the files people wrote (rule 2).
//!
//! **One definition per part, used in both directions.** Reading and writing
//! were two parallel sets of structs, and parallel sets drift: `flip` was
//! added to the wire model and to *neither* of them, so mirroring a part was
//! dropped on save and read back as `false` — the mirror survived until the
//! project was reopened. Sharing the definition makes that omission a
//! compile error instead of a silent loss.
//!
//! Its own module because the format and both conversions were four hundred
//! lines of `simulate.rs`, under a header that promised "the simulator, and
//! nothing else".

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{
    SimAnalog, SimBoard, SimButton, SimDisplay, SimLed, SimMotor, SimPot, SimRgb, SimSeven,
    UNWIRED_PIN,
};

#[derive(Debug, Default, Deserialize, Serialize)]
struct Board {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
}

/// Where a part sits, in the file's spelling.
///
/// Flattened, so the keys stay where a hand-written file puts them —
/// `x`, `y`, `rot`, `flip` directly inside `[[led]]`, not under a
/// sub-table nobody asked for. The wire model nests instead, for a reason
/// that does not apply here: TOML is self-describing and its own
/// deserializer types integers, which is what flatten needs.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    routes: Vec<Vec<(f64, f64)>>,
    #[serde(default, skip_serializing_if = "crate::model::is_upright")]
    rot: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    flip: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Led {
    pin: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Button {
    pin: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Seven {
    pins: [u8; 7],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Display {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// Absent means "not wired yet" — old board files carry no pins at
    /// all, and an unwired screen still shows text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sda: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scl: Option<u8>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Analog {
    pin: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<u16>,
    /// What the count means on this board, in the author's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Motor {
    /// Absent means "not wired". A motor with no duty pin is drawn and
    /// says it has nothing to be driven by, rather than sitting at zero
    /// as though the firmware had commanded a stop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pwm: Option<u8>,
    /// The H-bridge direction pins. Both absent is a fan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    in1: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    in2: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Pot {
    pin: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

/// Values before tables: TOML puts `[board]` after nothing, and every
/// array-of-tables after that. Reordering these fields reorders the file.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Sheet {
    #[serde(default)]
    board: Board,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    led: Vec<Led>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    button: Vec<Button>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rgb: Vec<Rgb>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    seven: Vec<Seven>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    display: Vec<Display>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pot: Vec<Pot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    motor: Vec<Motor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    analog: Vec<Analog>,
}

impl Sheet {
    fn is_empty(&self) -> bool {
        self.led.is_empty()
            && self.button.is_empty()
            && self.rgb.is_empty()
            && self.seven.is_empty()
            && self.display.is_empty()
            && self.pot.is_empty()
            && self.motor.is_empty()
            && self.analog.is_empty()
    }
}

impl Place {
    /// `into_`, not `to_`: this consumes the file record so the route
    /// vectors move rather than being cloned on every board load.
    fn into_model(self) -> crate::model::Placement {
        crate::model::Placement {
            x: self.x,
            y: self.y,
            routes: self.routes,
            rot: self.rot,
            flip: self.flip,
        }
    }

    /// Positions are rounded on the way out: the canvas works in
    /// fractional pixels and a file full of `128.00000000000003` is a
    /// file nobody wants to read or diff.
    fn from_model(place: &crate::model::Placement) -> Self {
        Place {
            x: place.x.map(f64::round),
            y: place.y.map(f64::round),
            routes: place.routes.clone(),
            rot: place.rot,
            flip: place.flip,
        }
    }
}

/// What the file described, drawn for the chip the project builds for.
pub struct Loaded {
    pub board: SimBoard,
    /// Set when the file named a different chip — see [`load`].
    pub note: Option<String>,
}

/// The board `.rusty/sim.toml` describes, if the project carries one.
///
/// `chip` is the chip the project builds for, and it is the chip the board is
/// drawn with whatever the file says. The frontend draws the pin rows from
/// `SimBoard::chip`, and a hand-written file — or one written before the
/// project switched parts — can name another: a C3 project drawn with the
/// ESP32 header offered GPIO34–39, pins the part does not have, and a wire
/// dropped on one was a bug nothing reported. A file with no chip used to
/// mean `esp32`, which was the same wrong header by default. So the header
/// follows the build, and a disagreement is said in the note rather than
/// drawn.
pub fn load(root: &Path, chip: &str) -> Option<Loaded> {
    let text = std::fs::read_to_string(root.join(".rusty/sim.toml")).ok()?;
    let parsed: Sheet = toml::from_str(&text).ok()?;
    if parsed.is_empty() {
        return None;
    }

    let project_chip = crate::chip::normalize(chip);
    let note = parsed
        .board
        .chip
        .as_deref()
        .map(crate::chip::normalize)
        .filter(|named| *named != project_chip)
        .map(|named| {
            format!(
                ".rusty/sim.toml says the board is an {named}, but this project builds for \
                 {project_chip}, so the board is drawn with the {project_chip}'s pins. Saving \
                 the board from the editor rewrites the file to match."
            )
        });

    let board = SimBoard {
        chip: project_chip,
        kit_x: parsed.board.x,
        kit_y: parsed.board.y,
        leds: parsed
            .led
            .into_iter()
            .map(|led| SimLed {
                label: led.label.unwrap_or_else(|| format!("GPIO{}", led.pin)),
                color: led.color.unwrap_or_else(|| "green".to_string()),
                pin: led.pin,
                place: led.place.into_model(),
            })
            .collect(),
        buttons: parsed
            .button
            .into_iter()
            .map(|b| SimButton {
                label: b.label.unwrap_or_else(|| format!("BTN{}", b.pin)),
                pin: b.pin,
                place: b.place.into_model(),
            })
            .collect(),
        rgbs: parsed
            .rgb
            .into_iter()
            .map(|rgb| SimRgb {
                label: rgb.label.unwrap_or_else(|| "RGB".to_string()),
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
                place: rgb.place.into_model(),
            })
            .collect(),
        sevens: parsed
            .seven
            .into_iter()
            .map(|seven| SimSeven {
                label: seven.label.unwrap_or_else(|| "7SEG".to_string()),
                pins: seven.pins,
                place: seven.place.into_model(),
            })
            .collect(),
        displays: parsed
            .display
            .into_iter()
            .map(|display| SimDisplay {
                label: display.label.unwrap_or_else(|| "DISPLAY".to_string()),
                sda: display.sda.unwrap_or(UNWIRED_PIN),
                scl: display.scl.unwrap_or(UNWIRED_PIN),
                place: display.place.into_model(),
            })
            .collect(),
        pots: parsed
            .pot
            .into_iter()
            .map(|pot| SimPot {
                label: pot.label.unwrap_or_else(|| format!("POT{}", pot.pin)),
                pin: pot.pin,
                place: pot.place.into_model(),
            })
            .collect(),
        analogs: parsed
            .analog
            .into_iter()
            .map(|a| SimAnalog {
                label: a.label.unwrap_or_else(|| format!("A{}", a.pin)),
                pin: a.pin,
                max: a.max.unwrap_or(4095),
                start: a.start.unwrap_or(0),
                note: a.note,
                place: a.place.into_model(),
            })
            .collect(),
        motors: parsed
            .motor
            .into_iter()
            .map(|motor| SimMotor {
                label: motor.label.unwrap_or_else(|| "MOTOR".to_string()),
                pwm: motor.pwm.unwrap_or(UNWIRED_PIN),
                in1: motor.in1.unwrap_or(UNWIRED_PIN),
                in2: motor.in2.unwrap_or(UNWIRED_PIN),
                place: motor.place.into_model(),
            })
            .collect(),
    };
    Some(Loaded { board, note })
}

/// Write the board back to `.rusty/sim.toml`, the file the editor edits.
///
/// Serialised through this module's structs, not the wire ones — the file
/// format is a contract with people who write it by hand, and it stays stable
/// when the wire model grows.
pub fn save(root: &Path, board: &SimBoard) -> Result<()> {
    let unwired = |pin: u8| (pin != UNWIRED_PIN).then_some(pin);

    let sheet = Sheet {
        board: Board {
            chip: Some(board.chip.clone()),
            x: board.kit_x.map(f64::round),
            y: board.kit_y.map(f64::round),
        },
        led: board
            .leds
            .iter()
            .map(|led| Led {
                pin: led.pin,
                color: Some(led.color.clone()),
                label: Some(led.label.clone()),
                place: Place::from_model(&led.place),
            })
            .collect(),
        button: board
            .buttons
            .iter()
            .map(|b| Button {
                pin: b.pin,
                label: Some(b.label.clone()),
                place: Place::from_model(&b.place),
            })
            .collect(),
        rgb: board
            .rgbs
            .iter()
            .map(|rgb| Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
                label: Some(rgb.label.clone()),
                place: Place::from_model(&rgb.place),
            })
            .collect(),
        seven: board
            .sevens
            .iter()
            .map(|seven| Seven {
                pins: seven.pins,
                label: Some(seven.label.clone()),
                place: Place::from_model(&seven.place),
            })
            .collect(),
        display: board
            .displays
            .iter()
            .map(|display| Display {
                label: Some(display.label.clone()),
                sda: unwired(display.sda),
                scl: unwired(display.scl),
                place: Place::from_model(&display.place),
            })
            .collect(),
        pot: board
            .pots
            .iter()
            .map(|pot| Pot {
                pin: pot.pin,
                label: Some(pot.label.clone()),
                place: Place::from_model(&pot.place),
            })
            .collect(),
        analog: board
            .analogs
            .iter()
            .map(|a| Analog {
                pin: a.pin,
                label: Some(a.label.clone()),
                max: Some(a.max),
                start: Some(a.start),
                note: a.note.clone(),
                place: Place::from_model(&a.place),
            })
            .collect(),
        motor: board
            .motors
            .iter()
            .map(|motor| Motor {
                pwm: unwired(motor.pwm),
                in1: unwired(motor.in1),
                in2: unwired(motor.in2),
                label: Some(motor.label.clone()),
                place: Place::from_model(&motor.place),
            })
            .collect(),
    };

    let dir = root.join(".rusty");
    let path = dir.join("sim.toml");
    let text = toml::to_string_pretty(&sheet).map_err(|error| Error::Encode {
        path: path.display().to_string(),
        detail: error.to_string(),
    })?;
    std::fs::create_dir_all(&dir).map_err(|source| Error::Write {
        path: dir.display().to_string(),
        source,
    })?;
    std::fs::write(&path, text).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Placement;

    /// A round trip proves nothing about a field left at its default.
    ///
    /// This test existed while `flip` was being dropped on save, and passed:
    /// the fixture set `flip: false` on every part, so a writer that never
    /// wrote it and a reader that hard-coded `false` agreed perfectly. The
    /// rule the fixture now follows is that **every optional field differs
    /// from its default** — that is what makes the comparison mean something.
    #[test]
    fn the_board_round_trips_through_save_and_load() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-rt")
            .tempdir()
            .expect("tempdir");
        let place = |x: f64, y: f64, rot: u16, flip: bool| Placement {
            x: Some(x),
            y: Some(y),
            routes: vec![vec![(120.0, 60.0), (200.0, 90.0)]],
            rot,
            flip,
        };
        let board = SimBoard {
            chip: "esp32".to_string(),
            kit_x: Some(420.0),
            kit_y: Some(30.0),
            leds: vec![SimLed {
                pin: 26,
                color: "green".to_string(),
                label: "G".to_string(),
                place: place(40.0, 60.0, 90, true),
            }],
            buttons: vec![SimButton {
                pin: 14,
                label: "BTN14".to_string(),
                place: place(30.0, 120.0, 180, true),
            }],
            rgbs: vec![SimRgb {
                r: 21,
                g: 22,
                b: 23,
                label: "RGB".to_string(),
                place: place(80.0, 160.0, 270, true),
            }],
            sevens: vec![SimSeven {
                pins: [1, 2, 3, 4, 5, 6, 7],
                label: "7SEG".to_string(),
                place: place(200.0, 40.0, 270, true),
            }],
            displays: vec![
                SimDisplay {
                    sda: 21,
                    scl: 22,
                    label: "DISPLAY".to_string(),
                    place: place(300.0, 200.0, 90, true),
                },
                // The sentinel has to survive too: a screen nobody has wired
                // writes no pins at all, and must read back unwired rather
                // than as GPIO0.
                SimDisplay {
                    sda: UNWIRED_PIN,
                    scl: UNWIRED_PIN,
                    label: "LOOSE".to_string(),
                    place: Placement::default(),
                },
            ],
            pots: vec![SimPot {
                pin: 34,
                label: "POT34".to_string(),
                place: place(20.0, 200.0, 90, true),
            }],
            analogs: vec![SimAnalog {
                pin: 35,
                label: "BATT".to_string(),
                max: 1023,
                start: 800,
                note: Some("1023 = 4.2 V through 100k/27k".to_string()),
                place: place(500.0, 320.0, 180, true),
            }],
            motors: vec![
                // An H-bridge drive: all three wired.
                SimMotor {
                    pwm: 5,
                    in1: 6,
                    in2: 7,
                    label: "DRIVE".to_string(),
                    place: place(400.0, 260.0, 270, true),
                },
                // And a fan, which is the same part with the direction pins
                // left off. Both spellings have to survive the file, or the
                // one nobody wrote a fixture for is the one that breaks.
                SimMotor {
                    pwm: 8,
                    in1: UNWIRED_PIN,
                    in2: UNWIRED_PIN,
                    label: "FAN".to_string(),
                    place: Placement::default(),
                },
            ],
        };
        save(dir.path(), &board).expect("save");
        let loaded = load(dir.path(), "esp32").expect("load");
        assert_eq!(loaded.board, board);
        assert!(loaded.note.is_none(), "the file and the project agree");

        // Named explicitly as well as compared: `assert_eq` on the whole
        // board says "something differs", and the field that differs is the
        // one worth naming.
        let loaded = loaded.board;
        assert!(loaded.leds[0].place.flip, "a mirrored part stays mirrored");
        assert_eq!(
            loaded.sevens[0].place.rot, 270,
            "and a turned one stays turned"
        );
        assert_eq!(loaded.displays[1].sda, UNWIRED_PIN);
        assert_eq!(
            loaded.analogs[0].max, 1023,
            "a source that is not a 12-bit ADC keeps saying so",
        );
        assert_eq!(
            loaded.analogs[0].note.as_deref(),
            Some("1023 = 4.2 V through 100k/27k"),
        );
        assert_eq!(
            loaded.motors[0].in1, 6,
            "an H-bridge keeps its direction pins"
        );
        assert_eq!(
            loaded.motors[1].in1, UNWIRED_PIN,
            "and a fan keeps not having any",
        );
    }

    #[test]
    fn the_board_file_converts_to_the_wire_model() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-board")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".rusty")).expect("dirs");
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"esp32\"\n[[led]]\npin = 26\ncolor = \"green\"\n[[led]]\npin = 27\ncolor = \"blue\"\nlabel = \"BLUE\"\n",
        )
        .expect("write");
        let board = load(dir.path(), "esp32").expect("board").board;
        assert_eq!(board.chip, "esp32");
        assert_eq!(board.leds.len(), 2);
        assert_eq!(board.leds[0].label, "GPIO26");
        assert_eq!(board.leds[1].label, "BLUE");
        assert!(load(Path::new("nowhere-at-all"), "esp32").is_none());
    }

    /// The pin rows follow the chip being simulated. A file that names
    /// another part is not drawn as that part — that was the C3 project
    /// showing GPIO34–39 — and a file that names none does not mean ESP32.
    #[test]
    fn the_board_is_drawn_for_the_projects_chip_whatever_the_file_says() {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim-chip")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".rusty")).expect("dirs");

        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"ESP32\"\n[[led]]\npin = 26\n",
        )
        .expect("write");
        let loaded = load(dir.path(), "esp32c3").expect("board");
        assert_eq!(loaded.board.chip, "esp32c3");
        let note = loaded.note.expect("the disagreement is said");
        assert!(
            note.contains("esp32") && note.contains("esp32c3"),
            "both parts are named: {note}"
        );

        std::fs::write(dir.path().join(".rusty/sim.toml"), "[[led]]\npin = 8\n").expect("write");
        let loaded = load(dir.path(), "esp32c3").expect("board");
        assert_eq!(
            loaded.board.chip, "esp32c3",
            "no chip in the file is the project's, not esp32"
        );
        assert!(loaded.note.is_none(), "and nothing to disagree with");

        // Spelling differences are not a disagreement.
        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "[board]\nchip = \"ESP32-C3\"\n[[led]]\npin = 8\n",
        )
        .expect("write");
        assert!(load(dir.path(), "esp32c3").expect("board").note.is_none());
    }
}
