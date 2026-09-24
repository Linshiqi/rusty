//! The signal lab: what a run's signals played and what the firmware made of
//! them, kept on the firmware's own clock so the two can be laid side by side
//! (`docs/signals.md`, "The analysis").

use std::collections::BTreeMap;

use leptos::prelude::*;

use rusty_embed::dsp::{Design, Pass};
use rusty_embed::wave::WaveTarget;

use crate::lab::record::Of;
use crate::lab::sweep;

/// Conversions kept per pin: a minute of a kilohertz loop. Past it the
/// oldest go, as a stream's should — what the converter just did is what
/// somebody is looking at.
pub const CONVERSIONS_KEPT: usize = 60_000;

/// Which of the lab's four instruments is in front.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LabView {
    /// The signal, what the converter took and what the firmware printed,
    /// on one clock.
    #[default]
    Time,
    /// Any of those, as a spectrum.
    Spectrum,
    /// A sine stepped across a band, each step measured going in and
    /// coming out.
    Response,
    /// A filter chosen and tuned against the signal without running
    /// anything, and its code.
    Design,
}

/// Where a signal is kept on the sheet: a generator's `signal`, or a sensor
/// reading's `signal.<reading>`.
#[derive(Clone, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Source {
    pub part: String,
    pub key: String,
}

impl Source {
    /// How the lab names it: `V1`, or `U2 · gz` for a sensor's reading.
    pub fn label(&self) -> String {
        match self.key.strip_prefix(rusty_embed::generator::SIGNAL_OF) {
            Some(reading) => format!("{} · {reading}", self.part),
            None => self.part.clone(),
        }
    }
}

/// The emulator's account of one table being switched, from
/// `[rusty:wave@<us>]`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Switched {
    /// When, on the emulator's virtual clock.
    pub at_us: u64,
    /// Where sample 0 of what is playing sits on that clock; `None` once
    /// it stopped.
    pub start_us: Option<u64>,
}

#[derive(Clone, Copy)]
pub struct Lab {
    /// Every conversion the emulator reported on each pin this run,
    /// `(µs, counts)`, oldest first and capped at [`CONVERSIONS_KEPT`]. The
    /// emulator reports a conversion when its value changed, which with a
    /// signal playing is nearly every one; a still pin is a single entry.
    pub conversions: RwSignal<BTreeMap<u8, Vec<(u64, u16)>>>,
    /// The last switch of each table this run.
    pub switched: RwSignal<BTreeMap<WaveTarget, Switched>>,
    /// What each source plays in this run where the Signals tab changed
    /// it; absent means what the sheet says. Per run: the sheet keeps its
    /// own until somebody changes it there.
    pub playing: RwSignal<BTreeMap<Source, String>>,
    pub view: RwSignal<LabView>,
    /// The source being studied.
    pub source: RwSignal<Option<Source>>,
    /// What the next sweep asks for.
    pub sweep: RwSignal<sweep::Plan>,
    /// Which record a sweep reads going in — what is played, unless a
    /// converter or a channel is picked — and which channel coming out.
    pub sweep_input: RwSignal<Option<Of>>,
    pub sweep_output: RwSignal<Option<String>>,
    /// What the last sweep measured, kept after its run for the chart.
    pub swept: RwSignal<Vec<sweep::Point>>,
    /// The sweep running now, by number: Stop and a newer sweep replace it,
    /// and a step of the old one that wakes finds itself stale.
    pub sweeping: RwSignal<Option<u64>>,
    pub sweep_number: RwSignal<u64>,
    /// How far it has got: the step, how many there are, its frequency.
    pub sweep_step: RwSignal<Option<(usize, usize, f64)>>,
    /// Why the last sweep stopped before its end, when it did.
    pub sweep_missed: RwSignal<Option<String>>,
    /// The filter being designed.
    pub design: RwSignal<Design>,
    /// The rate it is designed for, when somebody stated one; otherwise
    /// the rate the firmware's own records keep.
    pub design_rate: RwSignal<Option<f64>>,
    /// The name its code is exported under.
    pub code_name: RwSignal<String>,
    /// A signal for each axis of a sensor the firmware declared on its
    /// console (`[rusty:sensor]`), by the sensor's name; an empty text
    /// leaves the axis where its slider is. Fed at the host's pace.
    pub console: RwSignal<BTreeMap<String, Vec<String>>>,
    /// The feed running now: a newer one, or the end of the run, stops it.
    pub console_gen: RwSignal<u64>,
}

impl Lab {
    pub fn fresh() -> Self {
        Self {
            conversions: RwSignal::new(BTreeMap::new()),
            switched: RwSignal::new(BTreeMap::new()),
            playing: RwSignal::new(BTreeMap::new()),
            view: RwSignal::new(LabView::default()),
            source: RwSignal::new(None),
            sweep: RwSignal::new(sweep::Plan::default()),
            sweep_input: RwSignal::new(None),
            sweep_output: RwSignal::new(None),
            swept: RwSignal::new(Vec::new()),
            sweeping: RwSignal::new(None),
            sweep_number: RwSignal::new(0),
            sweep_step: RwSignal::new(None),
            sweep_missed: RwSignal::new(None),
            // A second-order low-pass at 10 Hz: what the first preset's
            // slow reading wants, and a curve with something to see.
            design: RwSignal::new(Design::Butterworth {
                pass: Pass::Low,
                order: 2,
                cutoff: 10.0,
            }),
            design_rate: RwSignal::new(None),
            code_name: RwSignal::new("LowPass".to_string()),
            console: RwSignal::new(BTreeMap::new()),
            console_gen: RwSignal::new(0),
        }
    }

    /// Forget the last run's account, keeping what is being looked at.
    pub fn clear_capture(&self) {
        self.conversions.set(BTreeMap::new());
        self.switched.set(BTreeMap::new());
        self.playing.set(BTreeMap::new());
        // The console's channels belong to the firmware that declared them.
        self.console.set(BTreeMap::new());
        self.console_gen.update(|generation| *generation += 1);
    }
}

/// One conversion onto a pin's record, the oldest dropped past the cap.
pub fn record_conversion(record: &mut Vec<(u64, u16)>, at_us: u64, counts: u16) {
    record.push((at_us, counts));
    if record.len() > CONVERSIONS_KEPT {
        let over = record.len() - CONVERSIONS_KEPT;
        record.drain(..over);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_keeps_the_newest_conversions() {
        let mut record = Vec::new();
        for n in 0..(CONVERSIONS_KEPT as u64 + 10) {
            record_conversion(&mut record, n, 1);
        }
        assert_eq!(record.len(), CONVERSIONS_KEPT);
        assert_eq!(record.first().map(|(at, _)| *at), Some(10));
    }

    #[test]
    fn a_source_is_named_by_its_part_and_reading() {
        let generator = Source {
            part: "V1".into(),
            key: "signal".into(),
        };
        let reading = Source {
            part: "U2".into(),
            key: "signal.gz".into(),
        };
        assert_eq!(generator.label(), "V1");
        assert_eq!(reading.label(), "U2 · gz");
    }
}
