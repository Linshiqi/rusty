//! Debugging: breakpoints, stepping, and what the registers say.
//!
//! The model is the session as data — breakpoints, frames, variables — and
//! both debuggers fold into it, so the window draws one shape whichever
//! protocol is behind it.

pub mod mi;
pub mod model;
pub mod placement;
pub mod pretty;

pub use model::*;

pub mod any;
pub mod dap;
pub mod session;
mod spawn;
pub use any::AnySession;
pub use dap::{Board, DapLaunch, DapSession};
pub use session::{Debugger, Error, Events, Launch, Target};
