//! The devkit's header — what each row is and which row a wire lands on —
//! and binding an imported board's microcontroller to it.

use serde::{Deserialize, Serialize};

use super::Rail;
use crate::model::{KIT_REFERENCE, PinRef, Sheet, Wire};

/// One row of the devkit's header: what is printed beside it, the pin
/// name a wire uses, and what it carries.
///
/// The name is `GPIO<n>` for a GPIO row whatever the label says — the
/// ESP32 devkit prints `RX` beside GPIO3 — and the label itself for a rail.
/// Nothing here is geometry: where a row is drawn is the frontend's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub label: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpio: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rail: Option<Rail>,
}

impl Row {
    fn gpio(n: u8, label: &str) -> Self {
        Row {
            label: label.to_string(),
            name: format!("GPIO{n}"),
            gpio: Some(n),
            rail: None,
        }
    }

    fn rail(label: &str, rail: Rail) -> Self {
        Row {
            label: label.to_string(),
            name: label.to_string(),
            gpio: None,
            rail: Some(rail),
        }
    }

    fn plain(label: &str) -> Self {
        Row {
            label: label.to_string(),
            name: label.to_string(),
            gpio: None,
            rail: None,
        }
    }
}

/// The classic 30-pin ESP32 devkit header, top to bottom, left then right.
///
/// The one module whose *header order* rusty knows. That order is a
/// property of the board, not the die, so it cannot be derived — and it
/// used to be drawn for every chip, which is why an ESP32-C3 board showed
/// GPIO36, 39, 34 and 35, none of which the part has.
const ESP32_DEVKIT: [(&str, Option<u8>); 30] = [
    ("EN", None),
    ("36", Some(36)),
    ("39", Some(39)),
    ("34", Some(34)),
    ("35", Some(35)),
    ("32", Some(32)),
    ("33", Some(33)),
    ("25", Some(25)),
    ("26", Some(26)),
    ("27", Some(27)),
    ("14", Some(14)),
    ("12", Some(12)),
    ("13", Some(13)),
    ("GND", None),
    ("VIN", None),
    ("3V3", None),
    ("GND", None),
    ("15", Some(15)),
    ("2", Some(2)),
    ("4", Some(4)),
    ("16", Some(16)),
    ("17", Some(17)),
    ("5", Some(5)),
    ("18", Some(18)),
    ("19", Some(19)),
    ("21", Some(21)),
    ("RX", Some(3)),
    ("TX", Some(1)),
    ("22", Some(22)),
    ("23", Some(23)),
];

/// The devkit's rows for a chip, given the GPIOs it actually has.
///
/// Two different drawings, and the difference is honest rather than
/// cosmetic. For the ESP32 the answer is a *module*: a real 30-pin devkit
/// whose header order somebody can match against the board on their desk.
/// For everything else rusty knows the die's pins and not any module's
/// header, so it draws a *chip* — the pins in numeric order, split down the
/// middle, with the rails around them. Every row is then a pin that exists.
///
/// An empty `gpio` means the catalogue does not say, and the part is drawn
/// with rails only rather than with somebody else's pins.
pub fn kit_rows(chip: &str, gpio: &[u32]) -> Vec<Row> {
    let row = |label: &str, pin: Option<u8>| match (label, pin) {
        (_, Some(n)) => Row::gpio(n, label),
        ("GND", None) => Row::rail("GND", Rail::Ground),
        ("3V3" | "VIN" | "5V", None) => Row::rail(label, Rail::Supply),
        (other, None) => Row::plain(other),
    };
    if chip == "esp32" {
        return ESP32_DEVKIT
            .iter()
            .map(|(label, pin)| row(label, *pin))
            .collect();
    }
    let half = gpio.len().div_ceil(2);
    let mut rows: Vec<Row> = Vec::with_capacity(gpio.len() + 4);
    rows.push(row("EN", None));
    rows.extend(
        gpio[..half]
            .iter()
            .map(|p| row(&p.to_string(), Some(*p as u8))),
    );
    rows.push(row("GND", None));
    // The right column starts here, so the rails sit at the top of each side
    // the way they do on a module.
    rows.push(row("3V3", None));
    rows.extend(
        gpio[half..]
            .iter()
            .map(|p| row(&p.to_string(), Some(*p as u8))),
    );
    rows.push(row("GND", None));
    rows
}

/// Which row a wire to `U1.<key>` lands on: by number (the row's 1-based
/// position, the spelling a file uses when a name repeats) and then by
/// name — `GPIO2`, `GND`, `3V3`. The first `GND` for a bare `GND`.
pub fn kit_pin(rows: &[Row], key: &str) -> Option<usize> {
    key.parse::<usize>()
        .ok()
        .filter(|n| (1..=rows.len()).contains(n))
        .map(|n| n - 1)
        .or_else(|| rows.iter().position(|r| r.name == key))
}

/// The GPIO a pin's *name* claims, or nothing.
///
/// Vendors spell the same pin three ways on one datasheet — `GPIO5`, `IO5`,
/// `GPIO05` — and a symbol drawn by hand uses whichever the author read.
/// Anything that is not one of those shapes is not a GPIO: `GPIO` alone,
/// `IO_MUX`, `VDD3P3` and a bare `5` all answer `None`, because a pin bound
/// to the wrong row is a lamp that lights when the firmware set a different
/// pin and nothing on screen to say so.
pub fn gpio_named(name: &str) -> Option<u8> {
    let digits = name
        .strip_prefix("GPIO")
        .or_else(|| name.strip_prefix("IO"))
        .or_else(|| name.strip_prefix("gpio"))
        .or_else(|| name.strip_prefix("io"))?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Wire an imported microcontroller to the devkit, so a schematic drawn
/// elsewhere can be simulated here.
///
/// **Why this is needed at all.** rusty drives pins through `U1`, whose
/// header rows *are* the GPIOs; a KiCad schematic has a module of its own
/// instead, and nothing on it reaches a row. So an imported board draws and
/// checks and does nothing when it is run. This reads the module's pin
/// *names* — what the author wrote — and joins each to the row of the same
/// number, which is the same reading `kit_rows` does from the other side.
///
/// **Exactly one candidate, or none.** Two parts that both look like the
/// microcontroller is a question with no right answer, and answering it
/// anyway means driving one module's pins through the other's — so that
/// case binds nothing and names both, the way `firmware_root` refuses two
/// excluded firmware crates. A part qualifies on four GPIO-named pins:
/// fewer is a header or a test point, and a connector that happens to be
/// labelled `IO0` should not become the chip.
///
/// The wires it adds are rusty's, not the file's, and the KiCad writer
/// knows it: a wire touching `U1` is neither written nor counted as a
/// change, so binding a board and exporting it again is still the identity.
pub fn bind_to_kit(sheet: &mut Sheet, rows: &[Row]) -> Vec<String> {
    let mut candidates: Vec<(String, Vec<(String, u8)>)> = Vec::new();
    for part in &sheet.parts {
        let Some(symbol) = sheet.symbol_of(&part.reference) else {
            continue;
        };
        let found: Vec<(String, u8)> = symbol
            .pins
            .iter()
            .filter(|pin| !pin.hidden)
            .filter_map(|pin| {
                let gpio = gpio_named(&pin.name)?;
                rows.iter()
                    .any(|row| row.gpio == Some(gpio))
                    .then(|| (pin.number.clone(), gpio))
            })
            .collect();
        if found.len() >= 4 {
            candidates.push((part.reference.clone(), found));
        }
    }

    match candidates.len() {
        0 => Vec::new(),
        1 => {
            let (reference, pins) = candidates.remove(0);
            let mut bound = 0usize;
            for (number, gpio) in pins {
                let Some(row) = rows.iter().find(|row| row.gpio == Some(gpio)) else {
                    continue;
                };
                let from = PinRef::new(&reference, &number);
                let to = PinRef::new(KIT_REFERENCE, &row.name);
                if sheet
                    .wires
                    .iter()
                    .any(|w| (w.from == from && w.to == to) || (w.from == to && w.to == from))
                {
                    continue;
                }
                sheet.wires.push(Wire {
                    from,
                    to,
                    bends: Vec::new(),
                });
                bound += 1;
            }
            if bound == 0 {
                return Vec::new();
            }
            vec![format!(
                "{reference} reads as this board's microcontroller, so its {bound} \
                 named GPIO pins were joined to the devkit's rows and the sheet \
                 can be simulated. Those joins are rusty's own and are not \
                 written back to the file."
            )]
        }
        _ => {
            let names: Vec<&str> = candidates.iter().map(|(r, _)| r.as_str()).collect();
            vec![format!(
                "{} both read as this board's microcontroller, so neither was \
                 joined to the devkit: driving one module's pins through the \
                 other's would be a guess. Wire the one you mean to U1 by hand.",
                names.join(" and ")
            )]
        }
    }
}
