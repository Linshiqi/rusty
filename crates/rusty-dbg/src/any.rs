//! One handle over two debuggers.
//!
//! The panel asks the same eight questions however the session was started —
//! step, resume, place a breakpoint, read the frame — so the backend holds
//! one type and the choice of protocol stops at this file. gdb's machine
//! interface serves every chip target and every host whose debug information
//! is DWARF; the Debug Adapter Protocol serves the rest, which today means
//! Windows, where Rust's default target emits a PDB that gdb cannot read.
//!
//! Deliberately an enum rather than a trait object: there are two of these
//! and there is no third coming, the methods are a closed set, and a `dyn`
//! boundary here would buy nothing but a vtable and a lifetime to explain.

use crate::dap::DapSession;
use crate::model::DebugState;
use crate::session::{Debugger, Result};

/// A live debug session, whichever protocol it speaks.
pub enum AnySession {
    /// gdb, through its machine interface.
    Gdb(Debugger),
    /// A debug adapter, through DAP.
    Dap(DapSession),
}

impl AnySession {
    /// What the caller has to know to word an error: which debugger is on
    /// the other end.
    pub fn debugger_name(&self) -> &'static str {
        match self {
            AnySession::Gdb(_) => "gdb",
            AnySession::Dap(_) => "the debug adapter",
        }
    }

    pub fn state(&self) -> DebugState {
        match self {
            AnySession::Gdb(session) => session.state(),
            AnySession::Dap(session) => session.state(),
        }
    }

    pub fn add_breakpoint(&self, file: &str, line: u32) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.add_breakpoint(file, line),
            AnySession::Dap(session) => session.add_breakpoint(file, line),
        }
    }

    pub fn remove_breakpoint(&self, number: u32) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.remove_breakpoint(number),
            AnySession::Dap(session) => session.remove_breakpoint(number),
        }
    }

    pub fn resume(&self) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.resume(),
            AnySession::Dap(session) => session.resume(),
        }
    }

    pub fn pause(&self) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.pause(),
            AnySession::Dap(session) => session.pause(),
        }
    }

    pub fn step_over(&self) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.step_over(),
            AnySession::Dap(session) => session.step_over(),
        }
    }

    pub fn step_into(&self) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.step_into(),
            AnySession::Dap(session) => session.step_into(),
        }
    }

    pub fn step_out(&self) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.step_out(),
            AnySession::Dap(session) => session.step_out(),
        }
    }

    pub fn refresh(&self, frame: u32) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.refresh(frame),
            AnySession::Dap(session) => session.refresh(frame),
        }
    }

    pub fn read_memory(&self, address: u64, bytes: u32) -> Result<()> {
        match self {
            AnySession::Gdb(session) => session.read_memory(address, bytes),
            AnySession::Dap(session) => session.read_memory(address, bytes),
        }
    }

    pub fn stop(&self) {
        match self {
            AnySession::Gdb(session) => session.stop(),
            AnySession::Dap(session) => session.stop(),
        }
    }
}
