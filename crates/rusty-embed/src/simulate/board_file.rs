//! `.rusty/sim.toml`, as it is written on disk.
//!
//! Its own types, converted to and from the wire model — the file/wire split
//! every user-authored TOML here gets, so a panel refactor cannot silently
//! break the files people wrote (rule 2).
//!
//! Two formats are read and one is written. **Version 2** is the schematic:
//! `[[part]]` entries placing a symbol by `library:name`, and `[[wire]]`
//! entries joining two pins (`U1.GPIO2` to `R1.1`). **Version 1** — no
//! `version` key — was the first board: `[[led]]`, `[[button]]` and friends,
//! each *being* the GPIO it sat on. A version-1 file is read by the reader
//! that always read it and migrated into a sheet: a lamp on GPIO 2 becomes a
//! `Device:LED` wired to GPIO2 and GND, with no resistor, because that is
//! exactly what the old board claimed; the rules then say what is wrong with
//! it. Saving writes version 2, so the migration is realised the first time
//! the editor saves and never silently before.
//!
//! **One definition per part, used in both directions.** Reading and writing
//! were two parallel sets of structs once, and parallel sets drift: `flip`
//! was added to the wire model and to *neither* of them, so mirroring a part
//! was dropped on save and read back as `false`. Sharing the definition
//! makes that omission a compile error instead of a silent loss.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{Instance, KIT_REFERENCE, PinRef, Sheet, Wire};

/// The `[board]` table, common to both versions.
#[derive(Debug, Default, Deserialize, Serialize)]
struct Board {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
    /// The devkit's turn, spelled exactly as a part's is. A first-format
    /// file has neither and reads as upright, which is what it was.
    #[serde(default, skip_serializing_if = "crate::model::is_upright")]
    rot: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    mirror: bool,
}

// ---------------------------------------------------------------- version 2

#[derive(Debug, Default, Deserialize, Serialize)]
struct Part {
    #[serde(rename = "ref")]
    reference: String,
    symbol: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    value: String,
    x: f64,
    y: f64,
    #[serde(default, skip_serializing_if = "crate::model::is_upright")]
    rot: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    mirror: bool,
    /// Behaviour-specific settings — an analog source's full scale — as
    /// text, so a part added tomorrow carries its knobs without a format
    /// change.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    props: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct WireRecord {
    from: String,
    to: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    bends: Vec<(f64, f64)>,
}

/// Values before tables: TOML puts `version` before `[board]`, and every
/// array-of-tables after that. Reordering these fields reorders the file.
#[derive(Debug, Default, Deserialize, Serialize)]
struct FileV2 {
    version: u32,
    #[serde(default)]
    board: Board,
    #[serde(default, rename = "part", skip_serializing_if = "Vec::is_empty")]
    parts: Vec<Part>,
    #[serde(default, rename = "wire", skip_serializing_if = "Vec::is_empty")]
    wires: Vec<WireRecord>,
    /// `no_connect = ["U2.7", "U2.8"]` — the pins the author has said reach
    /// nothing on purpose. A list of spellings rather than a table, because
    /// that is what a person writing this file by hand would write.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    no_connect: Vec<String>,
}

// ---------------------------------------------------------------- version 1

/// A display pin nobody had wired in the first format.
const UNWIRED_PIN: u8 = 255;

/// Where a part sat, in the first format's spelling: `x`, `y`, `rot`,
/// `flip` directly inside `[[led]]`. `routes` were the wires' bends toward
/// the chip; the migration drops them, since a wire pin to pin routes
/// itself.
#[derive(Debug, Default, Deserialize)]
struct Place {
    #[serde(default)]
    x: Option<f64>,
    #[serde(default)]
    y: Option<f64>,
    #[serde(default)]
    #[allow(dead_code)]
    routes: Vec<Vec<(f64, f64)>>,
    #[serde(default)]
    rot: u16,
    #[serde(default)]
    flip: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Led {
    pin: u8,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    active_low: bool,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Button {
    pin: u8,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    active_low: bool,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    active_low: bool,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Seven {
    pins: [u8; 7],
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    active_low: bool,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Display {
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    sda: Option<u8>,
    #[serde(default)]
    scl: Option<u8>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Analog {
    pin: u8,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(default)]
    max: Option<u16>,
    #[serde(default)]
    start: Option<u16>,
    #[serde(default)]
    note: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Motor {
    #[serde(default)]
    pwm: Option<u8>,
    #[serde(default)]
    in1: Option<u8>,
    #[serde(default)]
    in2: Option<u8>,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct Pot {
    pin: u8,
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    #[serde(flatten)]
    place: Place,
}

#[derive(Debug, Default, Deserialize)]
struct FileV1 {
    #[serde(default)]
    board: Board,
    #[serde(default)]
    led: Vec<Led>,
    #[serde(default)]
    button: Vec<Button>,
    #[serde(default)]
    rgb: Vec<Rgb>,
    #[serde(default)]
    seven: Vec<Seven>,
    #[serde(default)]
    display: Vec<Display>,
    #[serde(default)]
    pot: Vec<Pot>,
    #[serde(default)]
    motor: Vec<Motor>,
    #[serde(default)]
    analog: Vec<Analog>,
}

impl FileV1 {
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

/// What the file described, drawn for the chip the project builds for.
pub struct Loaded {
    pub sheet: Sheet,
    /// Set when the file named a different chip — see [`load`].
    pub note: Option<String>,
}

/// The sheet `.rusty/sim.toml` describes, if the project carries one.
///
/// `chip` is the chip the project builds for, and it is the chip the board is
/// drawn with whatever the file says. A hand-written file — or one written
/// before the project switched parts — can name another: a C3 project drawn
/// with the ESP32 header offered GPIO34–39, pins the part does not have, and
/// a wire dropped on one was a bug nothing reported. So the header follows
/// the build, and a disagreement is said in the note rather than drawn.
///
/// A file that does not parse is `None` here and reported by the plan: the
/// editor then starts empty, and saving would overwrite — which is why the
/// plan's note names the file first.
pub fn load(root: &Path, chip: &str) -> Option<Loaded> {
    let text = std::fs::read_to_string(root.join(".rusty/sim.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    let version = table
        .get("version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);
    let project_chip = crate::chip::normalize(chip);
    let (board, mut sheet) = if version >= 2 {
        let parsed: FileV2 = table.try_into().ok()?;
        let sheet = read_v2(&parsed, &project_chip);
        (parsed.board, sheet)
    } else {
        let parsed: FileV1 = table.try_into().ok()?;
        if parsed.is_empty() {
            return None;
        }
        let sheet = migrate(&parsed, &project_chip);
        (parsed.board, sheet)
    };
    sheet.kit_x = board.x;
    sheet.kit_y = board.y;
    sheet.kit_rot = board.rot % 360;
    sheet.kit_mirror = board.mirror;

    let note = board
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
    Some(Loaded { sheet, note })
}

fn read_v2(file: &FileV2, chip: &str) -> Sheet {
    let mut sheet = Sheet::empty(chip);
    for part in &file.parts {
        if part.reference == KIT_REFERENCE || sheet.part(&part.reference).is_some() {
            sheet.notes.push(format!(
                "`{}` is placed twice in .rusty/sim.toml; the second one was skipped",
                part.reference
            ));
            continue;
        }
        sheet.parts.push(Instance {
            reference: part.reference.clone(),
            symbol: part.symbol.clone(),
            value: part.value.clone(),
            x: part.x,
            y: part.y,
            rot: part.rot % 360,
            mirror: part.mirror,
            props: part.props.clone(),
        });
    }
    for wire in &file.wires {
        match (PinRef::parse(&wire.from), PinRef::parse(&wire.to)) {
            (Some(from), Some(to)) => sheet.wires.push(Wire {
                from,
                to,
                bends: wire.bends.clone(),
            }),
            _ => sheet.notes.push(format!(
                "a wire from `{}` to `{}` in .rusty/sim.toml does not name two pins as `part.pin`, and was skipped",
                wire.from, wire.to
            )),
        }
    }
    for spelling in &file.no_connect {
        match PinRef::parse(spelling) {
            Some(pin) => sheet.no_connect.push(pin),
            // Skipped and said, not silently dropped: a no-connect that did
            // not land turns a deliberate answer back into a finding, and
            // the user would see the finding and not the reason.
            None => sheet.notes.push(format!(
                "`{spelling}` in .rusty/sim.toml's no_connect does not name a pin as `part.pin`, and was skipped"
            )),
        }
    }
    sheet
}

/// The first format as a schematic: what each old part claimed, drawn with
/// the wires it implied and none it did not. The user's bends are dropped —
/// they ran to the chip's header, and a wire pin to pin routes itself.
fn migrate(file: &FileV1, chip: &str) -> Sheet {
    let mut sheet = Sheet::empty(chip);
    let gpio = |n: u8| format!("{KIT_REFERENCE}.GPIO{n}");
    let rail = |high: bool| format!("{KIT_REFERENCE}.{}", if high { "3V3" } else { "GND" });
    // The old position was a body's top-left corner; the symbol's anchor
    // sits near where its body was, on the grid.
    let anchor = |place: &Place, dx: f64, dy: f64| {
        let x = place.x.unwrap_or(60.0) + dx;
        let y = place.y.unwrap_or(60.0) + dy;
        ((x / 8.0).round() * 8.0, (y / 8.0).round() * 8.0)
    };

    fn add(
        sheet: &mut Sheet,
        prefix: &str,
        symbol: &str,
        value: &str,
        at: (f64, f64),
        place: &Place,
    ) -> String {
        let reference = sheet.next_reference(prefix);
        sheet.parts.push(Instance {
            reference: reference.clone(),
            symbol: symbol.to_string(),
            value: value.to_string(),
            x: at.0,
            y: at.1,
            rot: place.rot % 360,
            mirror: place.flip,
            props: BTreeMap::new(),
        });
        reference
    }
    fn join(sheet: &mut Sheet, from: &str, to: &str) {
        if let (Some(from), Some(to)) = (PinRef::parse(from), PinRef::parse(to)) {
            sheet.wires.push(Wire {
                from,
                to,
                bends: Vec::new(),
            });
        }
    }
    let wired = |pin: u8| pin != UNWIRED_PIN;

    for (index, led) in file.led.iter().enumerate() {
        let mut place = Place {
            x: led.place.x,
            y: led.place.y,
            ..Default::default()
        };
        if place.x.is_none() {
            place.y = Some(40.0 + index as f64 * 56.0);
        }
        let reference = add(
            &mut sheet,
            "D",
            "Device:LED",
            led.color.as_deref().unwrap_or("green"),
            anchor(&place, 48.0, 16.0),
            &led.place,
        );
        // Active-high: the GPIO sources the anode. Active-low: the anode
        // sits on 3V3 and the GPIO sinks the cathode — what most devkits'
        // onboard lamps do.
        if led.active_low {
            join(&mut sheet, &rail(true), &format!("{reference}.A"));
            join(&mut sheet, &format!("{reference}.K"), &gpio(led.pin));
        } else {
            join(&mut sheet, &gpio(led.pin), &format!("{reference}.A"));
            join(&mut sheet, &format!("{reference}.K"), &rail(false));
        }
    }
    for button in &file.button {
        let reference = add(
            &mut sheet,
            "SW",
            "Device:SW_Push",
            "",
            anchor(&button.place, 48.0, 16.0),
            &button.place,
        );
        join(&mut sheet, &gpio(button.pin), &format!("{reference}.1"));
        join(
            &mut sheet,
            &format!("{reference}.2"),
            &rail(!button.active_low),
        );
    }
    for rgb in &file.rgb {
        let reference = add(
            &mut sheet,
            "D",
            "rusty:RGB_LED",
            "",
            anchor(&rgb.place, 56.0, 32.0),
            &rgb.place,
        );
        for (pin, channel) in [(rgb.r, "R"), (rgb.g, "G"), (rgb.b, "B")] {
            if wired(pin) {
                join(&mut sheet, &gpio(pin), &format!("{reference}.{channel}"));
            }
        }
        // Common anode lights a channel pulled low, which is what the old
        // `active_low` meant.
        join(
            &mut sheet,
            &format!("{reference}.COM"),
            &rail(rgb.active_low),
        );
    }
    for seven in &file.seven {
        let reference = add(
            &mut sheet,
            "DS",
            "rusty:7SEG",
            "",
            anchor(&seven.place, 40.0, 64.0),
            &seven.place,
        );
        for (pin, segment) in seven.pins.iter().zip(["a", "b", "c", "d", "e", "f", "g"]) {
            if wired(*pin) {
                join(&mut sheet, &gpio(*pin), &format!("{reference}.{segment}"));
            }
        }
        join(
            &mut sheet,
            &format!("{reference}.COM"),
            &rail(seven.active_low),
        );
    }
    for display in &file.display {
        let reference = add(
            &mut sheet,
            "DS",
            "rusty:Display",
            "",
            anchor(&display.place, 72.0, 24.0),
            &display.place,
        );
        if let Some(sda) = display.sda.filter(|p| wired(*p)) {
            join(&mut sheet, &gpio(sda), &format!("{reference}.SDA"));
        }
        if let Some(scl) = display.scl.filter(|p| wired(*p)) {
            join(&mut sheet, &gpio(scl), &format!("{reference}.SCL"));
        }
        join(&mut sheet, &format!("{reference}.VCC"), &rail(true));
        join(&mut sheet, &format!("{reference}.GND"), &rail(false));
    }
    for pot in &file.pot {
        let reference = add(
            &mut sheet,
            "RV",
            "rusty:Pot",
            "",
            anchor(&pot.place, 64.0, 16.0),
            &pot.place,
        );
        join(&mut sheet, &format!("{reference}.W"), &gpio(pot.pin));
        join(&mut sheet, &format!("{reference}.1"), &rail(true));
        join(&mut sheet, &format!("{reference}.3"), &rail(false));
    }
    for analog in &file.analog {
        let reference = add(
            &mut sheet,
            "V",
            "rusty:Analog",
            analog.note.as_deref().unwrap_or(""),
            anchor(&analog.place, 64.0, 16.0),
            &analog.place,
        );
        let props = &mut sheet.parts.last_mut().expect("just pushed").props;
        if let Some(max) = analog.max {
            props.insert("max".to_string(), max.to_string());
        }
        if let Some(start) = analog.start {
            props.insert("start".to_string(), start.to_string());
        }
        join(&mut sheet, &format!("{reference}.OUT"), &gpio(analog.pin));
        join(&mut sheet, &format!("{reference}.GND"), &rail(false));
    }
    for motor in &file.motor {
        let reference = add(
            &mut sheet,
            "M",
            "rusty:Motor",
            "",
            anchor(&motor.place, 64.0, 24.0),
            &motor.place,
        );
        for (pin, name) in [(motor.pwm, "PWM"), (motor.in1, "IN1"), (motor.in2, "IN2")] {
            if let Some(pin) = pin.filter(|p| wired(*p)) {
                join(&mut sheet, &gpio(pin), &format!("{reference}.{name}"));
            }
        }
    }
    sheet.notes.push(
        ".rusty/sim.toml is in the first board format and was read as a schematic: each lamp \
         is a Device:LED wired to its GPIO and a rail, each button a Device:SW_Push, and the \
         rest the rusty library's parts. No resistors were added, because the old board had \
         none. Saving the board from the editor rewrites the file in the new format."
            .to_string(),
    );
    sheet
}

/// Write the sheet back to `.rusty/sim.toml`, in version 2.
///
/// Serialised through this module's structs, not the wire ones — the file
/// format is a contract with people who write it by hand, and it stays
/// stable when the wire model grows. Positions are rounded on the way out:
/// the canvas works in fractional pixels and a file full of
/// `128.00000000000003` is a file nobody wants to read or diff.
pub fn save(root: &Path, sheet: &Sheet) -> Result<()> {
    let file = FileV2 {
        version: 2,
        board: Board {
            chip: Some(sheet.chip.clone()),
            x: sheet.kit_x.map(f64::round),
            y: sheet.kit_y.map(f64::round),
            rot: sheet.kit_rot,
            mirror: sheet.kit_mirror,
        },
        parts: sheet
            .parts
            .iter()
            .map(|p| Part {
                reference: p.reference.clone(),
                symbol: p.symbol.clone(),
                value: p.value.clone(),
                x: p.x.round(),
                y: p.y.round(),
                rot: p.rot,
                mirror: p.mirror,
                props: p.props.clone(),
            })
            .collect(),
        wires: sheet
            .wires
            .iter()
            .map(|w| WireRecord {
                from: w.from.to_string(),
                to: w.to.to_string(),
                bends: w
                    .bends
                    .iter()
                    .map(|(x, y)| (x.round(), y.round()))
                    .collect(),
            })
            .collect(),
        no_connect: sheet.no_connect.iter().map(PinRef::to_string).collect(),
    };
    let path = root.join(".rusty/sim.toml");
    let dir = root.join(".rusty");
    let text = toml::to_string(&file).map_err(|error| Error::Encode {
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

    fn project(text: &str) -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("rusty-sim")
            .tempdir()
            .expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".rusty")).expect("dirs");
        std::fs::write(dir.path().join(".rusty/sim.toml"), text).expect("write");
        dir
    }

    /// A round trip proves nothing about a field left at its default, so
    /// every optional field here differs from its default: a turned,
    /// mirrored part with a value and a prop, a wire with bends.
    #[test]
    fn a_sheet_round_trips_through_save_and_load() {
        let dir = project("");
        let mut sheet = Sheet::empty("esp32c3");
        sheet.kit_x = Some(420.0);
        sheet.kit_y = Some(30.0);
        sheet.kit_rot = 90;
        sheet.kit_mirror = true;
        sheet.parts.push(Instance {
            reference: "D1".into(),
            symbol: "Device:LED".into(),
            value: "red".into(),
            x: 96.0,
            y: 64.0,
            rot: 90,
            mirror: true,
            props: BTreeMap::new(),
        });
        sheet.parts.push(Instance {
            reference: "V1".into(),
            symbol: "rusty:Analog".into(),
            value: "1023 = 4.2 V through 100k/27k".into(),
            x: 200.0,
            y: 120.0,
            rot: 270,
            mirror: false,
            props: [("max".to_string(), "1023".to_string())]
                .into_iter()
                .collect(),
        });
        sheet.wires.push(Wire {
            from: PinRef::new("U1", "GPIO2"),
            to: PinRef::new("D1", "A"),
            bends: vec![(120.0, 64.0), (120.0, 40.0)],
        });
        sheet.wires.push(Wire {
            from: PinRef::new("D1", "K"),
            to: PinRef::new("U1", "9"),
            bends: Vec::new(),
        });
        sheet.no_connect.push(PinRef::new("V1", "OUT"));
        save(dir.path(), &sheet).expect("save");
        let text = std::fs::read_to_string(dir.path().join(".rusty/sim.toml")).unwrap();
        assert!(text.starts_with("version = 2\n"), "{text}");
        assert!(text.contains("[[part]]\nref = \"D1\""), "{text}");
        assert!(text.contains("from = \"U1.GPIO2\""), "{text}");

        let loaded = load(dir.path(), "esp32c3").expect("load");
        assert!(loaded.note.is_none());
        assert_eq!(loaded.sheet, sheet);
        assert!(
            loaded.sheet.parts[0].mirror,
            "a mirrored part stays mirrored"
        );
        assert_eq!(
            (loaded.sheet.kit_rot, loaded.sheet.kit_mirror),
            (90, true),
            "the devkit's own turn survives the file"
        );
        assert_eq!(loaded.sheet.parts[1].props["max"], "1023");
        assert_eq!(loaded.sheet.wires[0].bends.len(), 2);
        assert_eq!(
            loaded.sheet.no_connect,
            vec![PinRef::new("V1", "OUT")],
            "a pin said to reach nothing on purpose still says so"
        );
    }

    #[test]
    fn a_first_format_file_is_read_as_the_circuit_it_claimed() {
        let dir = project(
            "[board]\nchip = \"esp32\"\nx = 400\ny = 20\n\
             [[led]]\npin = 26\ncolor = \"green\"\nx = 40\ny = 60\nrot = 90\nflip = true\n\
             [[led]]\npin = 27\ncolor = \"blue\"\nactive_low = true\n\
             [[button]]\npin = 14\nactive_low = true\n\
             [[rgb]]\nr = 21\ng = 22\nb = 23\nactive_low = true\n\
             [[seven]]\npins = [1, 2, 3, 4, 5, 6, 255]\n\
             [[display]]\nsda = 21\nscl = 22\n\
             [[pot]]\npin = 34\n\
             [[analog]]\npin = 35\nmax = 1023\nstart = 800\nnote = \"1023 = 4.2 V\"\n\
             [[motor]]\npwm = 5\nin1 = 6\nin2 = 7\n\
             [[motor]]\npwm = 8\n",
        );
        let loaded = load(dir.path(), "esp32").expect("load");
        let sheet = loaded.sheet;
        assert_eq!((sheet.kit_x, sheet.kit_y), (Some(400.0), Some(20.0)));
        let references: Vec<&str> = sheet.parts.iter().map(|p| p.reference.as_str()).collect();
        assert_eq!(
            references,
            vec![
                "D1", "D2", "SW1", "D3", "DS1", "DS2", "RV1", "V1", "M1", "M2"
            ]
        );
        let d1 = sheet.part("D1").unwrap();
        assert_eq!(
            (d1.symbol.as_str(), d1.value.as_str()),
            ("Device:LED", "green")
        );
        assert_eq!((d1.rot, d1.mirror), (90, true));
        assert_eq!((d1.x, d1.y), (88.0, 80.0), "near the old body, on the grid");

        let wire = |from: &str, to: &str| {
            sheet.wires.iter().any(|w| {
                w.from == PinRef::parse(from).unwrap() && w.to == PinRef::parse(to).unwrap()
            })
        };
        // Active-high: GPIO to anode, cathode to ground. Active-low: 3V3 to
        // anode, cathode to the GPIO.
        assert!(wire("U1.GPIO26", "D1.A") && wire("D1.K", "U1.GND"));
        assert!(wire("U1.3V3", "D2.A") && wire("D2.K", "U1.GPIO27"));
        assert!(
            wire("U1.GPIO14", "SW1.1") && wire("SW1.2", "U1.GND"),
            "a pull-up button goes to ground"
        );
        assert!(
            wire("U1.GPIO21", "D3.R") && wire("D3.COM", "U1.3V3"),
            "common anode on 3V3"
        );
        assert!(
            wire("U1.GPIO1", "DS1.a") && !sheet.wires.iter().any(|w| w.to.pin == "g"),
            "an unwired segment stays unwired"
        );
        assert!(wire("DS1.COM", "U1.GND"));
        assert!(wire("U1.GPIO21", "DS2.SDA") && wire("DS2.VCC", "U1.3V3"));
        assert!(wire("RV1.W", "U1.GPIO34") && wire("RV1.1", "U1.3V3") && wire("RV1.3", "U1.GND"));
        let v1 = sheet.part("V1").unwrap();
        assert_eq!(v1.value, "1023 = 4.2 V");
        assert_eq!(v1.props["max"], "1023");
        assert_eq!(v1.props["start"], "800");
        assert!(wire("V1.OUT", "U1.GPIO35"));
        assert!(wire("U1.GPIO6", "M1.IN1"));
        assert_eq!(
            sheet.wires_of("M2").count(),
            1,
            "a fan wires only its duty pin"
        );
        assert_eq!(sheet.notes.len(), 1);
        assert!(
            sheet.notes[0].contains("first board format"),
            "{}",
            sheet.notes[0]
        );

        // Saved, it is version 2 and reads back as itself.
        save(dir.path(), &sheet).expect("save");
        let again = load(dir.path(), "esp32").expect("load").sheet;
        assert_eq!(again.parts, sheet.parts);
        assert_eq!(again.wires, sheet.wires);
        assert!(again.notes.is_empty(), "no migration the second time");
    }

    /// The pin rows follow the chip being simulated. A file that names
    /// another part is not drawn as that part — that was the C3 project
    /// showing GPIO34–39 — and a file that names none does not mean ESP32.
    #[test]
    fn the_board_is_drawn_for_the_projects_chip_whatever_the_file_says() {
        let dir = project("[board]\nchip = \"ESP32\"\n[[led]]\npin = 26\n");
        let loaded = load(dir.path(), "esp32c3").expect("board");
        assert_eq!(loaded.sheet.chip, "esp32c3");
        let note = loaded.note.expect("the disagreement is said");
        assert!(
            note.contains("esp32") && note.contains("esp32c3"),
            "both parts are named: {note}"
        );

        std::fs::write(dir.path().join(".rusty/sim.toml"), "[[led]]\npin = 8\n").unwrap();
        let loaded = load(dir.path(), "esp32c3").expect("board");
        assert_eq!(
            loaded.sheet.chip, "esp32c3",
            "no chip in the file is the project's"
        );
        assert!(loaded.note.is_none());

        std::fs::write(
            dir.path().join(".rusty/sim.toml"),
            "version = 2\n[board]\nchip = \"ESP32-C3\"\n[[part]]\nref = \"R1\"\nsymbol = \"Device:R\"\nx = 0\ny = 0\n",
        )
        .unwrap();
        assert!(
            load(dir.path(), "esp32c3").expect("board").note.is_none(),
            "spelling is not disagreement"
        );
        assert!(load(Path::new("nowhere-at-all"), "esp32").is_none());
    }

    #[test]
    fn a_bad_wire_or_a_repeated_reference_is_noted_and_skipped() {
        let dir = project(
            "version = 2\n[[part]]\nref = \"R1\"\nsymbol = \"Device:R\"\nx = 0\ny = 0\n\
             [[part]]\nref = \"R1\"\nsymbol = \"Device:C\"\nx = 8\ny = 8\n\
             [[part]]\nref = \"U1\"\nsymbol = \"Device:C\"\nx = 8\ny = 8\n\
             [[wire]]\nfrom = \"R1.1\"\nto = \"GND\"\n\
             [[wire]]\nfrom = \"R1.2\"\nto = \"U1.GND\"\n",
        );
        let sheet = load(dir.path(), "esp32c3").expect("board").sheet;
        assert_eq!(
            sheet.parts.len(),
            1,
            "the repeat and the kit's name are refused"
        );
        assert_eq!(sheet.wires.len(), 1);
        assert_eq!(sheet.notes.len(), 3, "{:?}", sheet.notes);
        assert!(sheet.notes[2].contains("`GND`"), "{}", sheet.notes[2]);
    }

    /// The repository's own examples open clean: no migration note, every
    /// symbol found, and nothing for the rules to point at. An example that
    /// showed a finding on a fresh clone would be teaching the wrong thing.
    #[test]
    fn the_examples_boards_load_without_notes_or_findings() {
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let library = crate::schematic::builtin();
        let rows = crate::nets::kit_rows("esp32c3", &(0..=21).collect::<Vec<u32>>());
        let mut seen = 0;
        for entry in std::fs::read_dir(&examples).expect("examples/") {
            let root = entry.expect("entry").path();
            if !root.join(".rusty/sim.toml").is_file() {
                continue;
            }
            seen += 1;
            let loaded = load(&root, "esp32c3").unwrap_or_else(|| panic!("{}", root.display()));
            let mut sheet = loaded.sheet;
            crate::simulate::resolve_symbols(&mut sheet, &library);
            assert!(
                loaded.note.is_none(),
                "{}: {:?}",
                root.display(),
                loaded.note
            );
            assert!(
                sheet.notes.is_empty(),
                "{}: {:?}",
                root.display(),
                sheet.notes
            );
            let reading = crate::nets::evaluate(crate::nets::Inputs {
                sheet: &sheet,
                rows: &rows,
                gpio: &Default::default(),
                pressed: &Default::default(),
            });
            assert!(
                reading.warnings.is_empty(),
                "{}: {:?}",
                root.display(),
                reading.warnings
            );
            // And what each sheet puts on the two buses, since a device
            // that is declared but not wired is exactly the mistake these
            // examples exist to be a correct answer to.
            let (bus, bus_said) = crate::nets::bus_devices(&sheet, &rows);
            let (wire, wire_said) = crate::nets::wire_devices(&sheet, &rows);
            assert!(
                bus_said.is_empty() && wire_said.is_empty(),
                "{}: {bus_said:?} {wire_said:?}",
                root.display()
            );
            if root.ends_with("sense-board") {
                assert_eq!(
                    bus.iter().map(|d| d.address).collect::<Vec<_>>(),
                    vec![0x68],
                    "the sensor is on the bus at the address the file names"
                );
                assert!(wire.is_empty(), "nothing on this board is on SPI");
            }
        }
        assert!(seen >= 5, "the examples carry boards");
    }
}
