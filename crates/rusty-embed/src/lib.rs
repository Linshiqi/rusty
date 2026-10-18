//! Embedded Rust domain logic for the rusty workbench.
//!
//! [`model`], [`protocol`], [`plant`], [`setup`], [`signal`] and [`dsp`] are
//! pure data and arithmetic, tested as such; what reads files or spawns
//! processes is in the modules beside them.
//!
//! The chip catalogue is TOML in three layers — built in, the user's, the
//! project's — read through one loader (`catalog`), so every answer about a
//! part includes the overlays.

#[cfg(test)]
pub(crate) mod fixtures;
pub mod model;
pub mod nets;
// The sheet averaged over a PWM period — the rules and the solver read at
// each moment of it. Pure, like both: the board view is its reader.
pub mod period;
// The plant is arithmetic and no IO, like `protocol` — the window runs it
// on a timer.
pub mod plant;
pub mod protocol;
// A sensor's registers from the readings a slider sets — a run encodes its
// first readings and every later move with it, and the window reads its
// channels to draw the sliders.
pub mod sensor;
// What a character LCD was told to show. Beside `screen` rather than in it
// because the bytes crossing the bus for an HD44780 are an expander's port
// states, not display data — the picture is recovered from their edges.
pub mod lcd;
// What a monochrome OLED was told to draw, from the bytes the emulator
// reports crossing the bus. Pure, with the rest: the window reads the
// stream as it passes and draws the screen from it.
pub mod screen;
// What a signal generator produces — tones, sweeps, noise, spikes and steps
// as one line of text — rendered into samples. Pure: the window draws what a
// run will play into the emulator's converter.
pub mod signal;
// Spectra, single tones and the filters a firmware runs, designed and held to
// their closed forms. Pure, like `signal`, which it measures.
pub mod dsp;
// Vectors, quaternions, Euler angles and frames, and the math toolbox's sheet
// language over them. Pure: the panel evaluates the sheet as it is typed.
pub mod spatial;
// What a program asked to be drawn — the `[rusty:draw]` lines `rusty-draw`
// writes — and the angles between what it drew. Pure, beside `protocol`:
// the window reads the stream as it passes.
pub mod draw;
// What a fresh machine is missing, derived from the toolchain report. Pure,
// so the setup screen can reason about a report it already holds rather than
// probing the machine again.
pub mod setup;
// What a run is doing and how it ended, read off the lines it prints — the
// title bar's activity view. Pure: lines in, a verdict out.
pub mod activity;
// Where a build's errors are, from what the compiler printed. Pure.
pub mod buildlog;
// The board sheet drawn and edited — the parts' art, the geometry, the
// layout. Pure: the window draws the board from it.
pub mod board;
// Named channels over time and the range each is drawn against: the Plot
// panel's arithmetic. Pure.
pub mod plot;
// Pin waveforms: the trace a run's gpio reports make, and the arithmetic
// of drawing it. Pure.
pub mod waves;
// Space as a view draws it: a camera, a projection, shapes sorted back to
// front, and what a `rusty-draw` scene puts in it. Pure.
pub mod scene;
// The signal lab's arithmetic: what a source plays, the records the
// instruments read, a sweep. Pure.
pub mod lab;

// Chip lookups read the catalogue's TOML, the user's and the project's
// overlay files included, so unlike `model` they do IO.
pub mod chip;

pub use model::*;
pub use plant::{Plant, PlantConfig};
// The whole of the serial protocol's public surface, so a caller can reach
// every line shape through one path. Half of it was here and the other half
// only under `protocol::`, which is how the same file ended up importing the
// two halves two ways.
pub use protocol::{
    AdcReport, Duty, GpioReport, I2cReport, Param, PinSource, PwmReport, RmtReport, SensorDef,
    SpiReport, Telemetry, analog_line, parse_adc_report, parse_display_report, parse_gpio_report,
    parse_i2c_report, parse_param, parse_pin_source, parse_pwm_report, parse_rmt_report,
    parse_sensor_def, parse_spi_report, parse_telemetry, sensor_line, set_param_line,
    strip_colours, to_vcd,
};

// The build systems beside Cargo: PlatformIO and CMake, recognised, built,
// flashed and searched for images.
pub mod buildsys;
// Which package and target of a cargo workspace a file is compiled in, and
// the arguments that select them — what a test's lens runs.
pub mod cargo_target;
pub mod catalog;
pub mod circuit;
pub mod config;
pub mod device;
mod error;
mod esp_env;
pub mod firmware;
pub mod flash;
// A signal generator on the sheet and the table it plays at each converter
// it reaches, stepped through the sheet's circuit. Pure, beside `circuit`
// and `live`, whose arithmetic it is.
pub mod generator;
pub mod host_debug;
pub mod image;
pub mod install;
mod layers;
pub mod live;
pub mod memory;
pub mod migrate;
pub mod net;
/// Reading a part's declaration — the file format behind [`sensor::Spec`].
pub mod partfile;
pub mod pins;
/// The playground: a project rusty keeps per chip for trying things.
pub mod playground;
pub mod process;
pub mod project;
pub mod scaffold;
pub mod schematic;
pub mod serial;
pub mod simulate;
pub mod solve;
pub mod svd;
pub mod toolchain;
pub mod tools;
mod union_find;
/// rusty updating itself: the release feed and the signature check.
pub mod update;
// The lines that put a rendered signal on the emulator and its account of
// playing it. Pure, with the protocol: the window reads the account to
// line the signal it drew up with the conversions the emulator reports.
pub mod wave;
pub mod wizard;

pub use error::{Error, Result};
