//! Embedded Rust domain logic for the rusty workbench.
//!
//! Split by the `backend` feature exactly as `rusty-core` is: [`model`],
//! [`protocol`], [`plant`] and [`setup`] are pure data and arithmetic and
//! compile to wasm so the Leptos frontend can `use` them; everything that
//! reads files or spawns processes is backend-only.
//!
//! The chip catalogue is *not* on the wasm side, deliberately: the lookups
//! need the TOML parser, and only the backend can see the user's and the
//! project's overlay files. The frontend gets the catalogue over IPC, which
//! is the one answer that includes those overlays.

pub mod model;
// The plant is arithmetic and no IO, like `protocol` — the frontend runs
// it on a timer, so it compiles to wasm with the model types.
pub mod plant;
pub mod protocol;
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
    GpioReport, Param, PinSource, PwmReport, SensorDef, Telemetry, analog_line,
    parse_display_report, parse_gpio_report, parse_param, parse_pin_source, parse_pwm_report,
    parse_sensor_def, parse_telemetry, sensor_line, set_param_line, to_vcd,
};

#[cfg(feature = "backend")]
pub mod catalog;
#[cfg(feature = "backend")]
pub mod config;
#[cfg(feature = "backend")]
pub mod device;
#[cfg(feature = "backend")]
mod error;
#[cfg(feature = "backend")]
pub mod firmware;
#[cfg(feature = "backend")]
pub mod flash;
#[cfg(feature = "backend")]
pub mod host_debug;
#[cfg(feature = "backend")]
pub mod install;
#[cfg(feature = "backend")]
pub mod memory;
#[cfg(feature = "backend")]
pub mod migrate;
#[cfg(feature = "backend")]
pub mod net;
#[cfg(feature = "backend")]
pub mod pins;
#[cfg(feature = "backend")]
pub mod process;
#[cfg(feature = "backend")]
pub mod project;
#[cfg(feature = "backend")]
pub mod scaffold;
#[cfg(feature = "backend")]
pub mod serial;
#[cfg(feature = "backend")]
pub mod simulate;
#[cfg(feature = "backend")]
pub mod svd;
#[cfg(feature = "backend")]
pub mod toolchain;
#[cfg(feature = "backend")]
pub mod tools;
#[cfg(feature = "backend")]
pub mod update;
#[cfg(feature = "backend")]
pub mod wizard;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
