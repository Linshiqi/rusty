//! What the lab and the sheet say about signals in the user's language: a
//! preset's name, why a line of text is not a signal, and why a filter or
//! a measurement was refused.
//!
//! The reader names what is wrong — a kind, a key, the number — rather than
//! putting it in an English sentence, and this is where the sentence is
//! made (`rusty_embed::signal::SignalError`).

use rusty_embed::dsp::Refusal;
use rusty_embed::signal::{self, Rule, SignalError};
use rusty_i18n::t;

/// A preset's name, by its id.
pub fn preset_name(id: &str) -> String {
    match id {
        "mains-hum" => t!("lab.preset-mains-hum"),
        "vibration" => t!("lab.preset-vibration"),
        "thermistor" => t!("lab.preset-thermistor"),
        "spikes" => t!("lab.preset-spikes"),
        "sweep" => t!("lab.preset-sweep"),
        "step" => t!("lab.preset-step"),
        "two-tones" => t!("lab.preset-two-tones"),
        other => other.to_string(),
    }
}

/// Why `text` is not a signal, said the way the field under it says it.
pub fn signal_error_text(error: &SignalError) -> String {
    let listed = |words: &[&str]| {
        words
            .iter()
            .map(|word| format!("`{word}`"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match error {
        SignalError::UnknownKind { word } => t!(
            "lab.error-unknown-kind",
            word = word.clone(),
            kinds = listed(&signal::KINDS)
        ),
        SignalError::UnknownKey { kind, key } => t!(
            "lab.error-unknown-key",
            kind = kind.to_string(),
            key = key.clone(),
            keys = listed(signal::keys(kind))
        ),
        SignalError::Stray { kind, word } => t!(
            "lab.error-stray",
            kind = kind.to_string(),
            word = word.clone()
        ),
        SignalError::Missing { kind, key } => t!(
            "lab.error-missing",
            kind = kind.to_string(),
            key = key.to_string()
        ),
        SignalError::Twice { kind, key } => t!(
            "lab.error-twice",
            kind = kind.to_string(),
            key = key.to_string()
        ),
        SignalError::NotANumber { kind, key, text } => t!(
            "lab.error-not-a-number",
            kind = kind.to_string(),
            key = key.to_string(),
            text = text.clone()
        ),
        SignalError::OutOfRange {
            kind,
            key,
            value,
            rule,
        } => t!(
            "lab.error-out-of-range",
            kind = kind.to_string(),
            key = key.to_string(),
            value = value.to_string(),
            rule = rule_text(*rule)
        ),
    }
}

/// Why `dsp` refused a measurement, a design or an export, in the words
/// the lab puts under the field it is about.
pub fn dsp_refusal_text(refusal: &Refusal) -> String {
    let number = |value: f64| format!("{value}");
    match refusal {
        Refusal::Rate { rate } => t!("lab.refusal-rate", rate = number(*rate)),
        Refusal::Frequency {
            what,
            freq,
            nyquist,
        } => t!(
            "lab.refusal-frequency",
            what = what_text(what),
            freq = number(*freq),
            nyquist = number(*nyquist)
        ),
        Refusal::Short { periods } => {
            t!("lab.refusal-short", periods = format!("{periods:.2}"))
        }
        Refusal::Taps { taps, least } => t!("lab.refusal-taps", taps = *taps, least = *least),
        Refusal::Order => t!("lab.refusal-order"),
        Refusal::Q { q } => t!("lab.refusal-q", q = number(*q)),
        Refusal::Alpha { alpha } => t!("lab.refusal-alpha", alpha = number(*alpha)),
        Refusal::Even { taps } => t!("lab.refusal-even", taps = *taps),
        Refusal::Edges { low, high } => {
            t!(
                "lab.refusal-edges",
                low = number(*low),
                high = number(*high)
            )
        }
        Refusal::Inexact { gain } => t!("lab.refusal-inexact", gain = format!("{gain:.6}")),
        Refusal::Name { name } => t!("lab.refusal-name", name = name.clone()),
        Refusal::Precision { error, limit } => t!(
            "lab.refusal-precision",
            error = format!("{:.3}%", error * 100.0),
            limit = format!("{:.3}%", limit * 100.0)
        ),
    }
}

/// Which frequency a refusal is about.
fn what_text(what: &str) -> String {
    match what {
        "tone" => t!("lab.what-tone"),
        "cutoff" => t!("lab.what-cutoff"),
        "low" => t!("lab.what-low"),
        "high" => t!("lab.what-high"),
        other => other.to_string(),
    }
}

fn rule_text(rule: Rule) -> String {
    match rule {
        Rule::Finite => t!("lab.rule-finite"),
        Rule::Positive => t!("lab.rule-positive"),
        Rule::NotNegative => t!("lab.rule-not-negative"),
        Rule::Fraction => t!("lab.rule-fraction"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_embed::signal::Signal;

    /// Every refusal names what it is about, in the words that were typed.
    #[test]
    fn a_refusal_names_what_it_is_about() {
        for (text, named) in [
            ("sin f=5 a=1", "sin"),
            ("sine fq=5 a=1", "fq"),
            ("sine f=5", "a"),
            ("sine f=5 f=6 a=1", "f"),
            ("sine f=five a=1", "five"),
            ("sine f=-5 a=1", "-5"),
        ] {
            let error = Signal::parse(text).unwrap_err();
            let said = signal_error_text(&error);
            assert!(said.contains(named), "{text}: {said}");
        }
    }

    /// A refusal names its numbers: the rate a cutoff has to stay under,
    /// the taps asked for and the fewest there can be.
    #[test]
    fn a_dsp_refusal_names_its_numbers() {
        let said = dsp_refusal_text(&Refusal::Frequency {
            what: "cutoff",
            freq: 600.0,
            nyquist: 500.0,
        });
        assert!(said.contains("600") && said.contains("500"), "{said}");
        let said = dsp_refusal_text(&Refusal::Taps { taps: 0, least: 1 });
        assert!(said.contains('0') && said.contains('1'), "{said}");
        let said = dsp_refusal_text(&Refusal::Name { name: "low".into() });
        assert!(said.contains("low"), "{said}");
    }

    /// Every preset has a name of its own, not its id.
    #[test]
    fn every_preset_has_a_name() {
        for preset in rusty_embed::signal::presets() {
            assert_ne!(preset_name(preset.id), preset.id, "{}", preset.id);
        }
    }
}
