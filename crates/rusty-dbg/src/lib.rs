//! Debugging: breakpoints, stepping, and what the registers say.
//!
//! Split like every other crate here — the protocol and the model compile
//! anywhere, the process that speaks to gdb sits behind `backend`. The
//! frontend uses the model types directly, so the wire contract cannot
//! drift from what the debugger actually reports.

pub mod mi;
pub mod model;
pub mod pretty;

pub use model::*;

#[cfg(feature = "backend")]
pub mod any;
#[cfg(feature = "backend")]
pub mod dap;
#[cfg(feature = "backend")]
pub mod session;
#[cfg(feature = "backend")]
pub use any::AnySession;
#[cfg(feature = "backend")]
pub use dap::{DapLaunch, DapSession};
#[cfg(feature = "backend")]
pub use session::{Debugger, Error, Events, Launch, Target};
