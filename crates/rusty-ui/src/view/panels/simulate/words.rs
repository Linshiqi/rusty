//! What the sheet says, in the window's language: a finding, a reading, a
//! limit, a net's level, why there is no number.

use super::*;

/// A finding of the rules, in the interface's language.
pub(super) fn warning_text(warning: &Warning) -> String {
    match warning {
        Warning::LedWithoutResistor { part } => t!("simulate.warning-led-resistor", part = part),
        Warning::DanglingWire { from, to } => t!("simulate.warning-dangling", from = from, to = to),
        Warning::Short { pins } => t!("simulate.warning-short", pins = pins.join(", ")),
        Warning::Conflict { pins } => t!("simulate.warning-conflict", pins = pins.join(", ")),
        Warning::SwitchDrivesNothing { part } => t!("simulate.warning-switch", part = part),
        Warning::BusAddressUnreadable { part, value } => {
            t!("simulate.warning-bus-address", part = part, value = value)
        }
        Warning::BusRegistersUnreadable { part, value } => {
            t!("simulate.warning-bus-registers", part = part, value = value)
        }
        Warning::BusNotWired { part } => t!("simulate.warning-bus-wiring", part = part),
        Warning::SensorModelUnknown { part, value } => {
            t!("simulate.warning-sensor-model", part = part, value = value)
        }
        Warning::WireSelectUnreadable { part, value } => {
            t!("simulate.warning-wire-select", part = part, value = value)
        }
        Warning::WireNotWired { part } => t!("simulate.warning-wire-wiring", part = part),
        Warning::PinReachesNothing { part, pin } => {
            t!("simulate.warning-loose-pin", part = part, pin = pin)
        }
        Warning::OutputsFighting { pins } => {
            t!("simulate.warning-outputs", pins = pins.join(", "))
        }
    }
}

/// A register sensor's reading, named in the window's language.
pub(super) fn reading_label(key: &str) -> String {
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

/// A reading as the sheet stores it and the slider shows it: no more
/// digits than a slider two hundred steps long can set.
pub(super) fn reading_text(value: f64) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// What the emulator cannot do on this chip, in the window's language: by
/// the limit's stable name, and in the backend's English for one this
/// frontend has no words for yet.
pub(super) fn limit_text(limit: &rusty_embed::SimLimit) -> String {
    match limit.kind.as_str() {
        "esp32-outdated" => t!("simulate.limit-esp32-outdated"),
        "cpu-fpu-off" => t!("simulate.limit-cpu-fpu-off"),
        "s3-unproven" => t!("simulate.limit-s3-unproven"),
        "signals-outdated" => t!("simulate.limit-signals-outdated"),
        _ => limit.text.clone(),
    }
}

/// What a net is doing, in words: its level, or how much of every period it
/// is high while a PWM pin drives it.
pub(super) fn level_word(level: Option<period::Level>) -> String {
    match level {
        Some(period::Level::High) => t!("simulate.net-high"),
        Some(period::Level::Low) => t!("simulate.net-low"),
        Some(period::Level::Switching(high)) => {
            t!("simulate.net-pwm", share = format!("{:.0}", high * 100.0))
        }
        None => t!("simulate.net-floating"),
    }
}

/// Why the sheet has no numbers on it, in the window's own language.
///
/// The same rule `warning_text` follows: the variant's *name* is the stable
/// half and the values travel beside it, so a refusal reads as a sentence
/// here and prints as English in the CLI. `Display` on these types is the
/// English one and is what the headless tools use; a panel calling it would
/// be the one place in the window that answers in the wrong language.
pub(super) fn unsolved_text(why: &circuit::Unsolved) -> String {
    use circuit::{Unsolved, Unstated};
    use rusty_embed::solve::Trouble;
    match why {
        Unsolved::Unstated(Unstated::Resistance { part, value }) => {
            t!("simulate.unstated-resistance", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::Supply { part, value }) => {
            t!("simulate.unstated-supply", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::ForwardVoltage { part }) => {
            t!("simulate.unstated-vf", part = part)
        }
        Unsolved::Unstated(Unstated::Capacitance { part, value }) => {
            t!("simulate.unstated-capacitance", part = part, value = value)
        }
        Unsolved::Unstated(Unstated::NoGround) => t!("simulate.unstated-ground"),
        Unsolved::Floating { pins } => t!(
            "simulate.unsolved-floating",
            pins = pins
                .iter()
                .map(PinRef::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Unsolved::Trouble(Trouble::Contradiction) => t!("simulate.unsolved-contradiction"),
        Unsolved::Trouble(Trouble::BadResistance { ohms }) => {
            t!("simulate.unsolved-resistance", ohms = ohms.to_string())
        }
        Unsolved::Trouble(Trouble::DidNotConverge { .. }) => t!("simulate.unsolved-converge"),
        // Neither can reach a panel: `operating_point` catches `Floating`
        // and turns it into the pins it is about, and a step length belongs
        // to a transient, which this memo never runs. They fall back to the
        // English the type itself writes rather than borrowing a key about
        // something else — no entry, no claim, which is the same rule the
        // backend's own text follows. Spelled out rather than left to a
        // catch-all so that a variant added later is a compile error here.
        Unsolved::Trouble(Trouble::Floating { .. } | Trouble::BadStep { .. }) => why.to_string(),
    }
}
