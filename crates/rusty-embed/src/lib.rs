//! Embedded Rust domain logic for the rusty workbench.
//!
//! Split by the `backend` feature exactly as `rusty-core` is: [`model`] and
//! [`chip`] are pure data and compile to wasm so the Leptos frontend can `use`
//! them; everything that reads files or spawns processes is backend-only.
//!
//! The chip catalogue is deliberately on the wasm side — the frontend needs to
//! render the part list in the project wizard before any backend call happens.

pub mod model;
pub mod protocol;

// Chip lookups need the TOML parser, so unlike `model` they are backend-only.
// The frontend gets the catalogue over IPC instead — which is correct anyway,
// since only the backend can see the user's and the project's overlay files.
#[cfg(feature = "backend")]
pub mod chip;

pub use model::*;
pub use protocol::{GpioReport, parse_display_report, parse_gpio_report, to_vcd};

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
pub mod memory;
#[cfg(feature = "backend")]
pub mod migrate;
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
pub mod update;
#[cfg(feature = "backend")]
pub mod wizard;

#[cfg(feature = "backend")]
pub use error::{Error, Result};
