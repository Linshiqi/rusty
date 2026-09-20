//! A Wokwi project's `diagram.json`, read onto the sheet.
//!
//! Wokwi is where many people drew their first ESP32 circuit, and its
//! diagram is plain JSON: parts with a type, a place and attributes, and
//! connections between `part:pin` pairs. The parts rusty has a counterpart
//! for come across with their wiring and their attributes; the ones it does
//! not are named in the notes with the connections they took with them,
//! rather than drawn as something they are not.
//!
//! The board's pins are read by name the way Wokwi spells them — `2`,
//! `GND.3`, `3V3.1`, `TX` — and land on the devkit row for the same GPIO of
//! the chip this project builds for. A pin that chip does not have is said,
//! not moved to one it does.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::model::{Instance, KIT_REFERENCE, PinRef, Sheet, Wire};
use crate::nets::Row;

#[derive(Debug, Deserialize)]
struct Diagram {
    #[serde(default)]
    parts: Vec<Part>,
    #[serde(default)]
    connections: Vec<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    #[serde(default)]
    top: f64,
    #[serde(default)]
    left: f64,
    #[serde(default)]
    rotate: f64,
    #[serde(default)]
    flip: bool,
    #[serde(default)]
    attrs: BTreeMap<String, serde_json::Value>,
}

impl Part {
    fn attr(&self, key: &str) -> Option<String> {
        match self.attrs.get(key)? {
            serde_json::Value::String(text) => Some(text.trim().to_string()),
            serde_json::Value::Number(number) => Some(number.to_string()),
            serde_json::Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        }
    }
}

/// What one Wokwi pin is on rusty's sheet.
#[derive(Debug, Clone, PartialEq)]
enum End {
    /// A pin of a part that came across: `D1.A`.
    Pin(PinRef),
    /// A 5 V supply, which a C3's devkit has no row for.
    FiveVolts,
    /// A pin rusty leaves out, with why.
    Dropped(String),
}

/// How a part's Wokwi pin names become its symbol's.
type PinMap = fn(&str) -> Option<&'static str>;

/// A Wokwi part that came across: its symbol, its value and props, and how
/// its pin names become the symbol's.
struct Counterpart {
    symbol: &'static str,
    prefix: &'static str,
    value: String,
    props: BTreeMap<String, String>,
    pins: PinMap,
}

fn counterpart(part: &Part) -> Result<Counterpart, String> {
    let with = |symbol, prefix, value: String, pins: PinMap| {
        Ok(Counterpart {
            symbol,
            prefix,
            value,
            props: BTreeMap::new(),
            pins,
        })
    };
    match part.kind.as_str() {
        "wokwi-led" => with(
            "Device:LED",
            "D",
            part.attr("color").unwrap_or_else(|| "red".into()),
            |pin| match pin {
                "A" => Some("A"),
                "C" => Some("K"),
                _ => None,
            },
        ),
        "wokwi-resistor" => with(
            "Device:R",
            "R",
            part.attr("value").unwrap_or_else(|| "1000".into()),
            |pin| match pin {
                "1" => Some("1"),
                "2" => Some("2"),
                _ => None,
            },
        ),
        // Four legs, two of them each side of the switch — as on the desk.
        "wokwi-pushbutton" | "wokwi-pushbutton-6mm" => with(
            "Device:SW_Push",
            "SW",
            part.attr("color").unwrap_or_default(),
            |pin| match pin {
                "1.l" | "1.r" => Some("1"),
                "2.l" | "2.r" => Some("2"),
                _ => None,
            },
        ),
        // The knob's zero is pin 1's end, which Wokwi calls GND.
        "wokwi-potentiometer" | "wokwi-slide-potentiometer" => {
            with("rusty:Pot", "RV", String::new(), |pin| match pin {
                "GND" => Some("1"),
                "SIG" => Some("W"),
                "VCC" => Some("3"),
                _ => None,
            })
        }
        "wokwi-rgb-led" => with("rusty:RGB_LED", "D", String::new(), |pin| match pin {
            "R" => Some("R"),
            "G" => Some("G"),
            "B" => Some("B"),
            "COM" => Some("COM"),
            _ => None,
        }),
        "wokwi-7segment" => {
            let digits = part.attr("digits").unwrap_or_else(|| "1".into());
            if digits != "1" {
                return Err(format!(
                    "a {digits}-digit display; rusty's digit is one digit"
                ));
            }
            with("rusty:7SEG", "U", String::new(), |pin| match pin {
                "A" => Some("a"),
                "B" => Some("b"),
                "C" => Some("c"),
                "D" => Some("d"),
                "E" => Some("e"),
                "F" => Some("f"),
                "G" => Some("g"),
                "COM" | "COM.1" | "COM.2" => Some("COM"),
                _ => None,
            })
        }
        "wokwi-buzzer" => with("rusty:Buzzer", "BZ", String::new(), |pin| match pin {
            "1" => Some("-"),
            "2" => Some("+"),
            _ => None,
        }),
        "wokwi-servo" => with("rusty:Servo", "M", String::new(), |pin| match pin {
            "PWM" => Some("SIG"),
            "V+" => Some("VCC"),
            "GND" => Some("GND"),
            _ => None,
        }),
        "board-ssd1306" | "wokwi-ssd1306" => {
            let address = part
                .attr("i2cAddress")
                .map(|a| a.trim_start_matches("0x").to_ascii_lowercase())
                .unwrap_or_else(|| "3c".into());
            let mut counterpart = with("rusty:Display", "DS", String::new(), |pin| match pin {
                "SDA" => Some("SDA"),
                "SCL" => Some("SCL"),
                "VCC" => Some("VCC"),
                "GND" => Some("GND"),
                _ => None,
            })?;
            counterpart.props.insert("addr".into(), address);
            // And which controller it is, so the bytes the firmware writes
            // are read as the picture they are. Wokwi's part is an SSD1306
            // by name, which is the one thing about it rusty need not guess.
            counterpart
                .props
                .insert("panel".into(), crate::screen::Panel::Ssd1306.id().into());
            Ok(counterpart)
        }
        // A sensor rusty answers for register by register, starting where
        // Wokwi's attributes left it.
        "wokwi-mpu6050" => {
            let mut counterpart = with("rusty:Sensor", "U", "MPU-6050".into(), |pin| match pin {
                "SDA" => Some("SDA"),
                "SCL" => Some("SCL"),
                "VCC" => Some("VCC"),
                "GND" => Some("GND"),
                _ => None,
            })?;
            counterpart.props.insert("addr".into(), "68".into());
            counterpart.props.insert("model".into(), "mpu6050".into());
            for (wokwi, ours) in [
                ("accelX", "ax"),
                ("accelY", "ay"),
                ("accelZ", "az"),
                ("rotationX", "gx"),
                ("rotationY", "gy"),
                ("rotationZ", "gz"),
                ("temperature", "temp"),
            ] {
                if let Some(value) = part.attr(wokwi) {
                    counterpart.props.insert(ours.into(), value);
                }
            }
            Ok(counterpart)
        }
        "wokwi-gnd" => with("rusty:GND", "#PWR", String::new(), |pin| match pin {
            "GND" => Some("GND"),
            _ => None,
        }),
        "wokwi-vcc" => with("rusty:Supply", "#PWR", "5V".into(), |pin| match pin {
            "VCC" => Some("VCC"),
            _ => None,
        }),
        other => Err(format!("rusty has no {other}")),
    }
}

/// The GPIOs a board's `TX` and `RX` are, by the chip it carries.
fn console_pins(board: &str) -> (u8, u8) {
    if board.contains("c3") {
        (21, 20)
    } else if board.contains("c6") {
        (16, 17)
    } else if board.contains("h2") {
        (24, 23)
    } else if board.contains("s3") || board.contains("s2") {
        (43, 44)
    } else {
        (1, 3)
    }
}

/// Whether a Wokwi part is the microcontroller board.
fn is_board(kind: &str) -> bool {
    kind.starts_with("board-esp32")
        || kind == "wokwi-esp32-devkit-v1"
        || kind.starts_with("board-xiao-esp32")
}

/// What a board pin is, by its Wokwi name.
fn board_pin(board: &str, pin: &str, rows: &[Row]) -> End {
    // `GND.3`, `3V3.1`, `8.2`: several header pins for one signal.
    let base = match pin.split_once('.') {
        Some((base, index)) if index.chars().all(|c| c.is_ascii_digit()) => base,
        _ => pin,
    };
    let gpio = |n: u8| -> End {
        match rows.iter().position(|row| row.gpio == Some(n)) {
            Some(index) => End::Pin(PinRef {
                part: KIT_REFERENCE.to_string(),
                pin: rows[index].name.clone(),
            }),
            None => End::Dropped(format!("GPIO{n} is not on this chip")),
        }
    };
    let rail = |name: &str| -> End {
        if rows.iter().any(|row| row.name == name) {
            End::Pin(PinRef {
                part: KIT_REFERENCE.to_string(),
                pin: name.to_string(),
            })
        } else {
            End::Dropped(format!("the devkit has no {name} row"))
        }
    };
    let (tx, rx) = console_pins(board);
    let devkit_v1 = board == "wokwi-esp32-devkit-v1";
    match base {
        "GND" => rail("GND"),
        "3V3" | "3.3V" => rail("3V3"),
        "5V" | "VIN" | "VBUS" | "VUSB" => {
            if rows.iter().any(|row| row.name == "VIN") {
                rail("VIN")
            } else {
                End::FiveVolts
            }
        }
        "TX" => gpio(tx),
        "RX" => gpio(rx),
        "TX0" => gpio(1),
        "RX0" => gpio(3),
        "TX2" => gpio(17),
        "RX2" => gpio(16),
        "VP" => gpio(36),
        "VN" => gpio(39),
        number if number.chars().all(|c| c.is_ascii_digit()) => match number.parse::<u8>() {
            Ok(n) => gpio(n),
            Err(_) => End::Dropped(format!("{pin} is not a pin")),
        },
        named if devkit_v1 && named.starts_with('D') && named[1..].parse::<u8>().is_ok() => {
            gpio(named[1..].parse().unwrap_or_default())
        }
        other => End::Dropped(format!("{other} is not a pin rusty's devkit carries")),
    }
}

/// A diagram, read. `rows` are the devkit's for `chip`.
pub(crate) fn read(text: &str, chip: &str, rows: &[Row]) -> Result<Sheet, String> {
    let diagram: Diagram = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let mut sheet = Sheet::empty(chip);
    let mut notes = Vec::new();

    let boards: Vec<&Part> = diagram.parts.iter().filter(|p| is_board(&p.kind)).collect();
    let board = boards.first().copied();
    for extra in boards.iter().skip(1) {
        notes.push(format!(
            "{} ({}) is a second microcontroller; rusty's sheet has one, so it was left out",
            extra.id, extra.kind
        ));
    }
    // Where the board sat, so everything keeps its place around the devkit.
    let (origin_left, origin_top) = board.map_or((0.0, 0.0), |b| (b.left, b.top));
    let (kit_x, kit_y) = (460.0, 40.0);
    if board.is_none() {
        notes.push(
            "the diagram has no ESP32 board, so nothing is wired to the devkit U1 and the \
             sheet can be drawn and checked but not simulated"
                .to_string(),
        );
    }

    // Every part rusty has a counterpart for, with its references.
    let mut placed: HashMap<String, (String, PinMap)> = HashMap::new();
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    let mut left_out: Vec<String> = Vec::new();
    for part in &diagram.parts {
        if is_board(&part.kind) {
            continue;
        }
        // Annotations and instruments with nothing to carry across.
        if matches!(part.kind.as_str(), "wokwi-text" | "wokwi-logo") {
            continue;
        }
        let counterpart = match counterpart(part) {
            Ok(counterpart) => counterpart,
            Err(why) => {
                left_out.push(part.id.clone());
                notes.push(format!("{} ({}) was left out: {why}", part.id, part.kind));
                continue;
            }
        };
        // KiCad's references, never the devkit's own `U1`.
        let count = counts.entry(counterpart.prefix).or_insert(0);
        let reference = loop {
            *count += 1;
            let candidate = format!("{}{}", counterpart.prefix, count);
            if candidate != KIT_REFERENCE {
                break candidate;
            }
        };
        let quarter = (((part.rotate / 90.0).round() as i64).rem_euclid(4) * 90) as u16;
        sheet.parts.push(Instance {
            reference: reference.clone(),
            symbol: counterpart.symbol.to_string(),
            value: counterpart.value,
            x: kit_x + (part.left - origin_left) + 20.0,
            y: kit_y + (part.top - origin_top) + 20.0,
            rot: quarter,
            mirror: part.flip,
            props: counterpart.props,
        });
        placed.insert(part.id.clone(), (reference, counterpart.pins));
    }

    let mut five_volts: Option<String> = None;
    let mut dropped = 0usize;
    for connection in &diagram.connections {
        let (Some(from), Some(to)) = (
            connection.first().and_then(|v| v.as_str()),
            connection.get(1).and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        // The serial monitor is rusty's console, which is always attached.
        if from.starts_with("$serialMonitor") || to.starts_with("$serialMonitor") {
            continue;
        }
        let end = |text: &str| -> End {
            let Some((id, pin)) = text.split_once(':') else {
                return End::Dropped(format!("{text} names no pin"));
            };
            if let Some(board) = board
                && board.id == id
            {
                return board_pin(&board.kind, pin, rows);
            }
            match placed.get(id) {
                Some((reference, pins)) => match pins(pin) {
                    Some(ours) => End::Pin(PinRef {
                        part: reference.clone(),
                        pin: ours.to_string(),
                    }),
                    None => End::Dropped(format!("{id}.{pin} has no counterpart")),
                },
                None if left_out.iter().any(|out| out == id) => {
                    End::Dropped(format!("{id} was left out"))
                }
                None => End::Dropped(format!("{id} is not in the diagram")),
            }
        };
        let mut resolve = |end: End, sheet: &mut Sheet| -> Option<PinRef> {
            match end {
                End::Pin(pin) => Some(pin),
                End::FiveVolts => {
                    let reference = five_volts.get_or_insert_with(|| {
                        sheet.parts.push(Instance {
                            reference: "#PWR5V".to_string(),
                            symbol: "rusty:Supply".to_string(),
                            value: "5V".to_string(),
                            x: kit_x - 60.0,
                            y: kit_y - 20.0,
                            rot: 0,
                            mirror: false,
                            props: BTreeMap::new(),
                        });
                        "#PWR5V".to_string()
                    });
                    Some(PinRef {
                        part: reference.clone(),
                        pin: "VCC".to_string(),
                    })
                }
                End::Dropped(_) => None,
            }
        };
        let (a, b) = (end(from), end(to));
        let reasons: Vec<String> = [&a, &b]
            .into_iter()
            .filter_map(|end| match end {
                End::Dropped(why) => Some(why.clone()),
                _ => None,
            })
            .collect();
        match (resolve(a, &mut sheet), resolve(b, &mut sheet)) {
            (Some(from), Some(to)) if from != to => sheet.wires.push(Wire {
                from,
                to,
                bends: Vec::new(),
            }),
            (Some(_), Some(_)) => {}
            _ => {
                dropped += 1;
                // The reasons that name a pin are worth saying one by one;
                // "was left out" has already been said about its part.
                for why in reasons
                    .into_iter()
                    .filter(|why| !why.ends_with("was left out"))
                {
                    notes.push(format!("the connection {from} — {to} was left out: {why}"));
                }
            }
        }
    }
    if dropped > 0 {
        notes.push(format!("{dropped} connection(s) did not come across"));
    }
    sheet.kit_x = Some(kit_x);
    sheet.kit_y = Some(kit_y);
    sheet.notes = notes;
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c3_rows() -> Vec<Row> {
        crate::nets::kit_rows(
            "esp32c3",
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 18, 19, 20, 21],
        )
    }

    const BLINKY: &str = r#"{
      "version": 1,
      "author": "somebody",
      "editor": "wokwi",
      "parts": [
        { "type": "board-esp32-c3-devkitm-1", "id": "esp", "top": 0, "left": 0, "attrs": {} },
        { "type": "wokwi-led", "id": "led1", "top": -60, "left": 120, "rotate": 90, "attrs": { "color": "green" } },
        { "type": "wokwi-resistor", "id": "r1", "top": 20, "left": 110, "attrs": { "value": "220" } },
        { "type": "wokwi-pushbutton", "id": "btn1", "top": 100, "left": 140, "attrs": { "color": "blue" } },
        { "type": "wokwi-mpu6050", "id": "imu1", "top": 200, "left": 140, "attrs": { "accelX": "0.5" } },
        { "type": "wokwi-hc-sr04", "id": "sonar", "top": 300, "left": 140, "attrs": {} }
      ],
      "connections": [
        [ "esp:TX", "$serialMonitor:RX", "", [] ],
        [ "esp:2", "r1:1", "green", [ "v0" ] ],
        [ "r1:2", "led1:A", "green", [] ],
        [ "led1:C", "esp:GND.3", "black", [] ],
        [ "btn1:1.l", "esp:9", "blue", [] ],
        [ "btn1:2.r", "esp:GND.1", "black", [] ],
        [ "imu1:SDA", "esp:5", "", [] ],
        [ "imu1:SCL", "esp:6", "", [] ],
        [ "imu1:VCC", "esp:5V.1", "", [] ],
        [ "sonar:TRIG", "esp:7", "", [] ],
        [ "esp:40", "led1:A", "", [] ]
      ]
    }"#;

    fn wired(sheet: &Sheet, a: &str, b: &str) -> bool {
        sheet.wires.iter().any(|w| {
            let (from, to) = (
                format!("{}.{}", w.from.part, w.from.pin),
                format!("{}.{}", w.to.part, w.to.pin),
            );
            (from == a && to == b) || (from == b && to == a)
        })
    }

    /// The parts rusty has come across with their values and their wiring,
    /// on the devkit rows for the same GPIOs.
    #[test]
    fn a_wokwi_diagram_comes_across_with_its_wiring() {
        let sheet = read(BLINKY, "esp32c3", &c3_rows()).unwrap();
        let led = sheet
            .parts
            .iter()
            .find(|p| p.symbol == "Device:LED")
            .unwrap();
        assert_eq!(
            (led.reference.as_str(), led.value.as_str(), led.rot),
            ("D1", "green", 90)
        );
        let resistor = sheet.parts.iter().find(|p| p.symbol == "Device:R").unwrap();
        assert_eq!(resistor.value, "220");
        assert!(wired(&sheet, "U1.GPIO2", "R1.1"), "{:?}", sheet.wires);
        assert!(wired(&sheet, "R1.2", "D1.A"));
        assert!(wired(&sheet, "D1.K", "U1.GND"));
        assert!(wired(&sheet, "SW1.1", "U1.GPIO9"));
        assert!(wired(&sheet, "SW1.2", "U1.GND"));

        // The sensor keeps what Wokwi's attributes said, and goes on the bus.
        let imu = sheet
            .parts
            .iter()
            .find(|p| p.symbol == "rusty:Sensor")
            .unwrap();
        assert_eq!(imu.props.get("model").map(String::as_str), Some("mpu6050"));
        assert_eq!(imu.props.get("addr").map(String::as_str), Some("68"));
        assert_eq!(imu.props.get("ax").map(String::as_str), Some("0.5"));
        assert_eq!(imu.reference, "U2", "the devkit is U1");
        assert!(wired(&sheet, "U2.SDA", "U1.GPIO5"));
        assert!(wired(&sheet, "U2.SCL", "U1.GPIO6"));

        // A C3's devkit has no 5 V row, so 5 V is a rail of its own.
        let supply = sheet
            .parts
            .iter()
            .find(|p| p.symbol == "rusty:Supply")
            .unwrap();
        assert_eq!(supply.value, "5V");
        assert!(
            sheet
                .wires
                .iter()
                .any(|w| w.to.part == supply.reference || w.from.part == supply.reference)
        );
    }

    /// What did not come across is named, with the connections it took
    /// with it — never drawn as something else.
    #[test]
    fn what_rusty_has_no_counterpart_for_is_named() {
        let sheet = read(BLINKY, "esp32c3", &c3_rows()).unwrap();
        assert!(sheet.parts.iter().all(|p| p.reference != "sonar"));
        let notes = sheet.notes.join("\n");
        assert!(
            notes.contains("sonar (wokwi-hc-sr04) was left out"),
            "{notes}"
        );
        assert!(notes.contains("GPIO40 is not on this chip"), "{notes}");
        assert!(
            notes.contains("2 connection(s) did not come across"),
            "{notes}"
        );
        assert!(
            !notes.contains("serialMonitor"),
            "the console is always attached"
        );
    }

    /// A board's `TX` is the chip's console pin, which is not the same GPIO
    /// on every chip.
    #[test]
    fn a_console_pin_is_the_chips_own() {
        let rows = c3_rows();
        assert_eq!(
            board_pin("board-esp32-c3-devkitm-1", "TX", &rows),
            End::Pin(PinRef {
                part: "U1".into(),
                pin: "GPIO21".into()
            })
        );
        let esp32 = crate::nets::kit_rows("esp32", &[]);
        assert_eq!(
            board_pin("board-esp32-devkit-c-v4", "RX", &esp32),
            End::Pin(PinRef {
                part: "U1".into(),
                pin: "GPIO3".into()
            })
        );
        assert_eq!(
            board_pin("wokwi-esp32-devkit-v1", "D13", &esp32),
            End::Pin(PinRef {
                part: "U1".into(),
                pin: "GPIO13".into()
            })
        );
        assert!(matches!(
            board_pin("board-esp32-devkit-c-v4", "CLK", &esp32),
            End::Dropped(_)
        ));
        assert!(
            matches!(
                board_pin("board-esp32-devkit-c-v4", "5V", &esp32),
                End::Pin(_)
            ),
            "VIN on the ESP32 devkit"
        );
    }

    #[test]
    fn a_file_that_is_not_a_diagram_is_refused() {
        assert!(read("{ not json", "esp32c3", &c3_rows()).is_err());
    }
}
