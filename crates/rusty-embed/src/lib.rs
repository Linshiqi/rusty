//! Embedded Rust domain logic for the rusty workbench.
//!
//! Split by the `backend` feature exactly as `rusty-core` is: [`model`],
//! [`protocol`], [`plant`], [`setup`], [`signal`] and [`dsp`] are pure data
//! and arithmetic and compile to wasm so the Leptos frontend can `use` them;
//! everything that reads files or spawns processes is backend-only.
//!
//! The chip catalogue is *not* on the wasm side, deliberately: the lookups
//! need the TOML parser, and only the backend can see the user's and the
//! project's overlay files. The frontend gets the catalogue over IPC, which
//! is the one answer that includes those overlays.

pub mod model;
// The plant is arithmetic and no IO, like `protocol` — the frontend runs
// it on a timer, so it compiles to wasm with the model types.
pub mod nets;
// The sheet averaged over a PWM period — the rules and the solver read at
// each moment of it. Wasm-safe with both: the board view is its reader.
pub mod period;
pub mod plant;
pub mod protocol;
// A sensor's registers from the readings a slider sets — the backend encodes
// a run's first readings and every later move with it, and the frontend
// reads its channels to draw the sliders.
pub mod sensor;
// What a character LCD was told to show. Beside `screen` rather than in it
// because the bytes crossing the bus for an HD44780 are an expander's port
// states, not display data — the picture is recovered from their edges.
pub mod lcd;
// What a monochrome OLED was told to draw, from the bytes the emulator
// reports crossing the bus. Wasm-safe with the rest: the frontend reads the
// stream as it passes and draws the screen from it.
pub mod screen;
// What a signal generator produces — tones, sweeps, noise, spikes and steps
// as one line of text — rendered into samples. Wasm-safe: the frontend draws
// what the backend will play into the emulator's converter.
pub mod signal;
// Spectra, single tones and the filters a firmware runs, designed and held to
// their closed forms. Wasm-safe with `signal`, which it measures.
pub mod dsp;
// What a fresh machine is missing, derived from the toolchain report. Pure,
// and unconditional so the setup screen can reason about a report it already
// holds rather than asking the backend what it just told it.
pub mod setup;

// Chip lookups need the TOML parser, so unlike `model` they are backend-only.
// The frontend gets the catalogue over IPC instead — which is correct anyway,
// since only the backend can see the user's and the project's overlay files.
#[cfg(feature = "backend")]
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

#[cfg(feature = "backend")]
pub mod catalog;
pub mod circuit;
#[cfg(feature = "backend")]
pub mod config;
#[cfg(feature = "backend")]
pub mod device;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
mod esp_env;
#[cfg(feature = "backend")]
pub mod firmware;
#[cfg(feature = "backend")]
pub mod flash;
#[cfg(feature = "backend")]
pub mod host_debug;
#[cfg(feature = "backend")]
pub mod install;
#[cfg(feature = "backend")]
mod layers;
pub mod live;
#[cfg(feature = "backend")]
pub mod memory;
#[cfg(feature = "backend")]
pub mod migrate;
#[cfg(feature = "backend")]
pub mod net;
/// Reading a part's declaration — the file format behind [`sensor::Spec`].
#[cfg(feature = "backend")]
pub mod partfile;
#[cfg(feature = "backend")]
pub mod pins;
/// The playground: a project rusty keeps per chip for trying things.
#[cfg(feature = "backend")]
pub mod playground;
#[cfg(feature = "backend")]
pub mod process;
#[cfg(feature = "backend")]
pub mod project;
#[cfg(feature = "backend")]
pub mod scaffold;
#[cfg(feature = "backend")]
pub mod schematic;
#[cfg(feature = "backend")]
pub mod serial;
#[cfg(feature = "backend")]
pub mod simulate;
pub mod solve;
#[cfg(feature = "backend")]
pub mod svd;
#[cfg(feature = "backend")]
pub mod toolchain;
#[cfg(feature = "backend")]
pub mod tools;
mod union_find;
#[cfg(feature = "backend")]
pub mod wizard;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
