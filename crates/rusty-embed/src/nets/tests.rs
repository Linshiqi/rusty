use std::collections::BTreeMap;

use super::*;
use crate::model::{
    Fill, Graphic, Instance, KIT_REFERENCE, Pin, PinKind, PinRef, Sheet, Symbol, Wire,
};

/// The parts rusty ships, which is what a sheet with no project behind
/// it resolves its `model` props against.
fn specs() -> Vec<crate::sensor::Spec> {
    crate::partfile::load(None).specs
}

fn pin(number: &str, name: &str, x: f64) -> Pin {
    Pin {
        number: number.into(),
        name: name.into(),
        kind: PinKind::Passive,
        at: (x, 0.0),
        length: 2.54,
        angle: if x < 0.0 { 0 } else { 180 },
        hidden: false,
    }
}

fn symbol(library: &str, name: &str, reference: &str, pins: &[(&str, &str)]) -> Symbol {
    Symbol {
        library: library.into(),
        name: name.into(),
        reference: reference.into(),
        value: name.into(),
        description: None,
        pins: pins
            .iter()
            .enumerate()
            .map(|(i, (number, name))| pin(number, name, if i % 2 == 0 { -5.08 } else { 5.08 }))
            .collect(),
        graphics: vec![Graphic::Rectangle {
            start: (-2.54, -1.27),
            end: (2.54, 1.27),
            width: 0.254,
            fill: Fill::None,
        }],
    }
}

fn place(sheet: &mut Sheet, reference: &str, symbol: &str) {
    sheet.parts.push(Instance {
        reference: reference.into(),
        symbol: symbol.into(),
        value: String::new(),
        x: 0.0,
        y: 0.0,
        rot: 0,
        mirror: false,
        props: BTreeMap::new(),
    });
}

fn wire(sheet: &mut Sheet, from: &str, to: &str) {
    sheet.wires.push(Wire {
        from: PinRef::parse(from).unwrap(),
        to: PinRef::parse(to).unwrap(),
        bends: Vec::new(),
    });
}

fn sheet() -> Sheet {
    let mut sheet = Sheet::empty("esp32c3");
    sheet.symbols = vec![
        symbol("Device", "LED", "D", &[("1", "K"), ("2", "A")]),
        symbol("Device", "R", "R", &[("1", "~"), ("2", "~")]),
        symbol("Device", "C", "C", &[("1", "~"), ("2", "~")]),
        symbol("Device", "SW_Push", "SW", &[("1", "1"), ("2", "2")]),
        symbol("lcsc", "C2286", "LED", &[("1", "A"), ("2", "K")]),
        symbol(
            "rusty",
            "RGB_LED",
            "D",
            &[("1", "R"), ("2", "G"), ("3", "B"), ("4", "COM")],
        ),
        symbol("rusty", "Pot", "RV", &[("1", "1"), ("2", "W"), ("3", "3")]),
        symbol("rusty", "GND", "#PWR", &[("1", "GND")]),
        symbol("rusty", "Supply", "#PWR", &[("1", "VCC")]),
        symbol("rusty", "Label", "#LBL", &[("1", "~")]),
        symbol("rusty", "Buzzer", "BZ", &[("1", "+"), ("2", "-")]),
        symbol(
            "rusty",
            "Sensor",
            "U",
            &[("1", "SDA"), ("2", "SCL"), ("3", "VCC"), ("4", "GND")],
        ),
        symbol(
            "rusty",
            "Keypad",
            "KP",
            &[
                ("1", "R1"),
                ("2", "R2"),
                ("3", "R3"),
                ("4", "R4"),
                ("5", "C1"),
                ("6", "C2"),
                ("7", "C3"),
                ("8", "C4"),
            ],
        ),
    ];
    sheet
}

/// A sensor wired to the bus, with whatever properties the test wants.
fn place_on_bus(sheet: &mut Sheet, reference: &str, props: &[(&str, &str)]) {
    place(sheet, reference, "rusty:Sensor");
    wire(sheet, &format!("{reference}.1"), "U1.GPIO5");
    wire(sheet, &format!("{reference}.2"), "U1.GPIO6");
    if let Some(part) = sheet.parts.iter_mut().find(|p| p.reference == reference) {
        for (key, value) in props {
            part.props.insert((*key).into(), (*value).into());
        }
    }
}

/// Place a part carrying a value — a rail's name, a label's net.
fn place_with(sheet: &mut Sheet, reference: &str, symbol: &str, value: &str) {
    place(sheet, reference, symbol);
    if let Some(part) = sheet.parts.iter_mut().find(|p| p.reference == reference) {
        part.value = value.to_string();
    }
}

fn rows() -> Vec<Row> {
    kit_rows("esp32c3", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 21])
}

fn eval(sheet: &Sheet, gpio: &[(u8, bool)], pressed: &[&str]) -> Evaluation {
    let rows = rows();
    evaluate(Inputs {
        sheet,
        rows: &rows,
        gpio: &gpio.iter().copied().collect(),
        pressed: &pressed.iter().map(|s| s.to_string()).collect(),
    })
}

#[test]
fn behaviour_is_read_off_the_library_then_the_prefix_and_the_pins() {
    let s = sheet();
    let of = |name: &str| behaviour_of(s.symbols.iter().find(|x| x.name == name).unwrap());
    assert_eq!(of("LED"), Behaviour::Led);
    assert_eq!(of("R"), Behaviour::Resistor);
    assert_eq!(of("C"), Behaviour::Capacitor);
    assert_eq!(of("SW_Push"), Behaviour::Switch);
    assert_eq!(of("C2286"), Behaviour::Led, "LCSC's prefix is LED");
    assert_eq!(of("RGB_LED"), Behaviour::Rgb);
    assert_eq!(of("Pot"), Behaviour::Pot);
    let diode = symbol("Device", "D", "D", &[("1", "K"), ("2", "A")]);
    assert_eq!(
        behaviour_of(&diode),
        Behaviour::Led,
        "an A and a K is a lamp"
    );
    let zener = symbol("Device", "D_Zener", "D", &[("1", "1"), ("2", "2")]);
    assert_eq!(behaviour_of(&zener), Behaviour::Other);
}

#[test]
fn the_kit_rows_name_gpios_and_rails_and_a_repeated_name_is_spelled_by_number() {
    let rows = rows();
    assert_eq!(rows[0].name, "EN");
    assert_eq!(rows[1].name, "GPIO0");
    assert_eq!(kit_pin(&rows, "GPIO2"), Some(3));
    assert_eq!(kit_pin(&rows, "4"), Some(3), "the fourth row, by number");
    assert_eq!(kit_pin(&rows, "GND"), Some(8), "the first GND");
    assert_eq!(kit_pin(&rows, "GPIO99"), None);
    assert_eq!(kit_pin(&rows, "0"), None);
    let esp32 = kit_rows("esp32", &[]);
    assert_eq!(esp32[26].name, "GPIO3", "RX is GPIO3 by name");
    assert_eq!(esp32[26].label, "RX");
    assert_eq!(esp32[15].rail, Some(Rail::Supply));
}

/// GPIO2 → R1 → D1 → GND: the everyday circuit. The lamp follows the
/// pin, through the resistor, and the sheet has nothing to say.
#[test]
fn a_lamp_through_a_resistor_follows_its_gpio() {
    let mut s = sheet();
    place(&mut s, "R1", "Device:R");
    place(&mut s, "D1", "Device:LED");
    wire(&mut s, "U1.GPIO2", "R1.1");
    wire(&mut s, "R1.2", "D1.A");
    wire(&mut s, "D1.K", "U1.GND");
    let on = eval(&s, &[(2, true)], &[]);
    assert!(on.is_lit("D1"));
    assert!(on.is_pin_lit("D1", "2"), "keyed by the anode's number");
    assert_eq!(on.level("D1", "A"), Some(true));
    assert_eq!(on.level("D1", "K"), Some(false));
    assert!(on.warnings.is_empty(), "{:?}", on.warnings);
    let off = eval(&s, &[(2, false)], &[]);
    assert!(!off.is_lit("D1"));
    let unknown = eval(&s, &[], &[]);
    assert!(!unknown.is_lit("D1"), "a pin never reported is not high");
    assert_eq!(unknown.level("D1", "A"), None);
}

#[test]
fn a_lamp_the_wrong_way_round_stays_dark_and_a_capacitor_passes_nothing() {
    let mut s = sheet();
    place(&mut s, "D1", "Device:LED");
    wire(&mut s, "U1.GPIO2", "D1.K");
    wire(&mut s, "D1.A", "U1.GND");
    assert!(!eval(&s, &[(2, true)], &[]).is_lit("D1"));

    let mut s = sheet();
    place(&mut s, "C1", "Device:C");
    place(&mut s, "D1", "Device:LED");
    wire(&mut s, "U1.GPIO2", "C1.1");
    wire(&mut s, "C1.2", "D1.A");
    wire(&mut s, "D1.K", "U1.GND");
    let e = eval(&s, &[(2, true)], &[]);
    assert!(!e.is_lit("D1"));
    assert_eq!(
        e.level("D1", "A"),
        None,
        "the anode floats behind the capacitor"
    );
}

#[test]
fn a_lamp_straight_on_a_gpio_lights_and_is_warned_about() {
    let mut s = sheet();
    place(&mut s, "D1", "Device:LED");
    wire(&mut s, "U1.GPIO2", "D1.A");
    wire(&mut s, "D1.K", "U1.GND");
    let e = eval(&s, &[(2, true)], &[]);
    assert!(e.is_lit("D1"));
    assert_eq!(
        e.warnings,
        vec![Warning::LedWithoutResistor { part: "D1".into() }]
    );
    // Active-low wiring: anode on 3V3, the GPIO sinking through the
    // cathode. Lit when the pin is low, and still no resistor.
    let mut s = sheet();
    place(&mut s, "D2", "lcsc:C2286");
    wire(&mut s, "U1.3V3", "D2.A");
    wire(&mut s, "D2.K", "U1.GPIO8");
    assert!(eval(&s, &[(8, false)], &[]).is_lit("D2"));
    assert!(!eval(&s, &[(8, true)], &[]).is_lit("D2"));
    assert_eq!(eval(&s, &[], &[]).warnings.len(), 1);
}

/// A key between two GPIOs joins them; it drives neither. That is the
/// difference a matrix rests on, and reading it as a drive is how a
/// scanned row would look like every key in its column held down.
#[test]
fn a_switch_between_two_gpios_is_a_tie_and_not_a_drive() {
    let mut s = sheet();
    place(&mut s, "SW1", "Device:SW_Push");
    wire(&mut s, "U1.GPIO9", "SW1.1");
    wire(&mut s, "SW1.2", "U1.GPIO4");
    let rows = rows();
    assert_eq!(
        switch_tie(&s, &rows, "SW1"),
        Some((4, 9)),
        "lower pin first"
    );
    assert_eq!(
        button_drives(&s, &rows, "SW1"),
        None,
        "there is no rail, so nothing is driven"
    );

    // A switch to a rail is the other thing, and must not read as a tie.
    let mut s = sheet();
    place(&mut s, "SW2", "Device:SW_Push");
    wire(&mut s, "U1.GPIO9", "SW2.1");
    wire(&mut s, "SW2.2", "U1.GND");
    assert_eq!(switch_tie(&s, &rows, "SW2"), None);
    assert_eq!(button_drives(&s, &rows, "SW2"), Some((9, false)));

    // And one side unwired joins nothing at all.
    let mut s = sheet();
    place(&mut s, "SW3", "Device:SW_Push");
    wire(&mut s, "U1.GPIO9", "SW3.1");
    assert_eq!(switch_tie(&s, &rows, "SW3"), None);
}

/// A keypad's key is named by its row and column, and the pins it joins
/// are whatever those two reach. A row wired and a column not is
/// nothing — a press that reaches no pin is said rather than invented.
#[test]
fn a_keypads_key_joins_the_gpios_its_row_and_column_reach() {
    let mut s = sheet();
    place(&mut s, "KP1", "rusty:Keypad");
    wire(&mut s, "KP1.R1", "U1.GPIO9");
    wire(&mut s, "KP1.R3", "U1.GPIO4");
    wire(&mut s, "KP1.C2", "U1.GPIO8");
    let rows = rows();
    assert_eq!(keypad_tie(&s, &rows, "KP1", 0, 1), Some((9, 8)));
    assert_eq!(keypad_tie(&s, &rows, "KP1", 2, 1), Some((4, 8)));
    assert_eq!(
        keypad_tie(&s, &rows, "KP1", 1, 1),
        None,
        "row 2 is not wired"
    );
    assert_eq!(
        keypad_tie(&s, &rows, "KP1", 0, 0),
        None,
        "column 1 is not wired"
    );
}

/// A pull-up button: GPIO9 — SW1 — GND. Pressing it puts ground on the
/// pin, which is what the emulator must be driven to, and what
/// `Input::is_low()` then reads.
#[test]
fn a_pressed_button_puts_its_rail_on_its_gpio() {
    let mut s = sheet();
    place(&mut s, "SW1", "Device:SW_Push");
    wire(&mut s, "U1.GPIO9", "SW1.1");
    wire(&mut s, "SW1.2", "U1.GND");
    let rows = rows();
    assert_eq!(button_drives(&s, &rows, "SW1"), Some((9, false)));
    let held = eval(&s, &[(9, true)], &["SW1"]);
    assert_eq!(
        held.level("SW1", "1"),
        Some(false),
        "ground wins over the pin's own report"
    );
    assert_eq!(held.level("U1", "GPIO9"), Some(false));
    let released = eval(&s, &[(9, true)], &[]);
    assert_eq!(released.level("SW1", "2"), Some(false));
    assert_eq!(
        released.level("SW1", "1"),
        Some(true),
        "the pin's own level again"
    );
    assert!(released.warnings.is_empty(), "{:?}", released.warnings);

    // To 3V3 instead: a press drives high.
    let mut s = sheet();
    place(&mut s, "SW2", "Device:SW_Push");
    wire(&mut s, "U1.GPIO4", "SW2.2");
    wire(&mut s, "SW2.1", "U1.3V3");
    assert_eq!(button_drives(&s, &rows, "SW2"), Some((4, true)));

    // One side unwired: nothing to drive, and the sheet says so.
    let mut s = sheet();
    place(&mut s, "SW3", "Device:SW_Push");
    wire(&mut s, "U1.GPIO4", "SW3.1");
    assert_eq!(button_drives(&s, &rows, "SW3"), None);
    assert_eq!(
        eval(&s, &[], &[]).warnings,
        vec![Warning::SwitchDrivesNothing { part: "SW3".into() }]
    );
}

/// A switch to a ground *symbol* is a switch to ground, as a lamp to one
/// is a lamp to ground: a power symbol is a rail wherever it is drawn.
/// The press reading looked only at the devkit's own rows, so a button
/// wired to a `rusty:GND` drove nothing — the press never reached the
/// emulator, the firmware read its pull-up for ever, and the sheet said
/// "pressing it changes nothing" about a circuit that is right.
#[test]
fn a_switch_to_a_power_symbol_drives_its_gpio() {
    let rows = rows();
    let mut s = sheet();
    place(&mut s, "SW1", "Device:SW_Push");
    place(&mut s, "#PWR1", "rusty:GND");
    wire(&mut s, "U1.GPIO4", "SW1.1");
    wire(&mut s, "SW1.2", "#PWR1.GND");
    assert_eq!(button_drives(&s, &rows, "SW1"), Some((4, false)));
    assert!(
        eval(&s, &[], &[]).warnings.is_empty(),
        "{:?}",
        eval(&s, &[], &[]).warnings
    );

    let mut s = sheet();
    place(&mut s, "SW2", "Device:SW_Push");
    place(&mut s, "#PWR2", "rusty:Supply");
    wire(&mut s, "U1.GPIO5", "SW2.2");
    wire(&mut s, "SW2.1", "#PWR2.VCC");
    assert_eq!(button_drives(&s, &rows, "SW2"), Some((5, true)));
}

/// The two findings that are about the drawing rather than the run, and
/// the three things that keep the first of them usable: it waits for
/// the part to be otherwise wired, it stands down for a no-connect, and
/// it stands down where something more specific already names the part.
#[test]
fn a_loose_pin_is_named_unless_it_was_answered_for() {
    let loose = |s: &Sheet| -> Vec<String> {
        eval(s, &[], &[])
            .warnings
            .into_iter()
            .filter_map(|w| match w {
                Warning::PinReachesNothing { part, pin } => Some(format!("{part}.{pin}")),
                _ => None,
            })
            .collect()
    };

    // A part nobody has started on says nothing: every pin is loose and
    // six findings for one untouched symbol is a list people stop reading.
    let mut fresh = sheet();
    place(&mut fresh, "R1", "Device:R");
    assert!(loose(&fresh).is_empty(), "{:?}", loose(&fresh));

    // One side joined, and the other is the question.
    let mut half = sheet();
    place(&mut half, "R1", "Device:R");
    wire(&mut half, "R1.1", "U1.GPIO2");
    assert_eq!(loose(&half), vec!["R1.2"]);

    // Answered: the author has said so.
    let mut answered = half.clone();
    answered.no_connect.push(PinRef::new("R1", "2"));
    assert!(loose(&answered).is_empty());

    // And a switch with one side loose is `SwitchDrivesNothing`, which
    // is the same fault said better — so the generic one stays quiet.
    let mut switch = sheet();
    place(&mut switch, "SW1", "Device:SW_Push");
    wire(&mut switch, "SW1.1", "U1.GPIO4");
    let found = eval(&switch, &[], &[]).warnings;
    assert!(
        matches!(&found[..], [Warning::SwitchDrivesNothing { .. }]),
        "one finding, the specific one: {found:?}"
    );
}

/// Two pins that both drive, wired together — true of the drawing and
/// not of any particular moment of a run, which is what tells it from
/// `Conflict`.
#[test]
fn two_driving_pins_on_one_net_are_named() {
    let mut s = sheet();
    s.symbols.push(symbol("Reg", "LDO", "U", &[("1", "OUT")]));
    if let Some(defined) = s.symbols.iter_mut().find(|x| x.name == "LDO") {
        defined.pins[0].kind = PinKind::PowerOut;
    }
    place(&mut s, "U2", "Reg:LDO");
    place(&mut s, "U3", "Reg:LDO");
    wire(&mut s, "U2.1", "U3.1");
    let found = eval(&s, &[], &[]).warnings;
    assert!(
        found
            .iter()
            .any(|w| matches!(w, Warning::OutputsFighting { pins } if pins.len() == 2)),
        "{found:?}"
    );

    // One of them alone is not a fault, however it is wired.
    let mut one = sheet();
    one.symbols.push(symbol("Reg", "LDO", "U", &[("1", "OUT")]));
    if let Some(defined) = one.symbols.iter_mut().find(|x| x.name == "LDO") {
        defined.pins[0].kind = PinKind::PowerOut;
    }
    place(&mut one, "U2", "Reg:LDO");
    wire(&mut one, "U2.1", "U1.GPIO2");
    assert!(
        !one.wires.is_empty()
            && !eval(&one, &[], &[])
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::OutputsFighting { .. }))
    );
}

/// A pin name is read for a GPIO the way three datasheets spell one,
/// and anything else is refused — a pin bound to the wrong row lights a
/// lamp the firmware never set.
#[test]
fn a_gpio_is_read_from_a_pins_name_or_refused() {
    assert_eq!(gpio_named("GPIO5"), Some(5));
    assert_eq!(gpio_named("IO5"), Some(5));
    assert_eq!(gpio_named("GPIO05"), Some(5));
    assert_eq!(gpio_named("io21"), Some(21));
    assert_eq!(gpio_named("GPIO"), None, "a prefix is not a pin");
    assert_eq!(gpio_named("IO_MUX"), None);
    assert_eq!(gpio_named("VDD3P3"), None);
    assert_eq!(gpio_named("5"), None, "a bare number names nothing");
    assert_eq!(gpio_named("GPIO5A"), None);
}

/// The join that turns an imported board from one rusty can draw into
/// one rusty can run — and the case where it refuses to.
#[test]
fn an_imported_module_is_joined_to_the_devkit_or_the_two_are_named() {
    let rows = rows();
    let module = |name: &str| {
        symbol(
            "RF_Module",
            name,
            "U",
            &[
                ("1", "IO2"),
                ("2", "IO3"),
                ("3", "GPIO4"),
                ("4", "IO5"),
                ("5", "GND"),
                ("6", "IO99"),
            ],
        )
    };

    let mut s = sheet();
    s.symbols.push(module("ESP32-C3-MINI-1"));
    place(&mut s, "U2", "RF_Module:ESP32-C3-MINI-1");
    let notes = bind_to_kit(&mut s, &rows);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("U2") && notes[0].contains("4"),
        "{}",
        notes[0]
    );

    let joined = |pin: &str, row: &str| {
        let (a, b) = (PinRef::new("U2", pin), PinRef::new(KIT_REFERENCE, row));
        s.wires
            .iter()
            .any(|w| (w.from == a && w.to == b) || (w.from == b && w.to == a))
    };
    assert!(joined("1", "GPIO2"), "IO2 reaches GPIO2");
    assert!(joined("3", "GPIO4"), "and GPIO4 reaches GPIO4");
    assert!(
        !s.wires.iter().any(|w| w.from.pin == "6" || w.to.pin == "6"),
        "IO99 is not a pin this chip has, so it joins nothing"
    );
    assert!(
        !s.wires.iter().any(|w| w.from.pin == "5" || w.to.pin == "5"),
        "and GND is not a GPIO"
    );

    // Run again: the joins are already there and are not doubled.
    let before = s.wires.len();
    assert!(bind_to_kit(&mut s, &rows).is_empty());
    assert_eq!(s.wires.len(), before);

    // Two modules is a question with no right answer.
    let mut two = sheet();
    two.symbols.push(module("ESP32-C3-MINI-1"));
    two.symbols.push(module("ESP32-C6-MINI-1"));
    place(&mut two, "U2", "RF_Module:ESP32-C3-MINI-1");
    place(&mut two, "U3", "RF_Module:ESP32-C6-MINI-1");
    let notes = bind_to_kit(&mut two, &rows);
    assert!(two.wires.is_empty(), "neither is bound");
    assert!(
        notes[0].contains("U2") && notes[0].contains("U3"),
        "and both are named: {}",
        notes[0]
    );

    // A part with a GPIO-ish pin or two is a connector, not the chip.
    let mut small = sheet();
    small
        .symbols
        .push(symbol("Conn", "Header", "J", &[("1", "IO0"), ("2", "GND")]));
    place(&mut small, "J1", "Conn:Header");
    assert!(bind_to_kit(&mut small, &rows).is_empty());
    assert!(small.wires.is_empty());
}

/// A resistance is read the way people write one, and a value that is
/// not a resistance is nothing rather than a number — the whole reason
/// `ohms` returns an `Option`.
#[test]
fn a_resistance_is_read_from_the_value_or_refused() {
    assert_eq!(ohms("220"), Some(220.0));
    assert_eq!(ohms("220R"), Some(220.0));
    assert_eq!(ohms(" 10k "), Some(10_000.0));
    assert_eq!(ohms("10K"), Some(10_000.0));
    assert_eq!(ohms("4k7"), Some(4700.0));
    assert_eq!(ohms("4.7k"), Some(4700.0));
    assert_eq!(ohms("1M"), Some(1_000_000.0));
    assert_eq!(ohms("1 kΩ"), Some(1000.0));
    assert_eq!(ohms(""), None);
    assert_eq!(ohms("red"), None, "a lamp's colour is not a resistance");
    assert_eq!(ohms("C25804"), None, "nor is a part number");
}

/// The one thing a resistor's *value* decides, and the reason reading
/// it is worth anything: where a pin sits between the rails. Asserted
/// against the arithmetic anybody would do by hand, and — as loudly —
/// against the cases where the sheet has not said enough.
#[test]
fn a_divider_is_read_from_the_values_on_the_sheet_or_refused() {
    let divider = |s: &Sheet, pin: &str| divider_at(s, &rows(), &PinRef::parse(pin).unwrap());

    // Two resistors, the midpoint tapped: the textbook divider.
    let mut s = sheet();
    place_with(&mut s, "R1", "Device:R", "20k");
    place_with(&mut s, "R2", "Device:R", "10k");
    wire(&mut s, "R1.1", "U1.3V3");
    wire(&mut s, "R1.2", "R2.1");
    wire(&mut s, "R2.2", "U1.GND");
    wire(&mut s, "R1.2", "U1.GPIO4");

    assert_eq!(
        eval(&s, &[], &[]).warnings,
        vec![],
        "a divider is not a short: two rails through resistance are a \
             circuit, not a fault"
    );
    let mid = divider(&s, "U1.GPIO4").expect("the sheet says enough");
    assert!(
        (mid.fraction - 1.0 / 3.0).abs() < 1e-9,
        "10k of 30k: {}",
        mid.fraction
    );
    assert_eq!((mid.to_supply, mid.to_ground), (20_000.0, 10_000.0));

    // The rails themselves are the ends of the scale.
    assert_eq!(divider(&s, "U1.3V3").map(|d| d.fraction), Some(1.0));
    assert_eq!(divider(&s, "U1.GND").map(|d| d.fraction), Some(0.0));

    // A pull-up alone: no current flows, so the pin rests at the rail
    // whatever the resistor is.
    let mut up = sheet();
    place_with(&mut up, "R1", "Device:R", "10k");
    wire(&mut up, "R1.1", "U1.3V3");
    wire(&mut up, "R1.2", "U1.GPIO4");
    assert_eq!(divider(&up, "U1.GPIO4").map(|d| d.fraction), Some(1.0));
    // But with no value written it says nothing rather than 1.0: one
    // rule for an unmeasurable path, whichever side it is on, because
    // the version that special-cased this is the version that put an
    // unvalued divider's midpoint flat on ground.
    let mut vague = sheet();
    place(&mut vague, "R1", "Device:R");
    wire(&mut vague, "R1.1", "U1.3V3");
    wire(&mut vague, "R1.2", "U1.GPIO4");
    assert_eq!(divider(&vague, "U1.GPIO4"), None);

    // Two resistors to the same rail are parallel, which is exact.
    let mut par = sheet();
    place_with(&mut par, "R1", "Device:R", "10k");
    place_with(&mut par, "R2", "Device:R", "10k");
    place_with(&mut par, "R3", "Device:R", "5k");
    for (r, rail) in [("R1", "U1.3V3"), ("R2", "U1.3V3"), ("R3", "U1.GND")] {
        wire(&mut par, &format!("{r}.1"), rail);
        wire(&mut par, &format!("{r}.2"), "U1.GPIO4");
    }
    let both = divider(&par, "U1.GPIO4").expect("all three readable");
    assert_eq!(both.to_supply, 5000.0, "10k ∥ 10k");
    assert!((both.fraction - 0.5).abs() < 1e-9);

    // And the refusals, which matter more than the arithmetic.
    let mut blank = sheet();
    place(&mut blank, "R1", "Device:R");
    place_with(&mut blank, "R2", "Device:R", "10k");
    wire(&mut blank, "R1.1", "U1.3V3");
    wire(&mut blank, "R1.2", "R2.1");
    wire(&mut blank, "R2.2", "U1.GND");
    wire(&mut blank, "R1.2", "U1.GPIO4");
    assert_eq!(
        divider(&blank, "U1.GPIO4"),
        None,
        "one unreadable value makes the ratio a guess"
    );

    let mut lonely = sheet();
    place(&mut lonely, "D1", "Device:LED");
    wire(&mut lonely, "D1.A", "U1.GPIO4");
    assert_eq!(
        divider(&lonely, "U1.GPIO4"),
        None,
        "a pin that reaches no rail sits nowhere in particular"
    );
}

/// The potentiometer reaching the converter at last — and refusing
/// where the sheet has not said what its ends are on, which is the half
/// that kept it console-only until now.
#[test]
fn a_pot_across_the_rails_reads_as_counts_and_one_that_is_not_refuses() {
    let mut s = sheet();
    place(&mut s, "RV1", "rusty:Pot");
    wire(&mut s, "RV1.1", "U1.GND");
    wire(&mut s, "RV1.3", "U1.3V3");
    wire(&mut s, "RV1.W", "U1.GPIO4");
    let span = pot_span(&s, &rows(), "RV1").expect("both ends are on rails");
    assert_eq!((span.gpio, span.at_zero, span.at_full), (4, 0.0, 1.0));
    assert_eq!(span.counts(0, 4095), 0);
    assert_eq!(span.counts(255, 4095), 4095);
    assert_eq!(span.counts(128, 4095), 2056, "the wiper in the middle");

    // Wired the other way round it counts the other way: the sheet
    // says which end is which, and swapping the wires is the fix.
    let mut back = sheet();
    place(&mut back, "RV1", "rusty:Pot");
    wire(&mut back, "RV1.1", "U1.3V3");
    wire(&mut back, "RV1.3", "U1.GND");
    wire(&mut back, "RV1.W", "U1.GPIO4");
    let span = pot_span(&back, &rows(), "RV1").expect("still on rails");
    assert_eq!(span.counts(0, 4095), 4095);
    assert_eq!(span.counts(255, 4095), 0);

    // An end reaching nothing: no span, and no invented one.
    let mut loose = sheet();
    place(&mut loose, "RV1", "rusty:Pot");
    wire(&mut loose, "RV1.1", "U1.GND");
    wire(&mut loose, "RV1.W", "U1.GPIO4");
    assert_eq!(pot_span(&loose, &rows(), "RV1"), None);

    // An end behind a resistor: a divider with the pot's own track,
    // whose resistance the sheet never states.
    let mut through = sheet();
    place(&mut through, "RV1", "rusty:Pot");
    place_with(&mut through, "R1", "Device:R", "10k");
    wire(&mut through, "RV1.1", "U1.GND");
    wire(&mut through, "RV1.3", "R1.1");
    wire(&mut through, "R1.2", "U1.3V3");
    wire(&mut through, "RV1.W", "U1.GPIO4");
    assert_eq!(pot_span(&through, &rows(), "RV1"), None);
}

#[test]
fn rails_shorted_and_gpios_fighting_are_named_and_a_dangling_wire_too() {
    // A wire, not a resistor: a short is a connection with nothing in
    // it. The same two rails through a resistor is a load — and, with
    // its midpoint tapped, the commonest analog circuit there is.
    let mut s = sheet();
    wire(&mut s, "U1.GND", "U1.3V3");
    let e = eval(&s, &[], &[]);
    assert!(
        matches!(&e.warnings[..], [Warning::Short { pins }] if pins.len() == 2),
        "{:?}",
        e.warnings
    );

    let mut s = sheet();
    wire(&mut s, "U1.GPIO2", "U1.GPIO3");
    let e = eval(&s, &[(2, true), (3, false)], &[]);
    assert!(
        matches!(&e.warnings[..], [Warning::Conflict { .. }]),
        "{:?}",
        e.warnings
    );
    assert!(eval(&s, &[(2, true), (3, true)], &[]).warnings.is_empty());

    let mut s = sheet();
    wire(&mut s, "U1.GPIO2", "D9.A");
    let e = eval(&s, &[], &[]);
    assert_eq!(
        e.warnings,
        vec![Warning::DanglingWire {
            from: "U1.GPIO2".into(),
            to: "D9.A".into()
        }]
    );
}

/// A rail drawn on the sheet is the rail: a lamp whose cathode goes to
/// a ground symbol lights exactly as one wired all the way back to the
/// devkit's own GND pin does. That is what the symbol is for — a wire
/// across the whole sheet is what everybody draws it to avoid.
#[test]
fn a_power_symbol_is_a_rail_wherever_it_is_drawn() {
    let mut s = sheet();
    place(&mut s, "R1", "Device:R");
    place(&mut s, "D1", "Device:LED");
    place_with(&mut s, "GND1", "rusty:GND", "GND");
    wire(&mut s, "U1.GPIO2", "R1.1");
    wire(&mut s, "R1.2", "D1.A");
    wire(&mut s, "D1.K", "GND1.GND");

    let on = eval(&s, &[(2, true)], &[]);
    assert!(on.is_lit("D1"), "the ground symbol grounds the cathode");
    assert_eq!(on.level("D1", "K"), Some(false));
    assert!(on.warnings.is_empty(), "{:?}", on.warnings);
    assert!(!eval(&s, &[(2, false)], &[]).is_lit("D1"));

    // And the other way up: the anode on a supply symbol, the GPIO
    // sinking the cathode — a devkit's onboard lamp, drawn in full.
    let mut s = sheet();
    place(&mut s, "D2", "Device:LED");
    place_with(&mut s, "PWR1", "rusty:Supply", "3V3");
    wire(&mut s, "PWR1.VCC", "D2.A");
    wire(&mut s, "D2.K", "U1.GPIO8");
    assert!(eval(&s, &[(8, false)], &[]).is_lit("D2"));
    assert!(!eval(&s, &[(8, true)], &[]).is_lit("D2"));

    // A supply and a ground on one net is the short it always was,
    // whether the rails come from the devkit or from symbols.
    let mut s = sheet();
    place_with(&mut s, "GND2", "rusty:GND", "GND");
    place_with(&mut s, "PWR2", "rusty:Supply", "5V");
    wire(&mut s, "GND2.GND", "PWR2.VCC");
    assert!(
        matches!(&eval(&s, &[], &[]).warnings[..], [Warning::Short { .. }]),
        "{:?}",
        eval(&s, &[], &[]).warnings
    );
}

/// Two labels carrying the same name are one net, however far apart
/// they are drawn — and two different names are two nets, which is the
/// half that makes the first mean anything.
#[test]
fn labels_of_the_same_name_are_one_net_and_other_names_are_not() {
    let mut s = sheet();
    place(&mut s, "R1", "Device:R");
    place(&mut s, "D1", "Device:LED");
    place_with(&mut s, "L1", "rusty:Label", "SIG");
    place_with(&mut s, "L2", "rusty:Label", "SIG");
    wire(&mut s, "U1.GPIO2", "L1.1");
    wire(&mut s, "L2.1", "R1.1");
    wire(&mut s, "R1.2", "D1.A");
    wire(&mut s, "D1.K", "U1.GND");

    let on = eval(&s, &[(2, true)], &[]);
    assert!(on.is_lit("D1"), "the name carries the pin across the sheet");
    assert_eq!(
        on.net_of(&PinRef::new("L1", "1")),
        on.net_of(&PinRef::new("L2", "1"))
    );

    // Rename one of them and the two halves are two nets again.
    if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "L2") {
        part.value = "OTHER".to_string();
    }
    let split = eval(&s, &[(2, true)], &[]);
    assert!(!split.is_lit("D1"), "a different name is a different net");
    assert_ne!(
        split.net_of(&PinRef::new("L1", "1")),
        split.net_of(&PinRef::new("L2", "1"))
    );

    // A label with no name joins nothing — an empty tag is a tag
    // somebody has not written on yet, not a net called "".
    for part in s.parts.iter_mut() {
        if part.reference.starts_with('L') {
            part.value = String::new();
        }
    }
    assert!(!eval(&s, &[(2, true)], &[]).is_lit("D1"));
}

/// The nets a probe reads: every pin the current joins is one number,
/// and what is not joined is not.
/// A buzzer is on by the lamp's rule and off by it, and — unlike a
/// lamp — wants no series resistor, so the sheet does not ask for one.
#[test]
fn a_buzzer_sounds_the_way_a_lamp_lights_and_needs_no_resistor() {
    let mut s = sheet();
    place(&mut s, "BZ1", "rusty:Buzzer");
    wire(&mut s, "U1.GPIO4", "BZ1.+");
    wire(&mut s, "BZ1.-", "U1.GND");

    let on = eval(&s, &[(4, true)], &[]);
    assert!(on.is_lit("BZ1"), "driven the right way round, it sounds");
    assert!(
        on.warnings.is_empty(),
        "a buzzer is not a lamp missing its resistor: {:?}",
        on.warnings
    );
    assert!(!eval(&s, &[(4, false)], &[]).is_lit("BZ1"));

    // The other way round it is silent, which is the finding.
    let mut s = sheet();
    place(&mut s, "BZ2", "rusty:Buzzer");
    wire(&mut s, "U1.GPIO4", "BZ2.-");
    wire(&mut s, "BZ2.+", "U1.GND");
    assert!(!eval(&s, &[(4, true)], &[]).is_lit("BZ2"));
}

#[test]
fn a_net_holds_every_pin_the_current_reaches() {
    let mut s = sheet();
    place(&mut s, "R1", "Device:R");
    place(&mut s, "D1", "Device:LED");
    place(&mut s, "C1", "Device:C");
    wire(&mut s, "U1.GPIO2", "R1.1");
    wire(&mut s, "R1.2", "D1.A");
    wire(&mut s, "D1.K", "U1.GND");
    wire(&mut s, "C1.1", "D1.A");

    let e = eval(&s, &[(2, true)], &[]);
    let net = e
        .net_of(&PinRef::new("D1", "A"))
        .expect("the anode is in a net");
    let members: Vec<String> = e.members(net).iter().map(PinRef::to_string).collect();
    // Through the resistor, and through the wire to the capacitor's
    // near plate — but never through the capacitor itself.
    for wanted in ["D1.A", "R1.2", "R1.1", "C1.1", "U1.GPIO2"] {
        assert!(
            members.contains(&wanted.to_string()),
            "{wanted} in {members:?}"
        );
    }
    assert!(
        !members.contains(&"C1.2".to_string()),
        "a capacitor does not join its plates: {members:?}"
    );
    assert_ne!(
        e.net_of(&PinRef::new("D1", "K")),
        Some(net),
        "the cathode is the other side of the lamp"
    );
}

#[test]
fn an_rgb_lens_lights_each_channel_against_its_common_and_a_pot_knows_its_gpio() {
    let mut s = sheet();
    place(&mut s, "D1", "rusty:RGB_LED");
    wire(&mut s, "U1.GPIO2", "D1.R");
    wire(&mut s, "U1.GPIO3", "D1.G");
    wire(&mut s, "D1.COM", "U1.3V3");
    let e = eval(&s, &[(2, false), (3, true)], &[]);
    assert!(
        e.is_pin_lit("D1", "1"),
        "common anode: a channel pulled low lights"
    );
    assert!(!e.is_pin_lit("D1", "2"));
    assert!(!e.is_pin_lit("D1", "3"), "unwired channel: dark");
    assert!(e.is_lit("D1"));

    let mut s = sheet();
    place(&mut s, "RV1", "rusty:Pot");
    place(&mut s, "R1", "Device:R");
    wire(&mut s, "RV1.W", "R1.1");
    wire(&mut s, "R1.2", "U1.GPIO4");
    let rows = rows();
    assert_eq!(
        gpio_of(&s, &rows, "RV1", "W"),
        Some(4),
        "through the resistor"
    );
    assert_eq!(gpio_of(&s, &rows, "RV1", "1"), None);
}

/// A part reaches the bus by carrying an address and by being wired to
/// one, and by nothing else. Its kind does not decide: a sensor, a
/// display and a breakout imported from LCSC all get there the same way.
#[test]
fn a_part_with_an_address_and_wires_is_a_device_on_the_bus() {
    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "68"), ("regs", "75=68,3b=0102")]);
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].address, 0x68);
    assert_eq!(
        devices[0].regs,
        vec![(0x75, vec![0x68]), (0x3b, vec![0x01, 0x02])]
    );

    // 0x prefixed, and a device that answers zeros because nobody reads
    // from it — a display.
    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "0x3C")]);
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(devices[0].address, 0x3c);
    assert!(devices[0].regs.is_empty());
}

/// A sensor rusty answers for brings its own registers, and what the
/// sheet spells out lands over them. A model rusty does not know is
/// named, and the part keeps what its registers say.
#[test]
fn a_sensor_model_brings_its_registers_onto_the_bus() {
    let mut s = sheet();
    place_on_bus(
        &mut s,
        "U2",
        &[("addr", "68"), ("model", "MPU-6050"), ("regs", "75=70")],
    );
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert!(warnings.is_empty(), "{warnings:?}");
    let regs = &devices[0].regs;
    let who_am_i: Vec<u8> = regs
        .iter()
        .filter(|(at, _)| *at == 0x75)
        .map(|(_, bytes)| bytes[0])
        .collect();
    assert_eq!(who_am_i, vec![0x68, 0x70], "the model's, then the sheet's");
    assert!(
        regs.iter()
            .any(|(at, bytes)| *at == 0x3b && bytes.len() == 14)
    );

    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "68"), ("model", "mpu9250")]);
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert_eq!(devices.len(), 1, "still on the bus");
    assert!(devices[0].regs.is_empty());
    assert!(
        matches!(&warnings[..], [Warning::SensorModelUnknown { part, value }]
                     if part == "U2" && value == "mpu9250"),
        "{warnings:?}"
    );
}

/// No address is not an address of zero. A part without one is the part
/// it always was and the bus never hears of it.
#[test]
fn a_part_without_an_address_is_not_on_the_bus() {
    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[]);
    assert_eq!(bus_devices(&s, &rows(), &specs()).0.len(), 0);

    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "  ")]);
    assert_eq!(
        bus_devices(&s, &rows(), &specs()).0.len(),
        0,
        "blank is absent"
    );
}

/// Each refusal names the part, because the alternative is a device that
/// silently is not there. The wiring one is the load-bearing case: the
/// emulator's bus does not route through the GPIO matrix, so an
/// unwired device would answer in the simulator and be dead on the desk.
#[test]
fn a_bus_device_that_cannot_be_read_is_named_rather_than_dropped() {
    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "zz")]);
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert!(devices.is_empty());
    assert!(
        matches!(&warnings[..], [Warning::BusAddressUnreadable { part, value }]
                     if part == "U2" && value == "zz"),
        "{warnings:?}"
    );

    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "90")]);
    assert!(
        matches!(
            &bus_devices(&s, &rows(), &specs()).1[..],
            [Warning::BusAddressUnreadable { .. }]
        ),
        "0x90 is an eight-bit address written where a seven-bit one goes"
    );

    let mut s = sheet();
    place_on_bus(&mut s, "U2", &[("addr", "68"), ("regs", "75=6")]);
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert_eq!(devices.len(), 1, "it is still on the bus");
    assert!(devices[0].regs.is_empty(), "and answers zeros");
    assert!(
        matches!(&warnings[..], [Warning::BusRegistersUnreadable { .. }]),
        "half a byte is not a byte: {warnings:?}"
    );

    // Wired to nothing at all.
    let mut s = sheet();
    place(&mut s, "U2", "rusty:Sensor");
    if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "U2") {
        part.props.insert("addr".into(), "68".into());
    }
    let (devices, warnings) = bus_devices(&s, &rows(), &specs());
    assert!(devices.is_empty());
    assert!(
        matches!(&warnings[..], [Warning::BusNotWired { part }] if part == "U2"),
        "{warnings:?}"
    );

    // And wired on one side only, which is the mistake somebody actually
    // makes.
    let mut s = sheet();
    place(&mut s, "U2", "rusty:Sensor");
    wire(&mut s, "U2.1", "U1.GPIO5");
    if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "U2") {
        part.props.insert("addr".into(), "68".into());
    }
    assert!(
        matches!(
            &bus_devices(&s, &rows(), &specs()).1[..],
            [Warning::BusNotWired { .. }]
        ),
        "SDA alone is not a bus"
    );
}

/// The SPI half, by the same two rules: a `cs` prop puts a part on the
/// wire, and its clock and data have to reach GPIOs.
#[test]
fn a_part_with_a_chip_select_and_wires_is_a_device_on_the_wire() {
    let mut s = sheet();
    s.symbols.push(symbol(
        "Device",
        "Display",
        "U",
        &[("1", "SCK"), ("2", "MOSI"), ("3", "MISO"), ("4", "CS")],
    ));
    let place_wired = |s: &mut Sheet, props: &[(&str, &str)]| {
        place(s, "U2", "Device:Display");
        wire(s, "U2.1", "U1.GPIO6");
        wire(s, "U2.2", "U1.GPIO7");
        if let Some(part) = s.parts.iter_mut().find(|p| p.reference == "U2") {
            for (key, value) in props {
                part.props.insert((*key).into(), (*value).into());
            }
        }
    };

    let mut wired = s.clone();
    place_wired(&mut wired, &[("cs", "0"), ("miso", "1a68")]);
    let (devices, warnings) = wire_devices(&wired, &rows());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(devices[0].select, 0);
    assert_eq!(devices[0].miso, vec![0x1a, 0x68]);

    // A display: on a select, answering nothing.
    let mut quiet = s.clone();
    place_wired(&mut quiet, &[("cs", "1")]);
    let (devices, warnings) = wire_devices(&quiet, &rows());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(devices[0].select, 1);
    assert!(devices[0].miso.is_empty());

    // No `cs` is not chip select zero.
    let mut none = s.clone();
    place_wired(&mut none, &[("miso", "1a68")]);
    assert!(wire_devices(&none, &rows()).0.is_empty());

    // A select the peripheral does not have, and half a byte.
    let mut wrong = s.clone();
    place_wired(&mut wrong, &[("cs", "9")]);
    assert!(matches!(
        &wire_devices(&wrong, &rows()).1[..],
        [Warning::WireSelectUnreadable { .. }]
    ));
    let mut odd = s.clone();
    place_wired(&mut odd, &[("cs", "0"), ("miso", "1a6")]);
    assert!(matches!(
        &wire_devices(&odd, &rows()).1[..],
        [Warning::WireSelectUnreadable { .. }]
    ));

    // And wired to nothing.
    let mut loose = s.clone();
    place(&mut loose, "U2", "Device:Display");
    if let Some(part) = loose.parts.iter_mut().find(|p| p.reference == "U2") {
        part.props.insert("cs".into(), "0".into());
    }
    assert!(matches!(
        &wire_devices(&loose, &rows()).1[..],
        [Warning::WireNotWired { .. }]
    ));
}
