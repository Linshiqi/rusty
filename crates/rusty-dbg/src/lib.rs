//! Debugging: breakpoints, stepping, and what the registers say.
//!
//! Split like every other crate here — the protocol and the model compile
//! anywhere, the process that speaks to gdb sits behind `backend`. The
//! frontend uses the model types directly, so the wire contract cannot
//! drift from what the debugger actually reports.

#![cfg_attr(not(feature = "backend"), no_std)]

extern crate alloc;

pub mod mi;
pub mod model;

pub use model::*;

#[cfg(feature = "backend")]
pub mod session;
#[cfg(feature = "backend")]
pub use session::{Debugger, Events, Launch, Target};
