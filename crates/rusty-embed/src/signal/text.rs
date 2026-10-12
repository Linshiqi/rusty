//! A signal as a line of text: `dc 1.2; sine f=50 a=0.1; white rms=0.005`.
//!
//! The form a part's properties and `.rusty/sim.toml` keep, and the one a
//! person edits by hand. Components are separated by `;`, each is its kind
//! and then `key=value` pairs in any order — `dc` alone takes its level bare,
//! and `chirp` takes the word `log` — and every number is plain: the key
//! says the unit (hertz, seconds, degrees), so `f=50Hz` is not a number.
//!
//! **Read strictly.** An unknown kind, an unknown key, a key missing or
//! given twice, and a number that does not read or that its key cannot
//! take are each an error naming it. A reader that skipped what it did not
//! understand would feed the firmware a signal with a component quietly
//! missing — the confident wrong answer in miniature.
//!
//! **Written canonically.** [`Signal`]'s `Display` is this form, each number
//! the shortest that reads back to the same bits, and a key that holds its
//! default — a sine's `ph=0`, a square's `duty=0.5` — left out, so what a
//! person wrote comes back as they would have written it.

use std::fmt;
use std::str::FromStr;

use super::{Component, Signal};

/// Every kind the text form knows, in the order a message lists them —
/// and the words the frontend offers and translates, so a kind added here
/// is a kind it can name.
pub const KINDS: [&str; 10] = [
    "dc", "sine", "square", "triangle", "sawtooth", "chirp", "white", "pink", "spikes", "step",
];

/// The keys `kind` takes, in the order they are written; nothing for a
/// word that is not a kind. `dc` has one, `level`, which the text writes
/// bare — `dc 1.2` — and `chirp` takes the word `log` besides.
pub fn keys(kind: &str) -> &'static [&'static str] {
    match kind {
        "dc" => &["level"],
        "sine" | "triangle" | "sawtooth" => &["f", "a", "ph"],
        "square" => &["f", "a", "duty", "ph"],
        "chirp" => &["from", "to", "t", "a"],
        "white" | "pink" => &["rms"],
        "spikes" => &["rate", "a", "w"],
        "step" => &["at", "size"],
        _ => &[],
    }
}

/// What a key means, for a message that says it is missing.
fn meaning(kind: &str, key: &str) -> &'static str {
    match (kind, key) {
        (_, "level") => "the level",
        (_, "f") => "the frequency in Hz",
        (_, "a") => "the amplitude",
        (_, "ph") => "the phase in degrees",
        (_, "duty") => "the share of each period spent high",
        (_, "from") => "the frequency a sweep starts at, in Hz",
        (_, "to") => "the frequency a sweep ends at, in Hz",
        (_, "t") => "the seconds one sweep takes",
        (_, "rms") => "the RMS",
        ("spikes", "rate") => "how many a second on average",
        (_, "w") => "each one's width in seconds",
        (_, "at") => "the time it steps at, in seconds",
        (_, "size") => "how far it steps",
        _ => "",
    }
}

/// What a key's number has to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// A number at all: not infinite, not NaN.
    Finite,
    /// Above zero.
    Positive,
    /// Zero or above.
    NotNegative,
    /// From 0 to 1.
    Fraction,
}

impl Rule {
    fn allows(self, value: f64) -> bool {
        match self {
            Rule::Finite => value.is_finite(),
            Rule::Positive => value.is_finite() && value > 0.0,
            Rule::NotNegative => value.is_finite() && value >= 0.0,
            Rule::Fraction => (0.0..=1.0).contains(&value),
        }
    }
}

/// Why a line of text is not a signal, or a signal is not one that can be
/// rendered.
///
/// A kind and its key are named rather than put into a sentence, so the
/// frontend can say it in the user's language; `Display` is the English.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalError {
    /// A component whose first word is no kind of signal.
    UnknownKind { word: String },
    /// A key the kind does not take.
    UnknownKey { kind: &'static str, key: String },
    /// A word that is not `key=value`, nor a word the kind takes alone.
    Stray { kind: &'static str, word: String },
    /// A key the kind cannot do without.
    Missing {
        kind: &'static str,
        key: &'static str,
    },
    /// One key given twice in a component.
    Twice {
        kind: &'static str,
        key: &'static str,
    },
    /// A value that does not read as a number.
    NotANumber {
        kind: &'static str,
        key: &'static str,
        text: String,
    },
    /// A number its key cannot take.
    OutOfRange {
        kind: &'static str,
        key: &'static str,
        value: f64,
        rule: Rule,
    },
}

impl fmt::Display for SignalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalError::UnknownKind { word } => write!(
                f,
                "\"{word}\" is not a kind of signal: it is one of {}",
                list(&KINDS, "or")
            ),
            SignalError::UnknownKey { kind, key } => {
                let takes: Vec<String> = keys(kind).iter().map(|k| format!("{k}=")).collect();
                let takes: Vec<&str> = takes.iter().map(String::as_str).collect();
                write!(f, "{kind} has no {key}=: it takes {}", list(&takes, "and"))
            }
            SignalError::Stray { kind, word } => {
                write!(f, "{kind}: \"{word}\" is not key=value")
            }
            SignalError::Missing { kind: "dc", .. } => {
                f.write_str("dc needs its level after it, as in dc 1.2")
            }
            SignalError::Missing { kind, key } => {
                write!(f, "{kind} needs {key}=, {}", meaning(kind, key))
            }
            SignalError::Twice { kind, key } => write!(f, "{kind} has {key}= twice"),
            SignalError::NotANumber {
                kind: "dc", text, ..
            } => write!(f, "dc: \"{text}\" is not a number"),
            SignalError::NotANumber { kind, key, text } => {
                write!(f, "{kind}: {key}={text} is not a number")
            }
            SignalError::OutOfRange {
                kind,
                key,
                value,
                rule,
            } => {
                let should = match rule {
                    Rule::Finite => "has to be a finite number",
                    Rule::Positive => "has to be above zero",
                    Rule::NotNegative => "cannot be below zero",
                    Rule::Fraction => "has to be from 0 to 1",
                };
                if *kind == "dc" {
                    write!(f, "dc {value} {should}")
                } else {
                    write!(f, "{kind}: {key}={value} {should}")
                }
            }
        }
    }
}

impl std::error::Error for SignalError {}

/// `a, b, c or d`.
fn list(words: &[&str], last: &str) -> String {
    match words {
        [] => String::new(),
        [only] => (*only).to_string(),
        [init @ .., end] => format!("{} {last} {end}", init.join(", ")),
    }
}

impl Signal {
    /// Read the text form, strictly: see the module's head for what it
    /// refuses. Blank components — a `;` left at the end — are nothing, and
    /// an empty line is silence.
    pub fn parse(text: &str) -> Result<Signal, SignalError> {
        let mut components = Vec::new();
        for part in text.split(';') {
            let mut words = part.split_whitespace();
            if let Some(kind) = words.next() {
                components.push(component(kind, words)?);
            }
        }
        let signal = Signal { components };
        signal.check()?;
        Ok(signal)
    }

    /// Whether every number is one its key can take — what the reader asks
    /// of text, for a signal built some other way: a form, a slider, a
    /// preset somebody changed.
    pub fn check(&self) -> Result<(), SignalError> {
        self.components.iter().try_for_each(Component::check)
    }
}

impl FromStr for Signal {
    type Err = SignalError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Signal::parse(text)
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, component) in self.components.iter().enumerate() {
            if at > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{component}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `{}` on an f64 is the shortest decimal that reads back to the
        // same bits, so the round trip is exact without printing seventeen
        // digits of every number a person typed as `0.1`.
        let kind = self.kind();
        match *self {
            Component::Dc { level } => write!(f, "dc {level}"),
            Component::Sine {
                freq,
                amplitude,
                phase,
            }
            | Component::Triangle {
                freq,
                amplitude,
                phase,
            }
            | Component::Sawtooth {
                freq,
                amplitude,
                phase,
            } => {
                write!(f, "{kind} f={freq} a={amplitude}")?;
                if phase != 0.0 {
                    write!(f, " ph={phase}")?;
                }
                Ok(())
            }
            Component::Square {
                freq,
                amplitude,
                duty,
                phase,
            } => {
                write!(f, "square f={freq} a={amplitude}")?;
                if duty != 0.5 {
                    write!(f, " duty={duty}")?;
                }
                if phase != 0.0 {
                    write!(f, " ph={phase}")?;
                }
                Ok(())
            }
            Component::Chirp {
                from,
                to,
                period,
                amplitude,
                log,
            } => {
                write!(f, "chirp from={from} to={to} t={period} a={amplitude}")?;
                if log {
                    f.write_str(" log")?;
                }
                Ok(())
            }
            Component::White { rms } | Component::Pink { rms } => write!(f, "{kind} rms={rms}"),
            Component::Spikes {
                rate,
                amplitude,
                width,
            } => write!(f, "spikes rate={rate} a={amplitude} w={width}"),
            Component::Step { at, size } => write!(f, "step at={at} size={size}"),
        }
    }
}

/// One component's words, after its kind.
fn component<'a>(
    word: &str,
    words: impl Iterator<Item = &'a str>,
) -> Result<Component, SignalError> {
    let Some(&kind) = KINDS.iter().find(|&&kind| kind == word) else {
        return Err(SignalError::UnknownKind {
            word: word.to_string(),
        });
    };
    if kind == "dc" {
        // The one kind with one number, which is written bare.
        let mut words = words;
        let level = words
            .next()
            .ok_or(SignalError::Missing { kind, key: "level" })?;
        if let Some(stray) = words.next() {
            return Err(SignalError::Stray {
                kind,
                word: stray.to_string(),
            });
        }
        return Ok(Component::Dc {
            level: number(kind, "level", level)?,
        });
    }

    let fields = Fields::read(kind, words)?;
    Ok(match kind {
        "sine" => Component::Sine {
            freq: fields.number("f")?,
            amplitude: fields.number("a")?,
            phase: fields.or("ph", 0.0)?,
        },
        "square" => Component::Square {
            freq: fields.number("f")?,
            amplitude: fields.number("a")?,
            duty: fields.or("duty", 0.5)?,
            phase: fields.or("ph", 0.0)?,
        },
        "triangle" => Component::Triangle {
            freq: fields.number("f")?,
            amplitude: fields.number("a")?,
            phase: fields.or("ph", 0.0)?,
        },
        "sawtooth" => Component::Sawtooth {
            freq: fields.number("f")?,
            amplitude: fields.number("a")?,
            phase: fields.or("ph", 0.0)?,
        },
        "chirp" => Component::Chirp {
            from: fields.number("from")?,
            to: fields.number("to")?,
            period: fields.number("t")?,
            amplitude: fields.number("a")?,
            log: fields.log,
        },
        "white" => Component::White {
            rms: fields.number("rms")?,
        },
        "pink" => Component::Pink {
            rms: fields.number("rms")?,
        },
        "spikes" => Component::Spikes {
            rate: fields.number("rate")?,
            amplitude: fields.number("a")?,
            width: fields.number("w")?,
        },
        "step" => Component::Step {
            at: fields.number("at")?,
            size: fields.number("size")?,
        },
        _ => unreachable!("every kind in KINDS is matched"),
    })
}

/// A number, or an error naming whose number it was not.
fn number(kind: &'static str, key: &'static str, text: &str) -> Result<f64, SignalError> {
    text.parse().map_err(|_| SignalError::NotANumber {
        kind,
        key,
        text: text.to_string(),
    })
}

/// A component's `key=value` pairs, checked against the keys its kind takes
/// before any is read, so a misspelt key is named as one rather than
/// reported as the key it was meant to be being missing.
struct Fields<'a> {
    kind: &'static str,
    pairs: Vec<(&'static str, &'a str)>,
    /// The chirp's one bare word.
    log: bool,
}

impl<'a> Fields<'a> {
    fn read(kind: &'static str, words: impl Iterator<Item = &'a str>) -> Result<Self, SignalError> {
        let mut fields = Fields {
            kind,
            pairs: Vec::new(),
            log: false,
        };
        for word in words {
            let Some((key, value)) = word.split_once('=') else {
                if kind == "chirp" && word == "log" && !fields.log {
                    fields.log = true;
                    continue;
                }
                return Err(SignalError::Stray {
                    kind,
                    word: word.to_string(),
                });
            };
            let Some(&key) = keys(kind).iter().find(|&&known| known == key) else {
                return Err(SignalError::UnknownKey {
                    kind,
                    key: key.to_string(),
                });
            };
            if fields.pairs.iter().any(|&(seen, _)| seen == key) {
                return Err(SignalError::Twice { kind, key });
            }
            fields.pairs.push((key, value));
        }
        Ok(fields)
    }

    fn value(&self, key: &'static str) -> Option<&'a str> {
        self.pairs
            .iter()
            .find(|&&(seen, _)| seen == key)
            .map(|&(_, value)| value)
    }

    fn number(&self, key: &'static str) -> Result<f64, SignalError> {
        let kind = self.kind;
        let text = self.value(key).ok_or(SignalError::Missing { kind, key })?;
        number(kind, key, text)
    }

    fn or(&self, key: &'static str, default: f64) -> Result<f64, SignalError> {
        match self.value(key) {
            Some(text) => number(self.kind, key, text),
            None => Ok(default),
        }
    }
}

impl Component {
    /// Every number against what its key can take, in the order the text
    /// writes them.
    fn check(&self) -> Result<(), SignalError> {
        let kind = self.kind();
        let rules: Vec<(&'static str, f64, Rule)> = match *self {
            Component::Dc { level } => vec![("level", level, Rule::Finite)],
            Component::Sine {
                freq,
                amplitude,
                phase,
            }
            | Component::Triangle {
                freq,
                amplitude,
                phase,
            }
            | Component::Sawtooth {
                freq,
                amplitude,
                phase,
            } => vec![
                ("f", freq, Rule::Positive),
                ("a", amplitude, Rule::Finite),
                ("ph", phase, Rule::Finite),
            ],
            Component::Square {
                freq,
                amplitude,
                duty,
                phase,
            } => vec![
                ("f", freq, Rule::Positive),
                ("a", amplitude, Rule::Finite),
                ("duty", duty, Rule::Fraction),
                ("ph", phase, Rule::Finite),
            ],
            Component::Chirp {
                from,
                to,
                period,
                amplitude,
                log,
            } => {
                // A sweep by ratios has to start and end somewhere a ratio
                // can be taken from; a straight one may start at DC.
                let end = if log {
                    Rule::Positive
                } else {
                    Rule::NotNegative
                };
                vec![
                    ("from", from, end),
                    ("to", to, end),
                    ("t", period, Rule::Positive),
                    ("a", amplitude, Rule::Finite),
                ]
            }
            Component::White { rms } | Component::Pink { rms } => {
                vec![("rms", rms, Rule::NotNegative)]
            }
            Component::Spikes {
                rate,
                amplitude,
                width,
            } => vec![
                ("rate", rate, Rule::NotNegative),
                ("a", amplitude, Rule::Finite),
                ("w", width, Rule::Positive),
            ],
            Component::Step { at, size } => {
                vec![("at", at, Rule::NotNegative), ("size", size, Rule::Finite)]
            }
        };
        match rules
            .into_iter()
            .find(|&(_, value, rule)| !rule.allows(value))
        {
            Some((key, value, rule)) => Err(SignalError::OutOfRange {
                kind,
                key,
                value,
                rule,
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind, and every field set to something that is not its
    /// default — `log` on, `duty` off a half, a phase on every waveform — so
    /// a writer that dropped any one field and a reader that defaulted it
    /// could not agree by accident.
    const EVERYTHING: &str = "dc -0.25; sine f=50.5 a=0.1 ph=30; \
        square f=10 a=1.5 duty=0.25 ph=90; triangle f=3 a=2 ph=45; \
        sawtooth f=7 a=0.5 ph=-60; chirp from=1 to=200 t=10 a=0.5 log; \
        white rms=0.005; pink rms=0.02; spikes rate=2 a=0.5 w=0.001; \
        step at=0.5 size=1";

    #[test]
    fn every_field_survives_the_round_trip() {
        let signal = Signal::parse(EVERYTHING).expect("reads");
        assert_eq!(signal.components.len(), 10);
        assert_eq!(
            signal.components[2],
            Component::Square {
                freq: 10.0,
                amplitude: 1.5,
                duty: 0.25,
                phase: 90.0,
            }
        );
        assert_eq!(
            signal.components[5],
            Component::Chirp {
                from: 1.0,
                to: 200.0,
                period: 10.0,
                amplitude: 0.5,
                log: true,
            }
        );
        let written = signal.to_string();
        assert_eq!(Signal::parse(&written), Ok(signal.clone()));
        // And the text a person wrote is the text that comes back, spacing
        // aside: one `; ` between components, keys in their own order.
        let spaced: String = EVERYTHING.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(written, spaced);
    }

    /// A number a person typed is written back as they typed it, not as the
    /// seventeen digits of its binary value.
    #[test]
    fn numbers_are_written_as_short_as_they_read_back() {
        let signal = Signal::parse("sine f=0.1 a=0.3 ph=0.7").expect("reads");
        assert_eq!(signal.to_string(), "sine f=0.1 a=0.3 ph=0.7");
        let tiny = Signal::parse("spikes rate=1e3 a=2E-1 w=1e-4").expect("reads");
        assert_eq!(tiny.to_string(), "spikes rate=1000 a=0.2 w=0.0001");
    }

    #[test]
    fn defaults_fill_what_was_left_out_and_are_left_out_again() {
        let signal = Signal::parse("sine f=50 a=1; square f=10 a=1").expect("reads");
        assert_eq!(
            signal.components,
            vec![
                Component::Sine {
                    freq: 50.0,
                    amplitude: 1.0,
                    phase: 0.0,
                },
                Component::Square {
                    freq: 10.0,
                    amplitude: 1.0,
                    duty: 0.5,
                    phase: 0.0,
                },
            ]
        );
        assert_eq!(signal.to_string(), "sine f=50 a=1; square f=10 a=1");
    }

    #[test]
    fn keys_come_in_any_order_and_blank_components_are_nothing() {
        let signal = Signal::parse("  ; a=2 f=5 sine ;").map(|s| s.components);
        // `a=2` is where a kind belongs, and is named as not being one.
        assert_eq!(
            signal,
            Err(SignalError::UnknownKind {
                word: "a=2".to_string()
            })
        );
        let signal = Signal::parse(" sine  a=2   f=5 ;; dc 1;").expect("reads");
        assert_eq!(signal.to_string(), "sine f=5 a=2; dc 1");
        assert_eq!(Signal::parse(""), Ok(Signal::default()));
        assert_eq!(Signal::default().to_string(), "");
    }

    /// Each refusal names what it refused — the kind, the key and the text
    /// — because "could not read the signal" beside a field of forty
    /// characters sends somebody to read all forty.
    #[test]
    fn each_refusal_names_what_it_refused() {
        let refused = |text: &str| Signal::parse(text).expect_err(text);
        assert_eq!(
            refused("sine f=50 a=1; sin f=5 a=1"),
            SignalError::UnknownKind {
                word: "sin".to_string()
            }
        );
        assert_eq!(
            refused("sine f=50 amp=1"),
            SignalError::UnknownKey {
                kind: "sine",
                key: "amp".to_string()
            }
        );
        assert_eq!(
            refused("sine f=50"),
            SignalError::Missing {
                kind: "sine",
                key: "a"
            }
        );
        assert_eq!(
            refused("white rms=1 rms=2"),
            SignalError::Twice {
                kind: "white",
                key: "rms"
            }
        );
        assert_eq!(
            refused("sine f=50Hz a=1"),
            SignalError::NotANumber {
                kind: "sine",
                key: "f",
                text: "50Hz".to_string()
            }
        );
        assert_eq!(
            refused("square f=10 a=1 duty=1.5"),
            SignalError::OutOfRange {
                kind: "square",
                key: "duty",
                value: 1.5,
                rule: Rule::Fraction
            }
        );
        assert_eq!(
            refused("sine f=50 a=1 fast"),
            SignalError::Stray {
                kind: "sine",
                word: "fast".to_string()
            }
        );
        // `log` is the chirp's word and no other kind's.
        assert_eq!(
            refused("sine f=50 a=1 log"),
            SignalError::Stray {
                kind: "sine",
                word: "log".to_string()
            }
        );
        assert_eq!(
            refused("dc"),
            SignalError::Missing {
                kind: "dc",
                key: "level"
            }
        );
        assert_eq!(
            refused("dc 1 2"),
            SignalError::Stray {
                kind: "dc",
                word: "2".to_string()
            }
        );
        assert_eq!(
            refused("dc level=1"),
            SignalError::NotANumber {
                kind: "dc",
                key: "level",
                text: "level=1".to_string()
            }
        );
        // A number that reads but cannot be rendered.
        assert_eq!(
            refused("dc inf"),
            SignalError::OutOfRange {
                kind: "dc",
                key: "level",
                value: f64::INFINITY,
                rule: Rule::Finite
            }
        );
        // A sweep by ratios cannot start at DC; a straight one can.
        assert_eq!(
            refused("chirp from=0 to=100 t=1 a=1 log"),
            SignalError::OutOfRange {
                kind: "chirp",
                key: "from",
                value: 0.0,
                rule: Rule::Positive
            }
        );
        assert!(Signal::parse("chirp from=0 to=100 t=1 a=1").is_ok());
    }

    /// The vocabulary the frontend names is the reader's own: every kind in
    /// `KINDS`, given every key `keys` lists for it, reads as a component of
    /// that kind, and a word that is no kind has no keys.
    #[test]
    fn the_public_vocabulary_is_what_the_reader_reads() {
        for kind in KINDS {
            let text = if kind == "dc" {
                "dc 1".to_string()
            } else {
                let pairs: Vec<String> = keys(kind).iter().map(|key| format!("{key}=1")).collect();
                format!("{kind} {}", pairs.join(" "))
            };
            let signal = Signal::parse(&text).expect("reads");
            assert_eq!(signal.components[0].kind(), kind, "{text}");
        }
        assert!(keys("sin").is_empty());
    }

    /// A signal built without the reader — a form, a slider — is held to
    /// the reader's rules by `check`.
    #[test]
    fn a_signal_built_by_hand_is_checked_by_the_same_rules() {
        let signal = Signal {
            components: vec![Component::Dc { level: 1.0 }, Component::White { rms: -0.1 }],
        };
        assert_eq!(
            signal.check(),
            Err(SignalError::OutOfRange {
                kind: "white",
                key: "rms",
                value: -0.1,
                rule: Rule::NotNegative
            })
        );
        let nan = Signal {
            components: vec![Component::Spikes {
                rate: 1.0,
                amplitude: 1.0,
                width: f64::NAN,
            }],
        };
        assert!(matches!(
            nan.check(),
            Err(SignalError::OutOfRange {
                kind: "spikes",
                key: "w",
                rule: Rule::Positive,
                ..
            })
        ));
    }

    /// The English, for the one reader who gets it: the kind, the key and
    /// what it needed.
    #[test]
    fn the_english_says_which_key_and_why() {
        let said = |text: &str| Signal::parse(text).expect_err(text).to_string();
        assert_eq!(said("sine f=50"), "sine needs a=, the amplitude");
        assert_eq!(
            said("sine f=50 amp=1"),
            "sine has no amp=: it takes f=, a= and ph="
        );
        assert_eq!(said("dc"), "dc needs its level after it, as in dc 1.2");
        assert_eq!(
            said("square f=10 a=1 duty=2"),
            "square: duty=2 has to be from 0 to 1"
        );
        assert!(said("sin f=1 a=1").starts_with("\"sin\" is not a kind of signal"));
    }

    /// The wire form is the same words: a component is tagged with the kind
    /// the text calls it by, so a stored design and a typed one agree.
    #[test]
    fn the_wire_form_is_tagged_with_the_text_forms_kind() {
        let signal = Signal::parse("sawtooth f=2 a=1; white rms=0.5").expect("reads");
        let wire = serde_json::to_value(&signal).expect("serialises");
        assert_eq!(wire["components"][0]["kind"], "sawtooth");
        assert_eq!(wire["components"][1]["kind"], "white");
        let back: Signal = serde_json::from_value(wire).expect("deserialises");
        assert_eq!(back, signal);
    }
}
