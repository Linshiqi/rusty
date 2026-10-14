//! The signal lab: what a run's signals played and what the firmware made of
//! them, kept on the firmware's own clock so the two can be laid side by side
//! (`docs/signals.md`, "The analysis").

use std::collections::BTreeMap;

use leptos::prelude::*;

use rusty_embed::dsp::{Design, Pass};
use rusty_embed::wave::WaveTarget;

use crate::lab::record::Of;
use crate::lab::sweep;

pub use rusty_embed::lab::{Source, Switched, record_conversion};

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
    /// What the last measurement of a recording found, tone by tone.
    pub recorded: RwSignal<Option<String>>,
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
            recorded: RwSignal::new(None),
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
